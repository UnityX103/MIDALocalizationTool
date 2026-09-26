const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { webcrypto, createHash } = require('node:crypto');

const root = path.resolve(__dirname, '..');
const context = vm.createContext({ crypto: webcrypto, TextEncoder, structuredClone, console });
for (const file of ['workspace-store.js', 'workload.js']) {
  vm.runInContext(fs.readFileSync(path.join(root, file), 'utf8'), context, { filename: file });
}
const work = context.LocalizationWorkload;
const clone = value => structuredClone(value);
const source = (revision = 1) => ({
  projectId: 'acceptance-project', packageId: `source-${revision}`,
  fileVersion: { lineageId: 'acceptance-lineage', revision },
  assets: [{ type: 'localization-dialogues', partName: 'PartA' }],
});
const task = (text = '你好世界', version = 1) => ({
  partName: 'PartA', language: 'en', taskVersion: version,
  entries: [{
    key: 'Dialogue_1', currentSource: text, translation: '', translationAtExport: '',
    localizationSourceAtExport: '', speakerKey: '', speakerChineseName: '',
    issuesAtExport: [], review: { state: 'pending' },
  }],
});
function setup() {
  const current = task(), manifest = source(), ledger = work.mergeIncoming(work.create(), manifest, [current]);
  return { current, manifest, ledger };
}
function confirm(ledger, current, translation) {
  const entry = current.entries[0];
  const before = { translation: entry.translation, state: entry.review.state };
  const recorded = work.confirmed(ledger, current, { ...entry, translation }, before);
  entry.translation = translation;
  entry.review.state = 'confirmed';
  return recorded;
}
function packet(ledger, tasks, manifest = source()) {
  return work.makePacket(ledger, {
    format: 'mida-localization', formatVersion: 1, projectId: manifest.projectId,
    parentPackageId: manifest.packageId, fileVersion: clone(manifest.fileVersion),
    deliveryState: 'ready', tasks: clone(tasks),
  });
}
function receipt(delivery, ids = delivery.workload.records.map(record => record.id)) {
  return {
    version: 1, id: `receipt-${delivery.delivery.id}`, projectId: delivery.projectId,
    lineageId: delivery.fileVersion.lineageId, language: delivery.workload.scope.language,
    ledgerId: delivery.delivery.ledgerId,
    deliveryId: delivery.delivery.id, recordIds: ids, receivedAt: '2026-09-26T00:00:00Z',
  };
}

// Run the editor's functions verbatim; only rendering and media UI are stubbed.
function editorHarness(current, manifest, ledger) {
  const editor = vm.createContext({ crypto: webcrypto, TextEncoder, structuredClone, console });
  for (const file of ['workspace-store.js', 'workload.js']) {
    vm.runInContext(fs.readFileSync(path.join(root, file), 'utf8'), editor, { filename: file });
  }
  const html = fs.readFileSync(path.join(root, 'prototype.html'), 'utf8');
  for (const [start, end] of [
    ['const initialState=', 'const partNameOrder='],
    ['function isLegacyTaskWideReview(', 'function previewPartNames('],
    ['function captureWorkspace(', 'function saveWorkspace('],
  ]) {
    const begin = html.indexOf(start), finish = html.indexOf(end, begin);
    assert.ok(begin >= 0 && finish > begin, `Production function boundary missing: ${start}`);
    vm.runInContext(html.slice(begin, finish), editor, { filename: 'prototype.html' });
  }
  Object.assign(editor, {
    tasks: [clone(current)], editingEntries: new Map(), pendingTaskSaves: new Map(),
    currentProjectId: manifest.projectId, currentManifest: clone(manifest),
    currentPackageId: manifest.packageId, fileVersion: clone(manifest.fileVersion),
    exportSequence: 0, workLedger: clone(ledger), savedWorkIds: new Set(),
    previews: {}, previewPlayer: { snapshot: () => null, restore() {} },
    find: () => ({ value: '', textContent: '' }), renderAll() {},
    state: Object.assign(vm.runInContext('initialState()', editor), { active: 0, selected: new Set([0]) }),
  });
  return editor;
}

