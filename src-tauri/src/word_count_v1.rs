//! Fixed mida-gmx-fullwidth-v1 counter; mirrors workload.js and word_count.py.
use serde_json::Value;
use std::sync::OnceLock;

type Ranges = Vec<(u32, u32, u8)>;
struct Tables { word: Ranges, grapheme: Ranges, wide: Ranges }

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let value: Value = serde_json::from_str(include_str!("../../workload-counting.json"))
            .expect("bundled counting tables must be valid JSON");
        let read = |name: &str| value[name].as_array().expect("bundled range list").iter().map(|row|
            (row[0].as_u64().unwrap() as u32, row[1].as_u64().unwrap() as u32, row[2].as_u64().unwrap() as u8)
        ).collect();
        Tables { word: read("word"), grapheme: read("grapheme"), wide: read("wide") }
    })
}
fn property(rows: &[(u32, u32, u8)], point: u32) -> u8 {
    let index = rows.partition_point(|row| row.0 <= point);
    if index > 0 && point <= rows[index - 1].1 { rows[index - 1].2 } else { 0 }
}
fn whitespace(point: u32) -> bool {
    // GMX-V 4.2.7 excludes non-breaking spaces; NBSP retains its UAX #29 ALetter property.
    matches!(point, 0x9..=0xd | 0x1c..=0x20 | 0x1680 | 0x180e | 0x2000..=0x2006 |
        0x2008..=0x200b | 0x2028 | 0x2029 | 0x205f | 0x3000)
}
fn hangul_join(left: u8, right: u8) -> bool {
    (left == 2 && matches!(right, 2 | 3 | 5 | 6)) || (matches!(left, 3 | 5) && matches!(right, 3 | 4))
        || (matches!(left, 4 | 6) && right == 4)
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
            "aAeEiIoOuUyYàÀáÁâÂãÃäÄåÅæÆèÈéÉêÊëËìÌíÍîÎïÏòÒóÓôÔõÕöÖœŒùÙúÚûÛüÜýÝÿŸ".contains(ch)));
    }
    if (left == 2 && right == 6 && after == 2) || (before == 2 && left == 6 && right == 2) { return true; }
    false
}

pub fn count(source: &str) -> u64 {
    let table = tables();
    let mut tokens = Vec::with_capacity(source.len().min(4096));
    let mut previous_grapheme = 0;
    for ch in source.chars() {
        let point = ch as u32;
        let grapheme = property(&table.grapheme, point);
        if grapheme == 1 { previous_grapheme = grapheme; continue; }
        let joined = hangul_join(previous_grapheme, grapheme);
        previous_grapheme = grapheme;
        if joined { continue; }
        let mut kind = property(&table.word, point);
        if kind == 7 && !whitespace(point) { continue; }
        if whitespace(point) { kind = 0; }
        else if (0xff10..=0xff19).contains(&point) { kind = 2; }
        else if matches!(point, 0xff0c | 0xff0e | 0xff1b) { kind = 6; }
        else if matches!(point, 0x2d | 0x2010 | 0x58a | 0x30a0) { kind = 4; }
        tokens.push((point, kind));
    }
    // Numeric words, including internal separators, precede the fullwidth extension.
    for index in 0..tokens.len() {
        let (point, kind) = tokens[index];
        let numeric_separator = kind == 6 && index > 0 && tokens[index - 1].1 == 2
            && tokens.get(index + 1).is_some_and(|token| token.1 == 2);
        if property(&table.wide, point) != 0 && !whitespace(point) && kind != 2 && !numeric_separator {
            tokens[index].1 = 8;
        }
    }
    let (mut total, mut segment_word, mut segment_punctuation) = (0, false, 0);
    for (index, &(point, kind)) in tokens.iter().enumerate() {
        if index > 0 && !joins(&tokens, index) {
            total += if segment_word { 1 } else { segment_punctuation };
            segment_word = false;
            segment_punctuation = 0;
        }
        if kind == 8 { segment_punctuation += 1; }
        else if (1..=3).contains(&kind) && point != 0xa0 { segment_word = true; }
    }
    total + if segment_word { 1 } else { segment_punctuation }
}
