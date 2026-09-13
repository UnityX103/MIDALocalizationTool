const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const repository = 'UnityX103/MIDALocalizationTool';
const version = require('../package.json').version;
const tag = `v${version}`;
const directory = path.join(root, 'releases', version);
const checksum = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const gh = args => execFileSync('gh', args, { cwd: root, stdio: 'inherit' });

async function main() {
  if (process.env.GITHUB_REPOSITORY !== repository || process.env.GITHUB_REF !== 'refs/heads/main' || !['push', 'workflow_dispatch'].includes(process.env.GITHUB_EVENT_NAME)) throw new Error('仅允许 GitHub main 云构建发布');
  const response = await fetch(`https://api.github.com/repos/${repository}/releases/tags/${tag}`, { headers: { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/vnd.github+json' }, signal: AbortSignal.timeout(60000) });
  if (response.status !== 404 && !response.ok) throw new Error(`GitHub Release 检查失败：${response.status}`);
  const release = response.ok ? await response.json() : null;
  if (release && !release.draft) { console.log(`${tag} 已发布；本次构建保留为 Actions 附件，不覆盖正式版。`); return; }
  const info = JSON.parse(fs.readFileSync(path.join(directory, 'build-info.json'), 'utf8'));
  if (info.gitHead !== process.env.GITHUB_SHA || info.version !== version) throw new Error('构建身份与当前提交不一致');
  const names = Object.keys(info.artifacts);
  for (const name of names) if (path.basename(name) !== name || checksum(fs.readFileSync(path.join(directory, name))) !== info.artifacts[name]) throw new Error(`产物校验失败：${name}`);
  const policy = require('../release-policy.json')[version];
  if (!['mandatory', 'optional'].includes(policy)) throw new Error('缺少更新策略');
  const notes = fs.readFileSync(path.join(root, `docs/release-${version}.md`), 'utf8');
  const platform = name => ({ url: `https://github.com/${repository}/releases/download/${tag}/${name}`, signature: fs.readFileSync(path.join(directory, `${name}.sig`), 'utf8').trim() });
  const mac = platform(`MIDA-Localization-${version}-macOS-universal.app.tar.gz`);
  const windows = platform(`MIDA-Localization-${version}-Windows-x64-setup.exe`);
  const manifest = { version, mandatory: policy === 'mandatory', notes, pub_date: new Date().toISOString(), platforms: { 'darwin-aarch64': mac, 'darwin-x86_64': mac, 'windows-x86_64': windows } };
  fs.writeFileSync(path.join(directory, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
  names.push('build-info.json', 'latest.json');
  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), names.map(name => `${checksum(fs.readFileSync(path.join(directory, name)))}  ${name}\n`).join(''));
  if (!release) gh(['release', 'create', tag, '--repo', repository, '--target', info.gitHead, '--draft', '--title', `MIDA 本地化编辑器 ${version}`, '--notes-file', `docs/release-${version}.md`]);
  gh(['release', 'upload', tag, '--repo', repository, '--clobber', ...[...names, 'SHA256SUMS'].map(name => path.join(directory, name))]);
  gh(['release', 'edit', tag, '--repo', repository, '--draft=false', '--latest']);
  console.log(`已发布 https://github.com/${repository}/releases/tag/${tag}`);
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
