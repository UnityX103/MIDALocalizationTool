import argparse
import json
import os
import re
import sqlite3
import zipfile
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
from uuid import uuid4

from media_store import MediaError, MediaStore
from package_io import CHUNK_BYTES, MAX_EXPORT_BYTES, MAX_ZIP_BYTES, parse_json, write_package

ROOT = Path(__file__).resolve().parent
MAX_RANGE_BYTES = 8 * 1024 * 1024


def validate_delivery(document):
    if not isinstance(document, dict) or document.get('format') != 'mida-localization' or type(document.get('formatVersion')) is not int or document['formatVersion'] != 1:
        raise ValueError('无效的本地化交付格式')
    if document.get('demo') or document.get('deliveryState') != 'ready':
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
            if not isinstance(review, dict) or review.get('state') != 'confirmed' or not isinstance(translation, str) or not translation.strip():
                raise ValueError('仍有未完成或空译文')


class EditorServer(ThreadingHTTPServer):
    def __init__(self, port, output_directory):
        self.output_directory = output_directory.resolve()
        self.media_store = MediaStore(self.output_directory)
        super().__init__(('127.0.0.1', port), EditorHandler)
        self.origin = f'http://127.0.0.1:{self.server_port}'


class EditorHandler(BaseHTTPRequestHandler):
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
                    query = parse_qs(parsed.query, keep_blank_values=True, max_num_fields=4)
                    if any(len(values) != 1 for values in query.values()):
                        raise MediaError('媒体查询参数不能重复')
                    self.reply(200, self.server.media_store.info({key: values[0] for key, values in query.items()}))
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
        elif path in ('/workspace-store.js', '/preview-player.js'):
            try:
                self.reply(200, (ROOT / path[1:]).read_bytes(), 'text/javascript; charset=utf-8')
            except FileNotFoundError:
                self.reply(404, {'error': '脚本不存在'})
        elif path == '/api/info':
            self.reply(200, {'localExport': True, 'previewMedia': True, 'outputDirectory': str(self.server.output_directory)})
        else:
            self.reply(404, {'error': '页面不存在'})

    def send_video(self, media_id):
        with self.server.media_store.open_video(media_id) as (source, record):
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
        self.close_connection = True
        if not self.valid_host() or self.headers.get_all('Origin') != [self.server.origin]:
            self.reply(403, {'error': '不允许跨站操作'})
            return
        if self.path not in ('/api/export', '/api/import', '/api/media/commit', '/api/media/discard', '/api/media/info'):
            self.reply(404, {'error': '接口不存在'})
            return
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
                result = self.server.media_store.import_stream(self.rfile, length)
                self.reply(200, result)
                return
            payload = self.rfile.read(length)
            if len(payload) != length:
                raise ValueError('文件内容不完整')
            request = parse_json(payload.decode('utf-8'))
            if self.path == '/api/media/info':
                self.reply(200, self.server.media_store.info(request))
                return
            if self.path in ('/api/media/commit', '/api/media/discard'):
                fields = {'token', 'projectId'} if self.path == '/api/media/commit' else {'token'}
                if not isinstance(request, dict) or set(request) != fields:
                    raise ValueError('媒体确认或取消参数无效')
                result = (self.server.media_store.commit(request['token'], request['projectId'])
                          if self.path == '/api/media/commit' else self.server.media_store.discard(request['token']))
                self.reply(200, result)
                return
            if isinstance(request, dict) and 'document' in request:
                if set(request) - {'document'}:
                    raise ValueError('编辑器仅支持导出数据，不接受预览视频或其他导出选项')
                document = request['document']
            else:
                document = request
            validate_delivery(document)
            directory = self.server.output_directory
            directory.mkdir(parents=True, exist_ok=True)
            destination = directory / f'localization.ready.{uuid4().hex}.zip'
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
