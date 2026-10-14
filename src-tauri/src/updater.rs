use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const REPOSITORY: &str = "https://github.com/UnityX103/MIDALocalizationTool";
const CNB_RELEASES: &str = "https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool/-/releases?page=1&page_size=100";
const GITHUB_RELEASES: &str = "https://api.github.com/repos/UnityX103/MIDALocalizationTool/releases?per_page=100";
async fn bounded_metadata(mut response: reqwest::Response) -> Result<Vec<u8>, MetadataError> {
    const LIMIT: usize = 4 * 1024 * 1024;
    if response.content_length().is_some_and(|length| length > LIMIT as u64) { return Err(MetadataError::Invalid("版本清单超过 4 MiB 上限".into())); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| MetadataError::Unavailable(error.to_string()))? {
        if chunk.len() > LIMIT.saturating_sub(bytes.len()) { return Err(MetadataError::Invalid("版本清单超过 4 MiB 上限".into())); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn fetch_releases(client: &reqwest::Client, endpoint: &str) -> Result<Vec<Value>, String> {
    let response = client.get(endpoint).header("Accept", "application/json").send().await
        .map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?;
    serde_json::from_slice(&bounded_metadata(response).await.map_err(|error| match error { MetadataError::Unavailable(message) | MetadataError::Invalid(message) => message })?).map_err(|error| format!("版本清单无效：{error}"))
}

#[tauri::command]
pub async fn repository_history(operation_id: Option<String>) -> Result<Value, String> {
    crate::diagnostics::logged("repository_history", operation_id, async {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20))
        .user_agent("MIDA-Localization-About").build().map_err(|error| error.to_string())?;
    let mut releases = match fetch_releases(&client, GITHUB_RELEASES).await {
        Ok(releases) => releases,
        Err(_) => fetch_releases(&client, CNB_RELEASES).await?,
    };
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

    }).await
}

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<PendingUpdate>>,
    installing: AtomicBool,
}

fn release_url(value: &str, source: &str, tag: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|error| error.to_string())?;
    let (host, prefix) = if source == "cnb" {
        ("cnb.cool", format!("/nanzhaigame-xpy/MIDALocalizationTool/-/releases/download/{tag}/"))
    } else {
        ("github.com", format!("/UnityX103/MIDALocalizationTool/releases/download/{tag}/"))
    };
    if url.scheme() != "https" || url.host_str() != Some(host) || !url.path().starts_with(&prefix)
        || !url.username().is_empty() || url.password().is_some() {
        return Err("更新地址不属于本项目的对应版本与更新源".into());
    }
    Ok(url)
}

#[derive(Clone)]
struct PendingUpdate {
    update: Update,
    metadata_source: &'static str,
    tag: String,
    asset_name: String,
}

// Only availability failures allow a metadata fallback. Invalid metadata must not
// replace an already discovered version or policy with an older mirror release.
enum MetadataError {
    Unavailable(String),
    Invalid(String),
}

fn update_failure(details: impl AsRef<str>) -> String {
    format!("更新失败，请等待国内节点同步后重试。{}", details.as_ref())
}

async fn update_releases(client: &reqwest::Client, source: &str) -> Result<Vec<Value>, MetadataError> {
    let endpoint = if source == "github" { GITHUB_RELEASES } else { CNB_RELEASES };
    let response = client.get(endpoint).header("Accept", "application/json").send().await
        .map_err(|error| MetadataError::Unavailable(error.to_string()))?
        .error_for_status().map_err(|error| MetadataError::Unavailable(error.to_string()))?;
    let bytes = bounded_metadata(response).await?;
    serde_json::from_slice(&bytes).map_err(|error| MetadataError::Invalid(format!("版本列表无效：{error}")))
}

