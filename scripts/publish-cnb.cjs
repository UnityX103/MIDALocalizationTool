const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const version = require('../package.json').version;
const updatePolicy = require('../release-policy.json')[version];
if (!['mandatory', 'optional'].includes(updatePolicy)) throw new Error('请在 release-policy.json 为当前版本明确设置 mandatory 或 optional');
const repository = 'nanzhaigame-xpy/MIDALocalizationTool';
const api = `https://api.cnb.cool/${repository}/-/releases`;
const directory = path.resolve(process.argv[2] || path.join(root, 'releases', version));
const publish = process.argv.includes('--publish');
const checksum = bytes => crypto.createHash('sha256').update(bytes).digest('hex');

function credential() {
  if (process.env.CNB_TOKEN) return process.env.CNB_TOKEN;
  const result = execFileSync('git', ['credential', 'fill'], {
    cwd: root,
    input: `protocol=https\nhost=cnb.cool\npath=${repository}\n\n`,
    encoding: 'utf8',
    env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    stdio: ['pipe', 'pipe', 'ignore'],
  });
  const password = result.split('\n').find(line => line.startsWith('password='));
  if (!password) throw new Error('需要有 repo-release:rw 权限的 CNB 凭据');
  return password.slice('password='.length);
}

async function main() {
  const token = credential();
  async function request(url, method = 'GET', body) {
    const parsed = new URL(url);
    if (parsed.protocol !== 'https:' || !['api.cnb.cool', 'cnb.cool'].includes(parsed.hostname)) throw new Error('拒绝向未知主机发送 CNB 凭据');
    const response = await fetch(url, {
      method,
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.cnb.api+json', 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: 'error',
      signal: AbortSignal.timeout(60000),
    });
    if (!response.ok) throw new Error(`CNB ${method} 失败：HTTP ${response.status}`);
    const text = await response.text();
    return text ? JSON.parse(text) : null;
  }
  const macUpdate = `MIDA-Localization-${version}-macOS-universal.app.tar.gz`;
  const windowsUpdate = `MIDA-Localization-${version}-Windows-x64-setup.exe`;
  const required = [macUpdate, `${macUpdate}.sig`, windowsUpdate, `${windowsUpdate}.sig`, `MIDA-Localization-${version}-macOS-universal.dmg`, `MIDA-Localization-${version}-source.tar.gz`, 'build-info.json'];
  for (const name of required) if (!fs.statSync(path.join(directory, name)).size) throw new Error(`缺少完整产物：${name}`);
  const buildInfo = JSON.parse(fs.readFileSync(path.join(directory, 'build-info.json'), 'utf8'));
  if (buildInfo.version !== version) throw new Error('构建版本不匹配');
  if (buildInfo.sourceHashes['release-policy.json'] !== checksum(fs.readFileSync(path.join(root, 'release-policy.json')))) throw new Error('更新策略与源码快照不一致，请重新整理发布产物');
  for (const name of required.filter(name => name !== 'build-info.json')) {
    if (buildInfo.artifacts[name] !== checksum(fs.readFileSync(path.join(directory, name)))) throw new Error(`产物哈希不匹配：${name}`);
  }
  const release = await request(`${api}/tags/v${version}`);
  if (!release.draft) throw new Error('该版本已经发布，禁止覆盖；请提升版本号重新构建');
  async function upload(name) {
    const bytes = fs.readFileSync(path.join(directory, name));
    const ticket = await request(`${api}/${release.id}/asset-upload-url`, 'POST', { asset_name: name, size: bytes.length, overwrite: true, ttl: 0 });
    if (new URL(ticket.upload_url).protocol !== 'https:') throw new Error('上传地址必须为 HTTPS');
    const response = await fetch(ticket.upload_url, { method: 'PUT', body: bytes, redirect: 'error', signal: AbortSignal.timeout(600000) });
    if (!response.ok) throw new Error(`上传失败：${name} HTTP ${response.status}`);
    await request(ticket.verify_url, 'POST');
    const updated = await request(`${api}/${release.id}`);
    const asset = updated.assets.find(asset => asset.name === name);
    if (!asset || asset.size !== bytes.length) throw new Error(`远端附件大小不一致：${name}`);
    if (asset.hash_algo === 'sha256' && asset.hash_value !== checksum(bytes)) throw new Error(`远端附件哈希不一致：${name}`);
    console.log(`已上传 ${name} (${bytes.length} bytes)`);
    return asset;
  }
  const assets = {};
  for (const name of required) assets[name] = await upload(name);
  const platform = name => ({ url: assets[name].brower_download_url, signature: fs.readFileSync(path.join(directory, `${name}.sig`), 'utf8').trim() });
  for (const name of [macUpdate, windowsUpdate]) {
    const url = new URL(assets[name].brower_download_url);
    if (url.protocol !== 'https:' || url.hostname !== 'cnb.cool' || !url.pathname.startsWith(`/${repository}/-/releases/`)) throw new Error('更新下载地址不属于当前仓库');
  }
  const manifest = { version, mandatory: updatePolicy === 'mandatory', notes: fs.readFileSync(path.join(root, `docs/release-${version}.md`), 'utf8'), pub_date: new Date().toISOString(), platforms: { 'darwin-aarch64': platform(macUpdate), 'darwin-x86_64': platform(macUpdate), 'windows-x86_64': platform(windowsUpdate) } };
  fs.writeFileSync(path.join(directory, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
  await upload('latest.json');
  const names = [...required, 'latest.json'];
  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), names.map(name => `${checksum(fs.readFileSync(path.join(directory, name)))}  ${name}\n`).join(''));
  await upload('SHA256SUMS');
  if (publish) {
    await request(`${api}/${release.id}`, 'PATCH', { draft: false, prerelease: false, make_latest: 'true', body: fs.readFileSync(path.join(root, `docs/release-${version}.md`), 'utf8') });
    const result = await request(`${api}/${release.id}`);
    if (result.draft || !result.is_latest) throw new Error('发布状态未确认');
    console.log(`已正式发布：https://cnb.cool/${repository}/-/releases/tag/v${version}`);
  } else console.log('附件已上传到草稿；确认后使用 --publish 正式发布。');
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
