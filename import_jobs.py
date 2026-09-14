"""Thread-safe progress and cooperative cancellation for staged ZIP imports."""
import threading
import time
from uuid import uuid4


class ImportJobs:
    def __init__(self):
        self.lock = threading.Lock()
        self.jobs = {}

    def start(self, space):
        with self.lock:
            now = time.monotonic()
            self.jobs = {key: job for key, job in self.jobs.items()
                         if now - job['updated'] < 3600 or job['running']}
            if any(job['space'] == space and not job['done'] for job in self.jobs.values()):
                raise ValueError('另一个导入正在进行，请先取消或等待完成')
            if len(self.jobs) >= 64:
                finished = next((key for key, job in self.jobs.items() if job['done']), None)
                if finished is None:
                    raise ValueError('导入任务过多，请稍后重试')
                del self.jobs[finished]
            identifier = uuid4().hex
            self.jobs[identifier] = dict(space=space, fraction=0, phase='等待上传',
                                        cancelled=False, done=False, running=False, token=None, updated=now)
            return identifier

    def access(self, identifier, space, action='status'):
        with self.lock:
            job = self.jobs.get(identifier)
            if not job or job['space'] != space:
                raise ValueError('导入任务不存在或已过期')
            if action == 'cancel':
                job['cancelled'] = True
                if not job['running']:
                    job['done'] = True
            elif action == 'run':
                if job['running'] or job['done'] or job['cancelled']:
                    raise ValueError('导入任务已结束或正在执行')
                job['running'] = True
            job['updated'] = time.monotonic()
            result = {key: job[key] for key in ('fraction', 'phase', 'cancelled', 'done')}
            if action == 'cancel' and job['done']:
                result['token'] = job['token']
            return result

    def report(self, identifier, space, fraction, phase):
        with self.lock:
            job = self.jobs.get(identifier)
            if not job or job['space'] != space or job['cancelled']:
                raise ValueError('导入已取消')
            job.update(fraction=fraction, phase=phase, updated=time.monotonic())

    def finish(self, identifier, token=None):
        with self.lock:
            job = self.jobs.get(identifier)
            if job is not None:
                job.update(done=True, running=False, token=token, updated=time.monotonic())
                return job['cancelled']
            return False
