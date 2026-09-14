const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { execFileSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
process.chdir(root);
const run = (command, args) => execFileSync(command, args, { stdio: 'inherit' });
const version = require('../package.json').version;
const lock = require('../package-lock.json');
const cargo = fs.readFileSync('src-tauri/Cargo.toml', 'utf8').split('[package]')[1].split('\n[')[0];
const cargoLock = fs.readFileSync('src-tauri/Cargo.lock', 'utf8').split('[[package]]').find(block => /name = "mida-localization"/.test(block));
if (!/^\d+\.\d+\.\d+$/.test(version) ||
    [lock.version, lock.packages[''].version, require('../src-tauri/tauri.conf.json').version,
      cargo.match(/^version = "([^"]+)"/m)?.[1], cargoLock?.match(/^version = "([^"]+)"/m)?.[1]].some(value => value !== version)) {
  throw new Error('package、lockfile、Tauri 和 Rust 版本必须一致');
}
if (!['mandatory', 'optional'].includes(require('../release-policy.json')[version])) throw new Error('当前版本缺少更新策略');
if (!fs.readFileSync(`docs/release-${version}.md`, 'utf8').trim()) throw new Error('缺少更新简述');
for (const match of fs.readFileSync('prototype.html', 'utf8').matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
  if (match[1].trim()) new vm.Script(match[1], { filename: 'prototype.html' });
}
for (const file of ['workspace-store.js', 'preview-player.js', 'import-worker.js', ...fs.readdirSync('scripts').filter(file => file.endsWith('.cjs')).map(file => `scripts/${file}`)]) run(process.execPath, ['--check', file]);
run(process.platform === 'win32' ? 'python' : 'python3', ['-c', 'import ast,pathlib; [ast.parse(path.read_text(encoding="utf-8"), filename=str(path)) for path in pathlib.Path(".").glob("*.py")]']);
run(process.execPath, ['scripts/prepare-desktop.cjs']);
for (const [source, output] of [['prototype.html', 'index.html'], ['workspace-store.js', 'workspace-store.js'], ['preview-player.js', 'preview-player.js'], ['app-icon.svg', 'app-icon.svg'], ['import-worker.js', 'import-worker.js']]) {
  if (!fs.readFileSync(source).equals(fs.readFileSync(`dist/${output}`))) throw new Error(`前端产物不匹配：${output}`);
}
console.log('语法、版本、更新策略及前端生成检查通过');
