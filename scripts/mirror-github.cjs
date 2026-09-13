const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const upstream = 'https://github.com/UnityX103/MIDALocalizationTool.git';
const githubApi = 'https://api.github.com/repos/UnityX103/MIDALocalizationTool';
const cnbApi = 'https://api.cnb.cool/nanzhaigame-xpy/MIDALocalizationTool/-/releases';
const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');

async function json(url, method = 'GET', body) {
  const headers = { Accept: 'application/json' };
  if (url.startsWith(cnbApi)) { headers.Authorization = `Bearer ${process.env.CNB_TOKEN}`; headers['Content-Type'] = 'application/json'; }
  const response = await fetch(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), redirect: 'error', signal: AbortSignal.timeout(60000) });
  if (response.status === 404 && method === 'GET') return null;
  if (!response.ok) throw new Error(`镜像 API 请求失败：${new URL(url).hostname} HTTP ${response.status}`);
  return response.json();
}

async function main() {
  if (process.env.CNB_REPO_SLUG !== 'nanzhaigame-xpy/MIDALocalizationTool' || !process.env.CNB_TOKEN) throw new Error('仅允许在指定 CNB 仓库内同步');
  const args = ['fetch', '--no-tags'];
  if (git(['rev-parse', '--is-shallow-repository']) === 'true') args.push('--unshallow');
  git([...args, upstream, 'main:refs/remotes/github/main']);
  git(['push', 'origin', 'refs/remotes/github/main:refs/heads/main']);
  console.log('GitHub main 已快进同步；未强制覆盖 CNB 历史。');
  const release = await json(`${githubApi}/releases/latest`);
  if (!release) { console.log('GitHub 暂无正式版本，保留现有 CNB 更新包。'); return; }
  if (release.draft || release.prerelease || !/^v\d+\.\d+\.\d+$/.test(release.tag_name)) throw new Error('不支持的上游版本');
  const version = release.tag_name.slice(1);
  const existing = await json(`${cnbApi}/tags/${release.tag_name}`);
  if (existing && !existing.draft) { console.log(`${release.tag_name} 已镜像，跳过重复发布。`); return; }
  const latest = await json(`${cnbApi}/latest`);
  const compare = (left, right) => { const rightParts = right.split('.').map(Number); for (const [index, value] of left.split('.').map(Number).entries()) { if (value !== rightParts[index]) return value - rightParts[index]; } return 0; };
  if (latest && /^v\d+\.\d+\.\d+$/.test(latest.tag_name) && compare(version, latest.tag_name.slice(1)) < 0) throw new Error('上游版本低于 CNB 最新版，拒绝降级');
  const parent = path.join(root, 'src-tauri/target');
  fs.mkdirSync(parent, { recursive: true });
  const directory = fs.mkdtempSync(path.join(parent, 'github-mirror-'));
  async function download(name) {
    const asset = release.assets.find(asset => asset.name === name);
    if (!asset || asset.size <= 0 || asset.size > 512 * 1024 * 1024) throw new Error(`缺少或过大的上游附件：${name}`);
    const url = new URL(asset.browser_download_url);
    if (url.origin !== 'https://github.com' || !url.pathname.startsWith(`/UnityX103/MIDALocalizationTool/releases/download/${release.tag_name}/`)) throw new Error('上游下载地址不属于指定版本');
    const response = await fetch(url, { signal: AbortSignal.timeout(600000) });
    if (!response.ok) throw new Error(`下载失败：${name} HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length !== asset.size) throw new Error(`附件大小不符：${name}`);
    fs.writeFileSync(path.join(directory, name), bytes);
    return bytes;
  }
  const sums = (await download('SHA256SUMS')).toString('utf8');
  const names = [`MIDA-Localization-${version}-macOS-universal.dmg`, `MIDA-Localization-${version}-macOS-universal.app.tar.gz`, `MIDA-Localization-${version}-macOS-universal.app.tar.gz.sig`, `MIDA-Localization-${version}-Windows-x64-setup.exe`, `MIDA-Localization-${version}-Windows-x64-setup.exe.sig`, `MIDA-Localization-${version}-source.tar.gz`, 'build-info.json', 'latest.json'];
  for (const name of names) {
    const checksum = sums.split('\n').find(line => line.slice(66) === name)?.slice(0, 64);
    if (!/^[a-f0-9]{64}$/.test(checksum || '') || hash(await download(name)) !== checksum) throw new Error(`SHA256 校验失败：${name}`);
  }
  const info = JSON.parse(fs.readFileSync(path.join(directory, 'build-info.json'), 'utf8'));
  if (info.version !== version || !/^[a-f0-9]{40}$/.test(info.gitHead)) throw new Error('上游构建身份无效');
  git(['fetch', '--no-tags', upstream, `refs/tags/${release.tag_name}`]);
  if (git(['rev-parse', 'FETCH_HEAD^{commit}']) !== info.gitHead) throw new Error('上游标签与构建提交不一致');
  git(['merge-base', '--is-ancestor', info.gitHead, 'refs/remotes/github/main']);
  const source = path.join(directory, 'source-metadata');
  fs.mkdirSync(path.join(source, 'docs'), { recursive: true });
  for (const name of ['package.json', 'release-policy.json', `docs/release-${version}.md`]) {
    const bytes = execFileSync('git', ['show', `${info.gitHead}:${name}`], { cwd: root });
    if (hash(bytes) !== info.sourceHashes[name]) throw new Error(`源码快照不符：${name}`);
    fs.writeFileSync(path.join(source, name), bytes);
  }
  if (!existing) await json(cnbApi, 'POST', { tag_name: release.tag_name, target_commitish: info.gitHead, name: release.name, body: release.body, draft: true, prerelease: false, make_latest: 'false' });
  execFileSync(process.execPath, [path.join(root, 'scripts/publish-cnb.cjs'), directory, '--publish'], { cwd: root, stdio: 'inherit', env: { ...process.env, RELEASE_SOURCE_DIR: source } });
  console.log(`GitHub ${release.tag_name} 已完整同步为 CNB 默认更新源。`);
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