function applyMergePlan(editor, plan, manifest, incoming) {
  editor.workLedger = editor.LocalizationWorkload.mergeIncoming(editor.workLedger, manifest, [incoming]);
  editor.tasks.splice(0, editor.tasks.length, plan.merged);
  editor.editingEntries.clear();
  assert.ok(Array.isArray(plan.drafts), 'planMerge must return preserved drafts');
  for (const draft of plan.drafts) {
    const entry = plan.merged.entries.find(entry => entry.key === draft.key);
    assert.ok(entry, 'Preserved draft must belong to the merged task');
    editor.editingEntries.set(entry, clone(draft));
  }
  editor.currentManifest = clone(manifest);
  editor.currentPackageId = manifest.packageId;
  editor.fileVersion = clone(manifest.fileVersion);
}

function restartEditor(editor) {
  const saved = JSON.parse(JSON.stringify(editor.captureWorkspace()));
  const restarted = editorHarness(saved.tasks[0], saved.currentManifest, saved.workLedger);
  restarted.restoreWorkspace(saved);
  return restarted;
}

function confirmPreservedDraft(editor) {
  const current = editor.tasks[0], entry = current.entries[0], draft = editor.editingEntries.get(entry);
  assert.ok(draft?.before, 'Restored draft must retain its pre-edit baseline');
  const recorded = editor.LocalizationWorkload.confirmed(
    editor.workLedger, current, { ...entry, translation: draft.text }, draft.before,
  );
  entry.translation = draft.text;
  entry.review.state = 'confirmed';
  editor.editingEntries.delete(entry);
  return recorded;
}

test('han-v1 counts scalar values, excluding punctuation, ASCII, and whitespace', () => {
  assert.equal(work.count('你好，World 123!\n〇\u{20000}\u{30000}'), 5);
  assert.equal(work.count(' \t\r\n'), 0);
});

test('existing confirmed translations affect progress but not new labor', () => {
  const { ledger, current } = setup();
  current.entries[0].translation = 'Hello';
  current.entries[0].review.state = 'confirmed';
  assert.equal(confirm(ledger, current, 'Hello world'), false);
  assert.equal(ledger.records.length, 0);
  assert.equal(work.progress([current]).progressChars, 4);
});

test('repeat edits preserve the original work ID, count and immutable record', async () => {
  const { ledger, current } = setup();
  assert.equal(confirm(ledger, current, 'Hello world'), true);
  const original = clone(ledger.records[0]);
  const first = await packet(ledger, [current]);
  for (const text of ['Hello again', 'Welcome back', 'Hello world']) {
    assert.equal(confirm(ledger, current, text), false);
    const next = await packet(ledger, [current]);
    assert.equal(next.workload.cumulativeChars, 4);
    assert.equal(next.workload.handoverChars, 4);
    assert.deepEqual(clone(ledger.records[0]), original);
  }
  assert.equal(ledger.records.length, 1);
  assert.notEqual(ledger.deliveries.at(-1).id, first.delivery.id);
});

test('repeat export keeps package identity, timestamps, revisions and nonzero delta', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  const second = await packet(ledger, [current]);
  assert.deepEqual(second, first);
  assert.equal(second.workload.previousDeliveryDeltaChars, 4);
  assert.equal(second.workload.handoverChars, 4);
  assert.equal(ledger.deliveries.length, 1);
});

test('new source revision retains obsolete labor and adds new source count', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  await packet(ledger, [current]);
  const revised = task('欢迎回来', 2);
  revised.entries[0].translation = current.entries[0].translation;
  ledger = work.mergeIncoming(ledger, source(2), [revised]);
  assert.equal(confirm(ledger, revised, 'Welcome back'), true);
  const delivered = await packet(ledger, [revised], source(2));
  assert.equal(delivered.workload.cumulativeChars, 8);
  assert.equal(delivered.workload.totalChars, 4);
  assert.equal(delivered.workload.sinceSourceUpdateChars, 4);
  assert.equal(delivered.workload.previousDeliveryDeltaChars, 4);
  assert.equal(ledger.records[1].kind, 'source_revision');
});

