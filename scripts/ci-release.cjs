const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
process.chdir(root);
const version = require('../package.json').version;
const repository = 'nanzhaigame-xpy/MIDALocalizationTool';
const run = (command, args, options = {}) => execFileSync(command, args, { stdio: 'inherit', ...options });

async function main() {
  if (process.env.CNB_EVENT !== 'tag_push') throw new Error('发布仅允许由 tag_push 触发');
  if (!/^v\d+\.\d+\.\d+$/.test(process.env.CNB_BRANCH || '')) {
    console.log('非正式版本标签，跳过发布');
    return;
  }
  if (process.env.CNB_REPO_SLUG !== repository || process.env.CNB_BRANCH !== `v${version}`) throw new Error('仓库或版本标签与源码不一致');
  if (process.platform !== 'darwin') throw new Error('发布需要专用 Mac Runner');
  run('git', ['fetch', '--no-tags', 'origin', 'main']);
  run('git', ['merge-base', '--is-ancestor', 'HEAD', 'FETCH_HEAD']);
  if (run('git', ['status', '--porcelain'], { stdio: 'pipe', encoding: 'utf8' }).trim()) throw new Error('CI 发布必须使用干净的标签检出目录');
  const key = process.env.TAURI_SIGNING_PRIVATE_KEY || path.join(os.homedir(), '.local/share/mida-localization-release/updater.key');
  if (!fs.existsSync(key) || (fs.statSync(key).mode & 0o077)) throw new Error('请配置仓库外权限为 600 的原有更新签名私钥文件');
  if (!process.env.CNB_TOKEN) throw new Error('缺少 CNB 发布凭证');
  const target = fs.mkdtempSync(path.join(root, 'src-tauri/target/ci-release-'));
  process.env.CARGO_TARGET_DIR = target;
  process.env.MAC_BUNDLE_DIR = path.join(target, 'universal-apple-darwin/release/bundle');
  process.env.WINDOWS_BUNDLE_DIR = path.join(target, 'x86_64-pc-windows-msvc/release/bundle/nsis');
  process.env.TAURI_SIGNING_PRIVATE_KEY = key;
  const api = `https://api.cnb.cool/${repository}/-/releases`;
  async function request(suffix, method = 'GET', body) {
    const response = await fetch(api + suffix, { method, redirect: 'error', signal: AbortSignal.timeout(60000), headers: { Authorization: `Bearer ${process.env.CNB_TOKEN}`, 'Content-Type': 'application/json', Accept: 'application/vnd.cnb.api+json' }, body: body === undefined ? undefined : JSON.stringify(body) });
    if (response.status === 404 && method === 'GET') return null;
    if (!response.ok) throw new Error(`Release API 失败：HTTP ${response.status}`);
    return response.json();
  }
  const release = await request(`/tags/v${version}`);
  if (release && !release.draft) throw new Error('该版本已发布，请升版，禁止覆盖');
  run('npm', ['ci']);
  run(process.execPath, ['scripts/ci-check.cjs']);
  run('cargo', ['check', '--locked', '--manifest-path', 'src-tauri/Cargo.toml']);
  run('npm', ['run', 'release:mac', '--', '--ci']);
  run('npm', ['run', 'release:windows', '--', '--ci']);
  run(process.execPath, ['scripts/prepare-release.cjs']);
  if (!release) await request('', 'POST', { tag_name: `v${version}`, target_commitish: run('git', ['rev-parse', 'HEAD'], { stdio: 'pipe', encoding: 'utf8' }).trim(), name: `v${version}`, draft: true, prerelease: false, make_latest: 'false', body: fs.readFileSync(`docs/release-${version}.md`, 'utf8') });
  run(process.execPath, ['scripts/publish-cnb.cjs', `releases/${version}`, '--publish']);
}

fs.mkdirSync(path.join(root, 'src-tauri/target'), { recursive: true });
main().catch(error => { console.error(error.message); process.exitCode = 1; });