fn release_version(release: &Value) -> Option<semver::Version> {
    if release["draft"] == true || release["prerelease"] == true { return None; }
    let tag = release["tag_name"].as_str()?;
    let version = semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

fn asset_url(release: &Value, source: &str, name: &str) -> Result<reqwest::Url, String> {
    let tag = release["tag_name"].as_str().ok_or("缺少版本标签")?;
    let asset = release["assets"].as_array().and_then(|assets| assets.iter().find(|asset| asset["name"] == name))
        .ok_or_else(|| format!("该版本尚未提供 {name}"))?;
    let address = asset["browser_download_url"].as_str().or(asset["brower_download_url"].as_str())
        .ok_or("缺少资源下载地址")?;
    let url = release_url(address, source, tag)?;
    if url.path_segments().and_then(|mut segments| segments.next_back()) != Some(name)
        || url.query().is_some() || url.fragment().is_some() {
        return Err("更新资源名称或地址不匹配".into());
    }
    Ok(url)
}

async fn read_update(app: &tauri::AppHandle, release: &Value, source: &'static str, version: &semver::Version) -> Result<PendingUpdate, MetadataError> {
    let endpoint = asset_url(release, source, "latest.json").map_err(MetadataError::Invalid)?;
    let mut update = app.updater_builder().endpoints(vec![endpoint])
        .map_err(|error| MetadataError::Invalid(error.to_string()))?
        .timeout(Duration::from_secs(20))
        .configure_client(|client| client.connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(30)))
        .build().map_err(|error| MetadataError::Invalid(error.to_string()))?
        .check().await.map_err(|error| {
            let unavailable = match &error {
                tauri_plugin_updater::Error::Reqwest(error) => !error.is_decode() && !error.is_builder(),
                tauri_plugin_updater::Error::Network(_) | tauri_plugin_updater::Error::ReleaseNotFound => true,
                _ => false,
            };
            if unavailable { MetadataError::Unavailable(error.to_string()) }
            else { MetadataError::Invalid(format!("更新描述校验失败：{error}")) }
        })?.ok_or_else(|| MetadataError::Invalid("新版更新描述不可用".into()))?;
    if update.version != version.to_string() {
        return Err(MetadataError::Invalid("Release 版本与更新描述不一致".into()));
    }
    let tag = release["tag_name"].as_str().ok_or_else(|| MetadataError::Invalid("缺少版本标签".into()))?;
    release_url(update.download_url.as_str(), source, tag).map_err(MetadataError::Invalid)?;
    let asset_name = if cfg!(target_os = "macos") {
        format!("MIDA-Localization-{version}-macOS-universal.app.tar.gz")
    } else if cfg!(target_os = "windows") {
        format!("MIDA-Localization-{version}-Windows-x64-setup.exe")
    } else { return Err(MetadataError::Invalid("当前平台没有可用的更新安装包".into())); };
    if update.download_url.path_segments().and_then(|mut segments| segments.next_back()) != Some(asset_name.as_str())
        || update.download_url.query().is_some() || update.download_url.fragment().is_some() || update.signature.trim().is_empty() {
        return Err(MetadataError::Invalid("更新安装包名称、地址或签名无效".into()));
    }
    if update.raw_json.get("mandatory").is_some_and(|value| !value.is_boolean()) {
        return Err(MetadataError::Invalid("更新策略 mandatory 必须是布尔值".into()));
    }
    update.timeout = Some(Duration::from_secs(600));
    Ok(PendingUpdate { update, metadata_source: source, tag: tag.into(), asset_name })
}

