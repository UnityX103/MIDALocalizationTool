use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

const MAX_JSON_BYTES: u64 = 128 * 1024 * 1024;
const HISTORY_LIMIT: usize = 10;
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn read(root: &Path, store: &str, key: &str) -> Result<Value, String> {
    match (store, key) {
        ("workspace", "current" | "history") => {}
        ("backups" | "ledgers" | "ledger-index", identifier) if valid_id(identifier) => {}
        _ => return Err("不支持的自动保存存储或键名".into()),
    }
    let directory = root.join("workspace");
    if store == "ledgers" || store == "ledger-index" {
        let Some(mut archive) = read_archive(&directory.join("ledgers"), key)? else { return Ok(Value::Null); };
        if store == "ledger-index" { archive.as_object_mut().ok_or("归档格式无效")?.remove("packets"); return Ok(archive); }
        for packet in archive["packets"].as_array_mut().ok_or("归档交付无效")? {
            let mut wrapper = json!({"format":"mida-localization-workspace","version":1,"taskRefs":packet["taskRefs"],"currentProjectId":packet["projectId"]});
            expand_snapshot(&directory, &mut wrapper)?;
            packet["tasks"] = wrapper["tasks"].take();packet.as_object_mut().ok_or("归档交付无效")?.remove("taskRefs");
        }
        return Ok(archive);
    }
    if store == "workspace" && key == "current" {
        let state = load_state(root, &directory)?;
        let mut record = state["current"].clone();
        if !record.is_null() { expand_snapshot(&directory, &mut record["snapshot"])?; }
        return Ok(record);
    }
    // Recovery must remain available when the current index cannot be parsed.
    if store == "workspace" { return backup_history(&directory); }
    let Some(mut backup) = read_json(&backup_path(&directory.join("backups"), key))? else { return Ok(Value::Null); };
    expand_snapshot(&directory, &mut backup["snapshot"])?;
    validate_snapshot(&backup["snapshot"])?;
    unsigned(&backup, "savedAt")?;
    Ok(backup)
}

fn backup_history(directory: &Path) -> Result<Value, String> {
    let mut summaries = Vec::new();
    let backups = directory.join("backups");
    if !directory_exists(&backups)? { return Ok(json!([])); }
    for entry in fs::read_dir(&backups).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) else { continue; };
        let record = read_json(&entry.path());
        let (saved_at, count, error) = match record {
            Ok(Some(mut record)) => {
                let saved_at = record["savedAt"].as_u64().unwrap_or(0);
                let result = expand_snapshot(directory, &mut record["snapshot"]).and_then(|_| validate_snapshot(&record["snapshot"]));
                (saved_at, record["snapshot"]["tasks"].as_array().map_or(0, Vec::len), result.err())
            },
            Ok(None) => (0, 0, Some("备份不存在".into())),
            Err(error) => (0, 0, Some(error)),
        };
        summaries.push(json!({"id":id,"savedAt":saved_at,"taskCount":count,"reason":"本机备份","unavailable":error}));
    }
    summaries.sort_by_key(|item| std::cmp::Reverse(item["savedAt"].as_u64().unwrap_or(0)));
    Ok(json!(summaries))
}

