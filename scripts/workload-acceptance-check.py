"""Verify real round-trip artifacts without modifying any user workspace."""
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from package_io import read_package, write_package
from workload_protocol import validate_workload
from word_count import RULE, SCALE, count_units, character_counts, normalized_points, measure, prop


def verify_counting():
    cases = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/workload-counting-cases.json').read_text())
    for case in cases:
        source, expected = case['text'], case.get('units', case.get('words', 0) * SCALE)
        assert count_units(source) == expected, repr(source)
        task = {'partName': 'PartA', 'language': 'en', 'entries': [
            {'currentSource': source, 'translation': 'Translation', 'review': {'state': 'confirmed'}}]}
        record = {'id': 'record', 'partName': 'PartA', 'language': 'en', 'key': 'key',
                  'sourceText': source, 'translation': 'Translation', 'sourcePackageId': 'source',
                  'sourceRevision': 1, 'taskVersion': 1, 'kind': 'translation', 'chars': expected,
                  'confirmedAt': '2026-10-15T00:00:00Z', 'characterCounts': case.get('characterCounts', {'lao':0,'khmer':0,'myanmar':0})}
        assert character_counts(source) == record['characterCounts']
        document = {'projectId': 'project', 'packageId': 'delivery', 'fileVersion': {'lineageId': 'lineage'},
                    'delivery': {'version': 1, 'id': 'delivery', 'ledgerId': 'ledger', 'revision': 1, 'previousId': None},
                    'workload': {'version': 1, 'countingRule': RULE, 'countingScale': SCALE,
                                 'progressCharacterCounts': record['characterCounts'], 'totalCharacterCounts': record['characterCounts'],
                                 'scope': {'projectId': 'project', 'lineageId': 'lineage', 'language': 'en', 'partNames': ['PartA']},
                                 'records': [record], 'acknowledgedRecordIds': [], 'newRecordIds': ['record'],
                                 'sourceUpdateRecordIds': ['record'],
                                 **{field: expected for field in ('progressChars', 'totalChars', 'cumulativeChars',
                                     'handoverChars', 'previousDeliveryDeltaChars', 'sinceSourceUpdateChars')}}}
        validate_workload(document, [task])
        document['workload']['records'][0]['chars'] += 1
        try:
            validate_workload(document, [task])
        except ValueError:
            pass
        else:
            raise AssertionError('错误明细计数被接受')
    print(f'Python 按新口径通过 {len(cases)} 组计数及对账校验；未访问用户工作区')


def verify_unicode(directory):
    normalized = 0
    for line in (directory / 'NormalizationTest.txt').read_text(encoding='utf-8').splitlines():
        line = line.split('#')[0].strip()
        if not line or line.startswith('@'):
            continue
        forms = [''.join(chr(int(point, 16)) for point in col.split()) for col in line.split(';')[:5]]
        c1, c2, c3, c4, c5 = forms
        assert all(normalized_points(value) == list(map(ord, c3)) for value in (c1, c2, c3)), line
        assert all(normalized_points(value) == list(map(ord, c5)) for value in (c4, c5)), line
        assert measure(c1) == measure(c2) == measure(c3), line
        normalized += 1
    boundaries, excluded = 0, 0
    for line in (directory / 'WordBreakTest.txt').read_text(encoding='utf-8').splitlines():
        line = line.split('#')[0].strip()
        if not line or line.startswith('@'):
            continue
        fields = line.split()
        points = [int(value, 16) for value in fields if value not in ('÷', '×')]
        # GMX-V mandates changes to apostrophes, hyphens and ZWSP; fullwidth and
        # script estimates are covered by the shared project-specific examples.
        if any(prop('wide', p) or prop('scripts', p) or p in
               (0x27, 0x2019, 0x2D, 0x2010, 0x58A, 0x30A0, 0x200B) for p in points):
            excluded += 1
            continue
        segments, segment = [], []
        for value in fields:
            if value == '÷':
                if segment:
                    segments.append(segment)
                    segment = []
            elif value != '×':
                segment.append(int(value, 16))
        if segment:
            segments.append(segment)
        expected = sum(any(prop('word', p) in (1, 2, 3) and p != 0xA0 for p in segment)
                       for segment in segments) * SCALE
        assert count_units(''.join(map(chr, points))) == expected, line
        boundaries += 1
    assert normalized and boundaries
    print(f'Unicode 4.1 官方语料：{normalized} 组正规化、{boundaries} 组普通词界通过；'
          f'{excluded} 组涉及 GMX-V 或全角差异，另由共享用例覆盖')


