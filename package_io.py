import hashlib
import json
import re
import stat
import struct
import zipfile
from contextlib import nullcontext
from uuid import uuid4
from workload_protocol import FIELDS as WORKLOAD_FIELDS, validate_workload, text as has_translation_text


MAX_EXPORT_BYTES = 128 * 1024 * 1024
MAX_ENTRY_BYTES = 64 * 1024 * 1024
MAX_ZIP_BYTES = 2 * 1024 * 1024 * 1024
MAX_VIDEO_BYTES = 512 * 1024 * 1024
MAX_PARTS = 200
CHUNK_BYTES = 1024 * 1024
MAX_INTEGER = 9007199254740991
MEDIA_TYPES = {'localization-preview-video': ('video', 'video.mp4'),
               'localization-preview-map': ('map', 'dialogue-map.json')}
UNIT_FIELDS = ('unitKind', 'chapterName')


def parse_json(content):
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('JSON 字段重复：' + key)
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError('JSON 包含非有限数值：' + value)

    return json.loads(content, object_pairs_hook=unique_object, parse_constant=invalid_constant)


def valid_asset_path(path):
    return (isinstance(path, str) and 0 < len(path) < 240
            and not any(ord(char) < 32 or 127 <= ord(char) <= 159 or char in '\\:%' for char in path)
            and all(part and part not in ('.', '..') for part in path.split('/')))


def part_asset_path(part_name):
    if (not isinstance(part_name, str) or not part_name or part_name[-1] in '. '
            or any(ord(char) < 32 or 127 <= ord(char) <= 159 or char in '\\/:*?"<>|%' for char in part_name)
            or part_name.split('.')[0].upper() in {'CON', 'PRN', 'AUX', 'NUL',
                *('COM' + str(number) for number in range(1, 10)),
                *('LPT' + str(number) for number in range(1, 10))}):
        raise ValueError('片段名不能用作跨平台文件名')
    path = 'parts/' + part_name + '.json'
    if not valid_asset_path(path):
        raise ValueError('片段文件路径过长或无效')
    path.encode('utf-8')
    return path


def media_asset_path(part_name, kind):
    part_asset_path(part_name)
    path = 'previews/' + part_name + '/' + MEDIA_TYPES[kind][1]
    if not valid_asset_path(path):
        raise ValueError('媒体文件路径过长或无效')
    return path


def valid_text(value):
    return (isinstance(value, str) and bool(value.strip())
            and not any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in value))


