"""Freeze shared Unicode tables and the browser's embedded copy (no runtime downloads).

Rebuild: python3 scripts/generate-workload-counting.py --unicode-dir <ucd-files>
Verify:  python3 scripts/generate-workload-counting.py --check
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = {
    'WordBreakProperty.txt': 'https://www.unicode.org/Public/4.1.0/ucd/auxiliary/WordBreakProperty.txt',
    'GraphemeBreakProperty.txt': 'https://www.unicode.org/Public/4.1.0/ucd/auxiliary/GraphemeBreakProperty.txt',
    'EastAsianWidth.txt': 'https://www.unicode.org/Public/17.0.0/ucd/EastAsianWidth.txt',
    'Scripts.txt': 'https://www.unicode.org/Public/4.1.0/ucd/Scripts.txt',
    'UnicodeData.txt': 'https://www.unicode.org/Public/4.1.0/ucd/UnicodeData.txt',
}
WORD = {'ALetter': 1, 'Numeric': 2, 'Katakana': 3, 'ExtendNumLet': 4,
        'MidLetter': 5, 'MidNum': 6, 'Format': 7}
GRAPHEME = {'Extend': 1, 'L': 2, 'V': 3, 'T': 4, 'LV': 5, 'LVT': 6,
            'CR': 7, 'LF': 8, 'Control': 9}
SCRIPTS = {'Han': 1, 'Hiragana': 2, 'Katakana': 2, 'Hangul': 3,
           'Thai': 4, 'Lao': 5, 'Khmer': 6, 'Myanmar': 7}


def unicode_data(path):
    decompositions, combining, letters = {}, [], []
    first = None
    for line in path.read_text(encoding='utf-8').splitlines():
        fields = line.split(';')
        point, name, category, ccc, decomposition = int(fields[0], 16), fields[1], fields[2], int(fields[3]), fields[5]
        if decomposition and not decomposition.startswith('<'):
            decompositions[str(point)] = [int(value, 16) for value in decomposition.split()]
        if ccc:
            combining.append([point, point, ccc])
        if name.endswith(', First>'):
            first = point
        elif category.startswith(('L', 'M')):
            letters.append([first if name.endswith(', Last>') else point, point, 1])
            first = None
    def merge(rows):
        result = []
        for start, end, kind in rows:
            if result and result[-1][1] + 1 == start and result[-1][2] == kind:
                result[-1][1] = end
            else:
                result.append([start, end, kind])
        return result
    return decompositions, merge(combining), merge(letters)


def ranges(path, classes):
    result = []
    for line in path.read_text(encoding='utf-8').splitlines():
        line = line.split('#')[0].strip()
        if not line:
            continue
        code, name = map(str.strip, line.split(';'))
        if name not in classes:
            continue
        ends = code.split('..')
        result.append([int(ends[0], 16), int(ends[-1], 16), classes[name]])
    result.sort()
    merged = []
    for start, end, kind in result:
        if merged and merged[-1][1] + 1 == start and merged[-1][2] == kind:
            merged[-1][1] = end
        else:
            merged.append([start, end, kind])
    return merged


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--unicode-dir', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    destination = ROOT / 'workload-counting.json'
    if args.unicode_dir:
        decompositions, combining, letters = unicode_data(args.unicode_dir / 'UnicodeData.txt')
        data = {
            'rule': 'mida-gmx-fullwidth-v2',
            'scale': 462,
            'wordUnicodeVersion': '4.1.0',
            'widthUnicodeVersion': '17.0.0',
            'sources': [{'url': url, 'sha256': hashlib.sha256((args.unicode_dir / name).read_bytes()).hexdigest()}
                        for name, url in SOURCES.items()],
            'word': ranges(args.unicode_dir / 'WordBreakProperty.txt', WORD),
            'grapheme': ranges(args.unicode_dir / 'GraphemeBreakProperty.txt', GRAPHEME),
            'wide': ranges(args.unicode_dir / 'EastAsianWidth.txt', {'W': 1, 'F': 1}),
            'scripts': ranges(args.unicode_dir / 'Scripts.txt', SCRIPTS),
            'decomposition': decompositions,
            'combining': combining,
            'letters': letters,
        }
        destination.write_text(json.dumps(data, ensure_ascii=False, separators=(',', ':')) + '\n', encoding='utf-8')
    data = json.loads(destination.read_text(encoding='utf-8'))
    value = json.dumps(data, ensure_ascii=False, separators=(',', ':'))
    path = ROOT / 'workload.js'
    source = path.read_text(encoding='utf-8')
    before, rest = source.split('/* BEGIN COUNT DATA */', 1)
    _, after = rest.split('/* END COUNT DATA */', 1)
    expected = before + '/* BEGIN COUNT DATA */' + value + '/* END COUNT DATA */' + after
    if args.check:
        if source != expected:
            raise SystemExit('workload.js 的固定 Unicode 表与 workload-counting.json 不一致')
        print('共享计数表与浏览器内嵌数据一致')
    else:
        path.write_text(expected, encoding='utf-8')


if __name__ == '__main__':
    main()