fn index_fingerprint(directory: &Path) -> Result<String, String> {
    let path = directory.join("store.json");
    if !regular_file_exists(&path)? { return Ok("missing".into()); }
    let mut file = File::open(&path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop { let count = file.read(&mut buffer).map_err(|error| error.to_string())?; if count == 0 { break; } hash.update(&buffer[..count]); }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_archive_chunks(directory: &Path, id: &str, kind: &str, items: Vec<Value>) -> Result<Value, String> {
    let mut names = Vec::new();let mut chunk = Vec::new();let mut bytes = 2usize;
    let flush = |chunk: &mut Vec<Value>, names: &mut Vec<String>| -> Result<(), String> {
        let name = format!("{id}.{kind}-{}.json", names.len());
        stage_json(directory, &json!(chunk))?.persist_noclobber(directory.join(&name)).map_err(|error| error.to_string())?;
        names.push(name);chunk.clear();Ok(())
    };
    for item in items {
        let size = serde_json::to_vec(&item).map_err(|error| error.to_string())?.len() + 1;
        if !chunk.is_empty() && bytes.saturating_add(size) > 64 * 1024 * 1024 { flush(&mut chunk, &mut names)?;bytes = 2; }
        bytes = bytes.saturating_add(size);chunk.push(item);
    }
    if !chunk.is_empty() { flush(&mut chunk, &mut names)?; }
    Ok(json!(names))
}

fn read_archive(directory: &Path, id: &str) -> Result<Option<Value>, String> {
    let Some(mut archive) = read_json(&directory.join(format!("{id}.json")))? else { return Ok(None); };
    if archive["chunked"] == true {
        for (field, kind) in [("records", "records"), ("deliveries", "deliveries"), ("generatedDeliveries", "generated")] {
            let mut items = Vec::new();
            for (index, name) in archive[format!("{kind}Files")].as_array().ok_or("归档分块索引无效")?.iter().enumerate() {
                if name.as_str() != Some(format!("{id}.{kind}-{index}.json").as_str()) { return Err("归档分块路径无效".into()); }
                let value = read_json(&directory.join(name.as_str().ok_or("归档分块无效")?))?.ok_or("归档分块缺失")?;
                let incoming = value.as_array().ok_or("归档分块无效")?;let maximum = if field == "records" { 100_000 } else if field == "deliveries" { 10_000 } else { 5 };if incoming.len() > maximum - items.len() { return Err("归档明细超出上限".into()); }items.extend(incoming.iter().cloned());
            }
            if field == "generatedDeliveries" { archive[field] = json!(items); } else { archive["ledger"][field] = json!(items); }
        }
        let mut packets = Vec::new();
        let count = archive["packetCount"].as_u64().filter(|count| *count <= 5).ok_or("归档交付数量无效")?;
        for index in 0..count { packets.push(read_json(&directory.join(format!("{id}.packet-{index}.json")))?.ok_or("归档交付缺失")?); }
        archive["packets"] = json!(packets);
    }
    Ok(Some(archive))
}

pub fn archive_ledger(root: &Path, mut archive: Value, expected_revision: u64) -> Result<(), String> {
    let directory = root.join("workspace");let state = load_state(root, &directory)?;
    if state["current"]["revision"].as_u64() != Some(expected_revision) { return Err("存档已更改，归档未提交".into()); }
    let id = archive["id"].as_str().filter(|id| valid_id(id)).ok_or("归档标识无效")?.to_owned();
    if archive["format"] != "mida-localization-ledger-archive" || archive["version"] != 1 { return Err("归档格式无效".into()); }
    for packet in archive["packets"].as_array_mut().ok_or("归档交付无效")? { split_snapshot(&directory, packet)?;packet["workload"].as_object_mut().ok_or("归档工作量无效")?.remove("records"); }
    // Protect exact frozen task hashes before exposing the archive. Saving a later
    // entry may still reclaim unrelated versions of that task.
    let mut protected = archive_protected_blobs(&directory)?;
    for packet in archive["packets"].as_array().ok_or("归档交付无效")? { for reference in packet["taskRefs"].as_array().ok_or("归档片段索引无效")? { protected.insert(reference["blob"].as_str().ok_or("归档片段标识无效")?.to_owned()); } }
    stage_json(&directory, &json!(protected))?.persist(directory.join("archive-task-refs.json")).map_err(|error| error.to_string())?;sync_directory(&directory)?;
    let archives = directory.join("ledgers");ensure_directory(&archives)?;
    let records = archive["ledger"]["records"].as_array().ok_or("归档记录无效")?.clone();
    let deliveries = archive["ledger"]["deliveries"].as_array().ok_or("归档交付历史无效")?.clone();
    archive["recordsFiles"] = write_archive_chunks(&archives, &id, "records", records)?;
    archive["deliveriesFiles"] = write_archive_chunks(&archives, &id, "deliveries", deliveries)?;
    let generated = archive["generatedDeliveries"].as_array().cloned().unwrap_or_default();archive["generatedFiles"] = write_archive_chunks(&archives, &id, "generated", generated)?;archive.as_object_mut().ok_or("归档无效")?.remove("generatedDeliveries");
    archive["ledger"].as_object_mut().ok_or("归档台账无效")?.remove("records");archive["ledger"].as_object_mut().ok_or("归档台账无效")?.remove("deliveries");
    let packets = archive["packets"].as_array().ok_or("归档交付无效")?.clone();
    for (index, packet) in packets.iter().enumerate() { stage_json(&archives, packet)?.persist_noclobber(archives.join(format!("{id}.packet-{index}.json"))).map_err(|error| error.to_string())?; }
    archive["packetCount"] = json!(packets.len());archive.as_object_mut().ok_or("归档无效")?.remove("packets");archive["chunked"] = json!(true);
    // Publish the archive manifest last. Incomplete orphan chunks never replace current data.
    stage_json(&archives, &archive)?.persist_noclobber(archives.join(format!("{id}.json"))).map_err(|error| error.to_string())?;sync_directory(&archives)?;
    Ok(())
}

pub fn recovery_info(root: &Path) -> Result<Value, String> {
    let directory = root.join("workspace");
    let token = index_fingerprint(&directory)?;
    let raw = read_json(&directory.join("store.json")).ok().flatten();
    Ok(json!({"token":token,"current":raw.as_ref().map(|value| &value["current"]),"history":backup_history(&directory)?}))
}

pub fn recover(root: &Path, snapshot: Value, token: &str) -> Result<Value, String> {
    validate_snapshot(&snapshot)?;
    let directory = root.join("workspace");
    if index_fingerprint(&directory)? != token { return Err("工作区已被其他窗口更改，请重新打开恢复列表".into()); }
    ensure_directory(root)?; ensure_directory(&directory)?;
    let quarantine = directory.join("recovery"); ensure_directory(&quarantine)?;
    let index = directory.join("store.json");
    if regular_file_exists(&index)? {
        let target = quarantine.join(format!("{}.json", uuid::Uuid::new_v4()));
        fs::copy(&index, &target).map_err(|error| format!("保护原存档失败：{error}"))?;
        File::open(&target).and_then(|file| file.sync_all()).map_err(|error| error.to_string())?;
        sync_directory(&quarantine)?;
    }
    let raw = read_json(&index).ok().flatten();
    let revision = raw.as_ref().and_then(|value| value["current"]["revision"].as_u64()).filter(|revision| *revision < 9_007_199_254_740_991).unwrap_or(0);
    let state = json!({"current":null,"history":[],"quarantineTaskBlobs":existing_task_blobs(&directory)?});
    commit_snapshot(root, snapshot, revision, Some("恢复有效备份".into()), None, state, revision)
}

pub fn save(
    root: &Path,
    snapshot: Value,
    expected_revision: u64,
    backup_reason: Option<String>,
    media_cleanup: Option<Value>,
) -> Result<Value, String> {
    validate_snapshot(&snapshot)?;
    if is_legacy_demo(&snapshot) {
        return Err("请先导入 Unity 导出的 ZIP，空白或旧示例工作区不会保存".into());
    }
    let directory = root.join("workspace");
    let state = load_state(root, &directory)?;
    commit_snapshot(root, snapshot, expected_revision, backup_reason, media_cleanup, state, 0)
}

fn commit_snapshot(root: &Path, snapshot: Value, expected_revision: u64, backup_reason: Option<String>, media_cleanup: Option<Value>, mut state: Value, recovery_revision: u64) -> Result<Value, String> {
    let directory = root.join("workspace");
    let previous = &state["current"];
    let replacing_demo = is_legacy_demo(&previous["snapshot"]);
    let revision = if previous.is_null() {
        recovery_revision
    } else {
        unsigned(previous, "revision")?
    };
    if revision != expected_revision {
        return Err("另一个编辑器页面已保存更新，已暂停本页保存以避免覆盖；请保留本页内容并关闭其他页面后重新打开".into());
    }
    ensure_directory(root)?;
    ensure_directory(&directory)?;
    let full_snapshot = snapshot.clone();
    let mut snapshot = snapshot;
    split_snapshot(&directory, &mut snapshot)?;
    let next_revision = revision.checked_add(1).filter(|revision| *revision <= 9_007_199_254_740_991).ok_or("自动保存版本已达上限，请从备份恢复")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let now = u64::try_from(timestamp.as_millis()).map_err(|error| error.to_string())?;
    let mut last_backup_at = if previous.is_null() {
        0
    } else {
        unsigned(previous, "lastBackupAt")?
    };
    let reason = backup_reason.filter(|reason| !reason.is_empty());
    let needs_backup = previous.is_null()
        || replacing_demo
        || reason.is_some();
    let mut summaries = history(&state)?.to_vec();
    let mut backup = None;
    if needs_backup {
        let incoming = previous.is_null() || replacing_demo || reason.is_some();
        let saved_snapshot = if incoming {
            &snapshot
        } else {
            &previous["snapshot"]
        };
        let saved_at = if incoming {
            now
        } else {
            unsigned(previous, "savedAt")?
        };
        let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let seed = format!("{}-{}-{sequence}", timestamp.as_nanos(), std::process::id());
        let identifier = format!("{:x}", Sha256::digest(seed.as_bytes()));
        summaries.insert(
            0,
            json!({
                "id": identifier,
                "savedAt": saved_at,
                "reason": reason.unwrap_or_else(|| "首次保存".into()),
                "taskCount": saved_snapshot.get("taskRefs").or_else(|| saved_snapshot.get("tasks")).and_then(Value::as_array).ok_or("自动保存任务无效")?.len()
            }),
        );
        backup = Some((
            identifier,
            json!({"snapshot": saved_snapshot, "savedAt": saved_at}),
        ));
        last_backup_at = now;
    }
    let removed = summaries.split_off(summaries.len().min(HISTORY_LIMIT));
    let record = json!({
        "snapshot": snapshot,
        "savedAt": now,
        "lastBackupAt": last_backup_at,
        "revision": next_revision
    });
    if replacing_demo {
        state["legacyDemoCurrent"] = previous.clone();
    }
    state["current"] = record;
    state["history"] = Value::Array(summaries);
    if let Some(jobs) = media_cleanup {
        if state.get("mediaCleanupJobs").is_none() { state["mediaCleanupJobs"] = json!({}); }
        let pending = state["mediaCleanupJobs"].as_object_mut().ok_or("媒体清理索引无效")?;
        for (key, value) in jobs.as_object().ok_or("媒体清理任务无效")? { pending.insert(key.clone(), value.clone()); }
    }

    ensure_directory(root)?;
    ensure_directory(&directory)?;
    let backups = directory.join("backups");
    ensure_directory(&backups)?;
    let index_path = directory.join("store.json");
    regular_file_exists(&index_path)?;
    if let Some((identifier, contents)) = backup {
        let staged_backup = stage_json(&backups, &contents)?;
        staged_backup
            .persist_noclobber(backup_path(&backups, &identifier))
            .map_err(|error| format!("写入自动保存备份失败：{error}"))?;
        sync_directory(&backups)?;
    }
    compact_backups(&directory, &mut state)?;
    let staged_index = stage_json(&directory, &state)?;
    sync_directory(&directory)?;
    sync_directory(root)?;
    if let Some(parent) = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        sync_directory(parent)?;
    }
    staged_index
        .persist(&index_path)
        .map_err(|error| format!("提交自动保存失败：{error}"))?;
    let index_synced = sync_directory(&directory).is_ok();
    if index_synced {
        for entry in removed {
            if let Some(identifier) = entry["id"].as_str().filter(|id| valid_id(id)) {
                let path = backup_path(&backups, identifier);
                if !state["unavailableBackupIds"].as_array().is_some_and(|ids| ids.iter().any(|id| id == identifier)) && regular_file_exists(&path).unwrap_or(false) {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let _ = sync_directory(&backups);
    }
    let mut result = state["current"].clone();
    result["snapshot"] = full_snapshot;
    if index_synced { let _ = prune_tasks(&directory, &state); }
    Ok(result)
}

pub fn finish_media_cleanup(root: &Path) -> Result<(), String> {
    let directory = root.join("workspace");
    let mut state = load_state(root, &directory)?;
    if state.get("mediaCleanupJobs").is_none() { return Ok(()); }
    crate::media::clean_jobs(root, &state["mediaCleanupJobs"])?;
    state.as_object_mut().ok_or("工作区索引无效")?.remove("mediaCleanupJobs");
    let staged = stage_json(&directory, &state)?;
    let path = directory.join("store.json");
    regular_file_exists(&path)?;
    staged.persist(path).map_err(|error| error.to_string())?;
    sync_directory(&directory)
}

fn is_legacy_demo(snapshot: &Value) -> bool {
    !snapshot.is_null()
        && (snapshot["currentProjectId"]
            .as_str()
            .map_or(true, |project| project.trim().is_empty())
            || snapshot["fileVersion"]["lineageId"] == "demo-task-package"
            || snapshot["currentPackageId"] == "demo-import-v1")
}

fn validate_snapshot(snapshot: &Value) -> Result<(), String> {
    crate::media::validate_references(snapshot)?;
    if snapshot["format"] != "mida-localization-workspace"
        || snapshot["version"].as_u64() != Some(1)
    {
        return Err("不支持的自动保存格式，未覆盖当前内容".into());
    }
    let tasks = snapshot["tasks"].as_array().ok_or("自动保存缺少任务列表")?;
    if tasks.is_empty() || tasks.len() > 1000 {
        return Err("自动保存任务数量必须为 1 至 1000".into());
    }
    let mut entry_count = 0usize;
    for task in tasks {
        crate::package::unit_metadata(task)?;
        let entries = task["entries"]
            .as_array()
            .ok_or("自动保存任务缺少词条列表")?;
        entry_count = entry_count
            .checked_add(entries.len())
            .ok_or("自动保存词条过多")?;
        if entries.is_empty() || entry_count > 100_000 {
            return Err("自动保存任务词条不能为空，且总数不得超过 100000".into());
        }
        if entries.iter().any(|entry| !entry.is_object()) {
            return Err("自动保存词条必须为 JSON 对象".into());
        }
        for entry in entries {
            crate::package::validate_source_snapshots(entry)?;
        }
    }
    Ok(())
}

// Immutable task JSON files are committed before the small workspace index.
// Backups retain references to the same files, so confirming a line never copies a package.
fn validate_refs(snapshot: &Value) -> Result<(), String> {
    if snapshot["format"] != "mida-localization-workspace" || snapshot["version"] != 1 || snapshot.get("tasks").is_some() {
        return Err("片段存档索引格式无效".into());
    }
    let refs = snapshot["taskRefs"].as_array().ok_or("缺少片段索引")?;
    if refs.is_empty() || refs.len() > 1000 { return Err("片段索引数量无效".into()); }
    let mut identities = HashSet::new();
    let mut count = 0u64;
    for item in refs {
        let blob = item["blob"].as_str().ok_or("片段文件标识无效")?;
        if blob.len() != 64 || !blob.bytes().all(|c| c.is_ascii_hexdigit()) || !item["partName"].is_string() || !item["language"].is_string()
            || !identities.insert((item["partName"].clone().to_string(), item["language"].clone().to_string())) {
            return Err("片段文件索引无效或重复".into());
        }
        let entries = unsigned(item, "entryCount")?;
        count = count.checked_add(entries).ok_or("片段词条总数超限")?;
        if entries == 0 || count > 100_000 { return Err("片段词条数量无效".into()); }
    }
    Ok(())
}

fn write_task(directory: &Path, task: &Value) -> Result<Value, String> {
    let files = directory.join("tasks");
    if !directory_exists(&files)? {
        ensure_directory(&files)?;
        sync_directory(directory)?;
    }
    let bytes = serde_json::to_vec(task).map_err(|error| error.to_string())?;
    let blob = format!("{:x}", Sha256::digest(&bytes));
    let path = files.join(format!("{blob}.json"));
    if !regular_file_exists(&path)? {
        stage_json(&files, task)?.persist_noclobber(path).map_err(|error| error.to_string())?;
        sync_directory(&files)?;
    } else if read_json(&path)?.as_ref() != Some(task) {
        return Err("已有片段 JSON 损坏，未提交保存".into());
    }
    Ok(json!({"blob": blob, "partName": task["partName"], "language": task["language"], "entryCount": task["entries"].as_array().ok_or("片段词条无效")?.len()}))
}

fn split_snapshot(directory: &Path, snapshot: &mut Value) -> Result<(), String> {
    if snapshot.get("taskRefs").is_some() { return validate_refs(snapshot); }
    let tasks = snapshot["tasks"].as_array().ok_or("缺少片段数据")?;
    let refs = tasks.iter().map(|task| write_task(directory, task)).collect::<Result<Vec<_>, _>>()?;
    snapshot.as_object_mut().ok_or("工作区无效")?.remove("tasks");
    snapshot["taskRefs"] = json!(refs);
    Ok(())
}

fn expand_snapshot(directory: &Path, snapshot: &mut Value) -> Result<(), String> {
    if snapshot.get("taskRefs").is_none() { return Ok(()); }
    validate_refs(snapshot)?;
    let mut tasks = Vec::new();
    for item in snapshot["taskRefs"].as_array().ok_or("缺少片段索引")? {
        let blob = item["blob"].as_str().ok_or("片段标识无效")?;
        let task = read_json(&directory.join("tasks").join(format!("{blob}.json")))?.ok_or("片段 JSON 缺失，未覆盖存档")?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&task).map_err(|error| error.to_string())?));
        if digest != blob || task["partName"] != item["partName"] || task["language"] != item["language"] || task["entries"].as_array().map(|entries| entries.len() as u64) != item["entryCount"].as_u64() {
            return Err("片段 JSON 与索引不一致，未覆盖存档".into());
        }
        tasks.push(task);
    }
    snapshot.as_object_mut().ok_or("工作区无效")?.remove("taskRefs");
    snapshot["tasks"] = json!(tasks);
    validate_snapshot(snapshot)
}

pub fn save_entry(root: &Path, identity: Value, task_index: usize, project_id: &str, entry: Value, mut view: Value, delta: Value, expected_revision: u64) -> Result<Value, String> {
    let directory = root.join("workspace");let state = load_state(root, &directory)?;
    if state["current"]["revision"].as_u64() != Some(expected_revision) { return Err("工作区版本冲突".into()); }
    let snapshot = &state["current"]["snapshot"];
    let mut task = if let Some(reference) = snapshot["taskRefs"].get(task_index) {
        let mut wrapper = json!({"format":"mida-localization-workspace","version":1,"taskRefs":[reference],"currentProjectId":project_id});
        expand_snapshot(&directory, &mut wrapper)?;wrapper["tasks"][0].take()
    } else { snapshot["tasks"].get(task_index).ok_or("片段索引越界")?.clone() };
    if identity["partName"] != task["partName"] || identity["language"] != task["language"] { return Err("片段身份不匹配".into()); }
    let changes = if entry.is_array() { entry.as_array().ok_or("词条增量无效")?.clone() } else { vec![entry] };
    let entries = task["entries"].as_array_mut().ok_or("片段词条无效")?;
    let indexes: std::collections::HashMap<String, usize> = entries.iter().enumerate().filter_map(|(index, item)| item["key"].as_str().map(|key| (key.to_owned(), index))).collect();
    let mut confirmed_keys = Vec::new();
    for change in changes { let key = change["key"].as_str().ok_or("词条标识无效")?.to_owned();let index = *indexes.get(&key).ok_or("词条不属于片段")?;confirmed_keys.push(key);entries[index] = change; }
    let mut ledger = snapshot["workLedger"].clone();if ledger["id"] != delta["id"] { return Err("台账身份不匹配".into()); }
    let records = ledger["records"].as_array_mut().ok_or("台账记录无效")?;let mut ids: HashSet<String> = records.iter().filter_map(|record| record["id"].as_str().map(str::to_owned)).collect();
    for record in delta["records"].as_array().ok_or("台账增量无效")? { let id = record["id"].as_str().ok_or("记录标识无效")?;if !ids.insert(id.to_owned()) { return Err("工作记录重复".into()); }records.push(record.clone()); }
    if records.len() > 100_000 { return Err("请先归档工作量".into()); }
    view["workLedger"] = ledger;
    let mut candidates = snapshot["importedTranslations"].as_array().cloned().unwrap_or_default();candidates.retain(|candidate| candidate["partName"] != identity["partName"] || candidate["language"] != identity["language"] || !confirmed_keys.iter().any(|key| candidate["key"] == key.as_str()));view["importedTranslations"] = json!(candidates);
    save_task(root, task, task_index, project_id, view, confirmed_keys, expected_revision)
}

pub fn save_task(root: &Path, task: Value, task_index: usize, project_id: &str, view: Value, confirmed_keys: Vec<String>, expected_revision: u64) -> Result<Value, String> {
    let directory = root.join("workspace");
    let mut state = load_state(root, &directory)?;
    if state["current"]["revision"].as_u64() != Some(expected_revision) { return Err("工作区保存版本冲突，未覆盖片段".into()); }
    if state["current"]["snapshot"]["currentProjectId"] != project_id { return Err("片段不属于当前项目".into()); }
    // One-time migration of this editor's old monolithic workspace; original backups remain readable.
    if state["current"]["snapshot"].get("taskRefs").is_none() {
        split_snapshot(&directory, &mut state["current"]["snapshot"])?;
    }
    if state.get("backupTaskBlobs").is_none() { compact_backups(&directory, &mut state)?; }
    let snapshot = &mut state["current"]["snapshot"];
    let reference = snapshot["taskRefs"].get(task_index).ok_or("片段索引越界")?;
    if task["partName"] != reference["partName"] || task["language"] != reference["language"] || task["entries"].as_array().map(|entries| entries.len() as u64) != reference["entryCount"].as_u64() {
        return Err("片段身份或词条数量发生变化，请重新导入".into());
    }
    let old_blob = reference["blob"].as_str().ok_or("片段文件标识无效")?.to_owned();
    validate_snapshot(&json!({"format":"mida-localization-workspace","version":1,"tasks":[task]}))?;
    snapshot["taskRefs"][task_index] = write_task(&directory, &task)?;
    for field in ["state", "exportSequence", "workLedger", "importedTranslations", "previewPlayer"] {
        if let Some(value) = view.get(field) { snapshot[field] = value.clone(); }
    }
    // Confirmation saves translations only. Unconfirmed input remains in memory.
    let confirmed: HashSet<_> = confirmed_keys.iter().map(String::as_str).collect();
    if let Some(drafts) = snapshot["drafts"].as_array_mut() { drafts.retain(|draft| draft["taskIndex"].as_u64() != Some(task_index as u64) || !draft["key"].as_str().is_some_and(|key| confirmed.contains(key))); }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_millis() as u64;
    let revision = expected_revision.checked_add(1).filter(|revision| *revision <= 9_007_199_254_740_991).ok_or("自动保存版本已达上限，请从备份恢复")?;
    state["current"]["savedAt"] = json!(now);
    state["current"]["revision"] = json!(revision);
    let index = directory.join("store.json");
    regular_file_exists(&index)?;
    stage_json(&directory, &state)?.persist(&index).map_err(|error| error.to_string())?;
    // Once the atomic index replacement succeeds, report its revision even if a later sync fails.
    let sync_warning = sync_directory(&directory).err();
    if sync_warning.is_none() {
        let protected = ["backupTaskBlobs", "archiveTaskBlobs", "quarantineTaskBlobs"].iter().any(|field| state[*field].as_array().is_some_and(|blobs| blobs.iter().any(|blob| blob == &old_blob))) || archive_protected_blobs(&directory).map(|refs| refs.contains(&old_blob)).unwrap_or(true);
        let current = state["current"]["snapshot"]["taskRefs"].as_array().is_some_and(|refs| refs.iter().any(|item| item["blob"] == old_blob));
        if !protected && !current {
            let path = directory.join("tasks").join(format!("{old_blob}.json"));
            if regular_file_exists(&path).unwrap_or(false) { let _ = fs::remove_file(path); }
        }
    }
    Ok(json!({"savedAt":now,"revision":revision,"saveWarning":sync_warning}))
}

fn archive_protected_blobs(directory: &Path) -> Result<HashSet<String>, String> {
    let path = directory.join("archive-task-refs.json");
    if let Ok(Some(value)) = read_json(&path) { if let Some(values) = value.as_array() { return Ok(values.iter().filter_map(|value| value.as_str().map(str::to_owned)).collect()); } }
    // Missing/corrupt derived protection metadata can be rebuilt from immutable
    // archive manifests without depending on the current workspace index.
    let archives = directory.join("ledgers");let mut refs = HashSet::new();
    if !directory_exists(&archives)? { return Ok(refs); }
    for entry in fs::read_dir(&archives).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) else { continue; };
        let header = read_json(&entry.path())?.ok_or("归档文件缺失")?;
        let packets = if header["chunked"] == true { let count = header["packetCount"].as_u64().filter(|count| *count <= 5).ok_or("归档交付数量无效")?;(0..count).map(|index| read_json(&archives.join(format!("{id}.packet-{index}.json")))?.ok_or_else(|| "归档交付缺失".into())).collect::<Result<Vec<_>, String>>()? } else { header["packets"].as_array().ok_or("归档交付无效")?.clone() };
        for packet in packets { for item in packet["taskRefs"].as_array().ok_or("归档片段索引无效")? { refs.insert(item["blob"].as_str().ok_or("归档片段标识无效")?.to_owned()); } }
    }
    stage_json(directory, &json!(refs))?.persist(&path).map_err(|error| error.to_string())?;sync_directory(directory)?;Ok(refs)
}

