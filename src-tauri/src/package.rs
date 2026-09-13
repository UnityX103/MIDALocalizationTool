use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PARTS: usize = 100;
const MAX_TASKS: usize = 1000;
const MAX_ENTRIES: usize = 100_000;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const IDENTITY_FIELDS: [&str; 4] = ["projectId", "packageId", "fileVersion", "exportedAt"];

pub fn read_package(path: &Path, stage: &mut crate::media::StagedImport) -> Result<Value, String> {
    crate::media::safe_path(path)?;
    let mut file = File::open(path).map_err(|error| format!("无法打开 ZIP：{error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > crate::media::MAX_ZIP_BYTES {
        return Err("请选择非空的 ZIP 文件，且大小不能超过 2 GiB".into());
    }
    let (directory_start, records) = inspect_file_directory(&mut file)?;
    let mut headers = file.try_clone().map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| format!("ZIP 格式无效：{error}"))?;
    if archive.len() != records.len() || archive.central_directory_start() != directory_start {
        return Err("ZIP 存在重复文件或中央目录不一致".into());
    }
    let mut names = HashSet::new();
    let mut ranges = Vec::new();
    let mut files = HashMap::new();
    let mut videos = HashMap::new();
    let mut total = 0u64;
    let mut text_total = 0u64;
    for (index, record) in records.iter().enumerate() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_owned();
        if !valid_asset_path(&name) || !names.insert(casefold(&name)) || entry.is_dir() || entry.encrypted()
            || entry.name_raw() != record.name || entry.central_header_start() != record.offset
            || entry.unix_mode().is_some_and(|mode| !regular_mode(mode)) {
            return Err("ZIP 包含非法路径、重复文件、目录、特殊文件或加密内容".into());
        }
        let header_start = entry.header_start();
        let local = read_at(&mut headers, header_start, 30)?;
        let name_length = number(&local, 26, 2)? as usize;
        let extra_length = number(&local, 28, 2)?;
        if &local[..4] != b"PK\x03\x04" || number(&local, 6, 2)? != record.flags
            || number(&local, 8, 2)? != record.method
            || read_at(&mut headers, header_start + 30, name_length)? != record.name {
            return Err("ZIP 本地文件头与中央目录不一致".into());
        }
        let data_start = header_start + 30 + name_length as u64 + extra_length;
        let data_end = data_start.checked_add(entry.compressed_size()).ok_or("ZIP 压缩数据长度无效")?;
        if entry.data_start() != data_start || data_end > directory_start
            || ranges.iter().any(|&(start, end)| header_start < end && data_end > start) {
            return Err("ZIP 文件数据越界或重叠".into());
        }
        ranges.push((header_start, data_end));
        let video = name.starts_with("previews/") && name.ends_with("/video.mp4");
        let valid_text = name == "manifest.json" || (name.starts_with("parts/") && name.ends_with(".json"))
            || (name.starts_with("previews/") && name.ends_with("/dialogue-map.json"));
        if !video && !valid_text { return Err("ZIP 存在不支持的文件".into()); }
        let limit = if video { crate::media::MAX_VIDEO_BYTES } else { MAX_FILE_BYTES.min(MAX_BYTES.saturating_sub(text_total)) }
            .min(crate::media::MAX_ZIP_BYTES.saturating_sub(total));
        let declared_size = entry.size();
        if declared_size > limit { return Err("ZIP 单文件或解压总大小超过上限".into()); }
        if video {
            let identifier = uuid::Uuid::new_v4().to_string();
            let directory = stage.directory.join(&identifier);
            crate::media::ensure_directory(&directory)?;
            let path = directory.join("video.mp4");
            let mut output = File::create(&path).map_err(|error| error.to_string())?;
            let (hash, length) = crate::media::copy_verified(&mut entry, &mut output, limit)?;
            if length == 0 || length != declared_size { return Err("视频文件长度异常".into()); }
            output.sync_all().map_err(|error| error.to_string())?;
            videos.insert(name, (identifier, path, hash, length));
        } else {
            let mut content = Vec::new();
            (&mut entry).take(limit + 1).read_to_end(&mut content).map_err(|error| format!("ZIP 文件读取或 CRC 校验失败：{name}：{error}"))?;
            if content.len() as u64 > limit || content.len() as u64 != declared_size { return Err("ZIP 文件长度异常或解压内容超限".into()); }
            text_total += declared_size;
            files.insert(name, content);
        }
        total += declared_size;
    }
    let manifest_bytes = files
        .remove("manifest.json")
        .ok_or("ZIP 根目录缺少 manifest.json")?;
    let manifest: Value = serde_json::from_slice(
        manifest_bytes
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&manifest_bytes),
    )
    .map_err(|error| format!("清单不是有效的 UTF-8 JSON：{error}"))?;
    if manifest.get("format").and_then(Value::as_str) != Some("mida-localization-manifest")
        || !matches!(manifest.get("formatVersion").and_then(Value::as_u64), Some(2 | 3))
    {
        return Err("需要纯数据 v2 或含媒体 v3 清单，请重新导出 ZIP".into());
    }
    validate_metadata(&manifest)?;
    let assets = nonempty_array(&manifest, "assets", MAX_PARTS * 3)?;
    let version = manifest["formatVersion"].as_u64().ok_or("清单版本无效")?;
    stage.project_id = nonempty_text(&manifest, "projectId")?.to_owned();
    let mut texts = Map::new();
    let mut paths = HashSet::new();
    let mut delivery = None;
    let mut counts = TaskCounts::default();
    for asset in assets {
        if asset.get("type").and_then(Value::as_str) != Some("localization-dialogues") {
            if version == 3 && matches!(asset["type"].as_str(), Some("localization-preview-video" | "localization-preview-map")) { continue; }
            return Err("不支持的片段资产类型".into());
        }
        let part_name = nonempty_text(asset, "partName")?;
        stage.parts.push(part_name.to_owned());
        if stage.parts.len() > MAX_PARTS { return Err("一次最多导入 100 个片段".into()); }
        let asset_path = part_asset_path(part_name)?;
        if asset.get("id").and_then(Value::as_str) != Some(part_name)
            || asset.get("path").and_then(Value::as_str) != Some(asset_path.as_str())
            || !paths.insert(casefold(&asset_path))
        {
            return Err("片段资产标识、路径无效或重复".into());
        }
        let content = files
            .remove(&asset_path)
            .ok_or_else(|| format!("片段文件缺失：{asset_path}"))?;
        if asset.get("sha256").and_then(Value::as_str) != Some(sha256(&content).as_str()) {
            return Err(format!("片段文件校验失败：{asset_path}"));
        }
        let text =
            String::from_utf8(content).map_err(|error| format!("片段文件不是 UTF-8：{error}"))?;
        let data: Value = serde_json::from_str(&text)
            .map_err(|error| format!("片段文件不是有效 JSON：{asset_path}：{error}"))?;
        validate_data(&data, false)?;
        if IDENTITY_FIELDS
            .iter()
            .any(|field| data.get(*field) != manifest.get(*field))
        {
            return Err(format!("片段文件版本身份与清单不一致：{asset_path}"));
        }
        let current_delivery = (
            data["deliveryState"].clone(),
            data.get("demo").cloned().unwrap_or(Value::Bool(false)),
            data.get("parentPackageId").cloned().unwrap_or(Value::Null),
        );
        if delivery
            .as_ref()
            .is_some_and(|previous| previous != &current_delivery)
        {
            return Err("片段文件交付状态或来源不一致".into());
        }
        for (field, value) in [
            ("deliveryState", &current_delivery.0),
            ("demo", &current_delivery.1),
            ("parentPackageId", &current_delivery.2),
        ] {
            if manifest
                .get(field)
                .is_some_and(|expected| expected != value)
            {
                return Err(format!("片段文件的 {field} 与清单不一致"));
            }
        }
        delivery = Some(current_delivery);
        validate_tasks(&data, Some(part_name), &mut counts)?;
        texts.insert(asset_path, Value::String(text));
    }
    if stage.parts.is_empty() { return Err("ZIP 缺少对话片段".into()); }
    let mut media_assets: HashMap<&str, (Option<&Value>, Option<&Value>)> = HashMap::new();
    for asset in assets.iter().filter(|asset| asset["type"] != "localization-dialogues") {
        let part = nonempty_text(asset, "partName")?;
        if !stage.parts.iter().any(|name| name == part) { return Err("媒体没有对应对话片段".into()); }
        let video = asset["type"] == "localization-preview-video";
        let suffix = if video { "video.mp4" } else { "dialogue-map.json" };
        let expected_path = format!("previews/{part}/{suffix}");
        let expected_id = format!("{part}:{}", if video { "video" } else { "map" });
        if asset["id"] != expected_id || asset["path"] != expected_path || !valid_asset_path(&expected_path)
            || !paths.insert(casefold(&expected_path)) || !valid_hash(asset.get("sha256"))
            || !crate::media::valid_uuid(nonempty_text(asset, "recordingId")?) || !safe_integer(asset.get("byteLength"), 1) {
            return Err("媒体资产标识、路径、哈希、长度或录制 ID 无效".into());
        }
        let pair = media_assets.entry(part).or_default();
        if video { pair.0 = Some(asset); } else { pair.1 = Some(asset); }
    }
    for (part, (video_asset, map_asset)) in media_assets {
        let video_asset = video_asset.ok_or("媒体地图缺少配对视频")?;
        let map_asset = map_asset.ok_or("媒体视频缺少配对地图")?;
        let recording = nonempty_text(video_asset, "recordingId")?;
        if map_asset["recordingId"] != recording { return Err("媒体录制身份不配对".into()); }
        let video_path = nonempty_text(video_asset, "path")?;
        let (identifier, path, hash, length) = videos.remove(video_path).ok_or("媒体视频文件缺失")?;
        let map_bytes = files.remove(nonempty_text(map_asset, "path")?).ok_or("媒体地图文件缺失")?;
        if video_asset["sha256"] != hash || video_asset["byteLength"].as_u64() != Some(length)
            || map_asset["sha256"] != sha256(&map_bytes) || map_asset["byteLength"].as_u64() != Some(map_bytes.len() as u64) {
            return Err("媒体资产 SHA256 或 byteLength 不一致".into());
        }
        let map: Value = serde_json::from_slice(&map_bytes).map_err(|error| format!("媒体地图不是 UTF-8 JSON：{error}"))?;
        crate::media::validate_map(&map, &stage.project_id, part, recording, &hash)?;
        let map_path = path.parent().ok_or("媒体目录无效")?.join("dialogue-map.json");
        let mut output = File::create(map_path).map_err(|error| error.to_string())?;
        output.write_all(&map_bytes).map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())?;
        stage.previews.insert(part.into(), json!({"recordingId":recording,"mediaId":identifier,"map":map}));
    }
    if !files.is_empty() || !videos.is_empty() { return Err("ZIP 文件与清单不一致，存在未声明的资产".into()); }
    Ok(json!({"manifest": manifest, "assets": texts, "mediaImportToken":stage.token,"previews":stage.previews}))
}