def valid_hash(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def allows_empty_translation(task, entry):
    return (task.get('sourceKind') == 'unity-minigame'
            and type(task.get('assetProtocolVersion')) is int and task['assetProtocolVersion'] == 1
            and entry.get('allowEmpty') is True)


def accepts_translation(task, entry, text):
    return (isinstance(text, str)
            and (has_translation_text(text) or allows_empty_translation(task, entry)))


def validate_ready_entry(task, entry):
    if (not isinstance(entry.get('review'), dict) or entry['review'].get('state') != 'confirmed'
            or not accepts_translation(task, entry, entry.get('translation'))):
        raise ValueError('仍有未完成或空译文：' + str(entry.get('key')))


def validate_source_snapshots(entry):
    for field in ('currentSource', 'localizationSourceAtExport', 'translationAtExport'):
        if not isinstance(entry.get(field), str):
            raise ValueError('来源快照必须是文本：' + field)
    if 'englishTranslationAtExport' in entry and not isinstance(entry['englishTranslationAtExport'], str):
        raise ValueError('旧英文快照 englishTranslationAtExport 必须是文本')


def valid_integer(value, minimum=0):
    return type(value) is int and minimum <= value <= MAX_INTEGER


def package_names_hash(names):
    normalized = sorted(set(names), key=lambda name: name.encode('utf-8'))
    return hashlib.sha256(json.dumps(normalized, ensure_ascii=False, separators=(',', ':')).encode('utf-8')).hexdigest()


def task_package_names(tasks):
    names = set()
    for task in tasks:
        for entry in task['entries']:
            if not isinstance(entry, dict) or not valid_text(entry.get('key')):
                raise ValueError('词条标识无效')
            key = entry['key']
            separator = key.rfind('_')
            names.add(key[:separator] if separator > 0 else key)
    return names


def unit_metadata(value):
    if not isinstance(value, dict):
        raise ValueError('本地化单元结构无效')
    metadata = {field: value[field] for field in UNIT_FIELDS if field in value}
    if 'unitKind' in metadata and metadata['unitKind'] not in ('chapter', 'part'):
        raise ValueError('unitKind 必须是 chapter 或 part')
    validate_optional_unit_name(metadata, 'chapterName')
    return metadata


def validate_optional_unit_name(value, field):
    name = value.get(field)
    if name is not None:
        if not isinstance(name, str):
            raise ValueError(field + ' 必须是文本或空值')
        if name.strip():
            part_asset_path(name)


def preview_part_names(asset):
    if 'previewPartNames' in asset:
        part_asset_path(asset.get('partName'))
        names = asset['previewPartNames']
        if (asset.get('unitKind') != 'chapter' or not isinstance(names, list)
                or len(names) > MAX_PARTS):
            raise ValueError('只有章节可以声明 previewPartNames，且最多 200 个片段')
        paths = [part_asset_path(name).casefold() for name in names]
        if len(set(paths)) != len(paths) or asset['partName'].casefold() in [name.casefold() for name in names]:
            raise ValueError('章节预览片段重复或指向章节自身')
        return names
    return []


def validate_unit_tasks(asset, data):
    metadata = unit_metadata(asset)
    for value in [data, *data['tasks']]:
        if any((metadata.get(field) or '') != (item or '') for field, item in unit_metadata(value).items()):
            raise ValueError('JSON 根、任务与清单的本地化单元元数据不一致')
    return metadata


def validate_media_owner(mapping, part_name, dialogue_assets):
    direct = dialogue_assets.get(part_name)
    owners = [asset['partName'] for asset in dialogue_assets.values()
              if asset.get('unitKind') == 'chapter' and part_name in preview_part_names(asset)]
    if direct and direct.get('unitKind') == 'chapter':
        raise ValueError('章节不是录制片段，不能拥有自己的视频')
    if not direct and not owners:
        raise ValueError('媒体没有对应片段或章节声明')
    if mapping is None:
        return
    chapter = mapping.get('chapterName')
    if chapter and chapter.strip():
        if direct and direct.get('chapterName') and direct['chapterName'].strip() and chapter != direct['chapterName']:
            raise ValueError('预览映射与片段声明的章节不一致')
    for owner in owners:
        if chapter != owner or not any(event.get('sourceUnit') == owner for event in mapping['events']):
            raise ValueError('章节声明的媒体缺少匹配 chapterName 和 sourceUnit 的真实事件')


def validate_map(mapping, project_id, part_name, recording_id, video_hash, packages):
    fields = {'format', 'formatVersion', 'projectId', 'partName', 'recordingId',
              'hashAlgorithmVersion', 'catalogHash', 'recordedPackagesHash', 'packageNames',
              'recordedPackageNames', 'videoSha256', 'durationMs', 'frameRate', 'events'}
    if not isinstance(mapping, dict) or not fields <= set(mapping) or set(mapping) - fields - {'chapterName'}:
        raise ValueError('预览映射字段缺失或包含未知字段')
    validate_optional_unit_name(mapping, 'chapterName')
    if (mapping['format'] != 'mida-localization-preview'
            or type(mapping['formatVersion']) is not int or mapping['formatVersion'] != 1
            or mapping['hashAlgorithmVersion'] != 'package-names-v1'
            or mapping['projectId'] != project_id or mapping['partName'] != part_name
            or not valid_text(recording_id) or mapping['recordingId'] != recording_id
            or not valid_hash(video_hash) or mapping['videoSha256'] != video_hash
            or not valid_integer(mapping['durationMs'], 1)
            or type(mapping['frameRate']) is not int or mapping['frameRate'] != 30):
        raise ValueError('预览映射身份、视频校验、时长或帧率无效：' + part_name)
    for field, hash_field in (('packageNames', 'catalogHash'), ('recordedPackageNames', 'recordedPackagesHash')):
        names = mapping[field]
        if (not isinstance(names, list) or len(names) > 100000
                or not all(valid_text(name) for name in names)
                or not valid_hash(mapping[hash_field]) or package_names_hash(names) != mapping[hash_field]):
            raise ValueError('预览包名列表或哈希无效：' + field)
    catalog = set(mapping['packageNames'])
    recorded = set(mapping['recordedPackageNames'])
    if (packages is not None and catalog != set(packages)) or not recorded <= catalog:
        raise ValueError('预览目录与所选任务包名不一致，或录制包不属于目录：' + part_name)
    events = mapping['events']
    if not isinstance(events, list) or len(events) > 100000:
        raise ValueError('预览事件列表无效或超过上限')
    occurrences = {}
    previous_frame = -1
    previous_time = -1
    for event in events:
        event_fields = {'packageName', 'occurrence', 'frame', 'timeMs'}
        if not isinstance(event, dict) or not event_fields <= set(event) or set(event) - event_fields - {'sourceUnit'}:
            raise ValueError('预览事件包含缺失或未知字段')
        source = event.get('sourceUnit')
        if source is not None and (not isinstance(source, str) or source.strip() and not valid_text(source)):
            raise ValueError('预览事件 sourceUnit 必须是文本或空值')
        name, frame, time_ms = event['packageName'], event['frame'], event['timeMs']
        if (not valid_text(name) or name not in recorded
                or not valid_integer(event['occurrence'], 1)
                or event['occurrence'] != occurrences.get(name, 0) + 1
                or not valid_integer(frame) or not valid_integer(time_ms)
                or frame < previous_frame or time_ms < previous_time
                or time_ms >= mapping['durationMs'] or frame * 1000 >= mapping['durationMs'] * 30
                or abs(time_ms * 30 - frame * 1000) > 30):
            raise ValueError('预览事件包名、出现次数、帧或时间无效：' + part_name)
        occurrences[name] = event['occurrence']
        previous_frame, previous_time = frame, time_ms
    if set(occurrences) != recorded:
        raise ValueError('录制包列表与事件不一致：' + part_name)


def validate_manifest(manifest):
    if (not isinstance(manifest, dict) or manifest.get('format') != 'mida-localization-manifest'
            or type(manifest.get('formatVersion')) is not int or manifest['formatVersion'] not in (2, 3)):
        raise ValueError('需要逐片段文件的 v2 或 v3 清单')
    version = manifest.get('fileVersion')
    if (not valid_text(manifest.get('projectId')) or not valid_text(manifest.get('packageId'))
            or not valid_text(manifest.get('exportedAt')) or not isinstance(version, dict)
            or not valid_text(version.get('lineageId')) or not valid_integer(version.get('revision'), 1)):
        raise ValueError('清单项目或包版本身份无效')
    assets = manifest.get('assets')
    if not isinstance(assets, list) or not 0 < len(assets) <= MAX_PARTS * 3:
        raise ValueError('清单资产为空或超过 600 个')
    dialogue_assets, media_assets, paths = {}, {}, set()
    for asset in assets:
        if not isinstance(asset, dict):
            raise ValueError('资产结构无效')
        kind, part_name = asset.get('type'), asset.get('partName')
        dialogue_path = part_asset_path(part_name)
        if kind == 'localization-dialogues':
            unit_metadata(asset)
            preview_part_names(asset)
            expected_path, expected_id = dialogue_path, part_name
            if part_name in dialogue_assets:
                raise ValueError('同片段存在重复对话资产')
            dialogue_assets[part_name] = asset
        elif isinstance(kind, str) and kind in MEDIA_TYPES and manifest['formatVersion'] == 3:
            if set(asset) != {'id', 'partName', 'type', 'path', 'sha256', 'byteLength', 'recordingId'}:
                raise ValueError('媒体资产字段缺失或包含未知字段')
            expected_path = media_asset_path(part_name, kind)
            expected_id = part_name + ':' + MEDIA_TYPES[kind][0]
            if not valid_text(asset['recordingId']) or not valid_integer(asset['byteLength'], 1):
                raise ValueError('媒体长度或录制标识无效')
            group = media_assets.setdefault(part_name, {})
            if kind in group:
                raise ValueError('同片段只能包含一套预览媒体')
            group[kind] = asset
        else:
            raise ValueError('不支持的资产类型或清单版本')
        if (asset.get('id') != expected_id or asset.get('path') != expected_path
                or expected_path.casefold() in paths or not valid_hash(asset.get('sha256'))):
            raise ValueError('资产标识、路径、哈希无效或重复')
        paths.add(expected_path.casefold())
    if not 0 < len(dialogue_assets) <= MAX_PARTS:
        raise ValueError('对话片段为空或超过 200 个')
    for asset in dialogue_assets.values():
        if any(part not in media_assets for part in preview_part_names(asset)):
            raise ValueError('previewPartNames 只能声明本 ZIP 实际携带的配对媒体')
    for part_name, group in media_assets.items():
        validate_media_owner(None, part_name, dialogue_assets)
        if (set(group) != set(MEDIA_TYPES)
                or len({asset['recordingId'] for asset in group.values()}) != 1):
            raise ValueError('预览资产未成对、录制身份不一致或缺少所属片段')
    return dialogue_assets, media_assets


def read_entry(archive, entry, maximum, destination=None, progress=lambda count: None):
    digest, count = hashlib.sha256(), 0
    chunks = []
    with archive.open(entry) as source, (destination.open('xb') if destination else nullcontext()) as output:
        while True:
            progress(0)
            chunk = source.read(min(CHUNK_BYTES, maximum - count + 1))
            if not chunk:
                break
            count += len(chunk)
            progress(len(chunk))
            if count > maximum or count > entry.file_size:
                raise ValueError('ZIP 解压长度超过上限：' + entry.filename)
            digest.update(chunk)
            if output:
                output.write(chunk)
            else:
                chunks.append(chunk)
        if count != entry.file_size:
            raise ValueError('ZIP 文件长度异常：' + entry.filename)
    return (None if destination else b''.join(chunks)), digest.hexdigest(), count


def validate_zip_directory(source):
    source.seek(0, 2)
    length = source.tell()
    if not 0 < length <= MAX_ZIP_BYTES:
        raise ValueError('ZIP 总大小超过 2 GiB 或为空')
    source.seek(max(0, length - 65557))
    tail = source.read(65557)
    offset = tail.rfind(b'PK\x05\x06')
    if offset < 0 or len(tail) - offset < 22:
        raise ValueError('ZIP 缺少有效的中央目录结束记录')
    _, disk, directory_disk, disk_entries, entries, directory_size, directory_offset, comment_size = struct.unpack_from('<4s4H2IH', tail, offset)
    end_offset = length - len(tail) + offset
    if len(tail) - offset != 22 + comment_size or disk or directory_disk or disk_entries != entries:
        raise ValueError('ZIP 尾部或分卷信息无效')
    if end_offset >= 20:
        source.seek(end_offset - 20)
        locator = source.read(20)
        if locator[:4] == b'PK\x06\x07':
            _, zip64_disk, zip64_offset, disks = struct.unpack('<4sIQI', locator)
            if zip64_disk or disks != 1 or zip64_offset > end_offset - 76:
                raise ValueError('ZIP64 目录定位无效或使用分卷')
            source.seek(zip64_offset)
            header = source.read(56)
            if len(header) != 56:
                raise ValueError('ZIP64 目录结束记录不完整')
            signature, record_size, _, _, disk, directory_disk, disk_entries, entries, directory_size, directory_offset = struct.unpack('<4sQ2H2I4Q', header)
            if (signature != b'PK\x06\x06' or record_size != 44 or disk or directory_disk
                    or disk_entries != entries or zip64_offset + 56 != end_offset - 20):
                raise ValueError('ZIP64 目录结束记录无效')
            end_offset = zip64_offset
    if (not 0 < entries <= MAX_PARTS * 3 + 1 or not 0 < directory_size <= 32 * 1024 * 1024
            or directory_offset + directory_size != end_offset):
        raise ValueError('ZIP 文件数量、中央目录大小或位置超过限制')
    source.seek(0)


def read_package(source, media_directory, progress=lambda fraction, phase: None):
    progress(0, "检查 ZIP 目录")
    validate_zip_directory(source)
    with zipfile.ZipFile(source) as archive:
        entries = archive.infolist()
        if not 0 < len(entries) <= MAX_PARTS * 3 + 1:
            raise ValueError('ZIP 文件数量为空或超过 601 个')
        names, files, total = set(), {}, 0
        for entry in entries:
            name = entry.filename
            mode = entry.external_attr >> 16
            if (entry.orig_filename != name or not valid_asset_path(name) or name.casefold() in names
                    or entry.is_dir() or stat.S_ISLNK(mode)
                    or stat.S_IFMT(mode) not in (0, stat.S_IFREG) or entry.flag_bits & 1):
                raise ValueError('ZIP 包含非法路径、重复文件、特殊文件或加密内容')
            if entry.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
                raise ValueError('ZIP 仅支持标准存储或 Deflate 压缩')
            if entry.file_size < 0 or entry.file_size > MAX_VIDEO_BYTES:
                raise ValueError('ZIP 单个文件超过上限')
            total += entry.file_size
            if total > MAX_ZIP_BYTES:
                raise ValueError('ZIP 解压总大小超过 2 GiB')
            names.add(name.casefold())
            files[name] = entry
        if 'manifest.json' not in files or files['manifest.json'].file_size > MAX_ENTRY_BYTES:
            raise ValueError('ZIP 根目录缺少清单或清单过大')
        completed = 0
        def advance(count):
            nonlocal completed
            completed += count
            progress(min(0.99, completed / max(1, total)), '解压并校验文件')
        content, _, _ = read_entry(archive, files['manifest.json'], MAX_ENTRY_BYTES, progress=advance)
        manifest = parse_json(content.decode('utf-8-sig'))
        dialogue_assets, media_assets = validate_manifest(manifest)
        if set(files) != {'manifest.json', *(asset['path'] for asset in manifest['assets'])}:
            raise ValueError('ZIP 文件与清单不一致，存在缺失或未声明的资产')
        text_total = files['manifest.json'].file_size
        for asset in manifest['assets']:
            entry = files[asset['path']]
            maximum = MAX_VIDEO_BYTES if asset['type'] == 'localization-preview-video' else MAX_ENTRY_BYTES
            if entry.file_size > maximum:
                raise ValueError('ZIP 单个资产超过大小上限')
            if asset['type'] != 'localization-preview-video':
                text_total += entry.file_size
            if 'byteLength' in asset and (type(asset['byteLength']) is not int or asset['byteLength'] != entry.file_size):
                raise ValueError('资产声明长度不匹配')
        if text_total > MAX_EXPORT_BYTES:
            raise ValueError('ZIP 文本解压总大小超过 128 MiB')
        texts, tasks_by_part, previews = {}, {}, {}
        task_count, entry_count, delivery = 0, 0, None
        for part_name, asset in dialogue_assets.items():
            content, digest, _ = read_entry(archive, files[asset['path']], MAX_ENTRY_BYTES, progress=advance)
            if digest != asset['sha256']:
                raise ValueError('片段文件校验失败：' + asset['path'])
            text = content.decode('utf-8')
            data = parse_json(text)
            if (not isinstance(data, dict) or data.get('format') != 'mida-localization'
                    or type(data.get('formatVersion')) is not int or data['formatVersion'] != 1
                    or any(data.get(key) != manifest.get(key) for key in ('projectId', 'packageId', 'fileVersion', 'exportedAt'))):
                raise ValueError('片段文件格式或版本身份与清单不一致')
            current = (data.get('deliveryState'), data.get('demo', False), data.get('parentPackageId'))
            if current[0] not in ('draft', 'ready') or current[1] is not False or delivery is not None and current != delivery:
                raise ValueError('片段文件交付状态或来源不一致')
            delivery = current
            tasks = data.get('tasks')
            if not isinstance(tasks, list) or not tasks:
                raise ValueError('片段文件没有任务')
            validate_unit_tasks(asset, data)
            task_count += len(tasks)
            languages = set()
            for task in tasks:
                if (not isinstance(task, dict) or task.get('partName') != part_name
                        or not valid_text(task.get('language')) or task['language'] in languages):
                    raise ValueError('片段归属、语言任务无效或重复')
                languages.add(task['language'])
                entries = task.get('entries')
                if not isinstance(entries, list) or not entries:
                    raise ValueError('片段任务没有词条')
                keys = set()
                for index, entry in enumerate(entries):
                    if index % 256 == 0:
                        advance(0)
                    if not isinstance(entry, dict) or not valid_text(entry.get('key')) or entry['key'] in keys:
                        raise ValueError('词条标识无效或重复')
                    keys.add(entry['key'])
                    validate_source_snapshots(entry)
                    if data['deliveryState'] == 'ready':
                        validate_ready_entry(task, entry)
                entry_count += len(entries)
            if task_count > 1000 or entry_count > 100000:
                raise ValueError('任务或词条总数超过上限')
            texts[asset['path']], tasks_by_part[part_name] = text, tasks
        validate_workload(manifest, [task for group in tasks_by_part.values() for task in group])
        for part_name, group in media_assets.items():
            video, map_asset = group['localization-preview-video'], group['localization-preview-map']
            media_id = uuid4().hex
            _, digest, _ = read_entry(archive, files[video['path']], MAX_VIDEO_BYTES, media_directory / (media_id + '.mp4'), progress=advance)
            if digest != video['sha256']:
                raise ValueError('视频哈希校验失败：' + part_name)
            content, digest, _ = read_entry(archive, files[map_asset['path']], MAX_ENTRY_BYTES, progress=advance)
            if digest != map_asset['sha256']:
                raise ValueError('映射哈希校验失败：' + part_name)
            mapping = parse_json(content.decode('utf-8'))
            # Chapter recordings can contain source units whose dialogue JSON is not in this ZIP.
            has_chapter = isinstance(mapping, dict) and isinstance(mapping.get('chapterName'), str) and mapping['chapterName'].strip()
            packages = (task_package_names(tasks_by_part[part_name])
                        if part_name in tasks_by_part and not has_chapter else None)
            validate_map(mapping, manifest['projectId'], part_name, video['recordingId'], video['sha256'], packages)
            validate_media_owner(mapping, part_name, dialogue_assets)
            (media_directory / (media_id + '.json')).write_bytes(content)
            previews[part_name] = {'recordingId': video['recordingId'], 'map': mapping, 'mediaId': media_id}
        progress(1, '校验完成')
        return {'manifest': manifest, 'assets': texts, 'previews': previews}


def write_package(output, document):
    validate_workload(document, document['tasks'])
    manifest = {key: document[key] for key in ('projectId', 'packageId', 'fileVersion', 'exportedAt')}
    manifest.update({key: document[key] for key in WORKLOAD_FIELDS if key in document})
    manifest.update(format='mida-localization-manifest', formatVersion=2, assets=[])
    groups = {}
    for task in document['tasks']:
        for entry in task['entries']:
            validate_source_snapshots(entry)
            if document.get('deliveryState') == 'ready':
                validate_ready_entry(task, entry)
        groups.setdefault(task['partName'], []).append(task)
    if not 0 < len(groups) <= MAX_PARTS:
        raise ValueError('一次最多导出 200 个片段')
    files, paths, text_total = {}, set(), 0
    for part_name, tasks in groups.items():
        metadata = unit_metadata(tasks[0])
        if any(unit_metadata(task) != metadata for task in tasks):
            raise ValueError('同单元不同语言的章节元数据不一致')
        path = part_asset_path(part_name)
        if path.casefold() in paths:
            raise ValueError('片段文件名存在大小写冲突')
        paths.add(path.casefold())
        data = {key: value for key, value in document.items() if key not in ('tasks', 'previews', 'previewPartNames', *UNIT_FIELDS, *WORKLOAD_FIELDS)}
        exported_tasks = [{key: value for key, value in task.items() if key != 'previewPartNames'} for task in tasks]
        content = json.dumps(dict(data, **metadata, tasks=exported_tasks), ensure_ascii=False, indent=2).encode('utf-8')
        text_total += len(content)
        if len(content) > MAX_ENTRY_BYTES or text_total > MAX_EXPORT_BYTES:
            raise ValueError('片段文件或文本总大小超过上限')
        files[path] = content
        manifest['assets'].append({'id': part_name, 'partName': part_name, 'type': 'localization-dialogues',
                                   'path': path, 'sha256': hashlib.sha256(content).hexdigest(), **metadata})
        if metadata.get('unitKind') == 'chapter':
            manifest['assets'][-1]['previewPartNames'] = []
    manifest_bytes = json.dumps(manifest, ensure_ascii=False, indent=2).encode('utf-8')
    text_total += len(manifest_bytes)
    if len(manifest_bytes) > MAX_ENTRY_BYTES or text_total > MAX_EXPORT_BYTES:
        raise ValueError('清单、文本总大小或解压总大小超过上限')
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED, allowZip64=True) as archive:
        archive.writestr('manifest.json', manifest_bytes)
        for path, content in files.items():
            archive.writestr(path, content)
    if output.tell() > MAX_ZIP_BYTES:
        raise ValueError('压缩包超过 2 GiB')
