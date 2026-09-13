use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub const MAX_VIDEO_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ZIP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MAP_BYTES: u64 = 64 * 1024 * 1024;
const MISSING: &str = "该备份的视频已清理";

pub struct StagedImport {
    pub token: String,
    pub directory: PathBuf,
    pub project_id: String,
    pub parts: Vec<String>,
    pub previews: Map<String, Value>,
}

pub fn safe_path(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("不允许父目录路径".into());
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("本地存储路径不能包含符号链接".into());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("本地存储路径不能包含重解析点".into());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub fn ensure_directory(path: &Path) -> Result<(), String> {
    safe_path(path)?;
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    safe_path(path)?;
    if !path.is_dir() {
        return Err("本地存储目录无效".into());
    }
    Ok(())
}

pub fn remove_tree(path: &Path) -> Result<(), String> {
    safe_path(path)?;
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        safe_path(&entry.path())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            remove_tree(&entry.path())?;
        } else {
            fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
        }
    }
    fs::remove_dir(path).map_err(|error| error.to_string())
}

pub fn initialize(root: &Path) -> Result<(), String> {
    ensure_directory(&root.join("media"))?;
    let staging = root.join("media/staging");
    remove_tree(&staging)?;
    ensure_directory(&staging)?;
    ensure_directory(&root.join("media/recordings"))
}

pub fn new_stage(root: &Path) -> Result<StagedImport, String> {
    let token = Uuid::new_v4().to_string();
    let directory = root.join("media/staging").join(&token);
    ensure_directory(&directory)?;
    Ok(StagedImport {
        token,
        directory,
        project_id: String::new(),
        parts: Vec::new(),
        previews: Map::new(),
    })
}

pub fn discard(stage: &StagedImport) -> Result<(), String> {
    remove_tree(&stage.directory)
}

pub fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("媒体字段 {field} 无效"))
}

fn integer(value: &Value, field: &str, minimum: u64) -> Result<u64, String> {
    value[field]
        .as_u64()
        .filter(|number| (minimum..=9_007_199_254_740_991).contains(number))
        .ok_or_else(|| format!("媒体字段 {field} 必须是安全整数"))
}

fn names(value: &Value, field: &str) -> Result<Vec<String>, String> {
    let list = value[field]
        .as_array()
        .filter(|list| list.len() <= 100_000)
        .ok_or_else(|| format!("媒体字段 {field} 无效"))?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for name in list {
        let name = name
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or("对话包名无效")?;
        if !seen.insert(name) {
            return Err("对话包名重复".into());
        }
        result.push(name.to_owned());
    }
    Ok(result)
}

pub fn names_hash(names: &[String]) -> Result<String, String> {
    let mut names = names.to_vec();
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    names.dedup();
    Ok(package_hash(
        &serde_json::to_vec(&names).map_err(|error| error.to_string())?,
    ))
}

fn package_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn validate_map(
    map: &Value,
    project: &str,
    part: &str,
    recording: &str,
    video_hash: &str,
) -> Result<(), String> {
    if map["format"] != "mida-localization-preview"
        || map["formatVersion"].as_u64() != Some(1)
        || map["projectId"] != project
        || map["partName"] != part
        || map["recordingId"] != recording
        || !valid_uuid(recording)
        || map["hashAlgorithmVersion"] != "package-names-v1"
        || map["videoSha256"] != video_hash
        || video_hash.len() != 64
        || !video_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || map["frameRate"].as_u64() != Some(30)
    {
        return Err("媒体地图格式、项目/片段/录制身份或视频哈希不一致".into());
    }
    let duration = integer(map, "durationMs", 1)?;
    let catalog = names(map, "packageNames")?;
    let recorded = names(map, "recordedPackageNames")?;
    if map["catalogHash"] != names_hash(&catalog)?
        || map["recordedPackagesHash"] != names_hash(&recorded)?
    {
        return Err("媒体地图对话包哈希不一致".into());
    }
    let catalog: HashSet<_> = catalog.iter().map(String::as_str).collect();
    let recorded: HashSet<_> = recorded.iter().map(String::as_str).collect();
    if !recorded.is_subset(&catalog) {
        return Err("录制对话包不属于地图目录".into());
    }
    let events = map["events"]
        .as_array()
        .filter(|events| events.len() <= 100_000)
        .ok_or("媒体事件列表无效或过多")?;
    let mut occurrences: HashMap<&str, u64> = HashMap::new();
    let mut last = (0, 0);
    for event in events {
        let name = text(event, "packageName")?;
        let occurrence = integer(event, "occurrence", 1)?;
        let frame = integer(event, "frame", 0)?;
        let time = integer(event, "timeMs", 0)?;
        let expected_time = (u128::from(frame) * 1000 + 15) / 30;
        if !recorded.contains(name)
            || occurrence != occurrences.get(name).copied().unwrap_or(0) + 1
            || time >= duration
            || u128::from(frame) * 1000 >= u128::from(duration) * 30
            || u128::from(time).abs_diff(expected_time) > 1
            || frame < last.0
            || time < last.1
        {
            return Err("媒体事件包名、出现序号、时间/帧或时间轴边界无效".into());
        }
        occurrences.insert(name, occurrence);
        last = (frame, time);
    }
    if occurrences.keys().copied().collect::<HashSet<_>>() != recorded {
        return Err("录制包列表与事件不一致".into());
    }
    Ok(())
}