pub fn build_package(document: &Value, output: &mut File) -> Result<(), String> {
    validate_data(document, false)?;
    validate_tasks(document, None, &mut TaskCounts::default())?;
    let tasks = nonempty_array(document, "tasks", MAX_TASKS)?;
    let mut groups: Vec<(&str, Vec<&Value>)> = Vec::new();
    let mut group_indices = HashMap::new();
    for task in tasks {
        let part_name = nonempty_text(task, "partName")?;
        if let Some(&index) = group_indices.get(part_name) {
            let group: &mut (&str, Vec<&Value>) = &mut groups[index];
            group.1.push(task);
        } else {
            if groups.len() >= MAX_PARTS {
                return Err("一次最多导出 100 个片段".into());
            }
            group_indices.insert(part_name, groups.len());
            groups.push((part_name, vec![task]));
        }
    }
    let mut manifest = Map::new();
    for field in IDENTITY_FIELDS {
        manifest.insert(field.into(), document[field].clone());
    }
    manifest.insert("format".into(), json!("mida-localization-manifest"));
    manifest.insert("formatVersion".into(), json!(2));
    let mut assets = Vec::new();
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    let mut total = 0;
    let source = document.as_object().ok_or("无效的本地化交付格式")?;
    for (part_name, tasks) in groups {
        let path = part_asset_path(part_name)?;
        if !paths.insert(casefold(&path)) {
            return Err("片段文件名存在大小写冲突".into());
        }
        let mut data: Map<String, Value> = source
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "tasks" | "previews"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        data.insert(
            "tasks".into(),
            Value::Array(tasks.into_iter().cloned().collect()),
        );
        let content = json_bytes(&Value::Object(data), MAX_FILE_BYTES.min(MAX_BYTES - total))?;
        total += content.len() as u64;
        assets.push(json!({
            "id": part_name, "partName": part_name, "type": "localization-dialogues",
            "path": path, "sha256": sha256(&content)
        }));
        files.push((path, content));
    }
    manifest.insert("assets".into(), Value::Array(assets));
    let manifest_bytes = json_bytes(
        &Value::Object(manifest),
        MAX_FILE_BYTES.min(MAX_BYTES - total),
    )?;
    let mut archive = ZipWriter::new(BoundedFile { inner: output, limit: crate::media::MAX_ZIP_BYTES });
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    archive
        .start_file("manifest.json", options)
        .map_err(|error| format!("无法写入清单：{error}"))?;
    archive
        .write_all(&manifest_bytes)
        .map_err(|error| format!("无法写入清单：{error}"))?;
    for (path, content) in files {
        archive
            .start_file(&path, options)
            .map_err(|error| format!("无法创建片段文件：{error}"))?;
        archive
            .write_all(&content)
            .map_err(|error| format!("无法写入片段文件：{error}"))?;
    }
    let output = archive.finish().map_err(|error| format!("无法完成 ZIP：{error}"))?;
    if output.inner.metadata().map_err(|error| error.to_string())?.len() > crate::media::MAX_ZIP_BYTES { return Err("导出 ZIP 超过 2 GiB".into()); }
    Ok(())
}

