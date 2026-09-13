use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const REPOSITORY: &str = "https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool";

#[tauri::command]
pub async fn repository_history() -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20))
        .user_agent("MIDA-Localization-About").build().map_err(|error| error.to_string())?;
    let mut releases: Vec<Value> = client.get(format!("{REPOSITORY}/-/releases?page=1&page_size=100"))
        .header("Accept", "application/vnd.cnb.api+json").send().await
        .map_err(|error| format!("无法连接 CNB：{error}"))?.error_for_status().map_err(|error| error.to_string())?
        .json().await.map_err(|error| format!("版本历史无效：{error}"))?;
    releases.retain(|release| release["draft"] != true && release["prerelease"] != true && release["tag_name"].is_string());
    releases.sort_by(|left, right| {
        let date = |release: &Value| release["published_at"].as_str().or(release["created_at"].as_str()).unwrap_or("").to_owned();
        date(right).cmp(&date(left))
    });
    let history: Vec<Value> = releases.iter().take(5).map(|release| json!({
        "tag": release["tag_name"], "name": release["name"],
        "publishedAt": release["published_at"], "notes": release["body"]
    })).collect();
    Ok(json!({"repository": REPOSITORY, "releases": history}))
}

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
    installing: AtomicBool,
}

fn release_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|error| error.to_string())?;
    if url.scheme() != "https" || url.host_str() != Some("cnb.cool")
        || !url.path().starts_with("/nanzhaigame-xpy/MIDALocalizationTool/-/releases/") {
        return Err("更新地址不属于本项目的 CNB Release".into());
    }
    Ok(url)
}

#[tauri::command]
pub async fn check_app_update(app: tauri::AppHandle) -> Result<Value, String> {
    let current = app.package_info().version.clone();
    if cfg!(debug_assertions) { return Ok(json!({"available":false,"current":current.to_string(),"development":true})); }
    let state = app.state::<UpdateState>();
    if state.installing.load(Ordering::SeqCst) { return Err("正在安装更新".into()); }
    *state.pending.lock().map_err(|error| error.to_string())? = None;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20))
        .user_agent("MIDA-Localization-Updater").build().map_err(|error| error.to_string())?;
    let releases: Vec<Value> = client.get(format!("{REPOSITORY}/-/releases?page=1&page_size=100"))
        .header("Accept", "application/vnd.cnb.api+json").send().await
        .map_err(|error| format!("无法连接 CNB：{error}"))?.error_for_status().map_err(|error| error.to_string())?
        .json().await.map_err(|error| format!("版本清单无效：{error}"))?;
    let mut candidates = releases.iter().filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let version = semver::Version::parse(release["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
            if !version.pre.is_empty() { return None; }
            Some((version, release))
        }).collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((version, release)) = candidates.first() else { return Ok(json!({"available":false,"current":current.to_string()})); };
    if version <= &current { return Ok(json!({"available":false,"current":current.to_string()})); }
    let asset = release["assets"].as_array().and_then(|assets| assets.iter().find(|asset| asset["name"] == "latest.json"))
        .ok_or("新版尚未提供签名更新清单，请稍后重试")?;
    let endpoint = release_url(asset["brower_download_url"].as_str().ok_or("缺少更新清单下载地址")?)?;
    let update = app.updater_builder().endpoints(vec![endpoint]).map_err(|error| error.to_string())?
        .timeout(Duration::from_secs(20)).build().map_err(|error| error.to_string())?
        .check().await.map_err(|error| format!("更新校验失败：{error}"))?;
    if let Some(mut update) = update {
        if update.version != version.to_string() { return Err("Release 版本与更新清单不一致".into()); }
        release_url(update.download_url.as_str())?;
        update.timeout = Some(Duration::from_secs(600));
        let mandatory = match update.raw_json.get("mandatory") {
            Some(value) => value.as_bool().ok_or("更新策略 mandatory 必须是布尔值")?,
            None => false,
        };
        let info = json!({"available":true,"current":current.to_string(),"version":update.version,"notes":update.body,"mandatory":mandatory});
        *state.pending.lock().map_err(|error| error.to_string())? = Some(update);
        Ok(info)
    } else { Ok(json!({"available":false,"current":current.to_string()})) }
}

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) { return Err("开发版不执行应用更新".into()); }
    let state = app.state::<UpdateState>();
    if state.installing.swap(true, Ordering::SeqCst) { return Err("更新已在进行中".into()); }
    let result = async {
        let update = state.pending.lock().map_err(|error| error.to_string())?.clone().ok_or("请先检查更新")?;
        let mut downloaded = 0_u64;
        let bytes = update.download(|chunk, total| {
            downloaded += chunk as u64;
            let _ = app.emit("mida-update-progress", json!({"downloaded":downloaded,"total":total}));
        }, || {}).await.map_err(|error| format!("下载或签名校验失败：{error}"))?;
        update.install(bytes).map_err(|error| format!("安装失败，现有工作区保留：{error}"))?;
        app.state::<crate::NativeState>().allow_exit.store(true, Ordering::SeqCst);
        app.restart();
        #[allow(unreachable_code)]
        Ok(())
    }.await;
    state.installing.store(false, Ordering::SeqCst);
    result
}
