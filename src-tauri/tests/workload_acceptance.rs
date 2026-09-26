#![allow(dead_code)]

#[path = "../src/media.rs"]
mod media;
#[path = "../src/package.rs"]
mod package;
#[path = "../src/workload.rs"]
mod workload;
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