test('deleted source entries retain historical work in delivery ledger', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const replacement = task('新内容', 2);
  replacement.entries[0].key = 'Dialogue_2';
  ledger = work.mergeIncoming(ledger, source(2), [replacement]);
  const delivered = await packet(ledger, [replacement], source(2));
  assert.equal(delivered.workload.cumulativeChars, 4);
  assert.equal(delivered.workload.totalChars, 3);
  assert.equal(delivered.workload.records[0].key, 'Dialogue_1');
});

test('a receipt for D1 cannot acknowledge new records in D2; repeated receipts are idempotent', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  const revised = task('欢迎回来', 2);
  ledger = work.mergeIncoming(ledger, source(2), [revised]);
  confirm(ledger, revised, 'Welcome back');
  const second = await packet(ledger, [revised], source(2));
  const incoming = { ...source(3), workReceipts: [receipt(first)] };
  ledger = work.mergeIncoming(ledger, incoming, [revised]);
  ledger = work.mergeIncoming(ledger, incoming, [revised]);
  const next = await packet(ledger, [revised], incoming);
  assert.equal(next.workload.handoverChars, 4);
  assert.equal(next.workload.cumulativeChars, 8);
  assert.equal(ledger.acknowledgedIds.length, 1);
  assert.equal(ledger.receipts.length, 1);
  assert.equal(first.workload.handoverChars, 4);
  assert.equal(second.workload.handoverChars, 8);
});

test('skipping an intermediate export does not lose work; latest receipt covers both', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  await packet(ledger, [current]);
  const revised = task('欢迎回来', 2);
  ledger = work.mergeIncoming(ledger, source(2), [revised]);
  confirm(ledger, revised, 'Welcome back');
  const second = await packet(ledger, [revised], source(2));
  const incoming = { ...source(3), workReceipts: [receipt(second)] };
  ledger = work.mergeIncoming(ledger, incoming, [revised]);
  const next = await packet(ledger, [revised], incoming);
  assert.equal(next.workload.cumulativeChars, 8);
  assert.equal(next.workload.handoverChars, 0);
});

test('new source without receipt never acknowledges previous work', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  await packet(ledger, [current]);
  ledger = work.mergeIncoming(ledger, source(2), [current]);
  assert.equal((await packet(ledger, [current], source(2))).workload.handoverChars, 4);
});

test('foreign ledger receipts do not acknowledge; unknown delivery receipts wait', async () => {
  let { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  const foreign = { ...receipt(first), ledgerId: 'foreign-ledger' };
  ledger = work.mergeIncoming(ledger, { ...source(2), workReceipts: [foreign] }, [current]);
  assert.equal(ledger.acknowledgedIds.length, 0);
  const unknown = { ...receipt(first), id: 'pending-receipt', deliveryId: 'unknown-delivery' };
  ledger = work.mergeIncoming(ledger, { ...source(3), workReceipts: [unknown] }, [current]);
  assert.equal(ledger.acknowledgedIds.length, 0);
  assert.equal(ledger.receipts.length, 1);
});

test('invalid receipt cannot mutate the original ledger', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  const before = work.canonical(ledger);
  const incoming = { ...source(2), workReceipts: [receipt(first, ['not-in-delivery'])] };
  assert.throws(() => work.mergeIncoming(ledger, incoming, [current]), /未提交/);
  assert.equal(work.canonical(ledger), before);
});

test('invalid export scope does not poison the ledger or consume an identity', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const oversized = Array.from({ length: 201 }, (_, index) => ({ ...clone(current), partName: `Part${index}` }));
  await assert.rejects(packet(ledger, oversized), /统计片段/);
  assert.equal(ledger.deliveries.length, 0);
  work.validateLedger(ledger);
  assert.equal((await packet(ledger, [current])).workload.cumulativeChars, 4);
});

