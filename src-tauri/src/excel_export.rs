use base64::Engine;
use serde_json::{json, Value};
use std::io::Write;
use tauri_plugin_dialog::DialogExt;

const MAX_BYTES: usize = 64 * 1024 * 1024;

#[tauri::command]
pub async fn export_excel(app: tauri::AppHandle, content: String, file_name: String, operation_id: Option<String>) -> Result<Option<Value>, String> {
    crate::diagnostics::logged("export_excel", operation_id, async {
    tauri::async_runtime::spawn_blocking(move || {
        if content.len() > MAX_BYTES.div_ceil(3) * 4 { return Err("Excel 超过 64 MiB，请减少勾选任务".into()); }
        let bytes = base64::engine::general_purpose::STANDARD.decode(content).map_err(|_| "Excel 数据编码无效")?;
        if bytes.len() > MAX_BYTES || !bytes.starts_with(b"PK\x03\x04") { return Err("Excel 数据无效或超过 64 MiB".into()); }
        let safe_name = if file_name.len() <= 160 && file_name.ends_with(".xlsx")
            && file_name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) {
            file_name
        } else {
            "localization.xlsx".into()
        };
        let selected = app.dialog().file().set_title("导出对话 Excel")
            .set_file_name(safe_name).add_filter("Excel 工作簿", &["xlsx"]).blocking_save_file();
        let Some(selected) = selected else { return Ok(None); };
        let path = selected.into_path().map_err(|error| error.to_string())?;
        if !path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx")) {
            return Err("导出文件必须使用 .xlsx 扩展名".into());
        }
        crate::media::safe_path(&path)?;
        let parent = path.parent().ok_or("导出目录无效")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temporary.write_all(&bytes).map_err(|error| error.to_string())?;
        temporary.as_file().sync_all().map_err(|error| error.to_string())?;
        temporary.persist(&path).map_err(|error| error.to_string())?;
        Ok(Some(json!({"path": path.to_string_lossy()})))
    }).await.map_err(|error| error.to_string())?
    }).await
}
