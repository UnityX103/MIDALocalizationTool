use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

const MAX_BYTES: usize = 32 * 1024 * 1024;

fn validate_export(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES { return Err("原文 JSON 超过 32 MiB，请减少勾选任务".into()); }
    let document: Value = serde_json::from_str(text).map_err(|_| "原文 JSON 无效")?;
    if document["format"] != "mida-localization-translations" || document["formatVersion"] != 1 {
        return Err("原文 JSON 格式无效".into());
    }
    Ok(())
}

#[tauri::command]
pub fn copy_translation_json(app: tauri::AppHandle, text: String) -> Result<(), String> {
    validate_export(&text)?;
    app.clipboard().write_text(text).map_err(|error| format!("复制失败：{error}"))
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json")) {
        return Err("请选择 .json 文件".into());
    }
    crate::media::safe_path(path)
}

#[tauri::command]
pub async fn choose_translation_json(app: tauri::AppHandle, operation_id: Option<String>) -> Result<Option<Value>, String> {
    crate::diagnostics::logged("choose_translation_json", operation_id, async {
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app.dialog().file().set_title("导入候选译文 JSON")
            .add_filter("译文 JSON", &["json"]).blocking_pick_file();
        let Some(selected) = selected else { return Ok(None); };
        let path = selected.into_path().map_err(|error| error.to_string())?;
        validate_path(&path)?;
        let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_BYTES as u64 {
            return Err("译文 JSON 超过 32 MiB 上限".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BYTES { return Err("译文 JSON 超过 32 MiB 上限".into()); }
        let text = String::from_utf8(bytes).map_err(|_| "译文 JSON 必须使用 UTF-8 编码")?;
        Ok(Some(json!({"fileName": path.file_name().unwrap_or_default().to_string_lossy(), "text": text})))
    }).await.map_err(|error| error.to_string())?

    }).await
}

#[tauri::command]
pub async fn choose_translation_directory(app: tauri::AppHandle, operation_id: Option<String>) -> Result<Option<String>, String> {
    crate::diagnostics::logged("choose_translation_directory", operation_id, async {
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app.dialog().file().set_title("选择原文导出目录").blocking_pick_folder();
        let Some(selected) = selected else { return Ok(None); };
        let path = selected.into_path().map_err(|error| error.to_string())?;
        crate::media::safe_path(&path)?;
        if !path.is_dir() { return Err("请选择有效的导出目录".into()); }
        Ok(Some(path.to_string_lossy().into_owned()))
    }).await.map_err(|error| error.to_string())?

    }).await
}

#[tauri::command]
pub async fn export_translation_json(text: String, file_name: String, directory: String, operation_id: Option<String>) -> Result<Option<Value>, String> {
    crate::diagnostics::logged("export_translation_json", operation_id, async {
    tauri::async_runtime::spawn_blocking(move || {
        validate_export(&text)?;
        let safe_name = if file_name.len() <= 160 && file_name.ends_with(".json")
            && file_name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) {
            file_name
        } else {
            "untranslated.json".into()
        };
        let parent = Path::new(&directory);
        crate::media::safe_path(parent)?;
        if !parent.is_absolute() || !parent.is_dir() { return Err("导出目录不存在，请重新选择".into()); }
        let path = parent.join(safe_name);
        validate_path(&path)?;
        let parent = path.parent().ok_or("导出目录无效")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temporary.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
        temporary.as_file().sync_all().map_err(|error| error.to_string())?;
        temporary.persist_noclobber(&path).map_err(|error| error.to_string())?;
        Ok(Some(json!({"path": path.to_string_lossy()})))
    }).await.map_err(|error| error.to_string())?

    }).await
}