test('legacy same-version reimport does not reset source-update count', async () => {
  let ledger = work.create();
  const current = task(), manifest = source();
  work.alignSources(ledger, [current], manifest);
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  ledger = work.mergeIncoming(ledger, manifest, [current]);
  const second = await packet(ledger, [current]);
  assert.equal(first.workload.sinceSourceUpdateChars, 4);
  assert.equal(second.workload.sinceSourceUpdateChars, 4);
});

test('rollback keeps newer labor but assigns new work to historical source', () => {
  let { ledger, current } = setup();
  const oldSources = clone(ledger.sources);
  const newer = task('欢迎回来', 2);
  ledger = work.mergeIncoming(ledger, source(2), [newer]);
  confirm(ledger, newer, 'Welcome back');
  work.alignSources(ledger, [current], source(), oldSources);
  confirm(ledger, current, 'Hello world');
  assert.equal(ledger.records.length, 2);
  assert.equal(ledger.records[1].sourcePackageId, 'source-1');
  assert.equal(ledger.records[1].sourceRevision, 1);
  assert.equal(ledger.records[1].taskVersion, 1);
});

test('legacy partial manifest uses explicit unknown provenance, not unrelated source', () => {
  const ledger = work.create(), current = task();
  work.alignSources(ledger, [current], { ...source(), assets: [] });
  confirm(ledger, current, 'Hello world');
  assert.equal(ledger.records[0].sourcePackageId, null);
  assert.equal(ledger.records[0].sourceRevision, null);
  work.validateLedger(ledger);
});

test('fixed whitespace and explicitly permitted empty translations share progress rule', () => {
  const current = task();
  current.entries[0].review.state = 'confirmed';
  for (const whitespace of ['\u0085', '\ufeff', '\u001c', '\u3000']) {
    current.entries[0].translation = whitespace;
    assert.equal(work.progress([current]).progressChars, 0);
  }
  current.sourceKind = 'unity-minigame';
  current.assetProtocolVersion = 1;
  current.entries[0].allowEmpty = true;
  assert.equal(work.progress([current]).progressChars, 4);
});

test('wire summaries reject tampering, duplicate IDs and unsupported count rules', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  for (const change of [
    value => value.workload.cumulativeChars++,
    value => value.workload.records.push(clone(value.workload.records[0])),
    value => { value.workload.countingRule = 'unknown'; },
    value => { value.workload.records[0].sourceRevision = null; },
  ]) {
    const tampered = clone(first);
    change(tampered);
    assert.throws(() => work.validatePackage(tampered, tampered.tasks));
  }
});

test('serialization and reload preserve repeat-export identity', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const first = await packet(ledger, [current]);
  const restored = JSON.parse(JSON.stringify(ledger));
  work.validateLedger(restored);
  assert.deepEqual(await packet(restored, [current]), first);
});

test('actual planMerge preserves pending draft provenance across source updates and restart', async () => {
  const { ledger, current, manifest } = setup();
  let editor = editorHarness(current, manifest, ledger);
  editor.editingEntries.set(editor.tasks[0].entries[0], { text: 'Hello world' });
  const incoming = task('你好世界', 2);
  const plan = await editor.planMerge({ task: incoming }, async () => {});
  assert.equal(plan.merged.entries[0].translation, '');
  assert.deepEqual(clone(plan.drafts), [{
    key: 'Dialogue_1', text: 'Hello world', before: { translation: '', state: 'pending' },
  }]);
  applyMergePlan(editor, plan, source(2), incoming);
  editor = restartEditor(editor);
  assert.deepEqual(clone(editor.editingEntries.get(editor.tasks[0].entries[0]).before),
    { translation: '', state: 'pending' });

  const newer = task('你好世界', 3);
  applyMergePlan(editor, await editor.planMerge({ task: newer }, async () => {}), source(3), newer);
  editor = restartEditor(editor);
  assert.equal(confirmPreservedDraft(editor), true);
  const first = await packet(editor.workLedger, editor.tasks, source(3));
  assert.equal(first.workload.progressChars, 4);
  assert.equal(first.workload.cumulativeChars, 4);
  assert.equal(first.workload.handoverChars, 4);
  assert.equal(first.workload.records[0].sourcePackageId, 'source-3');
  const original = clone(first.workload.records);

  editor = restartEditor(editor);
  assert.equal(confirm(editor.workLedger, editor.tasks[0], 'Hello world'), false);
  assert.deepEqual(await packet(editor.workLedger, editor.tasks, source(3)), first);
  assert.deepEqual(clone(editor.workLedger.records), original);
});

