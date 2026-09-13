#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod package;
mod workspace;
mod media;
mod updater;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

#[derive(Default)]
struct NativeState {
    dropped_paths: Mutex<Vec<PathBuf>>,
    allow_exit: AtomicBool,
    workspace_lock: Mutex<()>,
    cache_epoch: AtomicU64,
    media_import: Mutex<Option<media::StagedImport>>,
    media_urls: Mutex<HashMap<String, (PathBuf, PathBuf)>>,
}

fn space_root(app: &tauri::AppHandle, space_id: Option<&str>) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    match space_id.filter(|value| !value.is_empty()) {
        None => Ok(root),
        Some(identifier) => {
            if !valid_space(identifier) { return Err("语言空间标识无效".into()); }
            let scoped = root.join("spaces").join(identifier);
            media::safe_path(&scoped)?;
            Ok(scoped)
        }
    }
}

fn valid_space(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
}

#[tauri::command]
async fn workspace_spaces(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let directory = space_root(&app, None)?.join("spaces");
    media::safe_path(&directory)?;
    if !directory.exists() { return Ok(Vec::new()); }
    let mut spaces = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if valid_space(&name) && entry.file_type().map_err(|error| error.to_string())?.is_dir() { spaces.push(name); }
    }
    spaces.sort();
    Ok(spaces)
}

fn read_selected(app: &tauri::AppHandle, path: PathBuf, space_id: Option<&str>) -> Result<Value, String> {
    if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("zip")) != Some(true) {
        return Err("请选择本地化 ZIP 压缩包".into());
    }
    let state = app.state::<NativeState>();
    let mut pending = state.media_import.try_lock().map_err(|_| "另一个导入正在进行")?;
    if pending.is_some() { return Err("请先确认或取消当前导入".into()); }
    let root = space_root(app, space_id)?;
    let mut stage = media::new_stage(&root)?;
    let mut result = match package::read_package(&path, &mut stage) {
        Ok(result) => result,
        Err(error) => { let _ = media::discard(&stage); return Err(error); }
    };
    result["fileName"] = json!(path.file_name().unwrap_or_default().to_string_lossy());
    *pending = Some(stage);
    Ok(result)
}

#[tauri::command]
async fn choose_package(app: tauri::AppHandle, space_id: Option<String>) -> Result<Option<Value>, String> {
    let selected = app.dialog().file().set_title("导入本地化 ZIP")
        .add_filter("本地化 ZIP", &["zip"]).blocking_pick_file();
    match selected {
        Some(path) => read_selected(&app, path.into_path().map_err(|error| error.to_string())?, space_id.as_deref()).map(Some),
        None => Ok(None),
    }
}

#[tauri::command]
async fn import_dropped_package(app: tauri::AppHandle, space_id: Option<String>) -> Result<Value, String> {
    let paths = std::mem::take(&mut *app.state::<NativeState>().dropped_paths.lock().map_err(|error| error.to_string())?);
    if paths.len() != 1 {
        return Err("请一次拖入一个 ZIP 压缩包".into());
    }
    read_selected(&app, paths.into_iter().next().ok_or("没有待导入的文件")?, space_id.as_deref())
}

#[tauri::command]
async fn export_package(app: tauri::AppHandle, document: Value) -> Result<Option<Value>, String> {
    package::selected_parts(&document)?;
    let package_id = document["packageId"].as_str().ok_or("缺少包标识")?;
    let safe_id: String = package_id.chars().filter(|character| character.is_ascii_alphanumeric() || *character == '-').take(64).collect();
    let selected = app.dialog().file().set_title("导出本地化完成稿")
        .set_file_name(format!("localization.ready.{safe_id}.zip"))
        .add_filter("本地化 ZIP", &["zip"]).blocking_save_file();
    let Some(selected) = selected else { return Ok(None) };
    let path = selected.into_path().map_err(|error| error.to_string())?;
    if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("zip")) != Some(true) {
        return Err("导出文件必须使用 .zip 扩展名".into());
    }
    let parent = path.parent().ok_or("导出目录无效")?;
    media::safe_path(&path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    package::build_package(&document, temporary.as_file_mut())?;
    temporary.as_file().sync_all().map_err(|error| error.to_string())?;
    temporary.persist(&path).map_err(|error| error.to_string())?;
    Ok(Some(json!({ "path": path.to_string_lossy() })))
}