#[tauri::command]
pub async fn check_app_update(app: tauri::AppHandle, operation_id: Option<String>) -> Result<Value, String> {
    crate::diagnostics::logged("check_app_update", operation_id, async {
    let current = app.package_info().version.clone();
    if cfg!(debug_assertions) { return Ok(json!({"available":false,"current":current.to_string(),"development":true})); }
    let state = app.state::<UpdateState>();
    if state.installing.load(Ordering::SeqCst) { return Err("正在安装更新".into()); }
    *state.pending.lock().map_err(|error| error.to_string())? = None;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).connect_timeout(Duration::from_secs(10))
        .user_agent("MIDA-Localization-Updater").build().map_err(|error| error.to_string())?;
    let mut known_latest = None;
    let mut errors = Vec::new();
    for source in ["github", "cnb"] {
        let result: Result<Option<PendingUpdate>, MetadataError> = async {
            let releases = update_releases(&client, source).await?;
            let selected = releases.iter().filter_map(|release| release_version(release).map(|version| (version, release)))
                .filter(|(version, _)| known_latest.as_ref().is_none_or(|expected| expected == version))
                .max_by(|left, right| left.0.cmp(&right.0));
            let Some((version, release)) = selected else {
                return if known_latest.is_some() { Err(MetadataError::Unavailable("国内节点尚未同步目标版本".into())) } else { Ok(None) };
            };
            if version <= current { return Ok(None); }
            known_latest = Some(version.clone());
            read_update(&app, release, source, &version).await.map(Some)
        }.await;
        match result {
            Ok(Some(pending)) => {
                let info = json!({"available":true,"current":current.to_string(),"version":pending.update.version,
                    "notes":pending.update.body,"mandatory":pending.update.raw_json["mandatory"].as_bool().unwrap_or(false),
                    "source":source});
                *state.pending.lock().map_err(|error| error.to_string())? = Some(pending);
                return Ok(info);
            }
            Ok(None) => return Ok(json!({"available":false,"current":current.to_string()})),
            Err(MetadataError::Unavailable(error)) => errors.push(format!("{source}：{error}")),
            Err(MetadataError::Invalid(error)) => return Err(update_failure(format!("{source}：{error}"))),
        }
    }
    Err(update_failure(errors.join("；")))

    }).await
}

async fn resource_url(client: &reqwest::Client, pending: &PendingUpdate, source: &str) -> Result<reqwest::Url, String> {
    if source == pending.metadata_source { return Ok(pending.update.download_url.clone()); }
    let releases = update_releases(client, source).await.map_err(|error| match error {
        MetadataError::Unavailable(message) | MetadataError::Invalid(message) => message,
    })?;
    let release = releases.iter().find(|release| release["tag_name"] == pending.tag && release_version(release).is_some())
        .ok_or("该下载源尚未提供目标版本")?;
    asset_url(release, source, &pending.asset_name)
}

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle, operation_id: Option<String>) -> Result<(), String> {
    crate::diagnostics::logged("install_app_update", operation_id, async {
    if cfg!(debug_assertions) { return Err("开发版不执行应用更新".into()); }
    let state = app.state::<UpdateState>();
    if state.installing.swap(true, Ordering::SeqCst) { return Err("更新已在进行中".into()); }
    let result = async {
        let pending = state.pending.lock().map_err(|error| error.to_string())?.clone().ok_or("请先检查更新")?;
        let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).connect_timeout(Duration::from_secs(10))
            .user_agent("MIDA-Localization-Updater").build().map_err(|error| error.to_string())?;
        let mut errors = Vec::new();
        let mut downloaded_update = None;
        for source in ["cnb", "github"] {
            let _ = app.emit("mida-update-progress", json!({"source":source,"phase":"connecting","downloaded":0,"total":null}));
            let mut update = pending.update.clone();
            let attempt: Result<Vec<u8>, String> = async {
                update.download_url = resource_url(&client, &pending, source).await?;
                let mut downloaded = 0_u64;
                // Keep the selected descriptor's signature for both mirrors.
                update.download(|chunk, total| {
                    downloaded += chunk as u64;
                    let _ = app.emit("mida-update-progress", json!({"source":source,"phase":"downloading","downloaded":downloaded,"total":total}));
                }, || { let _ = app.emit("mida-update-progress", json!({"source":source,"phase":"verifying"})); })
                    .await.map_err(|error| format!("下载或签名校验失败：{error}"))
            }.await;
            match attempt {
                Ok(bytes) => { downloaded_update = Some((update, bytes)); break; }
                Err(error) => errors.push(format!("{source}：{error}")),
            }
        }
        let (update, bytes) = downloaded_update.ok_or_else(|| update_failure(errors.join("；")))?;
        let _ = app.emit("mida-update-progress", json!({"phase":"installing"}));
        update.install(bytes).map_err(|error| format!("安装失败，现有工作区保留：{error}"))?;
        app.state::<crate::NativeState>().allow_exit.store(true, Ordering::SeqCst);
        app.restart();
        #[allow(unreachable_code)]
        Ok(())
    }.await;
    state.installing.store(false, Ordering::SeqCst);
    result

    }).await
}