fn existing_task_blobs(directory: &Path) -> Result<HashSet<String>, String> {
    let files = directory.join("tasks");let mut blobs = HashSet::new();
    if !directory_exists(&files)? { return Ok(blobs); }
    for entry in fs::read_dir(files).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(blob) = name.strip_suffix(".json").filter(|blob| blob.len() == 64 && blob.bytes().all(|byte| byte.is_ascii_hexdigit())) { if regular_file_exists(&entry.path())? { blobs.insert(blob.to_owned()); } }
    }
    Ok(blobs)
}

fn compact_backups(directory: &Path, state: &mut Value) -> Result<(), String> {
    let mut protected = HashSet::new();
    let mut unavailable: HashSet<String> = state["unavailableBackupIds"].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_owned)).collect();
    let mut quarantine: HashSet<String> = state["quarantineTaskBlobs"].as_array().into_iter().flatten().filter_map(|blob| blob.as_str().map(str::to_owned)).collect();
    let mut deferred = false;
    let mut summaries = history(state)?.clone();
    for summary in &mut summaries {
        let id = summary["id"].as_str().ok_or("备份标识无效")?.to_owned();
        let result: Result<(), String> = (|| {
            let path = backup_path(&directory.join("backups"), &id);
            let mut backup = read_json(&path)?.ok_or("备份文件缺失")?;
            if backup["snapshot"].get("taskRefs").is_none() {
                validate_snapshot(&backup["snapshot"])?;
                split_snapshot(directory, &mut backup["snapshot"])?;
                stage_json(&directory.join("backups"), &backup)?.persist(&path).map_err(|error| error.to_string())?;
                sync_directory(&directory.join("backups"))?;
            }
            validate_refs(&backup["snapshot"])?;
            for item in backup["snapshot"]["taskRefs"].as_array().ok_or("备份片段索引无效")? { protected.insert(item["blob"].as_str().ok_or("备份片段标识无效")?.to_owned()); }
            Ok(())
        })();
        summary.as_object_mut().ok_or("备份摘要无效")?.remove("unavailable");
        if let Err(error) = result { if unavailable.insert(id.to_owned()) { quarantine.extend(existing_task_blobs(directory)?); } deferred = true; summary["unavailable"] = json!(error); }
    }
    state["history"] = json!(summaries);
    // Unknown references must never cause cleanup of potentially recoverable blobs.
    state["backupCleanupDeferred"] = json!(deferred);
    state["unavailableBackupIds"] = json!(unavailable);state["quarantineTaskBlobs"] = json!(quarantine);
    state["backupTaskBlobs"] = json!(protected);
    Ok(())
}

