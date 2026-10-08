"""Fixed-target proxy. Per-request receipt keys never grant CNB permissions."""
import base64
import json
import re
from urllib.error import HTTPError
from urllib.request import Request, build_opener, HTTPRedirectHandler

BASE = 'https://server.nanzhaigame.cn:8020/cnb-feedback/v1/public-feedback/mida-localization/requests'
UUID = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}')


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def dispatch(value):
    if not isinstance(value, dict) or set(value) - {'action', 'id', 'key', 'data'}:
        raise ValueError('反馈参数无效')
    action, identity, key, data = (value.get(k) for k in ('action', 'id', 'key', 'data'))
    if not isinstance(identity, str) or not UUID.fullmatch(identity) or not isinstance(key, str) or not re.fullmatch(r'[0-9a-f]{64}', key):
        raise ValueError('反馈请求身份无效')
    body = None
    headers = {'X-Feedback-Key': key, 'Accept': 'application/json'}
    path, method = '/' + identity, 'GET'
    if action == 'create':
        if not isinstance(data, dict) or data.get('id') != identity:
            raise ValueError('反馈创建身份不一致')
        body = json.dumps(data, ensure_ascii=False).encode()
        if len(body) > 65536:
            raise ValueError('反馈文字过长')
        path, method = '', 'POST'
        headers['Content-Type'] = 'application/json'
    elif action == 'upload':
        if not isinstance(data, dict) or not isinstance(data.get('attachmentId'), str) or not re.fullmatch(r'[0-9a-f-]{36}', data['attachmentId']) or not isinstance(data.get('bytes'), str) or len(data['bytes']) > 14 * 1024 * 1024:
            raise ValueError('反馈附件参数无效')
        body = base64.b64decode(data['bytes'], validate=True)
        if not 0 < len(body) <= 10 * 1024 * 1024:
            raise ValueError('附件大小无效')
        path += '/attachments/' + data['attachmentId']
        method = 'PUT'
        headers['Content-Type'] = 'application/octet-stream'
    elif action in ('commit', 'abort'):
        path += '/' + action
        method = 'POST'
        body = b'{}'
        headers['Content-Type'] = 'application/json'
    elif action != 'get':
        raise ValueError('不支持的反馈操作')
    request = Request(BASE + path, data=body, method=method, headers=headers)
    try:
        response = build_opener(NoRedirect()).open(request, timeout=40)
    except HTTPError as error:
        response = error
    with response:
        raw = response.read(512 * 1024 + 1)
        if len(raw) > 512 * 1024:
            raise ValueError('反馈响应过大，须核对原请求')
        result = json.loads(raw)
        if not isinstance(result, dict):
            raise ValueError('反馈结果未确认，须核对原请求')
        return {'status': response.code, 'result': result}
