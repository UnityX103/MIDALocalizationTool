"""mida-gmx-fullwidth-v1. Keep the state machine in sync with JS and Rust.

Unicode 4.1 word/grapheme boundaries, GMX-V hyphen/apostrophe tailoring,
plus one unit per non-whitespace wide/fullwidth grapheme. Tables are frozen.
"""
import json
from bisect import bisect_right
from functools import lru_cache
from pathlib import Path

DATA = json.loads(Path(__file__).with_name('workload-counting.json').read_text(encoding='utf-8'))
RULE = 'mida-gmx-fullwidth-v1'
STARTS = {name: [row[0] for row in DATA[name]] for name in ('word', 'grapheme', 'wide')}
WIDE = 8
HYPHENS = (0x2D, 0x2010, 0x58A, 0x30A0)
APOSTROPHES = (0x27, 0x2019)
VOWELS = set(map(ord, 'aAeEiIoOuUyYàÀáÁâÂãÃäÄåÅæÆèÈéÉêÊëËìÌíÍîÎïÏòÒóÓôÔõÕöÖœŒùÙúÚûÛüÜýÝÿŸ'))
# GMX-V 4.2.7 excludes non-breaking spaces; in UAX #29 4.1 NBSP is ALetter.
WHITESPACE = set(range(9, 14)) | set(range(0x1C, 0x21)) | {
    0x1680, 0x180E, *range(0x2000, 0x2007), *range(0x2008, 0x200C),
    0x2028, 0x2029, 0x205F, 0x3000,
}


def prop(name, point):
    index = bisect_right(STARTS[name], point) - 1
    return DATA[name][index][2] if index >= 0 and point <= DATA[name][index][1] else 0


def hangul_join(left, right):
    return ((left == 2 and right in (2, 3, 5, 6))
            or (left in (3, 5) and right in (3, 4))
            or (left in (4, 6) and right == 4))


@lru_cache(maxsize=100000)
def count_words(source):
    tokens = []
    previous_grapheme = 0
    for char in source:
        point = ord(char)
        grapheme = prop('grapheme', point)
        if grapheme == 1:
            previous_grapheme = grapheme
            continue
        joined = hangul_join(previous_grapheme, grapheme)
        previous_grapheme = grapheme
        if joined:
            continue
        kind = prop('word', point)
        if kind == 7 and point not in WHITESPACE:
            continue
        if point in WHITESPACE:
            kind = 0
        elif 0xFF10 <= point <= 0xFF19:
            kind = 2
        elif point in (0xFF0C, 0xFF0E, 0xFF1B):
            kind = 6
        elif point in HYPHENS:
            kind = 4
        tokens.append((point, kind))

    # Recognize the complete numeric word before applying the fullwidth extension.
    # Fullwidth punctuation outside a numeric word still contributes one unit.
    for index, (point, kind) in enumerate(tokens):
        numeric_separator = (kind == 6 and index > 0 and index + 1 < len(tokens)
                             and tokens[index - 1][1] == tokens[index + 1][1] == 2)
        if prop('wide', point) and point not in WHITESPACE and kind != 2 and not numeric_separator:
            tokens[index] = (point, WIDE)

    def joins(index):
        point, right = tokens[index]
        left_point, left = tokens[index - 1]
        before = tokens[index - 2][1] if index > 1 else 0
        after = tokens[index + 1][1] if index + 1 < len(tokens) else 0
        if left == WIDE or right == WIDE:
            return False
        if left in (1, 2) and right in (1, 2):
            return True
        if left == right == 3:
            return True
        if (left in (1, 2, 3, 4) and right == 4) or (left == 4 and right in (1, 2, 3)):
            return True
        if left == 1 and right == 5 and after == 1:
            return True
        if before == 1 and left == 5 and right == 1:
            return not (left_point in APOSTROPHES and point in VOWELS)
        if (left == 2 and right == 6 and after == 2) or (before == 2 and left == 6 and right == 2):
            return True
        return False

    total, segment_word, segment_punctuation = 0, False, 0
    for index, (point, kind) in enumerate(tokens):
        if index and not joins(index):
            total += 1 if segment_word else segment_punctuation
            segment_word, segment_punctuation = False, 0
        if kind == WIDE:
            segment_punctuation += 1
        elif kind in (1, 2, 3) and point != 0xA0:
            segment_word = True
    return total + (1 if segment_word else segment_punctuation)
