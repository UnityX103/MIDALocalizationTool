import argparse
import json
import os
import re
import sqlite3
import shutil
import threading
import zipfile
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
from urllib.request import Request, urlopen
from uuid import uuid4

from media_store import MediaError, MediaStore
from import_jobs import ImportJobs
from package_io import CHUNK_BYTES, MAX_EXPORT_BYTES, MAX_ZIP_BYTES, parse_json, write_package

ROOT = Path(__file__).resolve().parent
MAX_RANGE_BYTES = 8 * 1024 * 1024


def validate_delivery(document, allow_unresolved=False):
    if not isinstance(document, dict) or document.get('format') != 'mida-localization' or type(document.get('formatVersion')) is not int or document['formatVersion'] != 1:
        raise ValueError('无效的本地化交付格式')
    if document.get('demo') or document.get('deliveryState') not in (('ready', 'draft') if allow_unresolved else ('ready',)):
        raise ValueError('只接受真实任务的已完成交付文件')
    if not isinstance(document.get('projectId'), str) or not document['projectId']:
        raise ValueError('缺少项目标识')
    version = document.get('fileVersion')
    if (not isinstance(document.get('packageId'), str) or not document['packageId']
            or not isinstance(document.get('exportedAt'), str)
            or not isinstance(version, dict) or not isinstance(version.get('lineageId'), str)
            or not version['lineageId'] or type(version.get('revision')) is not int
            or not 0 < version['revision'] <= 9007199254740991):
        raise ValueError('缺少有效的包版本标识')
    tasks = document.get('tasks')
    if not isinstance(tasks, list) or not tasks or len(tasks) > 1000:
        raise ValueError('任务列表为空或超过上限')
    identities = set()
    count = 0
    for task in tasks:
        if not isinstance(task, dict):
            raise ValueError('任务结构无效')
        identity = (task.get('partName'), task.get('language'))
        if not all(isinstance(value, str) and value for value in identity) or identity in identities:
            raise ValueError('片段语言标识无效或重复')
        identities.add(identity)
        entries = task.get('entries')
        if not isinstance(entries, list) or not entries:
            raise ValueError('任务词条为空')
        count += len(entries)
        if count > 100000:
            raise ValueError('词条超过上限')
        keys = set()
        for entry in entries:
            if not isinstance(entry, dict):
                raise ValueError('词条结构无效')
            key = entry.get('key')
            if not isinstance(key, str) or not key or key in keys:
                raise ValueError('词条标识无效或重复')
            keys.add(key)
            review = entry.get('review')
            translation = entry.get('translation')
            if not isinstance(review, dict) or review.get('state') not in ('pending', 'confirmed') or not isinstance(translation, str):
                raise ValueError('词条复核状态或译文格式无效')
            if document['deliveryState'] == 'ready' and (review['state'] != 'confirmed' or not translation.strip()):
                raise ValueError('仍有未完成或空译文')


class EditorServer(ThreadingHTTPServer):
    def __init__(self, port, output_directory):
        self.output_directory = output_directory.resolve()
        self.media_store = MediaStore(self.output_directory)
        self.space_stores = {}
        self.space_lock = threading.Lock()
        self.operation_lock = threading.RLock()
        self.import_jobs = ImportJobs()
        super().__init__(('127.0.0.1', port), EditorHandler)
        self.origin = f'http://127.0.0.1:{self.server_port}'


