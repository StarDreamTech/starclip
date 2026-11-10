import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, markRaw, nextTick } from 'vue';
import type { UpdatePreferences, UpdateStatus } from '../src/update-api.ts';

// Render the actual Vue panel in memory; no WebView, clipboard or user data is touched.
type TestNode = { type: string; text: string; props: Record<string, any>; children: TestNode[]; focus: () => void; open: boolean; showModal: () => void; close: () => void; parent?: TestNode };
function node(type: string, text = ''): TestNode { return markRaw({ type, text, props: {}, children: [], focus() {}, open: false, showModal() { this.open = true; }, close() { this.open = false; } }); }
const renderer = createRenderer<TestNode, TestNode>({
  createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
  setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.text = text; node.children = []; },
  patchProp: (node, key, _previous, value) => { node.props[key] = value; },
  insert: (child, parent, anchor) => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); child.parent = parent; const target = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(target >= 0 ? target : parent.children.length, 0, child); },
  remove: child => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); },
  parentNode: node => node.parent || null, nextSibling: node => node.parent?.children[(node.parent?.children.indexOf(node) ?? -1) + 1] || null,
});
const base: UpdateStatus = { revision: 0, currentVersion: '0.2.0', configured: true, phase: 'idle', downloaded: 0 };
const defaultPreferences: UpdatePreferences = { revision: 0, autoCheck: true, skippedVersion: null, lastCheckAt: null };
function nodes(root: TestNode): TestNode[] { return [root, ...root.children.flatMap(nodes)]; }
function text(root: TestNode): string { return root.type === '#comment' ? '' : root.text + root.children.map(text).join(''); }
function button(root: TestNode, label: string): TestNode { const found = nodes(root).find(item => item.type === 'button' && text(item) === label); assert(found, `missing button: ${label}`); return found; }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
let moduleNumber = 0;
async function mount(mock: Record<string, unknown>, props: Record<string, unknown> = {}, name = 'UpdateSettings') {
  Object.assign(globalThis, { updatePanelMock: mock });
  const api = `const api = globalThis.updatePanelMock; export const { updateDesktopAvailable, initialUpdateStatus, initialUpdatePreferences, getUpdatePreferences, setAutoUpdateCheck, skipUpdateVersion, onUpdatePreferencesChanged, watchStarclipUpdates, pendingDataDirectory, restartAfterUpdate } = api;`;
  const apiUrl = 'data:text/javascript;base64,' + Buffer.from(api + `\n// ${moduleNumber++}`).toString('base64');
  const source = fs.readFileSync(new URL(`../src/components/${name}.vue`, import.meta.url), 'utf8');
  const { descriptor } = parse(source);
  const script = compileScript(descriptor, { id: 'update-panel-test', inlineTemplate: true });
  const code = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText
    .replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replaceAll("'../update-api'", `'${apiUrl}'`)
    .replaceAll("'../update-state'", `'${new URL('../src/update-state.ts', import.meta.url).href}'`).replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`).replaceAll("'./i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  const root = node('root'); const app = renderer.createApp(component.default, props); app.mount(root); await flush();
  return { root, app };
}
function nativeMock(initial: UpdateStatus = base) {
  let receive: (state: UpdateStatus) => void = () => {};
  let state = { ...initial };
  let preferences = { ...defaultPreferences };
  let receivePreferences: (value: UpdatePreferences) => void = () => {};
  const preferenceCalls: string[] = [];
  const calls: string[] = [];
  const mock = {
    calls, preferenceCalls, updateDesktopAvailable: true, initialUpdateStatus: () => base,
    initialUpdatePreferences: () => ({ ...defaultPreferences }),
    async getUpdatePreferences() { return { ...preferences }; },
    async onUpdatePreferencesChanged(callback: typeof receivePreferences) { receivePreferences = callback; return () => { preferenceCalls.push('dispose'); }; },
    sendPreferences(next: UpdatePreferences) { preferences = next; receivePreferences(next); },
    async setAutoUpdateCheck(enabled: boolean) { preferenceCalls.push(`automatic:${enabled}`); preferences = { ...preferences, revision: preferences.revision + 1, autoCheck: enabled }; return { ...preferences }; },
    async skipUpdateVersion(version: string) { preferenceCalls.push(`skip:${version}`); preferences = { ...preferences, revision: preferences.revision + 1, skippedVersion: version }; return { ...preferences }; },
    send(next: UpdateStatus) { state = next; receive(next); },
    async downloadUpdate(version: string) { calls.push(`download:${version}`); mock.send({ ...base, revision: 3, phase: 'ready', available: { version }, downloaded: 100, total: 100 }); },
    async installUpdate(version: string) { calls.push(`install:${version}`); mock.send({ ...base, revision: 4, phase: 'installed' }); },
    watchStarclipUpdates(callback: typeof receive) {
      receive = callback;
      return {
        async read() { calls.push('read'); receive(state); },
        async check() { calls.push('check'); mock.send({ ...base, revision: 1, phase: 'up-to-date' }); },
        download: (version: string) => mock.downloadUpdate(version),
        install: (version: string) => mock.installUpdate(version),
        dispose() { calls.push('dispose'); },
      };
    },
    async pendingDataDirectory(): Promise<string | null> { return null; },
    async restartAfterUpdate() { calls.push('restart'); },
  };
  return mock;
}

test('download opens a ready dialog without installing; later preserves the package and confirmation installs then restarts', async () => {
  const mock = nativeMock(); const busy: boolean[] = [];
  const { root, app } = await mount(mock, { onBusy: (value: boolean) => busy.push(value) });
  assert.deepEqual(mock.calls, ['read']);
  await button(root, '检查更新').props.onClick(); await flush();
  assert.match(text(root), /已检查，当前没有更新/);
  mock.send({ ...base, revision: 2, phase: 'available', available: { version: '0.3.0', notes: '<img src=x onerror=alert(1)>' } }); await flush();
  assert.match(text(root), /<img src=x onerror=alert\(1\)>/); assert.equal(nodes(root).some(item => item.type === 'img'), false);
  await button(root, '下载更新').props.onClick(); await flush();
  assert.match(text(root), /新版本 0.3.0 已准备就绪/);
  assert(mock.calls.includes('download:0.3.0')); assert(!mock.calls.some(call => call.startsWith('install:')));
  assert.equal(busy.at(-1), false, 'downloaded package does not freeze editing');
  let dialog = nodes(root).find(item => item.type === 'dialog')!; assert.equal(dialog.open, true);
  await button(dialog, '稍后再说').props.onClick(); await flush();
  assert.equal(dialog.open, false); assert(!nodes(root).some(item => item.type === 'dialog'));
  mock.send({ ...base, revision: 4, phase: 'ready', available: { version: '0.3.0' }, downloaded: 100, total: 100 }); await flush();
  assert(!nodes(root).some(item => item.type === 'dialog'), 'repeated ready notifications do not undo the later choice');
  assert.match(text(root), /已下载并验证，等待安装/);
  await button(root, '安装并重启').props.onClick(); await flush();
  dialog = nodes(root).find(item => item.type === 'dialog')!;
  await button(dialog, '安装并重启').props.onClick(); await flush();
  assert(mock.calls.includes('install:0.3.0'), 'installation uses the version the user confirmed');
  assert.equal(busy.at(-1), true); assert.equal(button(root, '刷新状态').props.disabled, true);
  assert.deepEqual(mock.calls.slice(-2), ['install:0.3.0', 'restart'], 'confirmed installation restarts automatically');
  app.unmount(); assert.equal(busy.at(-1), false); assert(mock.calls.includes('dispose'));
});

test('updates expose no recovery controls or calls and pending directory changes still require restart', async () => {
  const mock = nativeMock();
  const forbidden = () => { mock.calls.push('unexpected-recovery-call'); throw new Error('Recovery controls must remain internal'); };
  const { root, app } = await mount({ ...mock, listRecoveryPoints: forbidden, createRecoveryPoint: forbidden, restoreRecoveryPoint: forbidden });
  assert.doesNotMatch(text(root), /恢复点|恢复此时数据|保存当前恢复点/);
  assert.match(text(root), /软件更新/);
  assert.deepEqual(mock.calls, ['read']);
  app.unmount();
  const pending = nativeMock(); pending.pendingDataDirectory = async () => '/test-only/pending-directory';
  const busy: boolean[] = [];
  const mounted = await mount(pending, { onBusy: (value: boolean) => busy.push(value) });
  assert.equal(busy.at(-1), true);
  assert.match(text(mounted.root), /数据位置已变更，重启后继续/);
  assert.equal(button(mounted.root, '刷新状态').props.disabled, true);
  await button(mounted.root, '重启星笺').props.onClick(); await flush();
  assert(pending.calls.includes('restart'));
  mounted.app.unmount();
});

test('ready state survives reopening settings; Escape and backdrop dismiss only its dialog; unsaved preferences block installation', async () => {
  const mock = nativeMock({ ...base, phase: 'ready', available: { version: '0.3.0' }, downloaded: 100, total: 100 });
  const { root, app } = await mount(mock, { unsavedChanges: true });
  let dialog = nodes(root).find(item => item.type === 'dialog')!; assert.equal(dialog.open, true);
  assert.equal(button(dialog, '安装并重启').props.disabled, true);
  await button(dialog, '安装并重启').props.onClick(); await flush();
  assert(!mock.calls.some(call => call.startsWith('install:')));
  assert.match(text(dialog), /尚未保存的偏好/);
  let stopped = 0, prevented = 0;
  dialog.props.onCancel({ preventDefault() { prevented++; }, stopPropagation() { stopped++; } }); await flush();
  assert.equal(stopped, 1); assert.equal(prevented, 1); assert.equal(dialog.open, false);
  await button(root, '安装并重启').props.onClick(); await flush();
  dialog = nodes(root).find(item => item.type === 'dialog')!;
  dialog.props.onClick({ target: dialog }); await flush();
  assert.equal(dialog.open, false); assert.match(text(root), /等待安装/);
  app.unmount();
});

test('changing the prepared version invalidates the open confirmation and stale clicks cannot install', async () => {
  const mock = nativeMock({ ...base, phase: 'ready', available: { version: '0.3.0' } });
  const { root, app } = await mount(mock);
  const dialog = nodes(root).find(item => item.type === 'dialog')!;
  const staleInstall = button(dialog, '安装并重启').props.onClick;
  mock.send({ ...base, revision: 3, phase: 'available', available: { version: '0.4.0' } }); await flush();
  assert.equal(dialog.open, false); assert(!nodes(root).some(item => item.type === 'dialog'));
  await staleInstall(); await flush();
  assert(!mock.calls.some(call => call.startsWith('install:')));
  assert(button(root, '下载更新')); app.unmount();
});

test('download or installation failure offers a new download and never restarts', async () => {
  const available: UpdateStatus = { ...base, phase: 'available', available: { version: '0.3.0' } };
  const mock = nativeMock(available); let failDownload = true;
  const successfulDownload = mock.downloadUpdate;
  mock.downloadUpdate = async version => {
    if (!failDownload) return successfulDownload(version);
    mock.send({ ...available, revision: 2, phase: 'error', error: '固定测试下载失败' });
    throw new Error('固定测试下载失败');
  };
  mock.installUpdate = async () => {
    mock.send({ ...available, revision: 4, phase: 'error', error: '固定测试备份失败' });
    throw new Error('固定测试备份失败');
  };
  const { root, app } = await mount(mock);
  await button(root, '下载更新').props.onClick(); await flush();
  assert.match(text(root), /固定测试下载失败/); assert(!nodes(root).some(item => item.type === 'dialog'));
  failDownload = false;
  await button(root, '重新下载更新').props.onClick(); await flush();
  const dialog = nodes(root).find(item => item.type === 'dialog')!;
  await button(dialog, '安装并重启').props.onClick(); await flush();
  assert.match(text(root), /固定测试备份失败/); assert(button(root, '重新下载更新'));
  assert(!mock.calls.includes('restart')); app.unmount();
});

test('lost install response recovers installed state; automatic restart failure stays frozen and can retry restart', async () => {
  const mock = nativeMock({ ...base, phase: 'ready', available: { version: '0.3.0' } }); const busy: boolean[] = [];
  mock.installUpdate = async version => { mock.calls.push(`install:${version}`); mock.send({ ...base, revision: 4, phase: 'installed' }); throw new Error('固定测试响应中断'); };
  let restartFails = true;
  mock.restartAfterUpdate = async () => { mock.calls.push('restart'); if (restartFails) throw new Error('固定测试重启失败'); };
  const { root, app } = await mount(mock, { onBusy: (value: boolean) => busy.push(value) });
  const dialog = nodes(root).find(item => item.type === 'dialog')!;
  await button(dialog, '安装并重启').props.onClick(); await flush();
  assert.match(text(root), /自动重启未完成.*再次点击/); assert.match(text(root), /固定测试重启失败/);
  assert.equal(busy.at(-1), true); assert.equal(button(root, '刷新状态').props.disabled, true);
  restartFails = false;
  await button(root, '重启星笺').props.onClick(); await flush();
  assert.equal(mock.calls.filter(call => call === 'restart').length, 2);
  assert.equal(mock.calls.filter(call => call.startsWith('install:')).length, 1);
  app.unmount();
});

test('browser preview is explicit and never reads native state', async () => {
  const mock = nativeMock(); mock.updateDesktopAvailable = false;
  const { root, app } = await mount(mock);
  assert.match(text(root), /浏览器演示没有已安装的软件/);
  assert.equal(nodes(root).filter(item => item.type === 'button').length, 0);
  assert.deepEqual(mock.calls, []); app.unmount();
});


test('installer launch failure keeps editing frozen and requests restart without claiming success', async () => {
  const mock = nativeMock(); const busy: boolean[] = [];
  const { root, app } = await mount(mock, { onBusy: (value: boolean) => busy.push(value) });
  mock.send({ ...base, revision: 4, phase: 'restart-required', error: '固定测试：安装器未启动，请先重启' }); await flush();
  assert.equal(busy.at(-1), true); assert.match(text(root), /安装未完成，请先重启星笺/);
  assert.doesNotMatch(text(root), /新版已安装/); assert.equal(button(root, '刷新状态').props.disabled, true);
  await button(root, '重启星笺').props.onClick(); await flush(); assert(mock.calls.includes('restart'));
  app.unmount();
});

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function automaticSwitch(root: TestNode) { return nodes(root).find(item => item.props['aria-label'] === '自动检查稳定版更新')!; }

test('automatic preference saves immediately, preserves confirmed value on failure and accepts only current revisions', async () => {
  const mock = nativeMock(); const busy: boolean[] = [];
  const saving = deferred<UpdatePreferences>();
  mock.setAutoUpdateCheck = async enabled => { mock.preferenceCalls.push(`automatic:${enabled}`); return saving.promise; };
  const { root, app } = await mount(mock, { onBusy: value => busy.push(value) });
  assert.equal(automaticSwitch(root).props['aria-checked'], true);
  const toggle = automaticSwitch(root).props.onClick(); await flush();
  assert.equal(busy.at(-1), true); assert.equal(automaticSwitch(root).props.disabled, true);
  assert.deepEqual(mock.preferenceCalls, ['automatic:false']);
  assert.equal(automaticSwitch(root).props['aria-checked'], true, 'switch waits for native confirmation');
  await automaticSwitch(root).props.onClick(); assert.equal(mock.preferenceCalls.length, 1);
  saving.reject(new Error('固定测试：保存失败')); await toggle; await flush();
  assert.equal(busy.at(-1), false); assert.equal(automaticSwitch(root).props['aria-checked'], true);
  assert.match(text(root), /保存失败/);
  const delayed = deferred<UpdatePreferences>(); mock.getUpdatePreferences = () => delayed.promise;
  const refreshing = button(root, '刷新状态').props.onClick(); await flush();
  mock.sendPreferences({ ...defaultPreferences, revision: 5, autoCheck: false });
  delayed.resolve({ ...defaultPreferences, revision: 2 }); await refreshing; await flush();
  assert.equal(automaticSwitch(root).props['aria-checked'], false, 'late read cannot undo an event');
  app.unmount(); assert.deepEqual(mock.preferenceCalls.slice(-1), ['dispose']);
});

test('late preference listeners are released after either settings or notice unmounts', async () => {
  for (const name of ['UpdateSettings', 'UpdateNotice']) {
    const mock = nativeMock(); const listening = deferred<() => void>(); let stops = 0;
    mock.onUpdatePreferencesChanged = () => listening.promise;
    const { app } = await mount(mock, {}, name); app.unmount();
    listening.resolve(() => { stops++; }); await flush();
    assert.equal(stops, 1, name); assert.equal(mock.calls.includes('read'), false, 'no read after disposal');
  }
});

test('notice only reads local status, skips one version, shows a later version and never auto-installs', async () => {
  const available: UpdateStatus = { ...base, revision: 1, phase: 'available', available: { version: '0.3.0' } };
  const mock = nativeMock(available); let opened = 0; const busy: boolean[] = [];
  const { root, app } = await mount(mock, { onOpenSettings: () => opened++, onBusy: value => busy.push(value) }, 'UpdateNotice');
  assert.match(text(root), /稳定版 0.3.0 可更新/); assert.deepEqual(mock.calls, ['read']);
  await button(root, '查看更新').props.onClick(); assert.equal(opened, 1); assert.deepEqual(mock.calls, ['read']);
  await button(root, '忽略此版本').props.onClick(); await flush();
  assert.deepEqual(mock.preferenceCalls, ['skip:0.3.0']); assert.equal(nodes(root).some(node => node.type === 'aside'), false);
  assert.deepEqual(busy, [true, false]);
  mock.send({ ...available, revision: 2, available: { version: '0.4.0' } }); await flush();
  assert.match(text(root), /稳定版 0.4.0 可更新/);
  mock.sendPreferences({ ...defaultPreferences, revision: 0 }); await flush();
  mock.send({ ...available, revision: 1 }); await flush(); assert.match(text(root), /0.4.0/, 'old states cannot restore skipped version');
  mock.sendPreferences({ ...defaultPreferences, revision: 3, autoCheck: false }); await flush();
  assert.equal(nodes(root).some(node => node.type === 'aside'), false);
  app.unmount(); assert(mock.calls.includes('dispose')); assert(mock.preferenceCalls.includes('dispose'));
});

test('notice skip failures remain visible and a delayed skip cannot suppress a newer candidate', async () => {
  const available: UpdateStatus = { ...base, revision: 1, phase: 'available', available: { version: '0.3.0' } };
  const mock = nativeMock(available); mock.skipUpdateVersion = async () => { throw new Error('fixture failure'); };
  const { root, app } = await mount(mock, {}, 'UpdateNotice');
  await button(root, '忽略此版本').props.onClick(); await flush(); assert.match(text(root), /暂时无法隐藏提醒/);
  const skip = deferred<UpdatePreferences>(); mock.skipUpdateVersion = () => skip.promise;
  const pending = button(root, '忽略此版本').props.onClick(); await flush();
  mock.send({ ...available, revision: 2, available: { version: '0.4.0' } });
  skip.resolve({ ...defaultPreferences, revision: 2, skippedVersion: '0.3.0' }); await pending; await flush();
  assert.match(text(root), /稳定版 0.4.0/); assert.doesNotMatch(text(root), /暂时无法隐藏提醒/); app.unmount();
});

test('background errors, blocked UI and browser mode never show an automatic prompt', async () => {
  const failed = nativeMock({ ...base, revision: 1, phase: 'error', error: 'fixed offline fixture' });
  const errorView = await mount(failed, {}, 'UpdateNotice'); assert.equal(nodes(errorView.root).some(node => node.type === 'aside'), false); errorView.app.unmount();
  const available: UpdateStatus = { ...base, revision: 1, phase: 'available', available: { version: '0.3.0' } };
  const blocked = await mount(nativeMock(available), { blocked: true }, 'UpdateNotice'); assert.equal(nodes(blocked.root).some(node => node.type === 'aside'), false); blocked.app.unmount();
  const browserMock = nativeMock(available); browserMock.updateDesktopAvailable = false;
  const browser = await mount(browserMock, {}, 'UpdateNotice'); assert.deepEqual(browserMock.calls, []); assert.deepEqual(browserMock.preferenceCalls, []); browser.app.unmount();
});
