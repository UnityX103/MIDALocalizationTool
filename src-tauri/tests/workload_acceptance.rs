#![allow(dead_code)]

#[path = "../src/media.rs"]
mod media;
#[path = "../src/package.rs"]
mod package;
#[path = "../src/workload.rs"]
mod workload;
#[path = "../src/word_count.rs"]
mod word_count;
#[path = "../src/workspace.rs"]
mod workspace;

use serde_json::{json, Value};
use std::{fs::File, path::{Path, PathBuf}};

fn packages(directory: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            paths.extend(packages(&path));
        } else if path.extension().and_then(|value| value.to_str()) == Some("zip")
            && !path.file_name().unwrap().to_string_lossy().starts_with("invalid-") {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

fn read(root: &Path, path: &Path) -> Value {
    let mut stage = media::new_stage(root).unwrap();
    let result = package::read_package(path, &mut stage, &mut |_, _| Ok(()));
    media::discard(&stage).unwrap();
    result.unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn project_counting_uses_shared_examples_for_acceptance() {
    let cases: Value = serde_json::from_str(include_str!("../../fixtures/workload-counting-cases.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let source = &case["text"];
        let expected = case.get("units").cloned().unwrap_or_else(|| json!(case["words"].as_u64().unwrap() * workload::COUNTING_SCALE));
        let character_counts = case.get("characterCounts").cloned().unwrap_or_else(|| json!({"lao":0,"khmer":0,"myanmar":0}));
        let task = json!({"partName":"PartA", "language":"en", "entries":[{
            "currentSource":source, "translation":"Translation", "review":{"state":"confirmed"}
        }]});
        let mut document = json!({
            "projectId":"project", "packageId":"delivery", "fileVersion":{"lineageId":"lineage"},
            "delivery":{"version":1,"id":"delivery","ledgerId":"ledger","revision":1,"previousId":null},
            "workload":{"version":1,"countingRule":workload::COUNTING_RULE,"countingScale":workload::COUNTING_SCALE,
                "progressCharacterCounts":character_counts,"totalCharacterCounts":character_counts,
                "scope":{"projectId":"project","lineageId":"lineage","language":"en","partNames":["PartA"]},
                "records":[{"id":"record","partName":"PartA","language":"en","key":"key","sourceText":source,
                    "translation":"Translation","sourcePackageId":"source","sourceRevision":1,"taskVersion":1,
                    "kind":"translation","chars":expected,"characterCounts":character_counts,"confirmedAt":"2026-10-15T00:00:00Z"}],
                "acknowledgedRecordIds":[],"newRecordIds":["record"],"sourceUpdateRecordIds":["record"],
                "progressChars":expected,"totalChars":expected,"cumulativeChars":expected,"handoverChars":expected,
                "previousDeliveryDeltaChars":expected,"sinceSourceUpdateChars":expected}
        });
        workload::validate(&document, &[&task]).unwrap_or_else(|error| panic!("{source}: {error}"));
        document["workload"]["records"][0]["chars"] = json!(expected.as_u64().unwrap() + 1);
        assert!(workload::validate(&document, &[&task]).is_err());
    }
}

#[test]
fn project_counting_reestimates_legacy_records_without_changing_identity() {
    let original = json!({"id":"record","sourceText":"你好，Hello 123。","chars":2});
    let delivery = json!({"id":"old-delivery","workload":{"countingRule":"han-v1","cumulativeChars":2}});
    let mut ledger = json!({"id":"ledger","records":[original],"deliveries":[delivery]});
    workload::migrate_ledger(&mut ledger).unwrap();
    assert_eq!(ledger["records"][0]["chars"], 6 * workload::COUNTING_SCALE);
    assert_eq!(ledger["records"][0]["id"], "record");
    assert_eq!(ledger["records"][0]["sourceText"], "你好，Hello 123。");
    assert_eq!(ledger["deliveries"][0], delivery);
    assert_eq!(ledger["countingRule"], workload::COUNTING_RULE);
    workload::migrate_ledger(&mut ledger).unwrap();
    assert_eq!(ledger["records"][0]["chars"], 6 * workload::COUNTING_SCALE);
}

#[test]
fn project_counting_migrates_before_incremental_save_and_keeps_failed_saves_atomic() {
    let temporary = std::env::temp_dir().canonicalize().unwrap();
    let root = tempfile::tempdir_in(temporary).unwrap();
    let entry = |key: &str, source: &str| json!({"key":key,"currentSource":source,"translation":"",
        "translationAtExport":"","localizationSourceAtExport":"","speakerKey":"","speakerChineseName":"",
        "issuesAtExport":[],"review":{"state":"pending"}});
    let record = |id: &str, key: &str, source: &str, chars: u64| json!({"id":id,"partName":"PartA","language":"en",
        "key":key,"sourceText":source,"translation":"Translation","sourcePackageId":"source","sourceRevision":1,
        "taskVersion":1,"kind":"translation","chars":chars,"characterCounts":{"lao":0,"khmer":0,"myanmar":0},"confirmedAt":"2026-10-15T00:00:00Z"});
    let original = record("old-record", "old-key", "你好，Hello 123。", 2);
    let mut confirmed = entry("new-key", "再见，World 42。");
    let task = json!({"partName":"PartA","language":"en","taskVersion":1,
        "entries":[entry("old-key","你好，Hello 123。"),confirmed]});
    let snapshot = json!({"format":"mida-localization-workspace","version":1,"tasks":[task],
        "currentProjectId":"project","previews":{},"drafts":[],"state":{},
        "workLedger":{"version":1,"id":"ledger","records":[original],"deliveries":[],
            "acknowledgedIds":[],"receipts":[],"sources":[]}});
    workspace::save(root.path(), snapshot, 0, None, None).unwrap();
    confirmed["translation"] = json!("Goodbye");
    confirmed["review"] = json!({"state":"confirmed"});
    let delta = json!({"id":"ledger","countingRule":workload::COUNTING_RULE,
        "records":[record("new-record","new-key","再见，World 42。",6 * workload::COUNTING_SCALE)]});
    let identity = json!({"partName":"PartA","language":"en"});
    workspace::save_entry(root.path(), identity.clone(), 0, "project", confirmed.clone(), json!({}), delta, 1).unwrap();
    let restored = workspace::read(root.path(), "workspace", "current").unwrap();
    let ledger = &restored["snapshot"]["workLedger"];
    assert_eq!(ledger["countingRule"], workload::COUNTING_RULE);
    assert_eq!(ledger["records"][0]["id"], "old-record");
    assert_eq!(ledger["records"][0]["chars"], 6 * workload::COUNTING_SCALE);
    assert_eq!(ledger["records"][1]["chars"], 6 * workload::COUNTING_SCALE);
    let bad = json!({"id":"ledger","countingRule":workload::COUNTING_RULE,
        "records":[record("wrong-count","new-key","再见，World 42。",2)]});
    assert!(workspace::save_entry(root.path(), identity, 0, "project", confirmed, json!({}), bad, 2).is_err());
    assert_eq!(workspace::read(root.path(), "workspace", "current").unwrap(), restored);
}

#[test]
fn project_counting_migrates_previous_gmx_draft_with_fractional_counts() {
    let mut ledger = json!({"id":"ledger","countingRule":"mida-gmx-fullwidth-v1",
        "records":[{"id":"record","sourceText":"ก l'ā","chars":1}],"deliveries":[]});
    workload::migrate_ledger(&mut ledger).unwrap();
    assert_eq!(ledger["countingScale"], 462);
    assert_eq!(ledger["records"][0]["chars"], 1001);
    assert_eq!(ledger["records"][0]["id"], "record");
    workload::validate_current_record_count(&ledger["records"][0]).unwrap();
    ledger["records"][0]["characterCounts"]["lao"] = json!(1);
    assert!(workload::validate_current_record_count(&ledger["records"][0]).is_err());
}

#[test]
#[ignore = "requires UNICODE_NORMALIZATION_TEST pointing to Unicode 4.1 NormalizationTest.txt"]
fn project_counting_matches_official_unicode_normalization_corpus() {
    let path = std::env::var("UNICODE_NORMALIZATION_TEST").unwrap();
    let source = std::fs::read_to_string(path).unwrap();
    let mut checked = 0;
    for line in source.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() || line.starts_with('@') { continue; }
        let forms: Vec<String> = line.split(';').take(3).map(|column| column.split_whitespace()
            .map(|point| char::from_u32(u32::from_str_radix(point,16).unwrap()).unwrap()).collect()).collect();
        let expected = word_count::measure(&forms[2]);
        for value in forms {
            let actual = word_count::measure(&value);
            assert_eq!(actual.units, expected.units, "{line}");
            assert_eq!(actual.characters, expected.characters, "{line}");
        }
        checked += 1;
    }
    assert_eq!(checked,17507);
}

// Opt-in only: the caller supplies isolated ZIPs from the real Unity round trip.
#[test]
#[ignore = "requires WORKLOAD_ACCEPTANCE_DIR containing real acceptance ZIPs"]
fn real_packages_preserve_manifest_and_workspace() {
    let directory = std::env::var("WORKLOAD_ACCEPTANCE_DIR").expect("WORKLOAD_ACCEPTANCE_DIR");
    let temporary = std::env::temp_dir().canonicalize().unwrap();
    let root = tempfile::tempdir_in(&temporary).unwrap();
    let mut checked = 0;
    for path in packages(Path::new(&directory)) {
        let imported = read(root.path(), &path);
        let mut tasks = Vec::new();
        let mut document = Value::Null;
        for text in imported["assets"].as_object().unwrap().values() {
            let data: Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
            tasks.extend(data["tasks"].as_array().unwrap().iter().cloned());
            document = data;
        }
        document["tasks"] = json!(tasks);
        for field in workload::FIELDS {
            if let Some(value) = imported["manifest"].get(field) {
                document[field] = value.clone();
            }
        }
        let exported_path = root.path().join("roundtrip.zip");
        package::build_package(&document, &mut File::create(&exported_path).unwrap()).unwrap();
        let exported = read(root.path(), &exported_path);
        for field in workload::FIELDS {
            assert_eq!(exported["manifest"].get(field), imported["manifest"].get(field), "{field}");
        }
        let store = tempfile::tempdir_in(&temporary).unwrap();
        let snapshot = json!({
            "format": "mida-localization-workspace", "version": 1,
            "tasks": tasks, "currentProjectId": document["projectId"],
            "currentManifest": imported["manifest"], "previews": {},
            "workLedger": { "records": document["workload"]["records"], "deliveries": [document["delivery"]] }
        });
        workspace::save(store.path(), snapshot.clone(), 0, Some("acceptance".into()), None).unwrap();
        assert_eq!(workspace::read(store.path(), "workspace", "current").unwrap()["snapshot"], snapshot);
        assert!(workspace::save(store.path(), snapshot.clone(), 0, None, None).is_err());
        assert_eq!(workspace::read(store.path(), "workspace", "current").unwrap()["snapshot"], snapshot);
        if document.get("workload").is_some() {
            let mut corrupted = document.clone();
            corrupted["workload"]["cumulativeChars"] = json!(document["workload"]["cumulativeChars"].as_u64().unwrap() + 1);
            assert!(package::build_package(&corrupted, &mut File::create(root.path().join("rejected.zip")).unwrap()).is_err());
        }
        checked += 1;
        println!("accepted and preserved {}", path.display());
    }
    assert!(checked >= 2, "expected real source and delivery ZIPs");
}

#[test]
#[ignore = "requires real-workspace.json exported from the acceptance UI"]
fn complete_workspace_survives_task_save() {
    let directory = std::env::var("WORKLOAD_ACCEPTANCE_DIR").expect("WORKLOAD_ACCEPTANCE_DIR");
    let snapshot: Value = serde_json::from_slice(&std::fs::read(Path::new(&directory).join("real-workspace.json")).unwrap()).unwrap();
    assert!(snapshot["workLedger"]["id"].is_string());
    assert!(snapshot["workLedger"]["deliveries"].as_array().unwrap().len() >= 5);
    let temporary = std::env::temp_dir().canonicalize().unwrap();
    let root = tempfile::tempdir_in(temporary).unwrap();
    workspace::save(root.path(), snapshot.clone(), 0, None, None).unwrap();
    let task = snapshot["tasks"][0].clone();
    let keys = task["entries"].as_array().unwrap().iter()
        .filter(|entry| entry["review"]["state"] == "confirmed")
        .map(|entry| entry["key"].as_str().unwrap().to_owned()).collect();
    let view = json!({
        "state": snapshot["state"], "workLedger": snapshot["workLedger"],
        "exportSequence": snapshot["exportSequence"], "previewPlayer": snapshot["previewPlayer"]
    });
    workspace::save_task(root.path(), task, 0, snapshot["currentProjectId"].as_str().unwrap(), view, keys, 1).unwrap();
    let restored = workspace::read(root.path(), "workspace", "current").unwrap();
    assert_eq!(restored["revision"], 2);
    assert_eq!(restored["snapshot"], snapshot);
    std::fs::write(Path::new(&directory).join("native-restored-workspace.json"),
        serde_json::to_vec_pretty(&restored["snapshot"]).unwrap()).unwrap();
}