class EditorHandler(BaseHTTPRequestHandler):
    @property
    def media_store(self):
        query = parse_qs(urlsplit(self.path).query, keep_blank_values=True, max_num_fields=5)
        space = self.headers.get('X-Localization-Space', '') or query.get('space', [''])[0]
        return self.store_for_space(space)

    def store_for_space(self, space):
        if not space:
            return self.server.media_store
        if not re.fullmatch(r'[a-z][a-z0-9_-]{0,31}', space):
            raise MediaError('语言空间标识无效')
        with self.server.space_lock:
            if space not in self.server.space_stores:
                parent = self.server.output_directory / '.spaces'
                directory = parent / space
                if parent.is_symlink() or directory.is_symlink():
                    raise MediaError('语言空间不能是符号链接')
                self.server.space_stores[space] = MediaStore(directory)
            return self.server.space_stores[space]

    def reply(self, status, data, content_type='application/json; charset=utf-8', headers=None):
        payload = data if isinstance(data, bytes) else json.dumps(data, ensure_ascii=False).encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(payload)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Cross-Origin-Resource-Policy', 'same-origin')
        self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'")
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        if self.command != 'HEAD':
            self.wfile.write(payload)

    def valid_host(self):
        return self.headers.get_all('Host') == [urlsplit(self.server.origin).netloc]

    def valid_media_origin(self):
        origins = self.headers.get_all('Origin')
        if origins is not None and origins != [self.server.origin]:
            return False
        site = self.headers.get('Sec-Fetch-Site')
        if site is not None and site != 'same-origin':
            return False
        referer = self.headers.get('Referer')
        if referer:
            reference = urlsplit(referer)
            if reference.scheme + '://' + reference.netloc != self.server.origin:
                return False
        return origins == [self.server.origin] or site == 'same-origin' or bool(referer)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        if urlsplit(self.path).path == '/api/repository-history':
            if not self.valid_host() or not self.valid_media_origin():
                self.reply(403, {'error': '仅允许本机编辑器读取版本历史'})
                return
            try:
                repository = 'https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool'
                request = Request(repository + '/-/releases?page=1&page_size=100', headers={
                    'Accept': 'application/vnd.cnb.api+json', 'User-Agent': 'MIDA-Localization-About'})
                with urlopen(request, timeout=20) as response:
                    content = response.read(4 * 1024 * 1024 + 1)
                if len(content) > 4 * 1024 * 1024:
                    raise ValueError('版本历史响应超过大小上限')
                releases = json.loads(content)
                if not isinstance(releases, list):
                    raise ValueError('版本历史格式无效')
                releases = [release for release in releases if isinstance(release, dict)
                    and release.get('draft') is not True and release.get('prerelease') is not True
                    and isinstance(release.get('tag_name'), str)]
                releases.sort(key=lambda release: str(release.get('published_at') or release.get('created_at') or ''), reverse=True)
                self.reply(200, {'repository': repository, 'releases': [
                    {'tag': release['tag_name'], 'name': release.get('name'),
                     'publishedAt': release.get('published_at'), 'notes': release.get('body')}
                    for release in releases[:5]]})
            except (OSError, ValueError) as error:
                self.reply(502, {'error': '读取仓库版本历史失败：' + str(error)})
            return
        with self.server.operation_lock:
            self.handle_get()

    def handle_get(self):
        if not self.valid_host():
            self.reply(403, {'error': '仅允许本机编辑器访问'})
            return
        parsed = urlsplit(self.path)
        path = parsed.path
        if path.startswith('/api/media/'):
            if not self.valid_media_origin():
                self.reply(403, {'error': '不允许跨站读取媒体'})
                return
            try:
                if path == '/api/media/info':
                    query = parse_qs(parsed.query, keep_blank_values=True, max_num_fields=5)
                    if any(len(values) != 1 for values in query.values()):
                        raise MediaError('媒体查询参数不能重复')
                    self.reply(200, self.media_store.info({key: values[0] for key, values in query.items() if key != 'space'}))
                else:
                    self.send_video(path.removeprefix('/api/media/'))
            except MediaError as error:
                self.reply(error.status, {'error': str(error)})
            except ValueError as error:
                self.reply(400, {'error': str(error)})
            except (OSError, sqlite3.Error) as error:
                self.reply(500, {'error': '本地媒体读取失败：' + str(error)})
            return
        if path in ('/', '/prototype.html'):
            self.reply(200, (ROOT / 'prototype.html').read_bytes(), 'text/html; charset=utf-8')
        elif path == '/app-icon.svg':
            self.reply(200, (ROOT / 'app-icon.svg').read_bytes(), 'image/svg+xml')
        elif path in ('/workspace-store.js', '/preview-player.js', '/import-worker.js'):
            try:
                self.reply(200, (ROOT / path[1:]).read_bytes(), 'text/javascript; charset=utf-8')
            except FileNotFoundError:
                self.reply(404, {'error': '脚本不存在'})
        elif path == '/api/info':
            self.reply(200, {'localExport': True, 'previewMedia': True, 'outputDirectory': str(self.server.output_directory)})
        else:
            self.reply(404, {'error': '页面不存在'})

    def send_video(self, media_id):
        with self.media_store.open_video(media_id) as (source, record):
            size = record['byteLength']
            start, end, status = 0, size - 1, 200
            etag = '"' + record['sha256'] + '"'
            ranges = self.headers.get_all('Range')
            if ranges and self.headers.get('If-Range', etag) == etag:
                match = re.fullmatch(r'bytes=([0-9]*)-([0-9]*)', ranges[0]) if len(ranges) == 1 and len(ranges[0]) < 100 else None
                if not match or not any(match.groups()):
                    self.reply(416, {'error': '只支持单个字节范围'}, headers={'Content-Range': f'bytes */{size}', 'Accept-Ranges': 'bytes'})
                    return
                first, last = match.groups()
                if first:
                    start = int(first)
                    end = min(int(last), size - 1) if last else size - 1
                else:
                    suffix = int(last)
                    start = max(0, size - suffix) if suffix else size
                if start >= size or end < start:
                    self.reply(416, {'error': '字节范围不可满足'}, headers={'Content-Range': f'bytes */{size}', 'Accept-Ranges': 'bytes'})
                    return
                end = min(end, start + MAX_RANGE_BYTES - 1)
                status = 206
            self.send_response(status)
            self.send_header('Content-Type', 'video/mp4')
            self.send_header('Content-Length', str(end - start + 1))
            self.send_header('Accept-Ranges', 'bytes')
            self.send_header('Cache-Control', 'no-store')
            self.send_header('ETag', etag)
            self.send_header('Cross-Origin-Resource-Policy', 'same-origin')
            self.send_header('X-Content-Type-Options', 'nosniff')
            if status == 206:
                self.send_header('Content-Range', f'bytes {start}-{end}/{size}')
            self.end_headers()
            if self.command == 'HEAD':
                return
            try:
                self.connection.settimeout(30)
                source.seek(start)
                remaining = end - start + 1
                while remaining:
                    chunk = source.read(min(CHUNK_BYTES, remaining))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    remaining -= len(chunk)
            except (OSError, TimeoutError):
                self.close_connection = True

    def do_POST(self):
        with self.server.operation_lock:
            self.handle_post()

    def handle_post(self):
        self.close_connection = True
        if not self.valid_host() or self.headers.get_all('Origin') != [self.server.origin]:
            self.reply(403, {'error': '不允许跨站操作'})
            return
        # Progress/cancel must remain reachable while upload holds the operation lock.
        if self.path in ('/api/import/start', '/api/import/status', '/api/import/cancel'):
            self.handle_import_control()
            return
        if self.path not in ('/api/export', '/api/import', '/api/media/commit', '/api/media/discard', '/api/media/info', '/api/media/relocate', '/api/cache/clear'):
            self.reply(404, {'error': '接口不存在'})
            return
    def handle_import_control(self):
        self.close_connection = True
        if not self.valid_host() or self.headers.get_all('Origin') != [self.server.origin]:
            self.reply(403, {'error': '不允许跨站操作'})
            return
        try:
            if self.headers.get_content_type() != 'application/json' or self.headers.get('Transfer-Encoding'):
                raise ValueError('请求文件类型不正确')
            lengths = self.headers.get_all('Content-Length', [])
            if len(lengths) != 1 or not re.fullmatch(r'[0-9]{1,4}', lengths[0]) or not 0 < int(lengths[0]) <= 1024:
                raise ValueError('请求长度无效')
            self.connection.settimeout(10)
            request = parse_json(self.rfile.read(int(lengths[0])).decode('utf-8'))
            space = self.headers.get('X-Localization-Space', '')
            if space and not re.fullmatch(r'[a-z][a-z0-9_-]{0,31}', space):
                raise ValueError('语言空间标识无效')
            if self.path == '/api/import/start':
                if request != {}:
                    raise ValueError('导入参数无效')
                result = {'requestId': self.server.import_jobs.start(space)}
            else:
                if not isinstance(request, dict) or set(request) != {'requestId'} or not isinstance(request['requestId'], str):
                    raise ValueError('导入任务标识无效')
                action = 'cancel' if self.path.endswith('/cancel') else 'status'
                result = self.server.import_jobs.access(request['requestId'], space, action)
                if action == 'cancel' and result.get('token'):
                    with self.server.operation_lock:
                        self.store_for_space(space).discard(result.pop('token'))
            self.reply(200, result)
        except (ValueError, OSError) as error:
            self.reply(400, {'error': str(error)})

        expected_type = 'application/zip' if self.path == '/api/import' else 'application/json'
        if self.headers.get_content_type() != expected_type or self.headers.get('Transfer-Encoding'):
            self.reply(415, {'error': '请求文件类型不正确'})
            return
        try:
            lengths = self.headers.get_all('Content-Length', [])
            if len(lengths) != 1 or not re.fullmatch(r'[0-9]{1,12}', lengths[0]):
                raise ValueError('Content-Length 无效或重复')
            length = int(lengths[0])
            maximum = MAX_ZIP_BYTES if self.path == '/api/import' else (65536 if self.path.startswith('/api/media/') else MAX_EXPORT_BYTES)
            if not 0 < length <= maximum:
                self.reply(413, {'error': '导出内容为空或超过大小上限'})
                return
            self.connection.settimeout(30)
            if self.path == '/api/import':
                identifier = self.headers.get('X-Import-Request', '')
                space = self.headers.get('X-Localization-Space', '')
                jobs = self.server.import_jobs
                jobs.access(identifier, space, 'run')
                result = None
                try:
                    progress = lambda fraction, phase: jobs.report(identifier, space, fraction, phase)
                    result = self.media_store.import_stream(self.rfile, length, progress)
                    progress(1, '校验完成')
                    self.reply(200, result)
                except (ValueError, OSError):
                    if result:
                        self.media_store.discard(result['mediaImportToken'])
                    raise
                finally:
                    token = result['mediaImportToken'] if result else None
                    if jobs.finish(identifier, token) and token:
                        self.media_store.discard(token)
                return
            payload = self.rfile.read(length)
            if len(payload) != length:
                raise ValueError('文件内容不完整')
            request = parse_json(payload.decode('utf-8'))
            if self.path == '/api/cache/clear':
                if not isinstance(request, dict) or set(request) != {'confirmed'} or request['confirmed'] is not True:
                    raise ValueError('清空操作尚未确认')
                directories = [self.server.output_directory / '.media-cache', self.server.output_directory / '.spaces']
                if any(directory.is_symlink() for directory in directories):
                    raise ValueError('缓存目录不能是符号链接')
                with self.server.space_lock:
                    for directory in directories:
                        if directory.exists():
                            shutil.rmtree(directory)
                    self.server.space_stores.clear()
                    self.server.media_store = MediaStore(self.server.output_directory)
                self.reply(200, {'ok': True})
                return
            if self.path == '/api/media/relocate':
                if not isinstance(request, dict) or set(request) != {'token', 'spaceId'} or not isinstance(request['spaceId'], str):
                    raise ValueError('导入转移参数无效')
                self.reply(200, self.media_store.relocate(request['token'], self.store_for_space(request['spaceId'])))
                return
            if self.path == '/api/media/info':
                self.reply(200, self.media_store.info(request))
                return
            if self.path in ('/api/media/commit', '/api/media/discard'):
                fields = {'token', 'projectId'} if self.path == '/api/media/commit' else {'token'}
                if not isinstance(request, dict) or set(request) != fields:
                    raise ValueError('媒体确认或取消参数无效')
                result = (self.media_store.commit(request['token'], request['projectId'])
                          if self.path == '/api/media/commit' else self.media_store.discard(request['token']))
                self.reply(200, result)
                return
            allow_unresolved = False
            if isinstance(request, dict) and 'document' in request:
                if set(request) - {'document', 'allowUnresolved'}:
                    raise ValueError('编辑器仅支持导出数据，不接受预览视频或其他导出选项')
                if 'allowUnresolved' in request and type(request['allowUnresolved']) is not bool:
                    raise ValueError('忽略待处理问题的确认参数无效')
                allow_unresolved = request.get('allowUnresolved', False)
                document = request['document']
            else:
                document = request
            validate_delivery(document, allow_unresolved)
            directory = self.server.output_directory
            directory.mkdir(parents=True, exist_ok=True)
            destination = directory / f"localization.{document['deliveryState']}.{uuid4().hex}.zip"
            temporary = destination.with_suffix('.tmp')
            try:
                with temporary.open('xb') as output:
                    write_package(output, document)
                    output.flush()
                    os.fsync(output.fileno())
                temporary.replace(destination)
            finally:
                temporary.unlink(missing_ok=True)
            self.reply(200, {'path': str(destination)})
        except MediaError as error:
            self.reply(error.status, {'error': str(error)})
        except (ValueError, UnicodeError, RecursionError, zipfile.BadZipFile, zlib.error, EOFError, NotImplementedError, RuntimeError) as error:
            self.reply(400, {'error': str(error)})
        except (OSError, TimeoutError, sqlite3.Error) as error:
            self.reply(500, {'error': '本地文件保存失败：' + str(error)})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='本地化翻译编辑器（仅监听本机）')
    parser.add_argument('--port', type=int, default=8034)
    parser.add_argument('--output', type=Path, default=ROOT / 'LocalOutput')
    arguments = parser.parse_args()
    with EditorServer(arguments.port, arguments.output) as server:
        print(f'本地化翻译编辑器：{server.origin}/', flush=True)
        print(f'ZIP 导出目录：{server.output_directory}', flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
