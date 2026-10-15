//! GMX-V word-count algorithm and agreed fullwidth extension, frozen Unicode 4.1.
use serde_json::{json, Value};
use std::{collections::HashMap, sync::OnceLock};

pub const RULE: &str = "mida-gmx-fullwidth-v2";
pub const SCALE: u64 = 462;
type Ranges = Vec<(u32, u32, u8)>;
struct Tables {
    word: Ranges, grapheme: Ranges, wide: Ranges, scripts: Ranges,
    combining: Ranges, letters: Ranges, decomposition: HashMap<u32, Vec<u32>>,
}
fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let value: Value = serde_json::from_str(include_str!("../../workload-counting.json"))
            .expect("bundled counting tables must be valid JSON");
        let read = |name: &str| value[name].as_array().unwrap().iter().map(|row|
            (row[0].as_u64().unwrap() as u32, row[1].as_u64().unwrap() as u32, row[2].as_u64().unwrap() as u8)
        ).collect();
        let decomposition = value["decomposition"].as_object().unwrap().iter().map(|(point, parts)|
            (point.parse().unwrap(), parts.as_array().unwrap().iter().map(|part| part.as_u64().unwrap() as u32).collect())
        ).collect();
        Tables { word: read("word"), grapheme: read("grapheme"), wide: read("wide"),
            scripts: read("scripts"), combining: read("combining"), letters: read("letters"), decomposition }
    })
}
fn property(rows: &[(u32, u32, u8)], point: u32) -> u8 {
    let index = rows.partition_point(|row| row.0 <= point);
    if index > 0 && point <= rows[index - 1].1 { rows[index - 1].2 } else { 0 }
}
fn whitespace(point: u32) -> bool {
    matches!(point, 0x9..=0xd | 0x1c..=0x20 | 0x1680 | 0x180e | 0x2000..=0x2006 |
        0x2008..=0x200b | 0x2028 | 0x2029 | 0x205f | 0x3000)
}
fn flush_marks(marks: &mut Vec<(u8, u32)>, output: &mut Vec<u32>) {
    marks.sort_by_key(|item| item.0);
    output.extend(marks.drain(..).map(|item| item.1));
}
fn decompose(point: u32, table: &Tables, marks: &mut Vec<(u8, u32)>, output: &mut Vec<u32>) {
    if (0xac00..=0xd7a3).contains(&point) {
        let syllable = point - 0xac00;
        decompose(0x1100 + syllable / 588, table, marks, output);
        decompose(0x1161 + syllable % 588 / 28, table, marks, output);
        if syllable % 28 != 0 { decompose(0x11a7 + syllable % 28, table, marks, output); }
    } else if let Some(parts) = table.decomposition.get(&point) {
        for &part in parts { decompose(part, table, marks, output); }
    } else {
        let ccc = property(&table.combining, point);
        if ccc != 0 { marks.push((ccc, point)); }
        else { flush_marks(marks, output); output.push(point); }
    }
}
fn grapheme_join(left: u8, right: u8) -> bool {
    if left == 7 && right == 8 { return true; }
    if matches!(left, 7 | 8 | 9) || matches!(right, 7 | 8 | 9) { return false; }
    right == 1 || (left == 2 && matches!(right, 2 | 3 | 5 | 6))
        || (matches!(left, 3 | 5) && matches!(right, 3 | 4)) || (matches!(left, 4 | 6) && right == 4)
}
fn joins(tokens: &[(u32, u8)], index: usize) -> bool {
    let (point, right) = tokens[index];
    let (left_point, left) = tokens[index - 1];
    let before = if index > 1 { tokens[index - 2].1 } else { 0 };
    let after = tokens.get(index + 1).map(|token| token.1).unwrap_or(0);
    if left == 8 || right == 8 { return false; }
    if matches!(left, 1 | 2) && matches!(right, 1 | 2) { return true; }
    if left == 3 && right == 3 { return true; }
    if ((1..=4).contains(&left) && right == 4) || (left == 4 && (1..=3).contains(&right)) { return true; }
    if left == 1 && right == 5 && after == 1 { return true; }
    if before == 1 && left == 5 && right == 1 {
        return !(matches!(left_point, 0x27 | 0x2019) && char::from_u32(point).is_some_and(|ch|
            "aAeEiIoOuUyYæÆœŒ".contains(ch)));
    }
    (left == 2 && right == 6 && after == 2) || (before == 2 && left == 6 && right == 2)
}

pub struct Measurement { pub units: u64, pub characters: [u64; 3] }
impl Measurement {
    pub fn character_counts(&self) -> Value {
        json!({"lao":self.characters[0],"khmer":self.characters[1],"myanmar":self.characters[2]})
    }
}
pub fn measure(source: &str) -> Measurement {
    let table = tables();
    let (mut points, mut marks) = (Vec::new(), Vec::new());
    for ch in source.chars() { decompose(ch as u32, table, &mut marks, &mut points); }
    flush_marks(&mut marks, &mut points);
    let mut tokens = Vec::with_capacity(points.len());
    let (mut previous, mut first) = (0, true);
    for point in points {
        let grapheme = property(&table.grapheme, point);
        let joined = !first && grapheme_join(previous, grapheme);
        previous = grapheme; first = false;
        if joined { continue; }
        let mut kind = property(&table.word, point);
        if kind == 7 && !whitespace(point) { continue; }
        if whitespace(point) { kind = 0; }
        else if (0xff10..=0xff19).contains(&point) { kind = 2; }
        else if matches!(point, 0xff0c | 0xff0e | 0xff1b) { kind = 6; }
        else if matches!(point, 0x2d | 0x2010 | 0x58a | 0x30a0) { kind = 4; }
        tokens.push((point, kind));
    }
    let (mut units, mut characters) = (0, [0; 3]);
    for index in 0..tokens.len() {
        let (point, kind) = tokens[index];
        let numeric_separator = kind == 6 && index > 0 && tokens[index - 1].1 == 2
            && tokens.get(index + 1).is_some_and(|token| token.1 == 2);
        if property(&table.wide, point) != 0 && !whitespace(point) && kind != 2 && !numeric_separator {
            units += SCALE; tokens[index].1 = 8;
        } else if kind != 2 && !numeric_separator {
            let script = property(&table.scripts, point);
            let script = if script == 0 && kind == 3 { 2 } else { script };
            if script != 0 && (property(&table.letters, point) != 0 || kind == 3) {
                units += match script { 1 => 165, 2 => 154, 3 => 140, 4 => 77, _ => 0 };
                if (5..=7).contains(&script) { characters[(script - 5) as usize] += 1; }
                tokens[index].1 = 8;
            }
        }
    }
    let mut word = false;
    for (index, &(point, kind)) in tokens.iter().enumerate() {
        if index > 0 && !joins(&tokens, index) { if word { units += SCALE; } word = false; }
        if (1..=3).contains(&kind) && point != 0xa0 { word = true; }
    }
    if word { units += SCALE; }
    Measurement { units, characters }
}
pub fn count(source: &str) -> u64 { measure(source).units }
