"""GMX-V 2.0 word counts + the agreed fullwidth extension (fixed Unicode 4.1).

Quantities use exact integer units of 1/462 word; no per-entry rounding.
Only the counting view is normalized. Source strings and identities stay intact.
"""
import json
from bisect import bisect_right
from functools import lru_cache
from pathlib import Path

DATA = json.loads(Path(__file__).with_name('workload-counting.json').read_text(encoding='utf-8'))
RULE, SCALE = DATA['rule'], DATA['scale']
STARTS = {name: [row[0] for row in DATA[name]] for name in ('word', 'grapheme', 'wide', 'scripts', 'combining', 'letters')}
VOWELS = set(map(ord, 'aAeEiIoOuUyYæÆœŒ'))
WHITESPACE = set(range(9, 14)) | set(range(0x1C, 0x21)) | {
    0x1680, 0x180E, *range(0x2000, 0x2007), *range(0x2008, 0x200C),
    0x2028, 0x2029, 0x205F, 0x3000,
}
CHARACTER_ONLY = {5: 'lao', 6: 'khmer', 7: 'myanmar'}
# 462 is the LCM of the denominators in /2.8, /3, /3.3 and /6.
FACTORS = {1: 165, 2: 154, 3: 140, 4: 77}


def prop(name, point):
    index = bisect_right(STARTS[name], point) - 1
    return DATA[name][index][2] if index >= 0 and point <= DATA[name][index][1] else 0


@lru_cache(maxsize=None)
def decompose(point):
    if 0xAC00 <= point <= 0xD7A3:
        syllable = point - 0xAC00
        return (0x1100 + syllable // 588, 0x1161 + syllable % 588 // 28) + ((0x11A7 + syllable % 28,) if syllable % 28 else ())
    return tuple(child for part in DATA['decomposition'].get(str(point), [point])
                 for child in (decompose(part) if part != point else (point,)))


def normalized_points(source):
    # UAX #29 is defined on NFD. Freeze decomposition and canonical ordering too.
    result, marks = [], []
    for point in (part for char in source for part in decompose(ord(char))):
        ccc = prop('combining', point)
        if ccc:
            marks.append((ccc, point))
        else:
            result.extend(point for _, point in sorted(marks, key=lambda item: item[0]))
            marks.clear()
            result.append(point)
    result.extend(point for _, point in sorted(marks, key=lambda item: item[0]))
    return result


def grapheme_join(left, right):
    if left == 7 and right == 8:
        return True
    if left in (7, 8, 9) or right in (7, 8, 9):
        return False
    return (right == 1 or (left == 2 and right in (2, 3, 5, 6))
            or (left in (3, 5) and right in (3, 4)) or (left in (4, 6) and right == 4))


@lru_cache(maxsize=100000)
def measure(source):
    tokens, previous, first = [], 0, True
    characters = {'lao': 0, 'khmer': 0, 'myanmar': 0}
    for point in normalized_points(source):
        grapheme = prop('grapheme', point)
        joined = not first and grapheme_join(previous, grapheme)
        previous, first = grapheme, False
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
        elif point in (0x2D, 0x2010, 0x58A, 0x30A0):
            kind = 4
        tokens.append([point, kind])

    units = 0
    for index, (point, kind) in enumerate(tokens):
        numeric_separator = (kind == 6 and index > 0 and index + 1 < len(tokens)
                             and tokens[index - 1][1] == tokens[index + 1][1] == 2)
        if prop('wide', point) and point not in WHITESPACE and kind != 2 and not numeric_separator:
            units += SCALE
            tokens[index][1] = 8
        elif kind != 2 and not numeric_separator:
            script = prop('scripts', point) or (2 if kind == 3 else 0)
            if script and (prop('letters', point) or kind == 3):
                units += FACTORS.get(script, 0)
                if script in CHARACTER_ONLY:
                    characters[CHARACTER_ONLY[script]] += 1
                tokens[index][1] = 8

    def joins(index):
        point, right = tokens[index]
        left_point, left = tokens[index - 1]
        before = tokens[index - 2][1] if index > 1 else 0
        after = tokens[index + 1][1] if index + 1 < len(tokens) else 0
        if left == 8 or right == 8:
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
            return not (left_point in (0x27, 0x2019) and point in VOWELS)
        return ((left == 2 and right == 6 and after == 2)
                or (before == 2 and left == 6 and right == 2))

    segment_word = False
    for index, (point, kind) in enumerate(tokens):
        if index and not joins(index):
            units += SCALE if segment_word else 0
            segment_word = False
        if kind in (1, 2, 3) and point != 0xA0:
            segment_word = True
    units += SCALE if segment_word else 0
    # Immutable cached result, so callers cannot alter validation state.
    return units, tuple(characters.values())


def count_units(source):
    return measure(source)[0]


def count_words(source):
    from fractions import Fraction
    return Fraction(count_units(source), SCALE)


def character_counts(source):
    return dict(zip(('lao', 'khmer', 'myanmar'), measure(source)[1]))
