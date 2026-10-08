use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

const FILE_LIMIT: u64 = 5 * 1024 * 1024;
struct Logger { directory: PathBuf, run: String, part: u32, error: String }
static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

fn now() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() }
fn private_file(options: &mut OpenOptions) {
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
}
fn files(logger: &Logger) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(&logger.directory).into_iter().flatten().flatten()
        .filter_map(|entry| { let p = entry.path(); (entry.file_type().ok()?.is_file() && p.file_name()?.to_str()?.starts_with("run-") && p.extension()?.to_str()? == "jsonl").then_some(p) }).collect();
    paths.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    paths
}
fn sanitize(text: &str) -> String {
    let mut result = text.to_owned();
    for key in ["HOME", "USERPROFILE"] { if let Ok(home) = std::env::var(key) { if !home.is_empty() { result = result.replace(&home, "<home>"); } } }
    // Stack traces carry code locations; never retain the exception's interpolated first line.
    result.chars().take(8000).collect()
}
pub fn initialize(root: PathBuf) {
    let directory = root.join("logs");
    let setup = crate::media::safe_path(&directory).and_then(|_| fs::create_dir_all(&directory).map_err(|e| e.to_string()));
    #[cfg(unix)] if setup.is_ok() { use std::os::unix::fs::PermissionsExt; let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)); }
    let _ = LOGGER.set(Mutex::new(Logger { directory, run: uuid::Uuid::new_v4().to_string(), part: 0, error: setup.err().map(|_| "日志目录暂不可写".to_owned()).unwrap_or_default() }));
    event("runtime.start", "INFO", None, None);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        event("runtime.panic", "ERROR", None, Some(json!({"stack":sanitize(&format!("{:?}\n{}", info.location(), std::backtrace::Backtrace::force_capture()))})));
        previous(info);
    }));
}
pub fn event(name: &str, level: &str, operation: Option<&str>, extra: Option<Value>) {
    let mut value = extra.unwrap_or_else(|| json!({}));
    value["source"] = json!("rust"); value["event"] = json!(name); value["level"] = json!(level);
    if let Some(operation) = operation { value["operationId"] = json!(operation); }
    record(value);
}
fn record(mut value: Value) {
    let Some(mutex) = LOGGER.get() else { return; };
    let Ok(mut logger) = mutex.lock() else { return; };
    let Some(object) = value.as_object_mut() else { return; };
    object.retain(|key, value| ["source", "event", "level", "operationId", "occurredAt", "durationMs", "status", "errorType", "stack"].contains(&key.as_str()) && (value.is_string() || value.is_number()));
    for field in object.values_mut() { if let Some(text) = field.as_str() { *field = json!(sanitize(text)); } }
    object.insert("time".into(), json!(now())); object.insert("runId".into(), json!(logger.run));
    let result = (|| -> std::io::Result<()> {
        crate::media::safe_path(&logger.directory).map_err(std::io::Error::other)?;
        fs::create_dir_all(&logger.directory)?;
        let payload = serde_json::to_vec(&value)?;
        let mut path = logger.directory.join(format!("run-{}-{:03}.jsonl", logger.run, logger.part));
        if fs::metadata(&path).is_ok_and(|m| m.len() + payload.len() as u64 + 1 > FILE_LIMIT) { logger.part += 1; path = logger.directory.join(format!("run-{}-{:03}.jsonl", logger.run, logger.part)); }
        crate::media::safe_path(&path).map_err(std::io::Error::other)?;
        let mut options = OpenOptions::new(); options.create(true).append(true); private_file(&mut options);
        let mut file = options.open(path)?; file.write_all(&payload)?; file.write_all(b"\n")?; file.flush()?;
        let mut paths = files(&logger); let mut total: u64 = paths.iter().filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).sum();
        while paths.len() > 20 || total > 50 * 1024 * 1024 { let old = paths.remove(0); total = total.saturating_sub(fs::metadata(&old)?.len()); fs::remove_file(old)?; }
        Ok(())
    })();
    logger.error = if result.is_ok() { String::new() } else { "日志保存失败，请检查磁盘空间及目录权限".into() };
}
pub async fn logged<T, E: std::fmt::Display>(name: &str, operation: Option<String>, future: impl std::future::Future<Output = Result<T, E>>) -> Result<T, E> {
    let operation = operation.filter(|id| uuid::Uuid::parse_str(id).is_ok()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let start = Instant::now(); event(&format!("{name}.start"), "INFO", Some(&operation), None);
    let result = future.await;
    let mut details = json!({"durationMs":start.elapsed().as_millis()});
    if result.is_err() { details["errorType"] = json!("command_failed"); details["stack"] = json!(sanitize(&std::backtrace::Backtrace::force_capture().to_string())); }
    event(&format!("{name}.{}", if result.is_ok() { "success" } else { "failed" }), if result.is_ok() { "INFO" } else { "ERROR" }, Some(&operation), Some(details));
    result
}
#[tauri::command]
pub fn diagnostics_info() -> Result<Value, String> {
    let logger = LOGGER.get().ok_or("日志尚未初始化")?.lock().map_err(|_| "日志锁不可用")?;
    Ok(json!({"runId":logger.run,"directory":logger.directory.to_string_lossy(),"error":logger.error,"version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"fileLimit":FILE_LIMIT}))
}
#[tauri::command]
pub fn diagnostics_write(events: Vec<Value>) -> Result<(), String> {
    if events.len() > 128 || events.iter().any(|v| !v.is_object() || v.to_string().len() > 16384) { return Err("日志批次格式或容量无效".into()); }
    for mut value in events { value["source"] = json!("frontend"); record(value); }
    let info = diagnostics_info()?;
    if info["error"].as_str().is_some_and(|s| !s.is_empty()) { return Err(info["error"].as_str().unwrap_or_default().into()); }
    Ok(())
}
#[tauri::command]
pub fn diagnostics_snapshot() -> Result<String, String> {
    let logger = LOGGER.get().ok_or("日志尚未初始化")?.lock().map_err(|_| "日志锁不可用")?;
    if !logger.error.is_empty() { return Err(logger.error.clone()); }
    let mut chunks = Vec::new(); let mut remaining = FILE_LIMIT as usize;
    for path in files(&logger).into_iter().rev().filter(|p| p.file_name().is_some_and(|name| name.to_string_lossy().contains(&logger.run))) {
        if remaining == 0 { break; } crate::media::safe_path(&path)?;
        let mut raw = fs::read(path).map_err(|_| "日志读取失败")?;
        if raw.len() > remaining { raw = raw.split_off(raw.len() - remaining); if let Some(end) = raw.iter().position(|b| *b == b'\n') { raw.drain(..=end); } }
        remaining -= raw.len(); chunks.push(raw);
    }
    chunks.reverse(); Ok(String::from_utf8_lossy(&chunks.concat()).into_owned())
}
#[tauri::command]
pub fn diagnostics_open(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let directory = app.path().app_data_dir().map_err(|_| "应用目录不可用")?.join("logs");
    crate::media::safe_path(&directory)?;
    app.opener().open_path(directory.to_string_lossy().to_string(), None::<&str>).map_err(|_| "无法打开日志目录".into())
}