pub fn selected_parts(document: &Value) -> Result<Vec<String>, String> {
    validate_data(document, false)?;
    validate_tasks(document, None, &mut TaskCounts::default())?;
    let mut parts = Vec::new();
    for task in nonempty_array(document, "tasks", MAX_TASKS)? {
        let part = nonempty_text(task, "partName")?;
        if !parts.iter().any(|name| name == part) { parts.push(part.to_owned()); }
    }
    if parts.len() > MAX_PARTS { return Err("一次最多导出 100 个片段".into()); }
    Ok(parts)
}

pub fn valid_part(part: &str) -> bool { part_asset_path(part).is_ok() }

fn nonempty_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{field} 缺失或不是有效文本"))
}

fn nonempty_array<'a>(
    value: &'a Value,
    field: &str,
    limit: usize,
) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= limit)
        .ok_or_else(|| format!("{field} 列表为空、无效或超过 {limit} 个"))
}

fn safe_integer(value: Option<&Value>, minimum: u64) -> bool {
    value
        .and_then(Value::as_u64)
        .is_some_and(|number| (minimum..=MAX_SAFE_INTEGER).contains(&number))
}

fn valid_hash(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_metadata(value: &Value) -> Result<(), String> {
    let project = nonempty_text(value, "projectId")?;
    if project == "DEMO-ONLY-NOT-A-REAL-PROJECT" {
        return Err("不接受演示任务".into());
    }
    nonempty_text(value, "packageId")?;
    nonempty_text(value, "exportedAt")?;
    let version = value.get("fileVersion").ok_or("缺少有效的包版本标识")?;
    nonempty_text(version, "lineageId")?;
    if !safe_integer(version.get("revision"), 1) {
        return Err("包版本 revision 必须是正安全整数".into());
    }
    if value
        .get("demo")
        .is_some_and(|demo| demo != &Value::Bool(false))
    {
        return Err("不接受演示任务或无效 demo 标记".into());
    }
    if value
        .get("parentPackageId")
        .is_some_and(|parent| !parent.is_null())
    {
        nonempty_text(value, "parentPackageId")?;
    }
    Ok(())
}

fn validate_data(value: &Value, require_ready: bool) -> Result<(), String> {
    if value.get("format").and_then(Value::as_str) != Some("mida-localization")
        || value.get("formatVersion").and_then(Value::as_u64) != Some(1)
    {
        return Err("无效的本地化 v1 数据格式".into());
    }
    validate_metadata(value)?;
    match value.get("deliveryState").and_then(Value::as_str) {
        Some("ready") => Ok(()),
        Some("draft") if !require_ready => Ok(()),
        _ => Err("交付状态无效；导出只接受真实任务的已完成交付文件".into()),
    }
}

#[derive(Default)]
struct TaskCounts {
    tasks: usize,
    entries: usize,
    identities: HashSet<(String, String)>,
}

pub(crate) fn validate_source_snapshots(entry: &Value) -> Result<(), String> {
    for field in ["currentSource", "localizationSourceAtExport", "translationAtExport"] {
        if entry.get(field).and_then(Value::as_str).is_none() {
            return Err(format!("来源快照必须是文本：{field}"));
        }
    }
    if let Some(english) = entry.get("englishTranslationAtExport") {
        if !english.is_string() {
            return Err("旧英文快照 englishTranslationAtExport 必须是文本".into());
        }
    }
    Ok(())
}

fn validate_tasks(
    document: &Value,
    expected_part: Option<&str>,
    counts: &mut TaskCounts,
) -> Result<(), String> {
    let tasks = nonempty_array(document, "tasks", MAX_TASKS)?;
    counts.tasks += tasks.len();
    if counts.tasks > MAX_TASKS {
        return Err("任务总数超过 1000 个".into());
    }
    let ready = document.get("deliveryState").and_then(Value::as_str) == Some("ready");
    for task in tasks {
        let part = nonempty_text(task, "partName")?;
        part_asset_path(part)?;
        let language = nonempty_text(task, "language")?;
        if expected_part.is_some_and(|expected| expected != part)
            || !counts.identities.insert((part.into(), language.into()))
        {
            return Err("片段文件包含其他片段或重复片段语言任务".into());
        }
        let target_missing = task.get("targetLocalizationHashAtExport") == Some(&Value::Null);
        if !safe_integer(task.get("taskVersion"), 1)
            || !valid_hash(task.get("sourceHash"))
            || (!target_missing && !valid_hash(task.get("targetLocalizationHashAtExport")))
        {
            return Err("任务版本或来源哈希无效".into());
        }
        let status = task.get("versionStatusAtExport").and_then(Value::as_str);
        let review_all = task.get("reviewAllRequired").and_then(Value::as_bool);
        if !matches!(status, Some("latest" | "outdated" | "unsupported")) || review_all.is_none() {
            return Err("任务版本状态无效".into());
        }
        if target_missing && (status != Some("unsupported") || review_all != Some(true)) {
            return Err("译文哈希缺失但没有对应的目标缺失状态".into());
        }
        let entries = nonempty_array(task, "entries", MAX_ENTRIES)?;
        counts.entries += entries.len();
        if counts.entries > MAX_ENTRIES {
            return Err("词条总数超过 100000 个".into());
        }
        let mut keys = HashSet::new();
        for entry in entries {
            validate_source_snapshots(entry)?;
            let key = nonempty_text(entry, "key")?;
            if !keys.insert(key) {
                return Err(format!("词条标识重复：{key}"));
            }
            for field in [
                "speakerKey",
                "speakerChineseName",
                "currentSource",
                "localizationSourceAtExport",
                "translationAtExport",
                "translation",
            ] {
                if entry.get(field).and_then(Value::as_str).is_none() {
                    return Err(format!("词条 {key} 的 {field} 不是文本"));
                }
            }
            let package = match key.rsplit_once('_') {
                Some((prefix, _)) if !prefix.is_empty() => prefix,
                _ => key,
            };
            if !safe_integer(entry.get("order"), 0)
                || entry.get("dialoguePackage").and_then(Value::as_str) != Some(package)
            {
                return Err(format!("词条顺序或对话包标识无效：{key}"));
            }
            let issues = entry
                .get("issuesAtExport")
                .and_then(Value::as_array)
                .ok_or("词条问题列表无效")?;
            if !issues.iter().all(Value::is_string)
                || (target_missing
                    && !issues.iter().any(|issue| {
                        matches!(
                            issue.as_str(),
                            Some("missing_localization_file" | "missing_language_column")
                        )
                    }))
            {
                return Err("词条问题列表无效或缺少目标缺失标记".into());
            }
            let review = entry
                .get("review")
                .and_then(Value::as_object)
                .ok_or("词条复核状态无效")?;
            let state = review.get("state").and_then(Value::as_str);
            if !matches!(state, Some("pending" | "confirmed"))
                || review
                    .get("reason")
                    .is_some_and(|reason| !reason.is_string())
            {
                return Err("词条复核状态或原因无效".into());
            }
            let translation = entry
                .get("translation")
                .and_then(Value::as_str)
                .ok_or("词条译文不是文本")?;
            if ready
                && (state != Some("confirmed")
                    || translation
                        .trim_matches(|character: char| {
                            character.is_whitespace() || character == '\u{feff}'
                        })
                        .is_empty())
            {
                return Err(format!("仍有未完成或空译文：{part} / {key}"));
            }
        }
    }
    Ok(())
}

fn safe_filename(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|character| character.is_control() || "\\/:*?\"<>|%".contains(character))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    !["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(*prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn valid_asset_path(path: &str) -> bool {
    !path.is_empty() && path.chars().count() < 240 && path.split('/').all(safe_filename)
}

fn part_asset_path(part: &str) -> Result<String, String> {
    if !safe_filename(part) {
        return Err("片段名不能用作跨平台文件名".into());
    }
    let path = format!("parts/{part}.json");
    if !valid_asset_path(&path) {
        return Err("片段文件路径过长或无效".into());
    }
    Ok(path)
}

fn sha256(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn regular_mode(mode: u32) -> bool {
    matches!(mode & 0o170000, 0 | 0o100000)
}

struct CentralRecord {
    offset: u64,
    name: Vec<u8>,
    flags: u64,
    method: u64,
}

fn bytes_at(payload: &[u8], offset: usize, length: usize) -> Result<&[u8], String> {
    let end = offset.checked_add(length).ok_or("ZIP 长度溢出")?;
    payload.get(offset..end).ok_or_else(|| "ZIP 文件头或数据被截断".into())
}

fn number(payload: &[u8], offset: usize, length: usize) -> Result<u64, String> {
    let mut result = 0;
    for (index, byte) in bytes_at(payload, offset, length)?.iter().enumerate() { result |= u64::from(*byte) << (index * 8); }
    Ok(result)
}

fn read_at(file: &mut File, offset: u64, length: usize) -> Result<Vec<u8>, String> {
    let position = file.stream_position().map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset)).map_err(|error| error.to_string())?;
    let mut bytes = vec![0; length];
    let result = file.read_exact(&mut bytes).map_err(|error| error.to_string());
    file.seek(SeekFrom::Start(position)).map_err(|error| error.to_string())?;
    result?;
    Ok(bytes)
}