fn prune_tasks(directory: &Path, state: &Value) -> Result<(), String> {
    let mut keep: HashSet<String> = ["archiveTaskBlobs", "quarantineTaskBlobs"].iter().flat_map(|field| state[*field].as_array().into_iter().flatten()).filter_map(|blob| blob.as_str().map(|blob| format!("{blob}.json"))).collect();
    keep.extend(archive_protected_blobs(directory)?.into_iter().map(|blob| format!("{blob}.json")));
    let mut collect = |snapshot: &Value| {
        if let Some(refs) = snapshot["taskRefs"].as_array() {
            for item in refs { if let Some(blob) = item["blob"].as_str() { keep.insert(format!("{blob}.json")); } }
        }
    };
    collect(&state["current"]["snapshot"]);
    for blob in state["backupTaskBlobs"].as_array().into_iter().flatten() { if let Some(blob) = blob.as_str() { keep.insert(format!("{blob}.json")); } }
    let files = directory.join("tasks");
    for entry in fs::read_dir(&files).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") && !keep.contains(&name) && regular_file_exists(&entry.path())? { fs::remove_file(entry.path()).map_err(|error| error.to_string())?; }
    }
    Ok(())
}

fn load_state(root: &Path, directory: &Path) -> Result<Value, String> {
    let empty = || json!({"current": null, "history": []});
    if !directory_exists(root)? || !directory_exists(directory)? {
        return Ok(empty());
    }
    let Some(state) = read_json(&directory.join("store.json"))? else {
        return Ok(empty());
    };
    let current = state.get("current").ok_or("自动保存索引缺少当前记录")?;
    let summaries = history(&state)?;
    if current.is_null() || summaries.is_empty() || summaries.len() > HISTORY_LIMIT {
        return Err("自动保存索引损坏，未覆盖已有内容".into());
    }
    if current["snapshot"].get("taskRefs").is_some() {
        validate_refs(&current["snapshot"])?;
    } else {
        validate_snapshot(&current["snapshot"])?;
    }
    unsigned(current, "savedAt")?;
    unsigned(current, "lastBackupAt")?;
    if unsigned(current, "revision")? == 0 || unsigned(current, "revision")? > 9_007_199_254_740_991 {
        return Err("自动保存版本无效".into());
    }
    let mut identifiers = HashSet::new();
    for summary in summaries {
        let identifier = summary["id"].as_str().ok_or("自动保存备份标识无效")?;
        if !valid_id(identifier) || !identifiers.insert(identifier) {
            return Err("自动保存备份标识无效或重复".into());
        }
        unsigned(summary, "savedAt")?;
        if !(1..=1000).contains(&unsigned(summary, "taskCount")?) || !summary["reason"].is_string()
        {
            return Err("自动保存备份摘要无效".into());
        }
    }
    Ok(state)
}

