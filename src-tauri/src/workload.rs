use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_INTEGER: u64 = 9_007_199_254_740_991;
pub const FIELDS: [&str; 3] = ["delivery", "workload", "workReceipts"];

fn fail(message: &str) -> String { format!("工作量清单：{message}") }
pub fn has_text(value: &str) -> bool {
    value.chars().any(|char| !matches!(char as u32, 0x0009..=0x000d | 0x001c..=0x0020 |
        0x0085 | 0x00a0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 |
        0x202f | 0x205f | 0x3000 | 0xfeff))
}
fn text(value: &Value) -> Result<&str, String> {
    value.as_str().filter(|value| has_text(value)).ok_or_else(|| fail("身份文本无效"))
}
fn integer(value: &Value, minimum: u64) -> Result<u64, String> {
    value.as_u64().filter(|number| *number >= minimum && *number <= MAX_INTEGER)
        .ok_or_else(|| fail("整数字段无效"))
}
fn array(value: &Value, maximum: usize) -> Result<&Vec<Value>, String> {
    value.as_array().filter(|items| items.len() <= maximum).ok_or_else(|| fail("列表无效或超出上限"))
}
fn strings(value: &Value, maximum: usize) -> Result<HashSet<&str>, String> {
    let mut result = HashSet::new();
    for item in array(value, maximum)? {
        if !result.insert(text(item)?) { return Err(fail("列表身份重复")); }
    }
    Ok(result)
}
fn count_han(source: &str) -> u64 {
    source.chars().filter(|char| matches!(*char as u32,
        0x3007 | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff |
        0x20000..=0x2ebef | 0x2f800..=0x2fa1f | 0x30000..=0x323af)).count() as u64
}

#[derive(Default)]
pub struct TaskProgress {
    parts: HashSet<String>,
    languages: HashSet<String>,
    progress: u64,
    total: u64,
}

impl TaskProgress {
    pub fn add(&mut self, task: &Value) -> Result<(), String> {
        self.parts.insert(text(&task["partName"])?.into());
        self.languages.insert(text(&task["language"])?.into());
        for entry in array(&task["entries"], 100000)? {
            let chars = count_han(entry["currentSource"].as_str().ok_or_else(|| fail("任务原文无效"))?);
            self.total += chars;
            let allows_empty = task["sourceKind"] == "unity-minigame"
                && task["assetProtocolVersion"].as_u64() == Some(1) && entry["allowEmpty"] == true;
            if entry["review"]["state"] == "confirmed"
                && entry["translation"].as_str().is_some_and(|value| has_text(value) || allows_empty) {
                self.progress += chars;
            }
        }
        Ok(())
    }
}

pub fn validate(document: &Value, tasks: &[&Value]) -> Result<(), String> {
    let mut progress = TaskProgress::default();
    if document.get("workload").is_some() {
        for task in tasks { progress.add(task)?; }
    }
    validate_summary(document, &progress)
}