fn inspect_file_directory(file: &mut File) -> Result<(u64, Vec<CentralRecord>), String> {
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let tail_start = length.saturating_sub(22 + 65535 + 20);
    let tail = read_at(file, tail_start, (length - tail_start) as usize)?;
    let end = (0..tail.len().saturating_sub(21)).rev().find(|&offset| {
        tail.get(offset..offset + 4) == Some(b"PK\x05\x06")
            && number(&tail, offset + 20, 2).is_ok_and(|count| offset + 22 + count as usize == tail.len())
    }).ok_or("ZIP 缺少有效的结束记录")?;
    if number(&tail, end + 4, 2)? != 0 || number(&tail, end + 6, 2)? != 0 { return Err("不支持分卷 ZIP".into()); }
    let mut count = number(&tail, end + 10, 2)?;
    let mut size = number(&tail, end + 12, 4)?;
    let mut start = number(&tail, end + 16, 4)?;
    let mut directory_end = tail_start + end as u64;
    if end >= 20 && bytes_at(&tail, end - 20, 4)? == b"PK\x06\x07" {
        let locator = end - 20;
        if number(&tail, locator + 4, 4)? != 0 || number(&tail, locator + 16, 4)? != 1 { return Err("不支持分卷 ZIP64".into()); }
        let offset = number(&tail, locator + 8, 8)?;
        let record = read_at(file, offset, 56)?;
        if &record[..4] != b"PK\x06\x06" || number(&record, 4, 8)? < 44
            || offset.checked_add(12).and_then(|value| value.checked_add(number(&record, 4, 8).ok()?)) != Some(tail_start + locator as u64)
            || number(&record, 16, 4)? != 0 || number(&record, 20, 4)? != 0
            || number(&record, 24, 8)? != number(&record, 32, 8)? { return Err("ZIP64 目录无效".into()); }
        count = number(&record, 32, 8)?;
        size = number(&record, 40, 8)?;
        start = number(&record, 48, 8)?;
        directory_end = offset;
    } else if number(&tail, end + 8, 2)? != count { return Err("ZIP 文件计数不一致".into()); }
    if !(1..=301).contains(&count) || start.checked_add(size) != Some(directory_end) || size > MAX_FILE_BYTES {
        return Err("ZIP 文件数量超过 301 个或目录长度无效".into());
    }
    let mut offset = start;
    let mut records = Vec::new();
    for _ in 0..count {
        let header = read_at(file, offset, 46)?;
        if &header[..4] != b"PK\x01\x02" { return Err("ZIP 中央目录格式无效".into()); }
        let flags = number(&header, 8, 2)?;
        let method = number(&header, 10, 2)?;
        let attributes = number(&header, 38, 4)? as u32;
        let name_length = number(&header, 28, 2)? as usize;
        let name = read_at(file, offset + 46, name_length)?;
        if flags & (1 | 0x40 | 0x2000) != 0 || !matches!(method, 0 | 8) || attributes & 0x18 != 0
            || !regular_mode(attributes >> 16) || name.is_empty() || name.contains(&0)
            || std::str::from_utf8(&name).is_err() || number(&header, 34, 2)? != 0 {
            return Err("ZIP 包含加密、特殊文件、无效文件名或不支持的压缩方式".into());
        }
        records.push(CentralRecord { offset, name, flags, method });
        offset += 46 + name_length as u64 + number(&header, 30, 2)? + number(&header, 32, 2)?;
        if offset > directory_end { return Err("ZIP 中央目录长度无效".into()); }
    }
    if offset != directory_end { return Err("ZIP 中央目录包含额外文件或未声明数据".into()); }
    Ok((start, records))
}