test('actual planMerge keeps existing confirmed polishing nonbillable after restart', async () => {
  const { ledger, current, manifest } = setup();
  current.entries[0].translation = 'Hello world';
  current.entries[0].review.state = 'confirmed';
  let editor = editorHarness(current, manifest, ledger);
  editor.editingEntries.set(editor.tasks[0].entries[0], {
    text: 'Greetings, world!', before: { translation: 'Hello world', state: 'confirmed' },
  });
  const incoming = task('你好世界', 2);
  const plan = await editor.planMerge({ task: incoming }, async () => {});
  assert.equal(plan.merged.entries[0].translation, 'Hello world');
  assert.equal(plan.drafts[0].before.state, 'confirmed');
  applyMergePlan(editor, plan, source(2), incoming);
  editor = restartEditor(editor);
  assert.equal(confirmPreservedDraft(editor), false);
  const delivered = await packet(editor.workLedger, editor.tasks, source(2));
  assert.equal(delivered.workload.progressChars, 4);
  assert.equal(delivered.workload.cumulativeChars, 0);
  assert.equal(delivered.workload.records.length, 0);
  assert.equal(delivered.tasks[0].entries[0].translation, 'Greetings, world!');
});

test('actual planMerge preserves confirmed draft baseline when only the speaker changes', async () => {
  const { ledger, current, manifest } = setup();
  current.entries[0].translation = 'Hello world';
  current.entries[0].review.state = 'confirmed';
  current.entries[0].speakerKey = 'SpeakerA';
  let editor = editorHarness(current, manifest, ledger);
  editor.editingEntries.set(editor.tasks[0].entries[0], { text: 'Greetings, world!' });

  for (const [revision, speaker] of [[2, 'SpeakerB'], [3, 'SpeakerC']]) {
    const incoming = task('你好世界', revision);
    incoming.entries[0].speakerKey = speaker;
    const plan = await editor.planMerge({ task: incoming }, async () => {});
    assert.equal(plan.stats.changed, 1);
    assert.equal(plan.merged.entries[0].speakerKey, speaker);
    assert.equal(plan.merged.entries[0].translation, 'Hello world');
    assert.equal(plan.merged.entries[0].review.state, 'pending');
    assert.deepEqual(clone(plan.drafts[0]), {
      key: 'Dialogue_1', text: 'Greetings, world!',
      before: { translation: 'Hello world', state: 'confirmed' },
    });
    applyMergePlan(editor, plan, source(revision), incoming);
    editor = restartEditor(editor);
    assert.deepEqual(clone(editor.editingEntries.get(editor.tasks[0].entries[0]).before),
      { translation: 'Hello world', state: 'confirmed' });
  }

  // No historical record may mask an incorrect billable-state decision.
  assert.equal(editor.workLedger.records.length, 0);
  assert.equal(confirmPreservedDraft(editor), false);
  const delivered = await packet(editor.workLedger, editor.tasks, source(3));
  assert.equal(delivered.workload.progressChars, 4);
  assert.equal(delivered.workload.totalChars, 4);
  assert.equal(delivered.workload.cumulativeChars, 0);
  assert.equal(delivered.workload.handoverChars, 0);
  assert.equal(delivered.workload.sinceSourceUpdateChars, 0);
  assert.equal(delivered.workload.records.length, 0);
  assert.equal(delivered.tasks[0].entries[0].translation, 'Greetings, world!');
  editor = restartEditor(editor);
  assert.deepEqual(await packet(editor.workLedger, editor.tasks, source(3)), delivered);
});