pub fn validate_summary(document: &Value, progress: &TaskProgress) -> Result<(), String> {
    if let Some(receipts) = document.get("workReceipts") {
        let mut seen = HashSet::new();
        for receipt in array(receipts, 10000)? {
            if receipt["version"].as_u64() != Some(1) { return Err(fail("不支持的回执版本")); }
            for field in ["id", "projectId", "lineageId", "language", "ledgerId", "deliveryId", "receivedAt"] {
                text(&receipt[field])?;
            }
            if !seen.insert(text(&receipt["id"])?) { return Err(fail("回执 ID 重复")); }
            if receipt["projectId"] != document["projectId"] || receipt["lineageId"] != document["fileVersion"]["lineageId"] {
                return Err(fail("回执项目或谱系不匹配"));
            }
            strings(&receipt["recordIds"], 100000)?;
        }
    }
    if document.get("delivery").is_none() && document.get("workload").is_none() { return Ok(()); }
    let delivery = &document["delivery"];
    if delivery["version"].as_u64() != Some(1) || delivery["id"] != document["packageId"] {
        return Err(fail("交付身份无效"));
    }
    text(&delivery["id"])?;
    text(&delivery["ledgerId"])?;
    integer(&delivery["revision"], 1)?;
    let previous = delivery.get("previousId").ok_or_else(|| fail("缺少上一交付身份"))?;
    if !previous.is_null() { text(previous)?; }
    if previous == &delivery["id"] { return Err(fail("交付指向自身")); }
    let workload = &document["workload"];
    if workload["version"].as_u64() != Some(1) || workload["countingRule"] != "han-v1" {
        return Err(fail("不支持的计数规则"));
    }
    let scope = &workload["scope"];
    if scope["projectId"] != document["projectId"] || scope["lineageId"] != document["fileVersion"]["lineageId"] {
        return Err(fail("统计项目或谱系不匹配"));
    }
    let language = text(&scope["language"])?;
    let parts = strings(&scope["partNames"], 200)?;
    if progress.languages.len() != 1 || !progress.languages.contains(language) { return Err(fail("统计语言与任务不一致")); }
    let actual_parts: HashSet<_> = progress.parts.iter().map(String::as_str).collect();
    if parts.is_empty() || parts != actual_parts { return Err(fail("统计片段与任务不一致")); }
    let mut records = HashMap::new();
    let mut identities = HashSet::new();
    for record in array(&workload["records"], 100000)? {
        for field in ["id", "partName", "language", "key", "confirmedAt"] { text(&record[field])?; }
        let source = record["sourceText"].as_str().ok_or_else(|| fail("记录原文无效"))?;
        if !record["translation"].is_string() { return Err(fail("记录译文无效")); }
        let chars = integer(&record["chars"], 0)?;
        if chars != count_han(source) { return Err(fail("工作记录汉字数不一致")); }
        let source_id = record.get("sourcePackageId").ok_or_else(|| fail("缺少来源包身份"))?;
        let source_revision = record.get("sourceRevision").ok_or_else(|| fail("缺少来源版本"))?;
        if !(source_id.is_null() && source_revision.is_null()) {
            text(source_id)?; integer(source_revision, 1)?;
        }
        integer(&record["taskVersion"], 1)?;
        if !matches!(record["kind"].as_str(), Some("translation" | "source_revision")) {
            return Err(fail("工作类型无效"));
        }
        let part = text(&record["partName"])?;
        if !parts.contains(part) || text(&record["language"])? != language { return Err(fail("记录不属于统计范围")); }
        if !identities.insert((part, language, text(&record["key"])?, source))
            || records.insert(text(&record["id"])?, chars).is_some() {
            return Err(fail("工作记录重复"));
        }
    }
    let acknowledged = strings(&workload["acknowledgedRecordIds"], 100000)?;
    let new_records = strings(&workload["newRecordIds"], 100000)?;
    let source_records = strings(&workload["sourceUpdateRecordIds"], 100000)?;
    for ids in [&acknowledged, &new_records, &source_records] {
        if ids.iter().any(|id| !records.contains_key(id)) { return Err(fail("统计子集包含未知记录")); }
    }
    let cumulative: u64 = records.values().sum();
    let handover: u64 = records.iter().filter(|(id, _)| !acknowledged.contains(*id)).map(|(_, chars)| *chars).sum();
    let sum = |ids: &HashSet<&str>| -> u64 { ids.iter().map(|id| records[id]).sum() };
    for (field, expected) in [
        ("progressChars", progress.progress), ("totalChars", progress.total), ("cumulativeChars", cumulative),
        ("handoverChars", handover), ("previousDeliveryDeltaChars", sum(&new_records)),
        ("sinceSourceUpdateChars", sum(&source_records)),
    ] {
        if integer(&workload[field], 0)? != expected { return Err(fail(&format!("{field} 与明细不一致"))); }
    }
    Ok(())
}