struct BoundedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: u64,
}

struct BoundedFile<'a> {
    inner: &'a mut File,
    limit: u64,
}

impl Write for BoundedFile<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.limit.saturating_sub(self.inner.stream_position()?) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ZIP 超过 2 GiB"));
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> { self.inner.flush() }
}

impl Seek for BoundedFile<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.inner.metadata()?.len()) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.inner.stream_position()?) + i128::from(offset),
        };
        if target < 0 || target > i128::from(self.limit) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "ZIP 写入位置超限")); }
        self.inner.seek(SeekFrom::Start(target as u64))
    }
}

impl BoundedBuffer {
    fn new(limit: u64) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
        }
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() as u64 > self.limit.saturating_sub(self.inner.position()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "文件或压缩包超过大小上限",
            ));
        }
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedBuffer {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => self.inner.get_ref().len() as i128 + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.inner.position()) + i128::from(offset),
        };
        if target < 0 || target > i128::from(self.limit) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ZIP 写入位置超限",
            ));
        }
        self.inner.seek(SeekFrom::Start(target as u64))
    }
}

fn json_bytes(value: &Value, limit: u64) -> Result<Vec<u8>, String> {
    let mut output = BoundedBuffer::new(limit);
    serde_json::to_writer_pretty(&mut output, value)
        .map_err(|error| format!("JSON 编码失败或大小超限：{error}"))?;
    Ok(output.inner.into_inner())
}

