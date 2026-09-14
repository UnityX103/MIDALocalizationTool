import hashlib
import json
import os
import re
import shutil
import sqlite3
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from uuid import uuid4

from package_io import (CHUNK_BYTES, MAX_ENTRY_BYTES, MAX_VIDEO_BYTES, MAX_ZIP_BYTES, parse_json,
                        read_package, validate_map, valid_text)


class MediaError(ValueError):
    def __init__(self, message, status=400):
        super().__init__(message)
        self.status = status


def opaque_id(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{32}', value) is not None


def sync_directory(path):
    if os.name == 'nt':
        return
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


@contextmanager
def process_lock(handle):
    if os.name == 'nt':
        import msvcrt
        handle.seek(0, 2)
        if handle.tell() == 0:
            handle.write(b'\0')
            handle.flush()
        handle.seek(0)
        msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        try:
            yield
        finally:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
    else:
        import fcntl
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


class MediaStore:
    def __init__(self, output_directory):
        self.root = Path(output_directory) / '.media-cache'
        self.root.mkdir(parents=True, exist_ok=True)
        if self.root.is_symlink():
            raise ValueError('媒体缓存目录不能是符号链接')
        self.root = self.root.resolve()
        self.staging = self.root / 'staging'
        self.objects = self.root / 'objects'
        for directory in (self.staging, self.objects):
            directory.mkdir(exist_ok=True)
            if directory.is_symlink():
                raise ValueError('媒体缓存子目录不能是符号链接')
        self.mutex = threading.RLock()
        self.lock_path = self.root / 'store.lock'
        self.database_path = self.root / 'store.sqlite3'
        with self.locked() as database:
            database.executescript('''
                CREATE TABLE IF NOT EXISTS imports (
                    token TEXT PRIMARY KEY, project_id TEXT NOT NULL,
                    state TEXT NOT NULL, parts TEXT NOT NULL, records TEXT NOT NULL,
                    baseline TEXT NOT NULL, created REAL NOT NULL
                );
                CREATE TABLE IF NOT EXISTS current_parts (
                    project_id TEXT NOT NULL, part_name TEXT NOT NULL,
                    revision INTEGER NOT NULL, token TEXT NOT NULL,
                    media_id TEXT UNIQUE, record TEXT,
                    PRIMARY KEY (project_id, part_name)
                );
                CREATE TABLE IF NOT EXISTS garbage (
                    media_id TEXT PRIMARY KEY, token TEXT NOT NULL,
                    project_id TEXT NOT NULL, part_name TEXT NOT NULL
                );
            ''')
            database.commit()
            self.cleanup_garbage(database)

    @contextmanager
    def locked(self):
        with self.mutex:
            if self.lock_path.is_symlink() or self.database_path.is_symlink():
                raise MediaError('媒体缓存索引路径无效', 500)
            with self.lock_path.open('a+b') as lock, process_lock(lock):
                database = None
                try:
                    database = sqlite3.connect(self.database_path, timeout=30)
                    database.row_factory = sqlite3.Row
                    database.execute('PRAGMA synchronous=FULL')
                    yield database
                finally:
                    if database is not None:
                        database.close()

    def stage_directory(self, token):
        if not opaque_id(token):
            raise MediaError('媒体导入 token 无效')
        path = self.staging / token
        if path.is_symlink() or path.resolve().parent != self.staging:
            raise MediaError('媒体暂存路径无效', 500)
        return path

    def object_path(self, media_id, suffix='.mp4'):
        if not opaque_id(media_id) or suffix not in ('.mp4', '.json'):
            raise MediaError('视频不存在或已清理', 404)
        path = self.objects / (media_id + suffix)
        if self.objects.is_symlink() or path.is_symlink() or path.resolve().parent != self.objects:
            raise MediaError('视频不存在或已清理', 404)
        return path

    def staged_path(self, token, media_id, suffix):
        if not opaque_id(media_id) or suffix not in ('.mp4', '.json'):
            raise MediaError('媒体标识无效')
        path = self.stage_directory(token) / (media_id + suffix)
        if path.is_symlink():
            raise MediaError('媒体暂存文件无效', 500)
        return path

    def revisions(self, database, project_id, parts):
        result = {}
        for part_name in parts:
            row = database.execute('SELECT revision FROM current_parts WHERE project_id=? AND part_name=?',
                                   (project_id, part_name)).fetchone()
            result[part_name] = row['revision'] if row else 0
        return result

    def import_stream(self, source, length, progress=lambda fraction, phase: None):
        if type(length) is not int or not 0 < length <= MAX_ZIP_BYTES:
            raise MediaError('上传长度为空或超过 2 GiB', 413)
        token = uuid4().hex
        with self.locked():
            directory = self.stage_directory(token)
            directory.mkdir()
        upload = directory / 'upload.zip'
        registered = False
        try:
            remaining = length
            with upload.open('xb') as output:
                while remaining:
                    progress(0.2 * (length - remaining) / length, '上传压缩包')
                    chunk = source.read(min(CHUNK_BYTES, remaining))
                    if not chunk:
                        raise MediaError('上传文件内容不完整')
                    output.write(chunk)
                    remaining -= len(chunk)
            with upload.open('rb') as archive:
                result = read_package(archive, directory, lambda fraction, phase: progress(0.2 + 0.8 * fraction, phase))
            upload.unlink()
            project_id = result['manifest']['projectId']
            parts = [asset['partName'] for asset in result['manifest']['assets'] if asset['type'] == 'localization-dialogues']
            records = {}
            for part_name, preview in result['previews'].items():
                media_id = preview['mediaId']
                video = self.staged_path(token, media_id, '.mp4')
                mapping = self.staged_path(token, media_id, '.json')
                map_bytes = mapping.read_bytes()
                records[part_name] = {'mediaId': media_id, 'recordingId': preview['recordingId'],
                    'sha256': preview['map']['videoSha256'], 'byteLength': video.stat().st_size,
                    'mapSha256': hashlib.sha256(map_bytes).hexdigest(), 'mapByteLength': len(map_bytes)}
                for path in (video, mapping):
                    with path.open('rb') as stored:
                        os.fsync(stored.fileno())
            sync_directory(directory)
            sync_directory(self.staging)
            progress(1, '校验完成')
            with self.locked() as database:
                baseline = self.revisions(database, project_id, parts)
                database.execute('INSERT INTO imports VALUES (?, ?, ?, ?, ?, ?, ?)',
                                 (token, project_id, 'staged', json.dumps(parts), json.dumps(records),
                                  json.dumps(baseline), time.time()))
                database.commit()
                registered = True
            result['mediaImportToken'] = token
            return result
        finally:
            if not registered:
                with self.locked():
                    shutil.rmtree(directory, ignore_errors=True)

    def get_import(self, database, token):
        if not opaque_id(token):
            raise MediaError('媒体导入 token 无效')
        row = database.execute('SELECT * FROM imports WHERE token=?', (token,)).fetchone()
        if row is None:
            raise MediaError('媒体导入暂存不存在', 404)
        return row

    def verify_record(self, record, video, mapping):
        if (not video.is_file() or video.is_symlink() or not mapping.is_file() or mapping.is_symlink()
                or not 0 < record['byteLength'] <= MAX_VIDEO_BYTES
                or not 0 < record['mapByteLength'] <= MAX_ENTRY_BYTES
                or video.stat().st_size != record['byteLength'] or mapping.stat().st_size != record['mapByteLength']):
            raise MediaError('预览媒体已丢失或已清理', 404)
        digest = hashlib.sha256()
        count = 0
        with video.open('rb') as source:
            while True:
                chunk = source.read(CHUNK_BYTES)
                if not chunk:
                    break
                count += len(chunk)
                if count > record['byteLength']:
                    raise MediaError('预览视频长度发生变化', 409)
                digest.update(chunk)
        if count != record['byteLength'] or digest.hexdigest() != record['sha256']:
            raise MediaError('预览视频校验失败', 409)
        with mapping.open('rb') as source:
            content = source.read(MAX_ENTRY_BYTES + 1)
        if len(content) != record['mapByteLength'] or hashlib.sha256(content).hexdigest() != record['mapSha256']:
            raise MediaError('预览映射校验失败', 409)
        return dict(record, videoPath=video, mapBytes=content, map=parse_json(content.decode('utf-8')))

    def cleanup_garbage(self, database, token=None):
        rows = database.execute('SELECT media_id, token FROM garbage' + (' WHERE token=?' if token else ''),
                                (token,) if token else ()).fetchall()
        pending = False
        for row in rows:
            if database.execute('SELECT 1 FROM current_parts WHERE media_id=?', (row['media_id'],)).fetchone():
                pending = True
                continue
            try:
                for suffix in ('.mp4', '.json'):
                    self.object_path(row['media_id'], suffix).unlink(missing_ok=True)
                sync_directory(self.objects)
            except (OSError, MediaError):
                pending = True
                continue
            database.execute('DELETE FROM garbage WHERE media_id=?', (row['media_id'],))
            database.commit()
        return pending

    def cleanup_stage(self, token):
        try:
            directory = self.stage_directory(token)
            if directory.exists():
                shutil.rmtree(directory)
                sync_directory(self.staging)
            return False
        except (OSError, MediaError):
            return True

    def commit(self, token, project_id):
        if not valid_text(project_id):
            raise MediaError('缺少项目标识')
        with self.locked() as database:
            row = self.get_import(database, token)
            if row['project_id'] != project_id:
                raise MediaError('媒体导入不属于该项目', 409)
            if row['state'] == 'discarded':
                raise MediaError('媒体导入已取消', 409)
            if row['state'] == 'staged':
                parts, records, baseline = json.loads(row['parts']), json.loads(row['records']), json.loads(row['baseline'])
                if self.revisions(database, project_id, parts) != baseline:
                    raise MediaError('同片段已由其他导入更新，请重新导入；旧媒体未修改', 409)
                for record in records.values():
                    paths = {}
                    for suffix in ('.mp4', '.json'):
                        staged = self.staged_path(token, record['mediaId'], suffix)
                        target = self.object_path(record['mediaId'], suffix)
                        paths[suffix] = staged if staged.exists() else target
                    self.verify_record(record, paths['.mp4'], paths['.json'])
                for record in records.values():
                    for suffix in ('.mp4', '.json'):
                        staged = self.staged_path(token, record['mediaId'], suffix)
                        if staged.exists():
                            staged.replace(self.object_path(record['mediaId'], suffix))
                sync_directory(self.objects)
                sync_directory(self.stage_directory(token))
                try:
                    database.execute('BEGIN IMMEDIATE')
                    for part_name in parts:
                        previous = database.execute('SELECT media_id FROM current_parts WHERE project_id=? AND part_name=?',
                                                    (project_id, part_name)).fetchone()
                        if previous and previous['media_id']:
                            database.execute('INSERT OR IGNORE INTO garbage VALUES (?, ?, ?, ?)',
                                             (previous['media_id'], token, project_id, part_name))
                        database.execute('UPDATE garbage SET token=? WHERE project_id=? AND part_name=?',
                                         (token, project_id, part_name))
                        record = records.get(part_name)
                        database.execute('INSERT OR REPLACE INTO current_parts VALUES (?, ?, ?, ?, ?, ?)',
                            (project_id, part_name, baseline[part_name] + 1, token,
                             record['mediaId'] if record else None, json.dumps(record) if record else None))
                    database.execute("UPDATE imports SET state='committed' WHERE token=?", (token,))
                    database.commit()
                except BaseException:
                    database.rollback()
                    raise
            try:
                pending = self.cleanup_garbage(database, token)
            except (OSError, sqlite3.Error):
                pending = True
            pending = self.cleanup_stage(token) or pending
            return {'ok': True, 'token': token, 'projectId': project_id, 'status': 'committed', 'cleanupPending': pending}

    def relocate(self, token, destination):
        if destination.root == self.root:
            return {'ok': True}
        with self.locked() as source_database, destination.locked() as target_database:
            row = self.get_import(source_database, token)
            if row['state'] != 'staged':
                raise MediaError('只能转移尚未提交的导入事务', 409)
            source = self.stage_directory(token)
            target = destination.stage_directory(token)
            if target.exists() or target_database.execute('SELECT token FROM imports WHERE token=?', (token,)).fetchone():
                raise MediaError('目标空间已有同名导入事务', 409)
            baseline = destination.revisions(target_database, row['project_id'], json.loads(row['parts']))
            source.rename(target)
            try:
                target_database.execute('INSERT INTO imports VALUES (?, ?, ?, ?, ?, ?, ?)',
                    (token, row['project_id'], 'staged', row['parts'], row['records'], json.dumps(baseline), row['created']))
                target_database.commit()
                source_database.execute("UPDATE imports SET state='discarded' WHERE token=?", (token,))
                source_database.commit()
            except BaseException:
                target_database.rollback()
                target_database.execute('DELETE FROM imports WHERE token=?', (token,))
                target_database.commit()
                target.rename(source)
                raise
            sync_directory(self.staging)
            sync_directory(destination.staging)
            return {'ok': True}

    def discard(self, token):
        with self.locked() as database:
            row = self.get_import(database, token)
            if row['state'] == 'committed':
                raise MediaError('已提交媒体不能取消', 409)
            for record in json.loads(row['records']).values():
                media_id = record['mediaId']
                if database.execute('SELECT 1 FROM current_parts WHERE media_id=?', (media_id,)).fetchone():
                    raise MediaError('媒体正在使用，不能删除', 409)
                for suffix in ('.mp4', '.json'):
                    self.object_path(media_id, suffix).unlink(missing_ok=True)
            directory = self.stage_directory(token)
            if directory.exists():
                shutil.rmtree(directory)
            database.execute("UPDATE imports SET state='discarded' WHERE token=?", (token,))
            database.commit()
            return {'ok': True, 'token': token, 'status': 'discarded'}

    def current_record(self, database, media_id, identity=None):
        if not opaque_id(media_id):
            raise MediaError('视频不存在或已清理', 404)
        row = database.execute('SELECT * FROM current_parts WHERE media_id=?', (media_id,)).fetchone()
        if row is None:
            raise MediaError('视频不存在或已清理', 404)
        record = json.loads(row['record'])
        if identity is not None and (row['project_id'] != identity.get('projectId')
                or row['part_name'] != identity.get('partName') or record['recordingId'] != identity.get('recordingId')):
            raise MediaError('视频不存在或已清理', 404)
        path = self.object_path(media_id)
        try:
            available = path.is_file() and path.stat().st_size == record['byteLength']
        except FileNotFoundError:
            available = False
        if not available:
            raise MediaError('视频不存在或已清理', 404)
        return record, path

    def info(self, identity):
        if not isinstance(identity, dict) or set(identity) != {'projectId', 'partName', 'recordingId', 'mediaId'}:
            raise MediaError('需要 projectId、partName、recordingId、mediaId')
        with self.locked() as database:
            self.current_record(database, identity['mediaId'], identity)
        return {'url': '/api/media/' + identity['mediaId']}

    @contextmanager
    def open_video(self, media_id):
        with self.locked() as database:
            record, path = self.current_record(database, media_id)
            try:
                source = path.open('rb')
            except FileNotFoundError:
                raise MediaError('视频不存在或已清理', 404) from None
            if os.fstat(source.fileno()).st_size != record['byteLength']:
                source.close()
                raise MediaError('视频不存在或已清理', 404)
        try:
            yield source, record
        finally:
            source.close()