test('actual planMerge makes a confirmed draft pending for changed Chinese and bills it once', async () => {
  const { ledger, current, manifest } = setup();
  confirm(ledger, current, 'Hello world');
  const original = clone(ledger.records[0]);
  let editor = editorHarness(current, manifest, ledger);
  editor.editingEntries.set(editor.tasks[0].entries[0], {
    text: 'Hello new world', before: { translation: 'Hello world', state: 'confirmed' },
  });
  const incoming = task('你好新世界', 2);
  const plan = await editor.planMerge({ task: incoming }, async () => {});
  assert.equal(plan.merged.entries[0].translation, 'Hello world');
  assert.deepEqual(clone(plan.drafts[0].before), { translation: 'Hello world', state: 'pending' });
  applyMergePlan(editor, plan, source(2), incoming);
  editor = restartEditor(editor);
  assert.equal(confirmPreservedDraft(editor), true);
  const delivered = await packet(editor.workLedger, editor.tasks, source(2));
  assert.equal(delivered.workload.totalChars, 5);
  assert.equal(delivered.workload.cumulativeChars, 9);
  assert.equal(delivered.workload.sinceSourceUpdateChars, 5);
  assert.deepEqual(clone(editor.workLedger.records[0]), original);
  editor = restartEditor(editor);
  assert.equal(confirm(editor.workLedger, editor.tasks[0], 'Hello new world'), false);
  assert.deepEqual(await packet(editor.workLedger, editor.tasks, source(2)), delivered);
});

test('known delivery identity rejects altered acknowledgment, summary, metadata and task content atomically', async t => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const delivered = await packet(ledger, [current]);
  const wireFingerprint = await work.fingerprint(delivered);
  assert.equal(wireFingerprint, ledger.deliveries[0].fingerprint);
  const baseline = work.canonical(ledger);
  const reimported = work.mergeIncoming(ledger, delivered, delivered.tasks, wireFingerprint);
  assert.equal(work.canonical(reimported), baseline);
  for (const [name, mutate] of [
    ['acknowledged records', value => {
      value.workload.acknowledgedRecordIds = value.workload.records.map(record => record.id);
      value.workload.handoverChars = 0;
    }],
    ['delta summary', value => {
      value.workload.newRecordIds = [];
      value.workload.previousDeliveryDeltaChars = 0;
    }],
    ['exported timestamp', value => { value.exportedAt = '2026-09-25T00:00:00Z'; }],
    ['file revision', value => { value.fileVersion.revision++; }],
    ['previous delivery', value => { value.delivery.previousId = 'different-previous-delivery'; }],
    ['delivery revision', value => { value.delivery.revision++; }],
    ['task translation', value => { value.tasks[0].entries[0].translation = 'Changed delivery text'; }],
  ]) {
    await t.test(name, async () => {
      const altered = clone(delivered);
      mutate(altered);
      work.validatePackage(altered, altered.tasks);
      const fingerprint = await work.fingerprint(altered);
      assert.throws(() => work.mergeIncoming(ledger, altered, altered.tasks, fingerprint), /交付.*冲突/);
      assert.equal(work.canonical(ledger), baseline);
    });
  }
  assert.throws(() => work.mergeIncoming(ledger, delivered, delivered.tasks), /交付|指纹/);
  assert.equal(work.canonical(ledger), baseline);
});

test('imported delivery remembers its wire fingerprint and rejects later task changes', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const delivered = await packet(ledger, [current]);
  const fingerprint = await work.fingerprint(delivered);
  const imported = work.mergeIncoming(work.create(), delivered, delivered.tasks, fingerprint);
  assert.equal(imported.deliveries[0].fingerprint, fingerprint);
  const baseline = work.canonical(imported);
  const changed = clone(delivered);
  changed.tasks[0].entries[0].translation = 'Changed after handoff';
  work.validatePackage(changed, changed.tasks);
  const changedFingerprint = await work.fingerprint(changed);
  assert.throws(() => work.mergeIncoming(imported, changed, changed.tasks, changedFingerprint), /交付.*冲突/);
  assert.equal(work.canonical(imported), baseline);
});