fn casefold(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if matches!(character, '\u{13a0}'..='\u{13f5}' | '\u{13f8}'..='\u{13fd}' | '\u{ab70}'..='\u{abbf}')
        {
            result.extend(character.to_uppercase());
            continue;
        }
        let expansion = match character {
            '\u{b5}' => "\u{3bc}",
            '\u{df}' => "\u{73}\u{73}",
            '\u{149}' => "\u{2bc}\u{6e}",
            '\u{17f}' => "\u{73}",
            '\u{1f0}' => "\u{6a}\u{30c}",
            '\u{345}' => "\u{3b9}",
            '\u{390}' => "\u{3b9}\u{308}\u{301}",
            '\u{3b0}' => "\u{3c5}\u{308}\u{301}",
            '\u{3c2}' => "\u{3c3}",
            '\u{3d0}' => "\u{3b2}",
            '\u{3d1}' => "\u{3b8}",
            '\u{3d5}' => "\u{3c6}",
            '\u{3d6}' => "\u{3c0}",
            '\u{3f0}' => "\u{3ba}",
            '\u{3f1}' => "\u{3c1}",
            '\u{3f5}' => "\u{3b5}",
            '\u{587}' => "\u{565}\u{582}",
            '\u{1c80}' => "\u{432}",
            '\u{1c81}' => "\u{434}",
            '\u{1c82}' => "\u{43e}",
            '\u{1c83}' => "\u{441}",
            '\u{1c84}' => "\u{442}",
            '\u{1c85}' => "\u{442}",
            '\u{1c86}' => "\u{44a}",
            '\u{1c87}' => "\u{463}",
            '\u{1c88}' => "\u{a64b}",
            '\u{1e96}' => "\u{68}\u{331}",
            '\u{1e97}' => "\u{74}\u{308}",
            '\u{1e98}' => "\u{77}\u{30a}",
            '\u{1e99}' => "\u{79}\u{30a}",
            '\u{1e9a}' => "\u{61}\u{2be}",
            '\u{1e9b}' => "\u{1e61}",
            '\u{1e9e}' => "\u{73}\u{73}",
            '\u{1f50}' => "\u{3c5}\u{313}",
            '\u{1f52}' => "\u{3c5}\u{313}\u{300}",
            '\u{1f54}' => "\u{3c5}\u{313}\u{301}",
            '\u{1f56}' => "\u{3c5}\u{313}\u{342}",
            '\u{1f80}' => "\u{1f00}\u{3b9}",
            '\u{1f81}' => "\u{1f01}\u{3b9}",
            '\u{1f82}' => "\u{1f02}\u{3b9}",
            '\u{1f83}' => "\u{1f03}\u{3b9}",
            '\u{1f84}' => "\u{1f04}\u{3b9}",
            '\u{1f85}' => "\u{1f05}\u{3b9}",
            '\u{1f86}' => "\u{1f06}\u{3b9}",
            '\u{1f87}' => "\u{1f07}\u{3b9}",
            '\u{1f88}' => "\u{1f00}\u{3b9}",
            '\u{1f89}' => "\u{1f01}\u{3b9}",
            '\u{1f8a}' => "\u{1f02}\u{3b9}",
            '\u{1f8b}' => "\u{1f03}\u{3b9}",
            '\u{1f8c}' => "\u{1f04}\u{3b9}",
            '\u{1f8d}' => "\u{1f05}\u{3b9}",
            '\u{1f8e}' => "\u{1f06}\u{3b9}",
            '\u{1f8f}' => "\u{1f07}\u{3b9}",
            '\u{1f90}' => "\u{1f20}\u{3b9}",
            '\u{1f91}' => "\u{1f21}\u{3b9}",
            '\u{1f92}' => "\u{1f22}\u{3b9}",
            '\u{1f93}' => "\u{1f23}\u{3b9}",
            '\u{1f94}' => "\u{1f24}\u{3b9}",
            '\u{1f95}' => "\u{1f25}\u{3b9}",
            '\u{1f96}' => "\u{1f26}\u{3b9}",
            '\u{1f97}' => "\u{1f27}\u{3b9}",
            '\u{1f98}' => "\u{1f20}\u{3b9}",
            '\u{1f99}' => "\u{1f21}\u{3b9}",
            '\u{1f9a}' => "\u{1f22}\u{3b9}",
            '\u{1f9b}' => "\u{1f23}\u{3b9}",
            '\u{1f9c}' => "\u{1f24}\u{3b9}",
            '\u{1f9d}' => "\u{1f25}\u{3b9}",
            '\u{1f9e}' => "\u{1f26}\u{3b9}",
            '\u{1f9f}' => "\u{1f27}\u{3b9}",
            '\u{1fa0}' => "\u{1f60}\u{3b9}",
            '\u{1fa1}' => "\u{1f61}\u{3b9}",
            '\u{1fa2}' => "\u{1f62}\u{3b9}",
            '\u{1fa3}' => "\u{1f63}\u{3b9}",
            '\u{1fa4}' => "\u{1f64}\u{3b9}",
            '\u{1fa5}' => "\u{1f65}\u{3b9}",
            '\u{1fa6}' => "\u{1f66}\u{3b9}",
            '\u{1fa7}' => "\u{1f67}\u{3b9}",
            '\u{1fa8}' => "\u{1f60}\u{3b9}",
            '\u{1fa9}' => "\u{1f61}\u{3b9}",
            '\u{1faa}' => "\u{1f62}\u{3b9}",
            '\u{1fab}' => "\u{1f63}\u{3b9}",
            '\u{1fac}' => "\u{1f64}\u{3b9}",
            '\u{1fad}' => "\u{1f65}\u{3b9}",
            '\u{1fae}' => "\u{1f66}\u{3b9}",
            '\u{1faf}' => "\u{1f67}\u{3b9}",
            '\u{1fb2}' => "\u{1f70}\u{3b9}",
            '\u{1fb3}' => "\u{3b1}\u{3b9}",
            '\u{1fb4}' => "\u{3ac}\u{3b9}",
            '\u{1fb6}' => "\u{3b1}\u{342}",
            '\u{1fb7}' => "\u{3b1}\u{342}\u{3b9}",
            '\u{1fbc}' => "\u{3b1}\u{3b9}",
            '\u{1fbe}' => "\u{3b9}",
            '\u{1fc2}' => "\u{1f74}\u{3b9}",
            '\u{1fc3}' => "\u{3b7}\u{3b9}",
            '\u{1fc4}' => "\u{3ae}\u{3b9}",
            '\u{1fc6}' => "\u{3b7}\u{342}",
            '\u{1fc7}' => "\u{3b7}\u{342}\u{3b9}",
            '\u{1fcc}' => "\u{3b7}\u{3b9}",
            '\u{1fd2}' => "\u{3b9}\u{308}\u{300}",
            '\u{1fd3}' => "\u{3b9}\u{308}\u{301}",
            '\u{1fd6}' => "\u{3b9}\u{342}",
            '\u{1fd7}' => "\u{3b9}\u{308}\u{342}",
            '\u{1fe2}' => "\u{3c5}\u{308}\u{300}",
            '\u{1fe3}' => "\u{3c5}\u{308}\u{301}",
            '\u{1fe4}' => "\u{3c1}\u{313}",
            '\u{1fe6}' => "\u{3c5}\u{342}",
            '\u{1fe7}' => "\u{3c5}\u{308}\u{342}",
            '\u{1ff2}' => "\u{1f7c}\u{3b9}",
            '\u{1ff3}' => "\u{3c9}\u{3b9}",
            '\u{1ff4}' => "\u{3ce}\u{3b9}",
            '\u{1ff6}' => "\u{3c9}\u{342}",
            '\u{1ff7}' => "\u{3c9}\u{342}\u{3b9}",
            '\u{1ffc}' => "\u{3c9}\u{3b9}",
            '\u{fb00}' => "\u{66}\u{66}",
            '\u{fb01}' => "\u{66}\u{69}",
            '\u{fb02}' => "\u{66}\u{6c}",
            '\u{fb03}' => "\u{66}\u{66}\u{69}",
            '\u{fb04}' => "\u{66}\u{66}\u{6c}",
            '\u{fb05}' => "\u{73}\u{74}",
            '\u{fb06}' => "\u{73}\u{74}",
            '\u{fb13}' => "\u{574}\u{576}",
            '\u{fb14}' => "\u{574}\u{565}",
            '\u{fb15}' => "\u{574}\u{56b}",
            '\u{fb16}' => "\u{57e}\u{576}",
            '\u{fb17}' => "\u{574}\u{56d}",
            _ => {
                result.extend(character.to_lowercase());
                continue;
            }
        };
        result.push_str(expansion);
    }
    result
}
