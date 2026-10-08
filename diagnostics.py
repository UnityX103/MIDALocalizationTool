"""Bounded, private run logs. Never serialize operation arguments or workspace data."""
import contextvars
import functools
import json
import os
import re
import sys
import threading
import time
import traceback
from pathlib import Path
from uuid import uuid4

TRACE = contextvars.ContextVar('mida_diagnostic_operation', default='')
FILE_LIMIT = 5 * 1024 * 1024
TOTAL_LIMIT = 50 * 1024 * 1024
FIELDS = {'source', 'level', 'event', 'operationId', 'occurredAt', 'durationMs', 'status', 'errorType', 'stack'}


def redact(value):
    text = str(value).replace(str(Path.home()), '<home>')
    text = re.sub(r'(?i)(Bearer\s+)[^\s,;"\']+', r'\1<redacted>', text)
    text = re.sub(r'(?i)(token|password|secret|authorization|accessKey)(\s*[=:]\s*)[^\s,;]+', r'\1\2<redacted>', text)
    text = re.sub(r'(?:/Users/|/home/|[A-Za-z]:\\Users\\)[^\s)]+', '<path>', text)
    return text[:8000]


def log_directory():
    if sys.platform == 'darwin':
        parent = Path.home() / 'Library/Application Support'
    elif os.name == 'nt':
        parent = Path(os.environ.get('LOCALAPPDATA', Path.home() / 'AppData/Local'))
    else:
        parent = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state'))
    return parent / 'com.mida.localization.editor.preview' / 'logs'


class RunLog:
    def __init__(self, directory=None):
        self.directory = Path(directory) if directory else log_directory()
        if any(p.is_symlink() for p in (self.directory, *self.directory.parents)):
            raise OSError('日志目录不能是符号链接')
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.run = str(uuid4())
        self.part = 0
        self.lock = threading.RLock()
        self.failure = ''
        self.write({'source': 'python', 'level': 'INFO', 'event': 'runtime.start'})

    def files(self):
        return sorted((p for p in self.directory.glob('run-*.jsonl') if p.is_file() and not p.is_symlink()), key=lambda p: p.stat().st_mtime_ns)

    def write(self, event):
        if not isinstance(event, dict):
            return
        row = {k: redact(v) if isinstance(v, str) else v for k, v in event.items()
               if k in FIELDS and isinstance(v, (str, int, float)) and not isinstance(v, bool)}
        row.update(time=round(time.time() * 1000), runId=self.run)
        if not row.get('operationId') and TRACE.get():
            row['operationId'] = TRACE.get()
        payload = (json.dumps(row, ensure_ascii=False) + '\n').encode()
        try:
            with self.lock:
                path = self.directory / f'run-{self.run}-{self.part:03}.jsonl'
                if path.exists() and path.stat().st_size + len(payload) > FILE_LIMIT:
                    self.part += 1
                    path = self.directory / f'run-{self.run}-{self.part:03}.jsonl'
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | getattr(os, 'O_NOFOLLOW', 0), 0o600)
                with os.fdopen(fd, 'ab') as stream:
                    stream.write(payload)
                    stream.flush()
                files = self.files()
                size = sum(p.stat().st_size for p in files)
                while len(files) > 20 or size > TOTAL_LIMIT:
                    old = files.pop(0)
                    size -= old.stat().st_size
                    old.unlink()
                self.failure = ''
        except OSError:
            self.failure = '日志保存失败，请检查磁盘空间及本机目录权限'

    def snapshot(self):
        with self.lock:
            chunks = []
            remaining = FILE_LIMIT
            for path in reversed(self.files()):
                if self.run not in path.name or remaining <= 0:
                    continue
                raw = path.read_bytes()
                if len(raw) > remaining:
                    raw = raw[-remaining:]
                    raw = raw.partition(b'\n')[2]
                chunks.insert(0, raw)
                remaining -= len(raw)
            return b''.join(chunks).decode('utf-8', errors='replace')

    def info(self):
        return {'runId': self.run, 'directory': str(self.directory), 'error': self.failure,
                'version': json.loads((Path(__file__).parent / 'package.json').read_text())['version'],
                'platform': sys.platform, 'fileLimit': FILE_LIMIT}


LOGGER = None


def record(event, level='INFO', **fields):
    if LOGGER:
        LOGGER.write({'source': 'python', 'level': level, 'event': event, **fields})


def instrument(owner, names):
    for name in names:
        fn = getattr(owner, name, None)
        if not callable(fn) or getattr(fn, '_mida_logged', False):
            continue
        def wrap(fn, name):
            @functools.wraps(fn)
            def call(*args, **kwargs):
                identity = TRACE.get() or str(uuid4())
                token = TRACE.set(identity)
                start = time.monotonic()
                record(name + '.start')
                try:
                    value = fn(*args, **kwargs)
                    record(name + '.success', durationMs=round((time.monotonic() - start) * 1000))
                    return value
                except Exception as error:
                    record(name + '.failed', 'ERROR', errorType=type(error).__name__, stack=''.join(traceback.format_tb(error.__traceback__)), durationMs=round((time.monotonic() - start) * 1000))
                    raise
                finally:
                    TRACE.reset(token)
            call._mida_logged = True
            return call
        setattr(owner, name, wrap(fn, getattr(owner, '__name__', 'backend') + '.' + name))
