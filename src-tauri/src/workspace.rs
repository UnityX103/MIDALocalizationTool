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
const BACKUP_INTERVAL_MS: u64 = 5 * 60 * 1000;
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
        ("workspace", "current") => Ok(state["current"].clone()),
        ("workspace", "history") => Ok(state["history"].clone()),
        _ => {
            if !history(&state)?.iter().any(|entry| entry["id"] == key) {
                return Ok(Value::Null);
            }
            let backups = directory.join("backups");
            if !directory_exists(&backups)? {
                return Ok(Value::Null);
            }
            let Some(backup) = read_json(&backup_path(&backups, key))? else {
                return Ok(Value::Null);
            };
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
        || reason.is_some()
        || now.saturating_sub(last_backup_at) >= BACKUP_INTERVAL_MS;
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
                "reason": reason.unwrap_or_else(|| "定时备份".into()),
                "taskCount": saved_snapshot["tasks"].as_array().ok_or("自动保存任务无效")?.len()
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
    let staged_index = stage_json(&directory, &state)?;
    if let Some((identifier, contents)) = backup {
        let staged_backup = stage_json(&backups, &contents)?;
        staged_backup
            .persist_noclobber(backup_path(&backups, &identifier))
            .map_err(|error| format!("写入自动保存备份失败：{error}"))?;
        sync_directory(&backups)?;
    }
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
    if sync_directory(&directory).is_ok() {
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
    Ok(state["current"].take())
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
    validate_snapshot(&current["snapshot"])?;
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
