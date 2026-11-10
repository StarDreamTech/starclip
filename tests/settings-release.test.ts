import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import { defaultAppearance } from '../src/appearance.ts';
import { emptyGrowth } from '../src/starpet-growth.ts';

// Real component setup and exposed guards; all responses are fixed in memory.
// No native window, storage, clipboard, token or personal profile is accessed.
const renderer = createRenderer<object, object>({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  setText() {}, setElementText() {}, patchProp() {}, insert() {}, remove() {},
  parentNode: () => null, nextSibling: () => null,
});

test('cleanup releases replaced and unmounted previews, including a late response', async () => {
  const cancelled: string[] = [];
  let resolve!: (value: { token: string; items: [] }) => void;
  const f = await mount('CleanupPanel', { isDesktop: true, api: {
    async cancelCleanup(token: string) { cancelled.push(token); },
    previewCleanup: () => new Promise(done => { resolve = done; }),
  } });
  f.vm.preview = { token: 'old-preview', items: [] };
  f.vm.kind = 'image';
  assert.deepEqual(cancelled, ['old-preview']);
  f.vm.preview = { token: 'visible-preview', items: [] };
  f.cleanup();
  assert.deepEqual(cancelled, ['old-preview', 'visible-preview']);
  const late = await mount('CleanupPanel', { isDesktop: true, api: {
    async cancelCleanup(token: string) { cancelled.push(token); },
    previewCleanup: () => new Promise(done => { resolve = done; }),
  } });
  const pending = late.vm.requestPreview();
  late.cleanup();
  resolve({ token: 'late-preview', items: [] });
  await pending;
  assert.equal(cancelled.at(-1), 'late-preview');
  assert.equal(late.vm.preview, null);
});
function moduleUrl(source: string) { return 'data:text/javascript;base64,' + Buffer.from(source).toString('base64'); }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
let sequence = 0;
async function mount(name: string, mock: Record<string, unknown> = {}, props: Record<string, unknown> = {}) {
  const savedDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { hidden: true, addEventListener() {}, removeEventListener() {}, getElementById() { return null; } } });
  Object.assign(globalThis, { settingsReleaseMock: mock });
  const { descriptor } = parse(fs.readFileSync(new URL(`../src/components/${name}.vue`, import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'settings-release-test' });
  const source = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText
    .replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replace(/(['"])\.\/[^'"]+\.vue\1/g, `'${moduleUrl('export default { render() { return null; } };')}'`)
    .replace(/import \{([^}]+)\} from ['"]\.\.\/(api|[^'"]+-api)['"];?/g, (_match, names: string) => {
      const url = moduleUrl(`export const { ${names} } = globalThis.settingsReleaseMock; // ${sequence++}`);
      return `import {${names}} from '${url}';`;
    })
    .replace(/from ['"]\.\.\/i18n['"]/g, `from '${new URL('../src/i18n/index.ts', import.meta.url).href}'`)
    .replace(/from ['"]\.\.\/([^'"]+)['"]/g, (_match, file: string) => `from '${new URL(`../src/${file}.ts`, import.meta.url).href}'`);
  const component = (await import(moduleUrl(source + `\n// ${sequence++}`))).default;
  const app = renderer.createApp({ ...component, render() { return null; } }, props);
  const exposed = app.mount({}) as { canRelease: boolean };
  const vm = app._instance!.setupState as Record<string, any>;
  return { vm, exposed, cleanup() { app.unmount(); if (savedDocument) Object.defineProperty(globalThis, 'document', savedDocument); else Reflect.deleteProperty(globalThis, 'document'); } };
}

test('privacy release waits for a successful read and preserves rule drafts and scan confirmation', async () => {
  const rules = { revision: 1, rules: [] };
  let finish!: (value: typeof rules) => void;
  const f = await mount('PrivacyRules', { isDesktop: true, api: {
    privacyRules: () => new Promise(resolve => { finish = resolve; }),
    async savePrivacyRules() { throw new Error('fixed save failure'); },
  } }, { revision: 1 });
  try {
    assert.equal(f.exposed.canRelease, false);
    finish(rules); await flush(); assert.equal(f.exposed.canRelease, true);
    f.vm.add(); assert.equal(f.exposed.canRelease, false);
    await f.vm.save(); assert.equal(f.exposed.canRelease, false, 'failed save keeps draft');
    f.vm.draft = structuredClone(rules); assert.equal(f.exposed.canRelease, true);
    f.vm.preview = { token: 'fixed-scan-preview' }; assert.equal(f.exposed.canRelease, false);
    f.vm.preview = null; assert.equal(f.exposed.canRelease, true);
    f.vm.busy = true; assert.equal(f.exposed.canRelease, false);
  } finally { f.cleanup(); }
});

test('AI release preserves unfinished forms and credentials, then accepts an explicitly completed form', async () => {
  const state = { enabled: true, available: true, connections: [], audit: [], limits: { maxRecords: 500, maxResults: 50, maxHours: 168 } };
  const f = await mount('AiSettings', { aiApi: { async state() { return state; }, async create() { return { state, config: { cli: 'fixed-test-configuration' } }; } } }, { clips: [], groups: [], revision: 1 });
  try {
    assert.equal(f.exposed.canRelease, false); await flush(); assert.equal(f.exposed.canRelease, true);
    for (const [field, value] of Object.entries({ name: '原创连接', fields: ['title'], maxResults: 10, expiresHours: 8, selected: [1], allowPetControl: true, search: '原创', group: 1 })) {
      const original = f.vm[field]; f.vm[field] = value; assert.equal(f.exposed.canRelease, false, field); f.vm[field] = original;
    }
    await flush(); assert.equal(f.exposed.canRelease, true);
    f.vm.allowPetControl = true; f.vm.name = '原创连接'; await flush(); f.vm.prepare();
    assert.equal(f.exposed.canRelease, false); await f.vm.create(); await flush();
    assert.equal(f.exposed.canRelease, false, 'new connection configuration must be explicitly dismissed');
    f.vm.config = null; assert.equal(f.exposed.canRelease, true, 'successfully submitted values become the clean baseline');
    f.vm.name = '尚未提交的新连接'; assert.equal(f.exposed.canRelease, false);
  } finally { f.cleanup(); }
});

test('appearance and data release protect draft angles, confirmations, pending changes and failed initial reads', async () => {
  const appearance = await mount('AppearanceSettings', {}, { value: defaultAppearance() });
  try {
    assert.equal(appearance.exposed.canRelease, true);
    appearance.vm.angleDraft = 45; assert.equal(appearance.exposed.canRelease, false);
    appearance.vm.angleDraft = defaultAppearance().angle; appearance.vm.removeConfirm = true; assert.equal(appearance.exposed.canRelease, false);
    appearance.vm.removeConfirm = false; appearance.vm.working = true; assert.equal(appearance.exposed.canRelease, false);
  } finally { appearance.cleanup(); }
  const data = await mount('DataSettings', { isDesktop: true, api: { async getDataLocation() { throw new Error('fixed unavailable location'); } } });
  try {
    assert.equal(data.exposed.canRelease, false); await flush(); assert.equal(data.exposed.canRelease, false);
    data.vm.location = { pendingPath: null }; assert.equal(data.exposed.canRelease, true);
    data.vm.plan = { targetPath: '/fixed-fixture/destination' }; assert.equal(data.exposed.canRelease, false);
    data.vm.plan = null; data.vm.location = { pendingPath: '/fixed-fixture/destination' }; assert.equal(data.exposed.canRelease, false);
  } finally { data.cleanup(); }
});

test('growth and pet settings release only after readiness and preserve import, reset and unapplied choices', async () => {
  const growth = await mount('StarPetGrowth', { starPetDesktopAvailable: true, async onStarPetGrowthChanged() { return () => {}; }, async getStarPetGrowth() { return emptyGrowth(); } });
  try {
    assert.equal(growth.exposed.canRelease, false); await flush(); assert.equal(growth.exposed.canRelease, true);
    growth.vm.pendingImport = { data: 'fixed-archive' }; assert.equal(growth.exposed.canRelease, false);
    growth.vm.pendingImport = null; await growth.vm.askReset(); assert.equal(growth.exposed.canRelease, false, 'revision zero is also a confirmation');
    growth.vm.resetRevision = null; assert.equal(growth.exposed.canRelease, true);
    growth.vm.busy = true; assert.equal(growth.exposed.canRelease, false);
  } finally { growth.cleanup(); }
  const pet = await mount('StarPetSettings', { starPetDesktopAvailable: true, async onStarPetChanged() { return () => {}; }, async getStarPetPreferences() { return { enabled: true, mood: 'idle', color: '#FFD66E', size: 100 }; } });
  try {
    assert.equal(pet.exposed.canRelease, false); await flush(); assert.equal(pet.exposed.canRelease, false, 'missing growth readiness stays protected');
    pet.vm.growthPanel = { canRelease: true }; assert.equal(pet.exposed.canRelease, true);
    pet.vm.selected = 'happy'; assert.equal(pet.exposed.canRelease, false);
    pet.vm.selected = 'idle'; pet.vm.sizeDraft = 120; assert.equal(pet.exposed.canRelease, false);
    pet.vm.sizeDraft = 100; pet.vm.growthPanel.canRelease = false; assert.equal(pet.exposed.canRelease, false);
  } finally { pet.cleanup(); }
});

test('update and sharing release preserve all in-progress work and explicit confirmations', async () => {
  const initial = { revision: 0, currentVersion: '0.0.0-test', configured: true, phase: 'idle', downloaded: 0 };
  const update = await mount('UpdateSettings', { updateDesktopAvailable: true, initialUpdateStatus: () => initial,
    initialUpdatePreferences: () => ({ revision: 0, autoCheck: true, skippedVersion: null, lastCheckAt: null }),
    async getUpdatePreferences() { return { revision: 0, autoCheck: true, skippedVersion: null, lastCheckAt: null }; },
    async onUpdatePreferencesChanged() { return () => {}; },
    watchStarclipUpdates: (receive: (value: unknown) => void) => ({ async read() { receive(initial); }, dispose() {} }),
    async pendingDataDirectory() { return null; },
  });
  try {
    assert.equal(update.exposed.canRelease, false); await flush(); assert.equal(update.exposed.canRelease, true);
    update.vm.installVersion = '0.0.1-test'; assert.equal(update.exposed.canRelease, false);
    update.vm.installVersion = null; update.vm.pendingPath = '/fixed-fixture/pending-directory'; assert.equal(update.exposed.canRelease, false);
    update.vm.pendingPath = null; update.vm.operation = 'download'; assert.equal(update.exposed.canRelease, false);
  } finally { update.cleanup(); }
  const share = await mount('ProductShare');
  try {
    assert.equal(share.exposed.canRelease, true); share.vm.operation = 'image'; assert.equal(share.exposed.canRelease, false);
    share.vm.operation = null; assert.equal(share.exposed.canRelease, true);
  } finally { share.cleanup(); }
});

test('AI record picker queries independent pages and keeps an explicit cross-page selection for review', async () => {
  const state = { enabled: true, available: true, connections: [], audit: [], limits: { maxRecords: 500, maxResults: 50, maxHours: 168 } };
  const make = (id: number) => ({ id, title: `原创授权样例 ${id}`, content: '原创固定正文', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0, scanVersion: 1 });
  const requests: any[] = [];
  const f = await mount('AiSettings', { aiApi: { async state() { return state; } } }, {
    clips: [make(999)], groups: [], revision: 1,
    async loadHistory(query: any) {
      requests.push(query);
      return { clips: [make(query.page === 2 ? 51 : 1)], history: { page: query.page, pageSize: 50, total: 51, counts: { all: 99 }, groupCounts: {}, tags: [], hosts: [], sources: [] } };
    },
  });
  try {
    await flush();
    assert.equal(requests[0].aiEligible, true);
    assert.deepEqual(f.vm.available.map((clip: { id: number }) => clip.id), [1], 'main-window page 999 does not limit the picker');
    f.vm.selectVisible(); await flush();
    f.vm.changeRecordPage(2); await flush();
    assert.deepEqual(f.vm.available.map((clip: { id: number }) => clip.id), [51]);
    f.vm.selectVisible(); await flush();
    assert.deepEqual(f.vm.selected, [1, 51]);
    f.vm.prepare(); await flush();
    assert.deepEqual(f.vm.preview.clipIds, [1, 51]);
    assert.deepEqual(f.vm.previewClips.map((clip: { id: number }) => clip.id), [1, 51]);
    f.vm.search = '新筛选'; await new Promise(resolve => setTimeout(resolve, 180)); await flush();
    assert.equal(requests.at(-1).page, 1); assert.equal(requests.at(-1).query, '新筛选');
    assert.deepEqual(f.vm.selected, [1, 51], 'a search cannot silently broaden or discard an explicit grant');
  } finally { f.cleanup(); }
});

test('AI paging revalidates off-page selections, discards deleted/private IDs and ignores old detail responses', async () => {
  const state = { enabled: true, available: true, connections: [], audit: [], limits: { maxRecords: 500, maxResults: 50, maxHours: 168 } };
  const make = (id: number) => ({ id, title: `原创复核 ${id}`, content: '原来的固定正文', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0, scanVersion: 1 });
  const database = new Map<number, any>([1, 2, 3, 51].map(id => [id, make(id)]));
  let nextDetails: Promise<any[]> | null = null;
  const selectedRequests: number[][] = [];
  const f = await mount('AiSettings', { aiApi: { async state() { return state; } } }, {
    clips: [], groups: [], revision: 1,
    async loadHistory(query: any) { return { clips: (query.page === 2 ? [51] : [1, 2, 3]).flatMap(id => database.has(id) ? [database.get(id)] : []), history: { page: query.page, pageSize: 50, total: 51, counts: { all: 51 }, groupCounts: {}, tags: [], hosts: [], sources: [] } }; },
    async loadClips(ids: number[]) { selectedRequests.push(ids); if (nextDetails) { const result = nextDetails; nextDetails = null; return result; } return ids.flatMap(id => database.has(id) ? [database.get(id)] : []); },
  });
  try {
    await flush(); f.vm.selectVisible(); await flush();
    f.vm.changeRecordPage(2); await flush();
    assert.deepEqual(f.vm.selected, [1, 2, 3]);
    database.set(1, { ...make(1), isPrivate: true }); database.delete(2); database.set(3, { ...make(3), content: '更新后的原创正文' });
    await f.vm.loadRecords(); await flush();
    assert.deepEqual(f.vm.selected, [3]); assert.deepEqual(selectedRequests.at(-1), [1, 2, 3]);
    assert.equal(f.vm.selectedRecords[0].content, '更新后的原创正文');
    await f.vm.prepare(); await flush();
    assert.deepEqual(f.vm.preview.clipIds, [3]); assert.equal(f.vm.previewClips[0].content, '更新后的原创正文');
    nextDetails = Promise.reject(new Error('fixed detail lookup failure'));
    await f.vm.loadRecords(); await flush();
    assert.equal(f.vm.recordReadFailed, true); assert.equal(f.vm.preview, null);
    await f.vm.prepare(); assert.equal(f.vm.preview, null, 'failed validation cannot reuse a stale grant preview');
    await f.vm.loadRecords(); await flush(); assert.equal(f.vm.recordReadFailed, false);
    let resolve!: (clips: any[]) => void;
    nextDetails = new Promise(done => { resolve = done; });
    const stale = f.vm.loadRecords(); await flush();
    assert.equal(f.vm.preview, null, 'an open grant preview disappears as soon as revalidation starts');
    database.delete(3); await f.vm.loadRecords(); await flush();
    resolve([make(3)]); await stale; await flush();
    assert.deepEqual(f.vm.selected, []); assert.deepEqual(f.vm.selectedRecords, [], 'late responses cannot restore an invalidated grant');
  } finally { f.cleanup(); }
});