fn history(state: &Value) -> Result<&Vec<Value>, String> {
    state["history"]
        .as_array()
        .ok_or_else(|| "自动保存历史索引无效".into())
}

fn unsigned(value: &Value, field: &str) -> Result<u64, String> {
    value[field]
        .as_u64()
        .ok_or_else(|| format!("自动保存字段 {field} 无效"))
}

fn valid_id(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= 128
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn backup_path(directory: &Path, identifier: &str) -> PathBuf {
    directory.join(format!("{identifier}.json"))
}

fn directory_exists(path: &Path) -> Result<bool, String> {
    crate::media::safe_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err("自动保存目录不是普通目录，或为符号链接".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("读取自动保存目录失败：{error}")),
    }
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    crate::media::safe_path(path)?;
    if !directory_exists(path)? {
        fs::create_dir_all(path).map_err(|error| format!("创建自动保存目录失败：{error}"))?;
        if !directory_exists(path)? {
            return Err("自动保存目录不存在".into());
        }
    }
    Ok(())
}

fn regular_file_exists(path: &Path) -> Result<bool, String> {
    crate::media::safe_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err("自动保存路径不是普通文件，或为符号链接".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("读取自动保存文件状态失败：{error}")),
    }
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    if !regular_file_exists(path)? {
        return Ok(None);
    }
    let file = File::open(path).map_err(|error| format!("读取自动保存失败：{error}"))?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_JSON_BYTES {
        return Err("自动保存 JSON 超过 128 MiB 上限".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取自动保存失败：{error}"))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err("自动保存 JSON 超过 128 MiB 上限".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("自动保存 JSON 损坏，未覆盖已有内容：{error}"))
}

struct LimitedWriter<'a> {
    file: &'a mut File,
    remaining: u64,
}

impl Write for LimitedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "自动保存 JSON 超过 128 MiB 上限",
            ));
        }
        let written = self.file.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn stage_json(directory: &Path, value: &Value) -> Result<NamedTempFile, String> {
    let mut temporary = NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    {
        let mut writer = io::BufWriter::new(LimitedWriter {
            file: temporary.as_file_mut(),
            remaining: MAX_JSON_BYTES,
        });
        serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    read_json(temporary.path())?.ok_or("自动保存临时文件不存在")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    Ok(temporary)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    crate::media::safe_path(path)?;
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
    #[cfg(not(windows))]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("同步自动保存目录失败：{error}"))
}
