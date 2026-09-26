const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const { webcrypto } = require('node:crypto');

const root = path.resolve(__dirname, '..');
const context = vm.createContext({ crypto: webcrypto, TextEncoder, structuredClone, console });
for (const file of ['workspace-store.js', 'workload.js']) {
  vm.runInContext(fs.readFileSync(path.join(root, file), 'utf8'), context, { filename: file });
}
const html = fs.readFileSync(path.join(root, 'prototype.html'), 'utf8');
for (const name of ['requireValue', 'validVersion', 'validAssetPath', 'validUnitName',
  'unitMetadata', 'ownsPreview', 'validateWorkspace']) {
  const start = html.indexOf(`function ${name}(`);
  const end = html.indexOf('\nfunction ', start + 1);
  assert(start >= 0 && end > start, `production function ${name} not found`);
  vm.runInContext(html.slice(start, end), context, { filename: `prototype.html:${name}` });
}
const snapshot = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
context.validateWorkspace(snapshot);
const document = {
  format: 'mida-localization', formatVersion: 1,
  projectId: snapshot.currentProjectId, parentPackageId: snapshot.currentPackageId,
  fileVersion: { ...snapshot.fileVersion, revision: snapshot.exportSequence },
  deliveryState: 'ready', tasks: snapshot.tasks,
};
(async () => {
  const previous = snapshot.workLedger.deliveries.at(-1);
  const packet = await context.LocalizationWorkload.makePacket(snapshot.workLedger, document);
  assert.equal(packet.delivery.id, previous.id);
  assert.equal(packet.delivery.revision, previous.revision);
  assert.equal(packet.workload.cumulativeChars, 12);
  assert.equal(packet.workload.handoverChars, 0);
  assert.equal(packet.workload.totalChars, 5);
  assert.equal(packet.workload.records.length, 3);
  console.log('真实工作区通过前端恢复校验；Rust save/save_task/read 后重复导出身份及 12 字累计不变');
})().catch(error => { console.error(error); process.exitCode = 1; });
