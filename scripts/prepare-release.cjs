const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const version = require('../package.json').version;
const directory = path.join(root, 'releases', version);
const mac = process.env.MAC_BUNDLE_DIR || path.join(root, 'src-tauri/target/universal-apple-darwin/release/bundle');
const windows = path.join(process.env.WINDOWS_BUNDLE_DIR || path.join(root, 'src-tauri/target/windows-build/x86_64-pc-windows-msvc/release/bundle/nsis'));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

fs.mkdirSync(directory, { recursive: true });
const files = [
  [path.join(mac, `dmg/MIDA 本地化编辑器_${version}_universal.dmg`), `MIDA-Localization-${version}-macOS-universal.dmg`],
  [path.join(mac, 'macos/MIDA 本地化编辑器.app.tar.gz'), `MIDA-Localization-${version}-macOS-universal.app.tar.gz`],
  [path.join(mac, 'macos/MIDA 本地化编辑器.app.tar.gz.sig'), `MIDA-Localization-${version}-macOS-universal.app.tar.gz.sig`],
  [path.join(windows, `MIDA 本地化编辑器_${version}_x64-setup.exe`), `MIDA-Localization-${version}-Windows-x64-setup.exe`],
  [path.join(windows, `MIDA 本地化编辑器_${version}_x64-setup.exe.sig`), `MIDA-Localization-${version}-Windows-x64-setup.exe.sig`],
];
for (const [source, name] of files) {
  if (!fs.statSync(source).size) throw new Error(`构建产物为空：${source}`);
  fs.copyFileSync(source, path.join(directory, name));
}
const sources = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root, encoding: 'utf8' }).split('\0').filter(Boolean).sort();
for (const file of sources) {
  if (/(^|\/)(node_modules|target|dist|LocalOutput|releases|\.git|\.env)(\/|$)|\.(key|zip|mp4|dmg|exe)$/i.test(file)) throw new Error(`源码快照包含不应发布的文件：${file}`);
  if (!fs.lstatSync(path.join(root, file)).isFile()) throw new Error(`源码快照只允许普通文件：${file}`);
}
const sourceName = `MIDA-Localization-${version}-source.tar.gz`;
execFileSync('tar', ['-czf', path.join(directory, sourceName), '--null', '-T', '-'], { cwd: root, input: sources.join('\0') + '\0', env: { ...process.env, COPYFILE_DISABLE: '1' } });
const artifacts = Object.fromEntries([...files.map(([, name]) => name), sourceName].map(name => [name, hash(path.join(directory, name))]));
const info = {
  version,
  preparedAt: new Date().toISOString(),
  gitHead: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  sourceOfTruth: sourceName,
  uncommittedSourceIncluded: Boolean(execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()),
  sourceHashes: Object.fromEntries(sources.map(file => [file, hash(path.join(root, file))])),
  artifacts,
};
fs.writeFileSync(path.join(directory, 'build-info.json'), JSON.stringify(info, null, 2) + '\n');
console.log(`已整理发布产物：${directory}`);