fn part_directory(root: &Path, project: &str, part: &str) -> PathBuf {
    root.join("media/recordings")
        .join(package_hash(project.as_bytes()))
        .join(package_hash(part.as_bytes()))
}

fn recording_directory(
    root: &Path,
    project: &str,
    part: &str,
    media_id: &str,
) -> Result<PathBuf, String> {
    if !valid_uuid(media_id) {
        return Err("媒体标识无效".into());
    }
    let path = part_directory(root, project, part).join(media_id);
    safe_path(&path)?;
    Ok(path)
}

fn read_map(path: &Path) -> Result<Value, String> {
    safe_path(path)?;
    let file = File::open(path).map_err(|_| MISSING.to_owned())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_MAP_BYTES {
        return Err("媒体地图过大".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MAP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MAP_BYTES {
        return Err("媒体地图过大".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn resolve(
    root: &Path,
    project: &str,
    part: &str,
    recording: &str,
    media_id: &str,
) -> Result<(PathBuf, Value), String> {
    if !valid_uuid(recording) {
        return Err("录制标识无效".into());
    }
    let directory = recording_directory(root, project, part, media_id)?;
    let map = read_map(&directory.join("dialogue-map.json"))?;
    if map["projectId"] != project || map["partName"] != part || map["recordingId"] != recording {
        return Err("媒体引用身份不一致".into());
    }
    let video = directory.join("video.mp4");
    safe_path(&video)?;
    if !video.is_file() {
        return Err(MISSING.into());
    }
    Ok((video, map))
}

pub fn validate_references(snapshot: &Value) -> Result<(), String> {
    let Some(previews) = snapshot.get("previews") else {
        return Ok(());
    };
    let previews = previews
        .as_object()
        .filter(|previews| previews.len() <= 1000)
        .ok_or("工作区媒体引用无效")?;
    for (part, preview) in previews {
        if !crate::package::valid_part(part)
            || !valid_uuid(text(preview, "recordingId")?)
            || !valid_uuid(text(preview, "mediaId")?)
        {
            return Err("工作区媒体引用身份无效".into());
        }
        if !preview["map"].is_object()
            || preview["map"]["partName"] != *part
            || preview["map"]["recordingId"] != preview["recordingId"]
        {
            return Err("工作区媒体地图引用无效".into());
        }
        let project = text(snapshot, "currentProjectId")?;
        if !snapshot["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| task["partName"] == *part))
        {
            return Err("工作区媒体引用没有对应任务".into());
        }
        validate_map(
            &preview["map"],
            project,
            part,
            text(preview, "recordingId")?,
            text(&preview["map"], "videoSha256")?,
        )?;
    }
    Ok(())
}

pub fn promote(
    root: &Path,
    stage: &StagedImport,
    snapshot: &mut Value,
) -> Result<Vec<PathBuf>, String> {
    let tasks = snapshot["tasks"].as_array().ok_or("工作区任务无效")?;
    for part in &stage.parts {
        if !tasks.iter().any(|task| task["partName"] == *part) {
            return Err("导入片段不在待保存工作区内".into());
        }
    }
    if snapshot["currentProjectId"].as_str() != Some(stage.project_id.as_str()) {
        return Err("导入项目与工作区不一致".into());
    }
    if snapshot.get("previews").is_none() {
        snapshot["previews"] = json!({});
    }
    let references = snapshot["previews"]
        .as_object_mut()
        .ok_or("工作区媒体引用无效")?;
    for part in &stage.parts {
        references.remove(part);
        if let Some(preview) = stage.previews.get(part) {
            references.insert(part.clone(), preview.clone());
        }
    }
    let mut promoted = Vec::new();
    let result = (|| {
        for (part, preview) in &stage.previews {
            let identifier = text(preview, "mediaId")?;
            let source = stage.directory.join(identifier);
            safe_path(&source)?;
            let destination = recording_directory(root, &stage.project_id, part, identifier)?;
            ensure_directory(destination.parent().ok_or("媒体目录无效")?)?;
            if destination.exists() {
                return Err("媒体目标已存在，未覆盖已有录像".into());
            }
            fs::rename(&source, &destination).map_err(|error| error.to_string())?;
            promoted.push(destination);
        }
        Ok(())
    })();
    if let Err(error) = result {
        rollback(stage, &promoted);
        return Err(error);
    }
    Ok(promoted)
}

pub fn rollback(stage: &StagedImport, promoted: &[PathBuf]) {
    for destination in promoted.iter().rev() {
        if let Some(identifier) = destination.file_name() {
            let source = stage.directory.join(identifier);
            if safe_path(destination).is_ok() && safe_path(&source).is_ok() {
                let _ = fs::rename(destination, source);
            }
        }
    }
}

pub fn cleanup_plan(stage: &StagedImport) -> Value {
    let mut jobs = Map::new();
    for part in &stage.parts {
        let key = format!(
            "{}-{}",
            package_hash(stage.project_id.as_bytes()),
            package_hash(part.as_bytes())
        );
        let keep = stage
            .previews
            .get(part)
            .and_then(|preview| preview["mediaId"].as_str());
        jobs.insert(
            key,
            json!({"projectId":stage.project_id,"partName":part,"mediaId":keep}),
        );
    }
    Value::Object(jobs)
}

pub fn clean_jobs(root: &Path, jobs: &Value) -> Result<(), String> {
    let Some(jobs) = jobs.as_object() else {
        if jobs.is_null() {
            return Ok(());
        }
        return Err("媒体清理任务无效".into());
    };
    let mut errors: Vec<String> = Vec::new();
    for job in jobs.values() {
        let result = (|| {
            let project = text(job, "projectId")?;
            let part = text(job, "partName")?;
            let keep = job["mediaId"].as_str();
            if !crate::package::valid_part(part)
                || (!job["mediaId"].is_null() && !keep.is_some_and(valid_uuid))
            {
                return Err("媒体清理引用无效".into());
            }
            let directory = part_directory(root, project, part);
            safe_path(&directory)?;
            if !directory.exists() {
                return Ok(());
            }
            for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if entry.file_name().to_str() != keep {
                    remove_tree(&entry.path())?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

pub fn response(
    root: &Path,
    grant: Option<&PathBuf>,
    request: &tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};
    let result = (|| -> Result<_, String> {
        let path = grant.ok_or(MISSING)?;
        if !path.starts_with(root.join("media/recordings"))
            || path.file_name().and_then(|name| name.to_str()) != Some("video.mp4")
        {
            return Err("媒体地址未授权".into());
        }
        safe_path(path)?;
        let mut file = File::open(path).map_err(|_| MISSING.to_owned())?;
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        if length == 0 || length > MAX_VIDEO_BYTES {
            return Err("媒体文件大小无效".into());
        }
        let mut builder = Response::builder()
            .header("Content-Type", "video/mp4")
            .header("Accept-Ranges", "bytes")
            .header("Cache-Control", "no-store")
            .header("Access-Control-Allow-Origin", "*")
            .header("X-Content-Type-Options", "nosniff");
        if request.method() == "HEAD" {
            return builder
                .header("Content-Length", length)
                .body(Vec::new())
                .map_err(|error| error.to_string());
        }
        if request.method() != "GET" {
            return Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Vec::new())
                .map_err(|error| error.to_string());
        }
        let header = request
            .headers()
            .get("range")
            .and_then(|header| header.to_str().ok())
            .unwrap_or("bytes=0-");
        let ranges =
            http_range::HttpRange::parse(header, length).map_err(|_| "无效的媒体 Range")?;
        if ranges.len() != 1 {
            return Err("只支持单段媒体 Range".into());
        }
        let start = ranges[0].start;
        let count = ranges[0].length.min(1024 * 1024);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        let mut bytes = vec![0; count as usize];
        file.read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header("Content-Length", count)
            .header(
                "Content-Range",
                format!("bytes {start}-{}/{length}", start + count - 1),
            );
        builder.body(bytes).map_err(|error| error.to_string())
    })();
    result.unwrap_or_else(|error| {
        Response::builder()
            .status(if error.contains("Range") {
                StatusCode::RANGE_NOT_SATISFIABLE
            } else {
                StatusCode::NOT_FOUND
            })
            .header("Cache-Control", "no-store")
            .body(error.into_bytes())
            .unwrap_or_default()
    })
}

pub fn copy_verified(
    reader: &mut impl Read,
    output: &mut impl Write,
    limit: u64,
) -> Result<(String, u64), String> {
    let mut digest = Sha256::new();
    let mut length = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("文件读取或 CRC 校验失败：{error}"))?;
        if count == 0 {
            break;
        }
        length += count as u64;
        if length > limit {
            return Err("文件解压或导出大小超过上限".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
        digest.update(&buffer[..count]);
    }
    Ok((format!("{:x}", digest.finalize()), length))
}
