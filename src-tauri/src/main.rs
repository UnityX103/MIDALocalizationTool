#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod package;
mod workspace;
mod media;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

#[derive(Default)]
struct NativeState {
    dropped_paths: Mutex<Vec<PathBuf>>,
    allow_exit: AtomicBool,
    workspace_lock: Mutex<()>,
    media_import: Mutex<Option<media::StagedImport>>,
    media_urls: Mutex<HashMap<String, PathBuf>>,
}

fn read_selected(app: &tauri::AppHandle, path: PathBuf) -> Result<Value, String> {
    if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("zip")) != Some(true) {
        return Err("请选择本地化 ZIP 压缩包".into());
    }
    let state = app.state::<NativeState>();
    let mut pending = state.media_import.try_lock().map_err(|_| "另一个导入正在进行")?;
    if pending.is_some() { return Err("请先确认或取消当前导入".into()); }
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?;
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
async fn choose_package(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let selected = app.dialog().file().set_title("导入本地化 ZIP")
        .add_filter("本地化 ZIP", &["zip"]).blocking_pick_file();
    match selected {
        Some(path) => read_selected(&app, path.into_path().map_err(|error| error.to_string())?).map(Some),
        None => Ok(None),
    }
}

#[tauri::command]
async fn import_dropped_package(app: tauri::AppHandle) -> Result<Value, String> {
    let paths = std::mem::take(&mut *app.state::<NativeState>().dropped_paths.lock().map_err(|error| error.to_string())?);
    if paths.len() != 1 {
        return Err("请一次拖入一个 ZIP 压缩包".into());
    }
    read_selected(&app, paths.into_iter().next().ok_or("没有待导入的文件")?)
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
async fn workspace_read(app: tauri::AppHandle, store: String, key: String) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    workspace::read(&root, &store, &key)
}

#[tauri::command]
async fn workspace_save(app: tauri::AppHandle, mut snapshot: Value, expected_revision: u64, backup_reason: Option<String>, media_import_token: Option<String>) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    if let Some(token) = media_import_token {
        let mut pending = state.media_import.lock().map_err(|error| error.to_string())?;
        let stage = pending.as_ref().filter(|stage| stage.token == token).ok_or("导入已取消、过期或 token 无效")?;
        let current = workspace::read(&root, "workspace", "current")?;
        if current["revision"].as_u64().unwrap_or(0) != expected_revision { return Err("工作区保存版本冲突，未替换旧媒体".into()); }
        let promoted = media::promote(&root, stage, &mut snapshot)?;
        match workspace::save(&root, snapshot, expected_revision, backup_reason, Some(media::cleanup_plan(stage))) {
            Ok(mut result) => {
                if let Err(error) = workspace::finish_media_cleanup(&root) { result["mediaCleanupWarning"] = json!(error); }
                if let Err(error) = media::discard(stage) { result["mediaStagingWarning"] = json!(error); }
                *pending = None;
                if let Ok(mut grants) = state.media_urls.lock() { grants.retain(|_, path| path.is_file()); }
                Ok(result)
            }
            Err(error) => { media::rollback(stage, &promoted); Err(error) }
        }
    } else {
        let mut result = workspace::save(&root, snapshot, expected_revision, backup_reason, None)?;
        if let Err(error) = workspace::finish_media_cleanup(&root) { result["mediaCleanupWarning"] = json!(error); }
        if let Ok(mut grants) = state.media_urls.lock() { grants.retain(|_, path| path.is_file()); }
        Ok(result)
    }
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
async fn get_preview_media(app: tauri::AppHandle, project_id: String, part_name: String, recording_id: String, media_id: String) -> Result<Value, String> {
    let state = app.state::<NativeState>();
    let _guard = state.workspace_lock.lock().map_err(|error| error.to_string())?;
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let (path, map) = media::resolve(&root, &project_id, &part_name, &recording_id, &media_id)?;
    let mut grants = state.media_urls.lock().map_err(|error| error.to_string())?;
    let identifier = grants.iter().find(|(_, stored)| *stored == &path).map(|(identifier, _)| identifier.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if grants.len() > 1000 { grants.retain(|_, path| path.is_file()); }
    grants.insert(identifier.clone(), path);
    let prefix = if cfg!(target_os = "windows") { "http://mida-media.localhost" } else { "mida-media://localhost" };
    Ok(json!({"url":format!("{prefix}/{identifier}/video.mp4"),"map":map}))
}

fn main() {
    let app = tauri::Builder::default()
        .manage(NativeState::default())
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
            Ok(())
        })
        .register_uri_scheme_protocol("mida-media", |context, request| {
            let app = context.app_handle();
            let state = app.state::<NativeState>();
            let path = request.uri().path();
            let identifier = path.strip_prefix('/').and_then(|path| path.strip_suffix("/video.mp4"));
            let grant = state.media_urls.lock().ok().and_then(|grants| identifier.and_then(|identifier| grants.get(identifier).cloned()));
            match app.path().app_data_dir() {
                Ok(root) => media::response(&root, grant.as_ref(), &request),
                Err(_) => tauri::http::Response::builder().status(404).body(Vec::new()).unwrap_or_default(),
            }
        })
        .invoke_handler(tauri::generate_handler![choose_package, import_dropped_package, export_package, confirm_action, finish_exit, workspace_read, workspace_save, discard_media_import, get_preview_media])
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