test('wire task hashes preserve delivery fingerprint before review normalization', async () => {
  const { ledger, current } = setup();
  confirm(ledger, current, 'Hello world');
  const delivered = await packet(ledger, [current]);
  const fingerprint = await work.fingerprint(delivered);
  const hashes = await work.hashTasks(delivered.tasks);
  assert.equal(await work.fingerprint(delivered, hashes), fingerprint);
  const normalized = clone(delivered);
  normalized.tasks[0].entries[0].review = { state: 'pending', reason: 'Imported review requirement' };
  assert.notEqual(await work.fingerprint(normalized), fingerprint);
  assert.equal(await work.fingerprint(normalized, hashes), fingerprint);
  assert.doesNotThrow(() => work.mergeIncoming(ledger, normalized, normalized.tasks, fingerprint));
});

test('chapter delivery stays idempotent after export strips only previewPartNames', async () => {
  const { ledger, current } = setup();
  Object.assign(current, {
    unitKind: 'chapter', chapterName: 'PartA', previewPartNames: ['SceneA', 'SceneB'],
    sourceHash: 'a'.repeat(64), targetLocalizationHashAtExport: 'b'.repeat(64),
  });
  confirm(ledger, current, 'Hello world');
  const delivered = await packet(ledger, [current]);
  const wire = clone(delivered);
  // Both ZIP writers omit this task-level media reference from data-only deliveries.
  delete wire.tasks[0].previewPartNames;
  work.validatePackage(wire, wire.tasks);
  assert.deepEqual(clone(delivered.tasks[0].previewPartNames), ['SceneA', 'SceneB']);
  const expectedHash = createHash('sha256').update(work.canonical(wire.tasks[0])).digest('hex');
  assert.deepEqual(clone(await work.hashTasks(delivered.tasks)), [expectedHash]);
  const wireHashes = await work.hashTasks(wire.tasks);
  assert.deepEqual(clone(wireHashes), [expectedHash]);
  const wireFingerprint = await work.fingerprint(wire, wireHashes);
  assert.equal(wireFingerprint, ledger.deliveries[0].fingerprint);

  const baseline = work.canonical(ledger);
  let imported = work.mergeIncoming(ledger, wire, wire.tasks, wireFingerprint);
  imported = work.mergeIncoming(imported, wire, wire.tasks, wireFingerprint);
  assert.equal(work.canonical(imported), baseline);
  imported = JSON.parse(JSON.stringify(imported));
  work.validateLedger(imported);
  const repeat = await packet(imported, wire.tasks);
  assert.equal(repeat.packageId, delivered.packageId);
  assert.equal(repeat.exportedAt, delivered.exportedAt);
  assert.deepEqual(repeat.delivery, delivered.delivery);
  assert.deepEqual(repeat.workload, delivered.workload);
  assert.equal(imported.deliveries.length, 1);

  for (const [field, value] of [
    ['taskVersion', 2], ['unitKind', 'part'], ['chapterName', 'DifferentChapter'],
    ['sourceHash', 'c'.repeat(64)], ['targetLocalizationHashAtExport', 'd'.repeat(64)],
  ]) {
    const changed = clone(wire);
    changed.tasks[0][field] = value;
    work.validatePackage(changed, changed.tasks);
    const changedFingerprint = await work.fingerprint(changed);
    assert.notEqual(changedFingerprint, wireFingerprint, `${field} must remain part of the fingerprint`);
    assert.throws(() => work.mergeIncoming(imported, changed, changed.tasks, changedFingerprint), /交付.*冲突/);
    assert.equal(work.canonical(imported), baseline);
  }
});

