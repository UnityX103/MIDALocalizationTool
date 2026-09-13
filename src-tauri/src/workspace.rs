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
        ("backups", identifier) if valid_id(identifier) => {}
        _ => return Err("不支持的自动保存存储或键名".into()),
    }
    let directory = root.join("workspace");
    let state = load_state(root, &directory)?;
    match (store, key) {
        ("workspace", "current") => {
            let mut record = state["current"].clone();
            if !record.is_null() { expand_snapshot(&directory, &mut record["snapshot"])?; }
            Ok(record)
        },
        ("workspace", "history") => Ok(state["history"].clone()),
        _ => {
            if !history(&state)?.iter().any(|entry| entry["id"] == key) {
                return Ok(Value::Null);
            }
            let backups = directory.join("backups");
            if !directory_exists(&backups)? {
                return Ok(Value::Null);
            }
            let Some(mut backup) = read_json(&backup_path(&backups, key))? else {
                return Ok(Value::Null);
            };
            expand_snapshot(&directory, &mut backup["snapshot"])?;
            validate_snapshot(&backup["snapshot"])?;
            unsigned(&backup, "savedAt")?;
            Ok(backup)
        }
    }
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
    let mut state = load_state(root, &directory)?;
    let previous = &state["current"];
    let replacing_demo = is_legacy_demo(&previous["snapshot"]);
    let revision = if previous.is_null() {
        0
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
    let next_revision = revision.checked_add(1).ok_or("自动保存版本已达上限")?;
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
                if regular_file_exists(&path).unwrap_or(false) {
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
    for field in ["state", "exportSequence", "previewPlayer"] {
        if let Some(value) = view.get(field) { snapshot[field] = value.clone(); }
    }
    // Confirmation saves translations only. Unconfirmed input remains in memory.
    let confirmed: HashSet<_> = confirmed_keys.iter().map(String::as_str).collect();
    if let Some(drafts) = snapshot["drafts"].as_array_mut() { drafts.retain(|draft| draft["taskIndex"].as_u64() != Some(task_index as u64) || !draft["key"].as_str().is_some_and(|key| confirmed.contains(key))); }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_millis() as u64;
    let revision = expected_revision.checked_add(1).ok_or("自动保存版本已达上限")?;
    state["current"]["savedAt"] = json!(now);
    state["current"]["revision"] = json!(revision);
    let index = directory.join("store.json");
    regular_file_exists(&index)?;
    stage_json(&directory, &state)?.persist(&index).map_err(|error| error.to_string())?;
    // Once the atomic index replacement succeeds, report its revision even if a later sync fails.
    let sync_warning = sync_directory(&directory).err();
    if sync_warning.is_none() {
        let protected = state["backupTaskBlobs"].as_array().is_some_and(|blobs| blobs.iter().any(|blob| blob == &old_blob));
        let current = state["current"]["snapshot"]["taskRefs"].as_array().is_some_and(|refs| refs.iter().any(|item| item["blob"] == old_blob));
        if !protected && !current {
            let path = directory.join("tasks").join(format!("{old_blob}.json"));
            if regular_file_exists(&path).unwrap_or(false) { let _ = fs::remove_file(path); }
        }
    }
    Ok(json!({"savedAt":now,"revision":revision,"saveWarning":sync_warning}))
}

fn compact_backups(directory: &Path, state: &mut Value) -> Result<(), String> {
    let mut protected = HashSet::new();
    for summary in history(state)? {
        let id = summary["id"].as_str().ok_or("备份标识无效")?;
        let path = backup_path(&directory.join("backups"), id);
        let mut backup = read_json(&path)?.ok_or("备份文件缺失")?;
        if backup["snapshot"].get("taskRefs").is_none() {
            validate_snapshot(&backup["snapshot"])?;
            split_snapshot(directory, &mut backup["snapshot"])?;
            stage_json(&directory.join("backups"), &backup)?.persist(&path).map_err(|error| error.to_string())?;
            sync_directory(&directory.join("backups"))?;
        }
        validate_refs(&backup["snapshot"])?;
        for item in backup["snapshot"]["taskRefs"].as_array().ok_or("备份片段索引无效")? {
            protected.insert(item["blob"].as_str().ok_or("备份片段标识无效")?.to_owned());
        }
    }
    state["backupTaskBlobs"] = json!(protected);
    Ok(())
}

fn prune_tasks(directory: &Path, state: &Value) -> Result<(), String> {
    let mut keep = HashSet::new();
    let mut collect = |snapshot: &Value| {
        if let Some(refs) = snapshot["taskRefs"].as_array() {
            for item in refs { if let Some(blob) = item["blob"].as_str() { keep.insert(format!("{blob}.json")); } }
        }
    };
    collect(&state["current"]["snapshot"]);
    for summary in history(state)? {
        let id = summary["id"].as_str().ok_or("备份标识无效")?;
        let backup = read_json(&backup_path(&directory.join("backups"), id))?.ok_or("备份文件缺失")?;
        collect(&backup["snapshot"]);
    }
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
    if unsigned(current, "revision")? == 0 {
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
