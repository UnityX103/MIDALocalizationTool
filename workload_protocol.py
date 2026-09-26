"""Manifest-only workload extension, shared with the desktop and Unity clients."""

MAX_RECORDS = 100000
MAX_INTEGER = 9007199254740991
FIELDS = ('delivery', 'workload', 'workReceipts')
WHITESPACE = ''.join(chr(value) for start, end in (
    (0x9, 0xD), (0x1C, 0x20), (0x85, 0x85), (0xA0, 0xA0),
    (0x1680, 0x1680), (0x2000, 0x200A), (0x2028, 0x2029),
    (0x202F, 0x202F), (0x205F, 0x205F), (0x3000, 0x3000), (0xFEFF, 0xFEFF),
) for value in range(start, end + 1))


def count_han(source):
    ranges = ((0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF),
              (0x20000, 0x2EBEF), (0x2F800, 0x2FA1F), (0x30000, 0x323AF))
    return sum(ord(char) == 0x3007 or any(start <= ord(char) <= end for start, end in ranges)
               for char in source)


def require(condition, message):
    if not condition:
        raise ValueError('工作量清单：' + message)


def text(value):
    return isinstance(value, str) and bool(value.strip(WHITESPACE))


def integer(value, minimum=0):
    return type(value) is int and minimum <= value <= MAX_INTEGER


def unique_strings(value, name, maximum=MAX_RECORDS):
    require(isinstance(value, list) and len(value) <= maximum and all(text(item) for item in value),
            name + '无效')
    require(len(set(value)) == len(value), name + '重复')
    return set(value)


def validate_workload(document, tasks):
    if 'workReceipts' in document:
        receipts = document['workReceipts']
        require(isinstance(receipts, list) and len(receipts) <= 10000, '回执列表无效')
        seen = set()
        for receipt in receipts:
            require(isinstance(receipt, dict) and type(receipt.get('version')) is int
                    and receipt['version'] == 1
                    and all(text(receipt.get(field)) for field in
                            ('id', 'projectId', 'lineageId', 'language', 'ledgerId', 'deliveryId', 'receivedAt')),
                    '回执身份无效')
            require(receipt['id'] not in seen, '回执 ID 重复')
            seen.add(receipt['id'])
            require(receipt['projectId'] == document.get('projectId')
                    and receipt['lineageId'] == document.get('fileVersion', {}).get('lineageId'),
                    '回执项目或谱系不匹配')
            unique_strings(receipt.get('recordIds'), '回执记录')
    if 'delivery' not in document and 'workload' not in document:
        return
    delivery, workload = document.get('delivery'), document.get('workload')
    require(isinstance(delivery, dict) and type(delivery.get('version')) is int
            and delivery['version'] == 1 and text(delivery.get('id'))
            and delivery['id'] == document.get('packageId') and text(delivery.get('ledgerId'))
            and integer(delivery.get('revision'), 1) and 'previousId' in delivery
            and (delivery['previousId'] is None or text(delivery['previousId']))
            and delivery['previousId'] != delivery['id'], '交付身份无效')
    require(isinstance(workload, dict) and type(workload.get('version')) is int
            and workload['version'] == 1 and workload.get('countingRule') == 'han-v1',
            '统计规则无效')
    scope = workload.get('scope')
    require(isinstance(scope, dict) and scope.get('projectId') == document.get('projectId')
            and scope.get('lineageId') == document.get('fileVersion', {}).get('lineageId')
            and text(scope.get('language')), '统计范围无效')
    parts = unique_strings(scope.get('partNames'), '统计片段', 200)
    require(parts and parts == {task.get('partName') for task in tasks}
            and all(task.get('language') == scope['language'] for task in tasks),
            '统计范围与任务不一致')
    records = workload.get('records')
    require(isinstance(records, list) and len(records) <= MAX_RECORDS, '工作记录超出上限')
    by_id, identities = {}, set()
    for record in records:
        require(isinstance(record, dict)
                and all(text(record.get(field)) for field in
                        ('id', 'partName', 'language', 'key', 'confirmedAt')),
                '工作记录身份缺失')
        require(isinstance(record.get('sourceText'), str) and isinstance(record.get('translation'), str),
                '工作记录文本无效')
        provenance = ('sourcePackageId' in record and 'sourceRevision' in record
                      and ((record['sourcePackageId'] is None and record['sourceRevision'] is None)
                           or (text(record['sourcePackageId']) and integer(record['sourceRevision'], 1))))
        require(integer(record.get('chars')) and record['chars'] == count_han(record['sourceText'])
                and provenance and integer(record.get('taskVersion'), 1)
                and record.get('kind') in ('translation', 'source_revision'), '工作记录字数或版本错误')
        require(record['partName'] in parts and record['language'] == scope['language'], '记录超出范围')
        identity = (record['partName'], record['language'], record['key'], record['sourceText'])
        require(record['id'] not in by_id and identity not in identities, '工作记录重复')
        by_id[record['id']] = record
        identities.add(identity)
    subsets = {}
    for field in ('acknowledgedRecordIds', 'newRecordIds', 'sourceUpdateRecordIds'):
        subsets[field] = unique_strings(workload.get(field), field)
        require(subsets[field] <= by_id.keys(), field + '含未知记录')
    progress, total = 0, 0
    for task in tasks:
        for entry in task['entries']:
            chars = count_han(entry['currentSource'])
            total += chars
            translation = entry.get('translation')
            allows_empty = (task.get('sourceKind') == 'unity-minigame'
                            and type(task.get('assetProtocolVersion')) is int
                            and task['assetProtocolVersion'] == 1 and entry.get('allowEmpty') is True)
            if (entry.get('review', {}).get('state') == 'confirmed'
                    and isinstance(translation, str) and (text(translation) or allows_empty)):
                progress += chars
    def sum_chars(ids):
        return sum(by_id[identifier]['chars'] for identifier in ids)
    expected = {
        'progressChars': progress, 'totalChars': total,
        'cumulativeChars': sum_chars(by_id),
        'handoverChars': sum_chars(by_id.keys() - subsets['acknowledgedRecordIds']),
        'previousDeliveryDeltaChars': sum_chars(subsets['newRecordIds']),
        'sinceSourceUpdateChars': sum_chars(subsets['sourceUpdateRecordIds']),
    }
    for field, value in expected.items():
        require(integer(workload.get(field)) and workload[field] == value, field + '与明细不一致')
