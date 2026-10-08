use base64::Engine;
use serde_json::{json, Value};
const BASE: &str = "https://server.nanzhaigame.cn:8020/cnb-feedback/v1/public-feedback/mida-localization/requests";

#[tauri::command]
pub fn feedback_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = reqwest::Url::parse(&url).map_err(|_| "反馈地址无效")?;
    let issue = parsed.path().strip_prefix("/nanzhaigame-xpy/MIDALocalizationTool/-/issues/")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
    let asset = parsed.path().starts_with("/-/imgs/issues/") || parsed.path().starts_with("/-/files/issues/");
    if parsed.scheme() != "https" || parsed.host_str() != Some("cnb.cool") || parsed.port().is_some_and(|p| p != 443)
        || !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() || !(issue || asset) {
        return Err("仅允许打开本项目反馈或 CNB 附件".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|_| "无法打开反馈链接".into())
}

#[tauri::command]
pub async fn feedback_request(action: String, id: String, key: String, data: Option<Value>, operation_id: Option<String>) -> Result<Value, String> {
    crate::diagnostics::logged("feedback_request", operation_id, async {
        if !uuid::Uuid::parse_str(&id).is_ok_and(|identity| identity.get_version_num() == 4 && identity.to_string() == id) || key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) { return Err("反馈请求身份无效".into()); }
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(40)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "反馈连接初始化失败")?;
        let mut path = format!("{BASE}/{id}");
        let method;
        let mut body = None;
        let mut content = "application/json";
        match action.as_str() {
            "create" => {
                let data = data.ok_or("缺少反馈内容")?;
                if data["id"].as_str() != Some(&id) { return Err("反馈身份不一致".into()); }
                let bytes = serde_json::to_vec(&data).map_err(|_| "反馈编码失败")?;
                if bytes.len() > 65536 { return Err("反馈文字过长".into()); }
                body = Some(bytes); path = BASE.into(); method = reqwest::Method::POST;
            }
            "upload" => {
                let data = data.ok_or("缺少附件")?; let aid = data["attachmentId"].as_str().ok_or("缺少附件身份")?;
                if !uuid::Uuid::parse_str(aid).is_ok_and(|identity| identity.to_string() == aid) { return Err("附件身份无效".into()); }
                let encoded = data["bytes"].as_str().ok_or("缺少附件字节")?;
                if encoded.len() > 14 * 1024 * 1024 { return Err("附件过大".into()); }
                let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "附件编码无效")?;
                if bytes.is_empty() || bytes.len() > 10 * 1024 * 1024 { return Err("附件大小无效".into()); }
                body = Some(bytes); path.push_str(&format!("/attachments/{aid}")); method = reqwest::Method::PUT; content = "application/octet-stream";
            }
            "commit" | "abort" => { path.push_str(&format!("/{action}")); method = reqwest::Method::POST; body = Some(b"{}".to_vec()); }
            "get" => { method = reqwest::Method::GET; }
            _ => { return Err("不支持的反馈操作".into()); }
        }
        let mut request = client.request(method, path).header("X-Feedback-Key", key).header("Accept", "application/json").header("Content-Type", content);
        if let Some(body) = body { request = request.body(body); }
        let mut response = request.send().await.map_err(|_| "反馈响应未确认，请核对原请求")?;
        let status = response.status().as_u16(); let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "反馈响应中断，请核对原请求")? { if bytes.len() + chunk.len() > 512 * 1024 { return Err("反馈响应过大，请核对原请求".into()); } bytes.extend_from_slice(&chunk); }
        let result: Value = serde_json::from_slice(&bytes).map_err(|_| "反馈结果未确认，请核对原请求")?;
        if !result.is_object() { return Err("反馈结果无效，请核对原请求".into()); }
        Ok(json!({"status":status,"result":result}))
    }).await
}