#[tauri::command]
async fn confirm_action(app: tauri::AppHandle, message: String) -> bool {
    app.dialog().message(message).title("MIDA 本地化编辑器")
        .buttons(MessageDialogButtons::OkCancelCustom("恢复备份".into(), "取消".into())).blocking_show()
}

#[tauri::command]
fn finish_exit(app: tauri::AppHandle) {
    app.state::<NativeState>().allow_exit.store(true, Ordering::SeqCst);
    app.exit(0);
}

#[tauri::command]
async fn workspace_read(app: tauri::AppHandle, store: String, key: String, space_id: Option<String>) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let root = space_root(&app, space_id.as_deref())?;
    workspace::read(&root, &store, &key)
}

#[tauri::command]
async fn workspace_save(app: tauri::AppHandle, mut snapshot: Value, expected_revision: u64, backup_reason: Option<String>, media_import_token: Option<String>, space_id: Option<String>, expected_cache_epoch: Option<u64>) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    if expected_cache_epoch.unwrap_or(0) != state.cache_epoch.load(Ordering::SeqCst) { return Err("缓存已被清空，请重新打开工作区；旧数据未重新保存".into()); }
    let root = space_root(&app, space_id.as_deref())?;
    let tasks = snapshot["tasks"].as_array().ok_or("工作区任务无效")?;
    let language = tasks.first().and_then(|task| task["language"].as_str()).ok_or("缺少目标语言")?;
    if tasks.iter().any(|task| task["language"].as_str() != Some(language))
        || space_id.as_deref().filter(|value| !value.is_empty()).is_some_and(|value| value != language) {
        return Err("当前空间只允许保存一种匹配的目标语言".into());
    }
    if let Some(token) = media_import_token {
        let mut pending = state.media_import.lock().map_err(|error| error.to_string())?;
        let stage = pending.as_ref().filter(|stage| stage.token == token).ok_or("导入已取消、过期或 token 无效")?;
        if stage.directory != root.join("media/staging").join(&token) { return Err("导入事务不属于当前语言空间".into()); }
        let current = workspace::read(&root, "workspace", "current")?;
        if current["revision"].as_u64().unwrap_or(0) != expected_revision { return Err("工作区保存版本冲突，未替换旧媒体".into()); }
        let promoted = media::promote(&root, stage, &mut snapshot)?;
        match workspace::save(&root, snapshot, expected_revision, backup_reason, Some(media::cleanup_plan(stage))) {
            Ok(mut result) => {
                if let Err(error) = workspace::finish_media_cleanup(&root) { result["mediaCleanupWarning"] = json!(error); }
                if let Err(error) = media::discard(stage) { result["mediaStagingWarning"] = json!(error); }
                *pending = None;
                if let Ok(mut grants) = state.media_urls.lock() { grants.retain(|_, (_, path)| path.is_file()); }
                Ok(result)
            }
            Err(error) => { media::rollback(stage, &promoted); Err(error) }
        }
    } else {
        let mut result = workspace::save(&root, snapshot, expected_revision, backup_reason, None)?;
        if let Err(error) = workspace::finish_media_cleanup(&root) { result["mediaCleanupWarning"] = json!(error); }
        if let Ok(mut grants) = state.media_urls.lock() { grants.retain(|_, (_, path)| path.is_file()); }
        Ok(result)
    }
}

#[tauri::command]
fn workspace_cache_epoch(app: tauri::AppHandle) -> u64 {
    app.state::<NativeState>().cache_epoch.load(Ordering::SeqCst)
}

#[tauri::command]
async fn clear_all_cache(app: tauri::AppHandle, confirmed: bool) -> Result<(), String> {
    if !confirmed { return Err("清空操作尚未确认".into()); }
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let mut pending = state.media_import.lock().map_err(|error| error.to_string())?;
    let root = space_root(&app, None)?;
    for name in ["workspace", "spaces", "media"] { media::safe_path(&root.join(name))?; }
    state.cache_epoch.fetch_add(1, Ordering::SeqCst);
    *pending = None;
    state.media_urls.lock().map_err(|error| error.to_string())?.clear();
    for name in ["workspace", "spaces", "media"] { media::remove_tree(&root.join(name))?; }
    media::initialize(&root)?;
    Ok(())
}

