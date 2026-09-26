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
    verify(Path(sys.argv[1]).resolve())