test('two-part partial deliveries acknowledge only the accepted scope without losing other work', async () => {
  const a = task(), b = { ...task('明天见'), partName: 'PartB' };
  const manifest = { ...source(), assets: ['PartA', 'PartB'].map(partName => ({ type: 'localization-dialogues', partName })) };
  let ledger = work.mergeIncoming(work.create(), manifest, [a, b]);
  confirm(ledger, a, 'Hello world');
  confirm(ledger, b, 'See you tomorrow');
  const full = await packet(ledger, [a, b], manifest);
  const onlyA = await packet(ledger, [a], manifest);
  const onlyB = await packet(ledger, [b], manifest);
  const bothAgain = await packet(ledger, [a, b], manifest);
  assert.equal(full.workload.cumulativeChars, 7);
  assert.equal(onlyA.workload.cumulativeChars, 4);
  assert.equal(onlyA.workload.previousDeliveryDeltaChars, 0);
  assert.equal(onlyB.workload.cumulativeChars, 3);
  assert.equal(onlyB.workload.previousDeliveryDeltaChars, 3);
  assert.equal(bothAgain.workload.previousDeliveryDeltaChars, 4);
  assert.equal(bothAgain.workload.handoverChars, 7);
  const incoming = { ...source(2), workReceipts: [receipt(onlyA)] };
  ledger = work.mergeIncoming(ledger, incoming, [a]);
  ledger = work.mergeIncoming(ledger, incoming, [a]);
  assert.deepEqual(clone(ledger.acknowledgedIds), onlyA.workload.records.map(record => record.id));
  const after = await packet(ledger, [a, b], incoming);
  assert.equal(after.workload.cumulativeChars, 7);
  assert.equal(after.workload.handoverChars, 3);
  assert.equal((await packet(ledger, [a], incoming)).workload.handoverChars, 0);
  const remaining = await packet(ledger, [b], incoming);
  assert.equal(remaining.workload.handoverChars, 3);
  assert.deepEqual(clone(after.workload.records), clone(full.workload.records));
  const bothReceipts = { ...source(3), workReceipts: [receipt(onlyA), receipt(remaining)] };
  ledger = work.mergeIncoming(ledger, bothReceipts, [a, b]);
  const final = await packet(ledger, [a, b], bothReceipts);
  assert.equal(final.workload.handoverChars, 0);
  assert.equal(final.workload.cumulativeChars, 7);
});

test('independent language ledgers and wrong-language receipts cannot acknowledge one another', async () => {
  const en = task(), ja = { ...task(), language: 'ja' };
  let english = work.mergeIncoming(work.create(), source(), [en]);
  let japanese = work.mergeIncoming(work.create(), source(), [ja]);
  confirm(english, en, 'Hello world');
  confirm(japanese, ja, 'こんにちは世界');
  const enDelivery = await packet(english, [en]), jaDelivery = await packet(japanese, [ja]);
  assert.notEqual(english.id, japanese.id);
  const enReceipt = receipt(enDelivery), jaReceipt = receipt(jaDelivery);
  const wrongLanguage = { ...enReceipt, id: 'wrong-language-receipt', language: 'ja' };
  english = work.mergeIncoming(english, { ...source(2), workReceipts: [wrongLanguage, jaReceipt] }, [en]);
  japanese = work.mergeIncoming(japanese, { ...source(2), workReceipts: [enReceipt] }, [ja]);
  assert.equal(english.acknowledgedIds.length, 0);
  assert.equal(japanese.acknowledgedIds.length, 0);
  english = work.mergeIncoming(english, { ...source(3), workReceipts: [enReceipt, jaReceipt] }, [en]);
  assert.equal((await packet(english, [en], source(3))).workload.handoverChars, 0);
  assert.equal((await packet(japanese, [ja], source(2))).workload.handoverChars, 4);
  japanese = work.mergeIncoming(japanese, { ...source(3), workReceipts: [enReceipt, jaReceipt] }, [ja]);
  const final = await packet(japanese, [ja], source(3));
  assert.equal(final.workload.handoverChars, 0);
  assert.equal(final.workload.cumulativeChars, 4);
  assert.deepEqual(clone(english.records), clone(enDelivery.workload.records));
  assert.deepEqual(clone(japanese.records), clone(jaDelivery.workload.records));
});