#[tauri::command]
async fn relocate_media_import(app: tauri::AppHandle, token: String, space_id: String) -> Result<(), String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let mut pending = state.media_import.lock().map_err(|error| error.to_string())?;
    let stage = pending.as_mut().filter(|stage| stage.token == token).ok_or("导入事务已取消或过期")?;
    let root = space_root(&app, Some(&space_id))?;
    let destination = root.join("media/staging").join(&token);
    if destination == stage.directory { return Ok(()); }
    media::safe_path(&stage.directory)?;
    media::safe_path(&destination)?;
    if destination.exists() { return Err("目标空间已有同名导入事务".into()); }
    media::ensure_directory(&root.join("media/staging"))?;
    std::fs::rename(&stage.directory, &destination).map_err(|error| error.to_string())?;
    stage.directory = destination;
    Ok(())
}

#[tauri::command]
async fn discard_media_import(app: tauri::AppHandle, token: String) -> Result<(), String> {
    let state = app.state::<NativeState>();
    let mut pending = state.media_import.lock().map_err(|error| error.to_string())?;
    if let Some(stage) = pending.as_ref() {
        if stage.token != token { return Err("导入 token 无效".into()); }
        media::discard(stage)?;
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
async fn get_preview_media(app: tauri::AppHandle, project_id: String, part_name: String, recording_id: String, media_id: String, space_id: Option<String>) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let root = space_root(&app, space_id.as_deref())?;
    let (path, map) = media::resolve(&root, &project_id, &part_name, &recording_id, &media_id)?;
    let mut grants = state.media_urls.lock().map_err(|error| error.to_string())?;
    let identifier = grants.iter().find(|(_, (_, stored))| stored == &path).map(|(identifier, _)| identifier.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if grants.len() > 1000 { grants.retain(|_, (_, path)| path.is_file()); }
    grants.insert(identifier.clone(), (root, path));
    let prefix = if cfg!(target_os = "windows") { "http://mida-media.localhost" } else { "mida-media://localhost" };
    Ok(json!({"url":format!("{prefix}/{identifier}/video.mp4"),"map":map}))
}

fn main() {
    let app = tauri::Builder::default()
        .manage(NativeState::default())
        .manage(updater::UpdateState::default())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            media::initialize(&root).map_err(std::io::Error::other)?;
            if let Err(error) = workspace::finish_media_cleanup(&root) { eprintln!("媒体缓存清理将在后续导入重试：{error}"); }
            let spaces = root.join("spaces");
            media::safe_path(&spaces).map_err(std::io::Error::other)?;
            if spaces.is_dir() {
                for entry in std::fs::read_dir(spaces)? {
                    let entry = entry?;
                    if valid_space(&entry.file_name().to_string_lossy()) && entry.file_type()?.is_dir() {
                        media::initialize(&entry.path()).map_err(std::io::Error::other)?;
                        if let Err(error) = workspace::finish_media_cleanup(&entry.path()) { eprintln!("语言空间媒体清理将在保存时重试：{error}"); }
                    }
                }
            }
            Ok(())
        })
        .register_uri_scheme_protocol("mida-media", |context, request| {
            let app = context.app_handle();
            let state = app.state::<NativeState>();
            let path = request.uri().path();
            let identifier = path.strip_prefix('/').and_then(|path| path.strip_suffix("/video.mp4"));
            let grant = state.media_urls.lock().ok().and_then(|grants| identifier.and_then(|identifier| grants.get(identifier).cloned()));
            match grant {
                Some((root, path)) => media::response(&root, Some(&path), &request),
                None => tauri::http::Response::builder().status(404).body(Vec::new()).unwrap_or_default(),
            }
        })
        .invoke_handler(tauri::generate_handler![choose_package, import_dropped_package, export_package, confirm_action, finish_exit, workspace_read, workspace_save, workspace_spaces, workspace_cache_epoch, clear_all_cache, relocate_media_import, discard_media_import, get_preview_media, updater::repository_history, updater::check_app_update, updater::install_app_update])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                if let Ok(mut dropped) = window.state::<NativeState>().dropped_paths.lock() {
                    *dropped = paths.clone();
                }
                let _ = window.emit("mida-package-drop", ());
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if !window.state::<NativeState>().allow_exit.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.emit("mida-exit-requested", ());
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("无法启动本地化编辑器");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !app.state::<NativeState>().allow_exit.load(Ordering::SeqCst) {
                api.prevent_exit();
                let _ = app.emit("mida-exit-requested", ());
            }
        }
    });
}