def load(path):
    with tempfile.TemporaryDirectory() as directory:
        with path.open('rb') as source:
            result = read_package(source, Path(directory))
    documents = [json.loads(text) for text in result['assets'].values()]
    document = dict(documents[0])
    document['tasks'] = [task for data in documents for task in data['tasks']]
    for field in ('delivery', 'workload', 'workReceipts'):
        if field in result['manifest']:
            document[field] = result['manifest'][field]
        assert all(field not in data for data in documents), 'Ledger must stay in manifest only'
    return result['manifest'], document


def verify(directory):
    summaries = []
    for path in sorted(directory.rglob('*.zip')):
        if path.name.startswith('invalid-'):
            continue
        manifest, document = load(path)
        with io.BytesIO() as output:
            write_package(output, document)
            output.seek(0)
            with zipfile.ZipFile(output) as archive:
                rebuilt = json.loads(archive.read('manifest.json'))
            for field in ('delivery', 'workload', 'workReceipts'):
                assert rebuilt.get(field) == manifest.get(field), (path.name, field)
        if 'workload' in document:
            corrupt = copy.deepcopy(document)
            corrupt['workload']['cumulativeChars'] += 1
            try:
                write_package(io.BytesIO(), corrupt)
            except ValueError:
                pass
            else:
                raise AssertionError('Tampered total accepted')
        summaries.append({
            'file': str(path.relative_to(directory)), 'packageId': manifest['packageId'],
            'delivery': manifest.get('delivery'),
            'counts': {key: value for key, value in manifest.get('workload', {}).items()
                       if key.endswith('Chars')},
            'receiptCount': len(manifest.get('workReceipts', [])),
        })
    first, _ = load(directory / 'delivery-1.zip')
    repeat, _ = load(directory / 'delivery-1-repeat.zip')
    polished, polished_data = load(directory / 'delivery-2-polished.zip')
    assert first == repeat, 'Repeated unchanged export must preserve full manifest'
    assert first['workload']['records'] == polished['workload']['records']
    assert polished['delivery']['revision'] == first['delivery']['revision'] + 1
    assert polished['workload']['previousDeliveryDeltaChars'] == 0
    assert polished['workload']['handoverChars'] == 7
    assert polished['workload']['cumulativeChars'] == 7
    for filename in ('delivery-2-retry.zip', 'delivery-2-reload.zip'):
        assert load(directory / filename)[0] == polished, filename
    for filename, expected in (
        ('delivery-3-acknowledged.zip', (3, 8, 7, 0, 0)),
        ('delivery-4-source-update.zip', (8, 8, 12, 5, 5)),
        ('delivery-5-deleted.zip', (5, 5, 12, 0, 0)),
    ):
        workload = load(directory / filename)[0]['workload']
        actual = tuple(workload[field] for field in (
            'progressChars', 'totalChars', 'cumulativeChars', 'handoverChars', 'previousDeliveryDeltaChars'))
        assert actual == expected, (filename, actual, expected)
    greeting = next(entry for task in polished_data['tasks'] for entry in task['entries']
                    if entry['currentSource'] == '你好世界')
    assert greeting['translation'] == 'Greetings, world!'
    print(json.dumps({'passed': True, 'packages': summaries}, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--unicode-dir':
        verify_counting()
        verify_unicode(Path(sys.argv[2]))
    elif sys.argv[1:] == ['--counting-only']:
        verify_counting()
    else:
        verify(Path(sys.argv[1]).resolve())
