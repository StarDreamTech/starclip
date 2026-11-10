import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import { defaultAppearance } from '../src/appearance.ts';
import type { AppState, HistoryQuery, Settings } from '../src/types.ts';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
function moduleUrl(source: string) {
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  return 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
}
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
function globals(values: Record<string, unknown>) {
  const saved = Object.keys(values).map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  for (const [key, value] of Object.entries(values)) Object.defineProperty(globalThis, key, { value, configurable: true, writable: true });
  return () => { for (const [key, descriptor] of saved) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); } };
}
function eventHub() {
  const listeners = new Map<string, Set<Function>>();
  return {
    addEventListener(name: string, callback: Function) { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name)!.add(callback); },
    removeEventListener(name: string, callback: Function) { listeners.get(name)?.delete(callback); },
    count() { return [...listeners.values()].reduce((sum, callbacks) => sum + callbacks.size, 0); },
  };
}

// Exercise the actual App setup, reactive guards and lifecycle. Its unrelated
// template/children are omitted; API responses are fixed in-memory fixtures.
const renderer = createRenderer<object, object>({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  setText() {}, setElementText() {}, patchProp() {}, insert() {}, remove() {},
  parentNode: () => null, nextSibling: () => null,
});
let sequence = 0;
test('image checkboxes reflect selection, reject overflow and never invoke single-copy shortcuts', async () => {
  const fixture = await mount();
  try {
    const vm = fixture.vm;
    const clip = { id: 91, kind: 'image', title: 'Original image check' };
    vm.imageSelectMode = true;
    vm.state.clips = [clip];
    const input = { checked: false };
    vm.checkImage(clip, { shiftKey: false, currentTarget: input });
    assert.equal(input.checked, true);
    assert.deepEqual([...vm.imageSelection], [91]);
    vm.checkImage(clip, { shiftKey: false, currentTarget: input });
    assert.equal(input.checked, false);
    vm.imageSelection = Array.from({ length: 50 }, (_, i) => i + 1);
    input.checked = true;
    vm.checkImage(clip, { shiftKey: false, currentTarget: input });
    assert.equal(input.checked, false);
    assert.equal(vm.imageSelection.length, 50);
    await vm.copy(clip);
    assert.deepEqual(fixture.copiedIds, []);
  } finally { fixture.cleanup(); }
});

test('shortcut status distinguishes confirmed binding, unsaved draft, saving and a rejected registration', async () => {
  const fixture = await mount(), { vm, mock } = fixture;
  let rejectSave!: (reason: Error) => void;
  const saving = new Promise<void>((_resolve, reject) => { rejectSave = reject; });
  (mock as any).saveSettings = () => saving;
  try {
    vm.openSettings();
    const previous = vm.state.settings.shortcut;
    assert.equal(vm.shortcutStatus.kind, 'active');
    vm.settingsDraft.shortcut = 'CommandOrControl+Shift+L';
    assert.equal(vm.shortcutStatus.kind, 'pending', 'an unchecked draft cannot show a green check');
    const save = vm.saveSettings(); await flush();
    assert.equal(vm.shortcutStatus.kind, 'checking');
    // The native command rejects before saving when OS registration fails.
    rejectSave(new Error('该快捷键无法注册，可能被其他应用占用。'));
    await save;
    assert.equal(vm.shortcutStatus.kind, 'error');
    assert.match(vm.shortcutStatus.label, /占用|already use/);
    assert.equal(vm.state.settings.shortcut, previous);
    assert.equal(vm.settingsDraft.shortcut, 'CommandOrControl+Shift+L');
    vm.settingsDraft.shortcut = 'CommandOrControl+Shift+K';
    assert.equal(vm.shortcutStatus.kind, 'pending', 'editing clears only the previous candidate error');
    vm.settingsDraft.shortcut = previous;
    assert.equal(vm.shortcutStatus.kind, 'active');
    vm.state.shortcutError = '快捷键注册失败，可在偏好设置中更换。';
    assert.equal(vm.shortcutStatus.kind, 'error');
  } finally { fixture.cleanup(); }
});

test('shortcut status is never confirmed before the native state has loaded', async () => {
  const fixture = await mount({ delayedRead: true });
  try {
    assert.equal(fixture.vm.shortcutStatus.kind, 'checking');
    fixture.vm.loading = false;
    assert.equal(fixture.vm.shortcutStatus.kind, 'unknown');
    fixture.finishRead(); await flush();
    assert.equal(fixture.vm.shortcutStatus.kind, 'active');
  } finally { fixture.cleanup(); }
});

test('expanded images retain the detail dialog, suppress copy keys and close when privacy or selection changes', async () => {
  const fixture = await mount(), { vm } = fixture;
  const image = { id: 7, kind: 'image', title: 'Original viewer fixture', image: { width: 100, height: 80, bytes: 200 }, content: '' };
  try {
    vm.openClip(image); vm.openImageViewer(); assert.equal(vm.imageViewerPosition, 0);
    vm.keyboard({ key: 'Enter' }); assert.deepEqual(fixture.copiedIds, []);
    vm.closeModal(); assert.equal(vm.imageViewerPosition, null); assert.equal(vm.modal, 'clip');
    vm.openClip({ ...image, isPrivate: true }); vm.openImageViewer(); assert.equal(vm.imageViewerPosition, null);
    vm.privateRevealed = true; vm.openImageViewer(); assert.equal(vm.imageViewerPosition, 0);
    vm.privateRevealed = false; assert.equal(vm.imageViewerPosition, null);
    vm.openClip({ ...image, id: 8, kind: 'bundle', images: [image.image, image.image, image.image] });
    vm.openImageViewer(2); assert.equal(vm.imageViewerPosition, 2);
    vm.openClip(image); assert.equal(vm.imageViewerPosition, null);
    vm.clipDetailsLoading = true; vm.openImageViewer(); assert.equal(vm.imageViewerPosition, null);
  } finally { fixture.cleanup(); }
});

test('App restores the last category, group, page and open settings tab after reconstruction', async () => {
  const cache = new Map<string, string>();
  const state = {
    groups: [{ id: 8, name: '原创导航分组', color: '#8b93d9' }],
    history: { page: 1, pageSize: 50, total: 150, counts: { all: 150 }, groupCounts: { 8: 150 }, sources: [], hosts: [], tags: [] },
  };
  for (const filter of ['pinned', 'kept', 'image', 'link', 'file', 'code', 'group-8']) {
    const first = await mount({ cache, state });
    try {
      first.vm.filter = filter;
      await first.vm.refresh();
      first.vm.changeHistoryPage(3); await flush();
      assert.equal(first.vm.historyPage, 3);
    } finally { first.cleanup(); }
    const reopened = await mount({ cache, state });
    try {
      assert.equal(reopened.vm.filter, filter);
      assert.equal(reopened.vm.historyPage, 3);
      assert.equal(reopened.requests[0]!.filter, filter, 'the first read already uses the restored category');
      assert.equal(reopened.requests[0]!.page, 3);
      assert.equal(reopened.vm.modal, '');
    } finally { reopened.cleanup(); }
  }
  const settings = await mount({ cache, state });
  try {
    settings.vm.openSettings(); settings.vm.changeSettingsTab('data'); await flush();
  } finally { settings.cleanup(); }
  const reopenedSettings = await mount({ cache, state });
  try {
    assert.equal(reopenedSettings.vm.modal, 'settings');
    assert.equal(reopenedSettings.vm.settingsTab, 'data');
    assert.equal(reopenedSettings.vm.dataTabVisited, true);
    assert.deepEqual({ ...reopenedSettings.vm.settingsDraft }, { ...reopenedSettings.vm.state.settings });
    reopenedSettings.vm.closeModal(); await flush();
  } finally { reopenedSettings.cleanup(); }
  const closedSettings = await mount({ cache, state });
  try {
    assert.equal(closedSettings.vm.modal, '');
    closedSettings.vm.openSettings(); assert.equal(closedSettings.vm.settingsTab, 'data');
  } finally { closedSettings.cleanup(); }
  const opening = await mount({ cache, state: { settings: { ...(await settings.mock.getState()).settings, showStartup: true } } });
  try {
    assert.equal(opening.vm.startupVisible, true);
    assert.equal(opening.vm.modal, '', 'settings restoration waits for the startup animation');
    assert.equal(JSON.parse(cache.get('starclip.navigation.v1')!).settingsOpen, true);
    opening.vm.finishStartup(); await flush();
    assert.equal(opening.vm.modal, 'settings'); assert.equal(opening.vm.settingsTab, 'data');
  } finally { opening.cleanup(); }
  const firstUse = await mount({ cache, state: { onboardingCompleted: false } });
  try {
    assert.equal(firstUse.vm.guideVisible, true);
    assert.equal(firstUse.vm.modal, '', 'first-use onboarding takes precedence over an old settings location');
  } finally { firstUse.cleanup(); }
});

test('restored navigation handles missing groups, reduced pages and temporary searches', async () => {
  const key = 'starclip.navigation.v1';
  const cache = new Map([[key, JSON.stringify({ filter: 'group-8', page: 3, settingsTab: 'appearance', settingsOpen: false })]]);
  const history = { page: 1, pageSize: 50, total: 60, counts: { all: 60 }, groupCounts: {}, sources: [], hosts: [], tags: [] };
  const missing = await mount({ cache, state: { history } });
  try {
    assert.equal(missing.vm.filter, 'all'); assert.equal(missing.vm.historyPage, 1);
    assert.deepEqual(missing.requests.map(request => [request.filter, request.page]), [['group-8', 3], ['all', 1]]);
    assert.equal(JSON.parse(cache.get(key)!).filter, 'all');
  } finally { missing.cleanup(); }
  cache.set(key, JSON.stringify({ filter: 'image', page: 3, settingsTab: 'appearance', settingsOpen: false }));
  const reduced = await mount({ cache, state: { history } });
  try {
    assert.equal(reduced.vm.historyPage, 2);
    assert.equal(JSON.parse(cache.get(key)!).page, 2, 'the backend-clamped page replaces the cached page');
    reduced.vm.query = '原创临时搜索'; await flush();
    reduced.vm.historyLoading = false;
    reduced.vm.changeHistoryPage(2); await flush();
    reduced.vm.openClip(); reduced.vm.clipContent = '原创未保存正文';
    assert.deepEqual(JSON.parse(cache.get(key)!), { filter: 'image', page: 1, settingsTab: 'appearance', settingsOpen: false });
  } finally { reduced.cleanup(); }
  const clean = await mount({ cache, state: { history } });
  try {
    assert.equal(clean.vm.filter, 'image'); assert.equal(clean.vm.historyPage, 1);
    assert.equal(clean.vm.query, ''); assert.equal(clean.vm.modal, ''); assert.equal(clean.vm.clipContent, '');
  } finally { clean.cleanup(); }
});

async function mount(options: { delayedRead?: boolean; delayedCloseListener?: boolean; initialCloseRequest?: number; delayedCloseQuery?: boolean; cache?: Map<string, string>; state?: Partial<AppState> } = {}) {
  const state: AppState = {
    onboardingCompleted: true, clips: [], groups: [],
    settings: { paused: true, pauseRevision: 0, uiLanguage: 'system', theme: 'dark', layout: 'grid', maxHistory: 0, retentionDays: 0, alwaysOnTop: false, showStartup: false, stickerPack: 'none', shortcut: 'CommandOrControl+Shift+V' },
    clipboardError: null, shortcutError: null, windowError: null,
    ...options.state,
  };
  const cache = options.cache ?? new Map<string, string>();
  const requests: HistoryQuery[] = [];
  const windowEvents = eventHub(), themeEvents = eventHub();
  const read = deferred<AppState>(), listening = deferred<() => void>();
  const closeQuery = deferred<number | null>();
  const calls: { release: boolean; requestId?: number }[] = [];
  const focusCalls: boolean[] = [], copiedIds: number[] = [];
  let closeRequest: ((requestId: number) => void) | undefined;
  const stopped = { change: 0, close: 0, growth: 0, quit: 0 };
  const stopClose = () => { stopped.close++; };
  const mock = {
    async getState(query?: HistoryQuery) {
      if (query) requests.push(structuredClone(query));
      if (options.delayedRead) return read.promise;
      const next = structuredClone(state);
      if (next.history && query) next.history.page = Math.min(query.page, Math.max(1, Math.ceil(next.history.total / next.history.pageSize)));
      return next;
    },
    async setNativeLanguage() {},
    async privacyRules() { return { revision: 1, rules: [] }; },
    async onChange() { return () => { stopped.change++; }; },
    async hideWindow(release: boolean, requestId?: number, returnFocus = false) { calls.push({ release, requestId }); focusCalls.push(returnFocus); return { requestId: requestId ?? 100, pending: !release }; },
    async copyClip(id: number) { copiedIds.push(id); },
    async getWindowCloseRequest() { return options.delayedCloseQuery ? closeQuery.promise : options.initialCloseRequest ?? null; },
    async onWindowCloseRequested(callback: (requestId: number) => void) { closeRequest = callback; return options.delayedCloseListener ? listening.promise : stopClose; },
    async setTheme(theme: string) { return { ...state.settings, theme }; },
  };
  const restore = globals({
    window: { ...windowEvents, matchMedia: () => ({ ...themeEvents, matches: false }) },
    document: { documentElement: { dataset: {}, style: { setProperty() {}, removeProperty() {} } } },
    navigator: { platform: 'MacIntel' },
    localStorage: { getItem: (key: string) => cache.get(key) ?? null, setItem: (key: string, value: string) => cache.set(key, value) },
    windowReleaseMock: {
      api: mock, getAppearance: async () => defaultAppearance(),
      recordStudioOpened() {}, async quitStarclip() { stopped.quit++; },
      async listen(name: string) { assert.equal(name, 'starpet-growth-error'); return () => { stopped.growth++; }; },
    },
  });
  const mockUrl = moduleUrl(`export const { api, getAppearance, listen, recordStudioOpened, quitStarclip } = globalThis.windowReleaseMock; export const isDesktop = true; export const appVersion = '0.0.0-test'; // ${sequence++}`);
  const stubUrl = moduleUrl('export default { render() { return null; } };');
  const { descriptor } = parse(fs.readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'window-release-test' });
  const source = script.content.replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replace(/(['"])\.\/components\/[^'"]+\.vue\1/g, `'${stubUrl}'`)
    .replaceAll("'./api'", `'${mockUrl}'`).replaceAll("'./appearance-api'", `'${mockUrl}'`).replaceAll("'./update-api'", `'${mockUrl}'`)
    .replaceAll("'./startup-api'", `'${mockUrl}'`)
    .replaceAll("'./telemetry-api'", `'${mockUrl}'`)
    .replaceAll("'@tauri-apps/api/event'", `'${mockUrl}'`)
    .replaceAll("'./appearance'", `'${new URL('../src/appearance.ts', import.meta.url).href}'`)
    .replaceAll("'./privacy'", `'${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replaceAll("'./clip-selection'", `'${new URL('../src/clip-selection.ts', import.meta.url).href}'`)
    .replaceAll("'./navigation'", `'${new URL('../src/navigation.ts', import.meta.url).href}'`)
    .replaceAll("'./theme'", `'${new URL('../src/theme.ts', import.meta.url).href}'`).replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`).replaceAll("'./i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = await import(moduleUrl(source));
  const app = renderer.createApp({ ...component.default, render() { return null; } });
  app.mount({}); await flush();
  const vm = app._instance!.setupState as Record<string, any>;
  let unmounted = false;
  const unmount = () => { if (!unmounted) { unmounted = true; app.unmount(); } };
  return {
    vm, mock, calls, focusCalls, copiedIds, stopped, windowEvents, themeEvents, unmount, requests,
    finishRead: () => read.resolve(structuredClone(state)), finishListener: () => listening.resolve(stopClose),
    finishCloseQuery: (requestId: number | null) => closeQuery.resolve(requestId),
    async close(requestId: number) { assert(closeRequest, 'native close handler registered'); closeRequest(requestId); await flush(); },
    cleanup() { unmount(); restore(); },
  };
}

test('idle App permits release while editing, settings and studio keep their in-memory drafts', async () => {
  const fixture = await mount(); const { vm, calls } = fixture;
  try {
    await fixture.close(11); assert.deepEqual(calls.pop(), { release: true, requestId: 11 });
    vm.openClip(); vm.clipContent = '原创未保存草稿';
    await fixture.close(12); assert.deepEqual(calls.pop(), { release: false, requestId: 12 });
    assert.equal(vm.clipContent, '原创未保存草稿'); assert.equal(vm.modal, 'clip');
    vm.closeModal(); vm.openSettings();
    assert.equal(vm.settingsDirty, false);
    await fixture.close(13); assert.deepEqual(calls.pop(), { release: false, requestId: 13 }, 'a settings child can own a draft before the parent is dirty');
    vm.settingsDraft.maxHistory = 500;
    assert.equal(vm.settingsDirty, true);
    await fixture.close(13); assert.deepEqual(calls.pop(), { release: false, requestId: 13 });
    assert.equal(vm.settingsDraft.maxHistory, 500); assert.equal(vm.modal, 'settings');
    vm.closeModal(); vm.openStudio();
    await fixture.close(14); assert.deepEqual(calls.pop(), { release: false, requestId: 14 }); assert.equal(vm.studioOpen, true);
    vm.studioOpen = false;
    vm.keyboard({ key: 'Escape', target: { matches: () => false, closest: () => null } }); await flush();
    assert.deepEqual(calls.pop(), { release: true, requestId: undefined }, 'Escape uses the same release guard');
  } finally { fixture.cleanup(); }
});

test('loading and active work defer release until they settle, including every child busy flag', async () => {
  const fixture = await mount({ delayedRead: true }); const { vm, mock, calls } = fixture;
  try {
    await vm.hideMainWindow(20); assert.deepEqual(calls.pop(), { release: false, requestId: 20 });
    fixture.finishRead(); await flush();
    const operation = deferred<void>();
    const running = vm.run(() => operation.promise); await flush();
    await fixture.close(21); assert.deepEqual(calls.pop(), { release: false, requestId: 21 });
    operation.resolve(); await running;
    const theme = deferred<any>(); mock.setTheme = () => theme.promise;
    const savingTheme = vm.changeTheme('forest'); await flush();
    await fixture.close(22); assert.deepEqual(calls.pop(), { release: false, requestId: 22 });
    theme.resolve({ ...vm.state.settings, theme: 'forest' }); await savingTheme;
    for (const flag of ['startupBusy', 'dataOperationBusy', 'updateBusy', 'shareBusy', 'appearanceBusy', 'privacyBusy', 'telemetryBusy', 'aiBusy', 'clipPrivacyBusy']) {
      vm[flag] = true; assert.equal(vm.dataBusy, true, flag);
      await fixture.close(23); assert.deepEqual(calls.pop(), { release: false, requestId: 23 }, flag);
      vm[flag] = false;
    }
    for (const flag of ['cleanupOpen', 'guideVisible', 'guideBusy', 'updateNoticeBusy']) {
      vm[flag] = true;
      await fixture.close(24); assert.deepEqual(calls.pop(), { release: false, requestId: 24 }, flag);
      vm[flag] = false;
    }
    await fixture.close(25); assert.deepEqual(calls.pop(), { release: true, requestId: 25 });
  } finally { fixture.cleanup(); }
});

test('close events during initial loading and before subscription are released once startup is ready', async () => {
  for (const initialCloseRequest of [undefined, 51]) {
    const fixture = await mount({ delayedRead: true, initialCloseRequest });
    try {
      if (initialCloseRequest === undefined) await fixture.close(51);
      assert.deepEqual(fixture.calls, [{ release: false, requestId: 51 }]);
      fixture.finishRead(); await flush();
      assert.deepEqual(fixture.calls, [{ release: false, requestId: 51 }, { release: true, requestId: 51 }]);
    } finally { fixture.cleanup(); }
  }
});

test('a delayed initial close query cannot replace a newer live close request', async () => {
  const fixture = await mount({ delayedCloseQuery: true });
  try {
    await fixture.close(62);
    fixture.finishCloseQuery(61); await flush();
    assert.deepEqual(fixture.calls, [{ release: false, requestId: 62 }, { release: true, requestId: 62 }]);
  } finally { fixture.cleanup(); }
});

test('closing during work automatically retries after work settles without another close', async () => {
  const fixture = await mount(), operation = deferred<void>();
  try {
    const running = fixture.vm.run(() => operation.promise); await flush();
    await fixture.close(70);
    assert.deepEqual(fixture.calls, [{ release: false, requestId: 70 }]);
    operation.resolve(); await running; await flush();
    assert.deepEqual(fixture.calls, [{ release: false, requestId: 70 }, { release: true, requestId: 70 }]);
  } finally { fixture.cleanup(); }
});

test('a late native reply cannot replace a newer queued close request', async () => {
  const fixture = await mount(), response = deferred<{ requestId: number; pending: boolean }>();
  fixture.mock.hideWindow = async (release, requestId) => {
    fixture.calls.push({ release, requestId });
    return requestId === 75 ? response.promise : { requestId: requestId!, pending: false };
  };
  try {
    await fixture.close(75); await fixture.close(76);
    assert.deepEqual(fixture.calls, [{ release: true, requestId: 75 }]);
    response.resolve({ requestId: 75, pending: true }); await flush();
    assert.deepEqual(fixture.calls, [{ release: true, requestId: 75 }, { release: true, requestId: 76 }]);
  } finally { fixture.cleanup(); }
});

test('native busy guards retry only a pending close and stop when a reopen invalidates it', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const fixture = await mount();
  let nativeBusy = true, reopened = false;
  fixture.mock.hideWindow = async (release, requestId) => {
    fixture.calls.push({ release, requestId });
    return { requestId: requestId ?? 100, pending: !reopened && (nativeBusy || !release) };
  };
  try {
    await fixture.close(80);
    assert.equal(fixture.calls.length, 1);
    nativeBusy = false;
    t.mock.timers.tick(1000); await flush();
    assert.deepEqual(fixture.calls, [{ release: true, requestId: 80 }, { release: true, requestId: 80 }]);
    t.mock.timers.tick(3000); await flush();
    assert.equal(fixture.calls.length, 2, 'completed requests do not keep polling');
    nativeBusy = true; await fixture.close(81);
    reopened = true;
    t.mock.timers.tick(1000); await flush();
    assert.equal(fixture.calls.length, 4);
    t.mock.timers.tick(3000); await flush();
    assert.equal(fixture.calls.length, 4, 'a stale native request cancels polling');
    reopened = false; await fixture.close(82);
    fixture.unmount(); t.mock.timers.tick(3000); await flush();
    assert.equal(fixture.calls.length, 5, 'unmount cancels the pending timer');
  } finally { fixture.cleanup(); }
});

test('unsaved drafts remain protected after background work finishes', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const fixture = await mount();
  try {
    fixture.vm.openClip(); fixture.vm.clipContent = '原创未保存草稿';
    fixture.vm.busy = true; await fixture.close(90);
    fixture.vm.busy = false; await flush();
    t.mock.timers.tick(5000); await flush();
    assert.deepEqual(fixture.calls, [{ release: false, requestId: 90 }]);
    assert.equal(fixture.vm.clipContent, '原创未保存草稿');
  } finally { fixture.cleanup(); }
});

test('clean settings release only after every mounted child is ready and free of drafts', async () => {
  const fixture = await mount(), { vm, calls } = fixture;
  try {
    vm.openSettings();
    await fixture.close(95);
    assert.deepEqual(calls, [{ release: false, requestId: 95 }], 'an unknown child is protected');
    vm.appearanceSettings = { canRelease: true }; await flush();
    assert.deepEqual(calls.at(-1), { release: true, requestId: 95 });

    vm.changeSettingsTab('privacy'); vm.privacySettings = { canRelease: false }; vm.telemetrySettings = { canRelease: true };
    vm.changeSettingsTab('ai'); vm.aiSettings = { canRelease: true };
    vm.changeSettingsTab('data'); vm.dataSettings = { canRelease: true };
    vm.changeSettingsTab('help'); vm.updateSettings = { canRelease: true };
    await fixture.close(96);
    assert.deepEqual(calls.at(-1), { release: false, requestId: 96 }, 'a hidden visited privacy draft still protects the window');
    vm.privacySettings.canRelease = true; await flush();
    assert.deepEqual(calls.at(-1), { release: true, requestId: 96 });

    for (const key of ['appearanceSettings', 'privacySettings', 'telemetrySettings', 'aiSettings', 'dataSettings', 'updateSettings']) {
      vm[key].canRelease = false; await flush();
      await fixture.close(97);
      assert.deepEqual(calls.at(-1), { release: false, requestId: 97 }, key);
      vm[key].canRelease = true; await flush();
      assert.deepEqual(calls.at(-1), { release: true, requestId: 97 }, key);
    }
    vm.changeSettingsTab('starpet'); vm.starPetSettings = { canRelease: false };
    await fixture.close(98); assert.deepEqual(calls.at(-1), { release: false, requestId: 98 });
    vm.starPetSettings.canRelease = true; await flush();
    assert.deepEqual(calls.at(-1), { release: true, requestId: 98 });
    vm.settingsDraft.maxHistory = 500;
    await fixture.close(99); assert.deepEqual(calls.at(-1), { release: false, requestId: 99 }, 'parent drafts remain protected');
  } finally { fixture.cleanup(); }
});

test('closing during telemetry work waits for both the operation and panel readiness before releasing', async () => {
  const fixture = await mount(), { vm, calls } = fixture;
  try {
    vm.openSettings(); vm.appearanceSettings = { canRelease: true };
    vm.changeSettingsTab('privacy'); vm.privacySettings = { canRelease: true };
    vm.telemetrySettings = { canRelease: false }; vm.telemetryBusy = true;
    await fixture.close(100);
    assert.deepEqual(calls, [{ release: false, requestId: 100 }]);
    vm.telemetryBusy = false; await flush();
    assert.deepEqual(calls, [{ release: false, requestId: 100 }], 'a completed operation cannot release a panel before its state is ready');
    vm.telemetrySettings.canRelease = true; await flush();
    assert.deepEqual(calls, [{ release: false, requestId: 100 }, { release: true, requestId: 100 }], 'the pending close completes without another click');
  } finally { fixture.cleanup(); }
});

test('vertical settings tabs use Up/Down and Home/End while preserving drafts and panel visits', async () => {
  const fixture = await mount(), { vm } = fixture, focused: string[] = [], scrolls: unknown[] = [];
  vm.dialog = { open: true, querySelector: (selector: string) => ({ focus: () => focused.push(selector) }) };
  vm.settingsScroll = { scrollTo: (options: unknown) => scrolls.push(options) };
  vm.openSettings(); vm.settingsDraft.maxHistory = 500;
  const press = async (key: string, extra: Record<string, unknown> = {}) => {
    let prevented = false;
    vm.settingsTabKeyboard({ key, preventDefault() { prevented = true; }, ...extra });
    await flush(); return prevented;
  };
  try {
    assert.equal(await press('ArrowUp'), true); assert.equal(vm.settingsTab, 'help');
    assert.equal(focused.at(-1), '#settings-tab-help');
    assert.equal(await press('ArrowDown'), true); assert.equal(vm.settingsTab, 'appearance');
    await press('End'); assert.equal(vm.settingsTab, 'help');
    await press('Home'); assert.equal(vm.settingsTab, 'appearance');
    for (const tab of ['starpet', 'habits', 'privacy', 'ai', 'data', 'stats', 'share']) {
      await press('ArrowDown'); assert.equal(vm.settingsTab, tab);
      assert.equal(focused.at(-1), `#settings-tab-${tab}`);
    }
    assert.equal(vm.privacyTabVisited, true); assert.equal(vm.aiTabVisited, true); assert.equal(vm.dataTabVisited, true);
    assert.equal(vm.settingsDraft.maxHistory, 500);
    assert.deepEqual(scrolls.at(-1), { top: 0 });
    for (const [key, extra] of [['ArrowLeft', {}], ['ArrowRight', {}], ['ArrowDown', { metaKey: true }], ['ArrowDown', { isComposing: true }]] as const) {
      assert.equal(await press(key, extra), false); assert.equal(vm.settingsTab, 'share');
    }
  } finally { fixture.cleanup(); }
});

test('busy settings and cleanup prevent tab changes or late focus from escaping the active dialog', async () => {
  const fixture = await mount(), { vm } = fixture, focused: string[] = [];
  vm.dialog = { open: true, querySelector: (selector: string) => ({ focus: () => focused.push(selector) }) };
  vm.openSettings();
  const press = () => vm.settingsTabKeyboard({ key: 'ArrowDown', preventDefault() {} });
  try {
    for (const flag of ['busy', 'dataOperationBusy', 'telemetryBusy', 'cleanupOpen']) {
      vm[flag] = true; press(); vm.changeSettingsTab('help'); await flush();
      assert.equal(vm.settingsTab, 'appearance', flag); assert.equal(focused.length, 0, flag);
      vm[flag] = false;
    }
    press(); vm.cleanupOpen = true; await flush();
    assert.equal(vm.settingsTab, 'starpet');
    assert.equal(focused.length, 0, 'cleanup owns focus even if opened before the tab nextTick finishes');
  } finally { fixture.cleanup(); }
});

test('a tray pause updates only the capture draft and survives saving unrelated preferences', async () => {
  const fixture = await mount(), { vm, mock } = fixture;
  const saved = await mock.getState(); saved.settings.paused = false; saved.settings.pauseRevision = 1;
  mock.getState = async () => structuredClone(saved);
  const writes: Array<{ settings: Settings; expectedPauseRevision?: number }> = [];
  (mock as any).saveSettings = async (settings: Settings, expectedPauseRevision?: number) => {
    writes.push({ settings, expectedPauseRevision }); saved.settings = { ...settings };
  };
  try {
    await vm.refresh(); vm.openSettings(); vm.settingsDraft.layout = 'list';
    saved.settings.paused = true; saved.settings.pauseRevision = 2; await vm.refresh();
    assert.equal(vm.settingsDraft.paused, true, 'newer tray pause replaces the old capture draft');
    assert.equal(vm.settingsDraft.layout, 'list', 'unrelated unsaved preference survives');
    await vm.saveSettings();
    assert.equal(writes.length, 1); assert.equal(writes[0]!.settings.paused, true);
    assert.equal(writes[0]!.settings.layout, 'list'); assert.equal(writes[0]!.expectedPauseRevision, 2);
    assert.equal(vm.state.settings.paused, true);
  } finally { fixture.cleanup(); }
});

test('unchanged saved capture state preserves an explicit resume draft and its baseline', async () => {
  const fixture = await mount(), { vm, mock } = fixture;
  const saved = await mock.getState(); mock.getState = async () => structuredClone(saved);
  const writes: Array<{ settings: Settings; expectedPauseRevision?: number }> = [];
  (mock as any).saveSettings = async (settings: Settings, expectedPauseRevision?: number) => {
    writes.push({ settings, expectedPauseRevision }); saved.settings = { ...settings, pauseRevision: 1 };
  };
  try {
    vm.openSettings(); vm.settingsDraft.paused = false;
    await vm.refresh();
    assert.equal(vm.settingsDraft.paused, false, 'ordinary refresh must not discard explicit intent');
    await vm.saveSettings();
    assert.equal(writes.length, 1); assert.equal(writes[0]!.settings.paused, false);
    assert.equal(writes[0]!.expectedPauseRevision, 0); assert.equal(vm.state.settings.paused, false);
    assert.equal(vm.settingsDraft.pauseRevision, 1); assert.equal(vm.settingsDirty, false);
  } finally { fixture.cleanup(); }
});

test('settings and layout saves pass the observed capture baseline when the tray changes before refresh', async () => {
  for (const action of ['settings', 'layout'] as const) {
    const fixture = await mount(), { vm, mock } = fixture;
    const saved = await mock.getState(); saved.settings.paused = false; saved.settings.pauseRevision = 1;
    mock.getState = async () => structuredClone(saved);
    const writes: Array<{ settings: Settings; expectedPauseRevision?: number }> = [];
    (mock as any).saveSettings = async (settings: Settings, expectedPauseRevision?: number) => {
      // The native store resolves this stale baseline atomically; leave its
      // latest paused response in place to exercise the ensuing UI refresh.
      writes.push({ settings, expectedPauseRevision });
    };
    try {
      await vm.refresh(); vm.openSettings(); vm.settingsDraft.layout = 'list';
      saved.settings.paused = true; saved.settings.pauseRevision = 2;
      if (action === 'settings') await vm.saveSettings(); else await vm.changeLayout('list');
      assert.equal(writes.length, 1); assert.equal(writes[0]!.settings.paused, false);
      assert.equal(writes[0]!.expectedPauseRevision, 1, 'backend must receive the old observed baseline');
      assert.equal(vm.state.settings.paused, true); assert.equal(vm.settingsDraft.paused, true);
    } finally { fixture.cleanup(); }
  }
});

test('capture revision supersedes an old resume draft even when the saved boolean returns to paused', async () => {
  for (const refreshBeforeSave of [true, false]) {
    const fixture = await mount(), { vm, mock } = fixture;
    const saved = await mock.getState(); mock.getState = async () => structuredClone(saved);
    const writes: Array<{ settings: Settings; expectedPauseRevision?: number }> = [];
    (mock as any).saveSettings = async (settings: Settings, expectedPauseRevision?: number) => {
      writes.push({ settings, expectedPauseRevision });
      // Fixed response from the native guard: the newer pause is preserved.
      saved.settings.layout = settings.layout;
    };
    try {
      vm.openSettings(); vm.settingsDraft.paused = false; vm.settingsDraft.layout = 'list';
      saved.settings.paused = false; saved.settings.pauseRevision = 1;
      saved.settings.paused = true; saved.settings.pauseRevision = 2;
      if (refreshBeforeSave) {
        await vm.refresh();
        assert.equal(vm.settingsDraft.paused, true, 'version change reveals the unseen round trip');
        assert.equal(vm.settingsDraft.pauseRevision, 2);
      }
      await vm.saveSettings();
      assert.equal(writes.length, 1);
      assert.equal(writes[0]!.expectedPauseRevision, refreshBeforeSave ? 2 : 0);
      assert.equal(writes[0]!.settings.paused, refreshBeforeSave);
      assert.equal(writes[0]!.settings.layout, 'list');
      assert.equal(vm.state.settings.paused, true); assert.equal(vm.settingsDraft.paused, true);
      assert.equal(vm.settingsDirty, false, 'the backend-owned revision does not leave phantom edits');
    } finally { fixture.cleanup(); }
  }
});

test('App unregisters close listeners and ignores close events after unmount, including late subscription', async () => {
  const fixture = await mount();
  try {
    fixture.unmount(); assert.deepEqual(fixture.stopped, { change: 1, close: 1, growth: 1, quit: 0 });
    assert.equal(fixture.windowEvents.count(), 0); assert.equal(fixture.themeEvents.count(), 0);
    await fixture.close(30); assert.deepEqual(fixture.calls, []);
  } finally { fixture.cleanup(); }
  const pending = await mount({ delayedCloseListener: true });
  try {
    pending.unmount(); pending.finishListener(); await flush();
    assert.deepEqual(pending.stopped, { change: 0, close: 1, growth: 0, quit: 0 });
    await pending.close(31); assert.deepEqual(pending.calls, []);
    assert.equal(pending.windowEvents.count(), 0); assert.equal(pending.themeEvents.count(), 0);
  } finally { pending.cleanup(); }
});

test('native close API forwards release and request ID, delivers payloads and returns unlisten; browser stays inert', async () => {
  const commands: { command: string; args: unknown }[] = [], requests: number[] = [];
  let handler!: (event: { payload: { requestId: number } }) => void, registrations = 0, stopped = 0;
  const unlisten = () => { stopped++; };
  const restore = globals({
    window: { __TAURI_INTERNALS__: {} },
    windowReleaseApiMock: {
      async invoke(command: string, args: unknown) { commands.push({ command, args }); return command === 'get_main_window_close_request' ? 42 : { requestId: 41, pending: true }; },
      async listen(name: string, callback: typeof handler) { assert.equal(name, 'main-window-close-requested'); registrations++; handler = callback; return unlisten; },
    },
  });
  const nativeUrl = moduleUrl('export const { invoke, listen } = globalThis.windowReleaseApiMock;');
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replaceAll("'@tauri-apps/api/core'", `'${nativeUrl}'`).replaceAll("'@tauri-apps/api/event'", `'${nativeUrl}'`)
    .replaceAll("'./privacy'", `'${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replaceAll("'./snippets'", `'${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replaceAll("'./i18n/index.ts'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  try {
    const { api } = await import(moduleUrl(source));
    assert.deepEqual(await api.hideWindow(), { requestId: 41, pending: true }); await api.hideWindow(true, 41); await api.hideWindow(false, 42);
    assert.equal(await api.getWindowCloseRequest(), 42);
    assert.deepEqual(commands, [
      { command: 'hide_window', args: { release: false, requestId: undefined } },
      { command: 'hide_window', args: { release: true, requestId: 41 } },
      { command: 'hide_window', args: { release: false, requestId: 42 } },
      { command: 'get_main_window_close_request', args: undefined },
    ]);
    const stop = await api.onWindowCloseRequested((requestId: number) => requests.push(requestId));
    handler({ payload: { requestId: 43 } }); assert.deepEqual(requests, [43]);
    assert.equal(stop, unlisten); stop(); assert.equal(stopped, 1);
    Object.assign(globalThis, { window: {} });
    const browser = await import(moduleUrl(source + '\n// browser close protocol'));
    await browser.api.hideWindow(true, 44); (await browser.api.onWindowCloseRequested(() => assert.fail('unexpected browser close event')))();
    assert.equal(await browser.api.getWindowCloseRequest(), null);
    assert.equal(commands.length, 4); assert.equal(registrations, 1); assert.equal(stopped, 1);
  } finally { restore(); }
});

test('history requests replace the page and an obsolete response cannot win during search debounce', async () => {
  const fixture = await mount(); const { vm } = fixture;
  const pending = deferred<AppState>();
  const initial = structuredClone(await fixture.mock.getState());
  const summary = { page: 1, pageSize: 50, total: 120, counts: { all: 120 }, groupCounts: {}, sources: [], hosts: [], tags: [] };
  const clip = (id: number, title: string) => ({ id, title, content: '原创分页测试内容', kind: 'text' as const, language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0 });
  const queries: any[] = [];
  fixture.mock.getState = async (...args: any[]) => {
    const query = args[0]; queries.push(query);
    if (query.page === 2) return pending.promise;
    return { ...initial, clips: [clip(9, '最新搜索结果')], history: { ...summary, page: 1, total: 1 } };
  };
  try {
    vm.state = { ...initial, clips: [clip(1, '旧页')], history: summary };
    vm.changeHistoryPage(2);
    assert.equal(vm.historyLoading, true);
    assert.deepEqual(vm.visible, [], 'old page rows cannot be acted on during replacement');
    vm.query = '最新'; await flush();
    pending.resolve({ ...initial, clips: [clip(2, '过期第二页')], history: { ...summary, page: 2 } });
    await flush();
    assert.equal(vm.historyLoading, true, 'obsolete response does not clear the new search loading state');
    assert(!vm.visible.some((row: { id: number }) => row.id === 2));
    await new Promise(resolve => setTimeout(resolve, 180)); await flush();
    assert.equal(queries.at(-1).query, '最新'); assert.equal(queries.at(-1).page, 1);
    assert.deepEqual(vm.visible.map((row: { id: number }) => row.id), [9]);
    assert.equal(vm.count('all'), 120, 'category count is global, not the current page size');
    assert.equal(vm.historyLoading, false);
  } finally { fixture.cleanup(); }
});

test('privacy changes refresh open details by ID even when the current page filters the record out', async () => {
  const fixture = await mount(); const { vm } = fixture;
  const original = { id: 7, title: '原创隐私回归', content: '原创只读正文', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0 };
  const fresh = { ...original, isPrivate: true, privacyManual: true };
  const detail = deferred<any[]>();
  const requested: number[][] = [];
  (fixture.mock as any).getClips = (ids: number[]) => { requested.push(ids); return detail.promise; };
  try {
    vm.openClip(original);
    const changing = vm.privacyChanged();
    assert.equal(vm.clipDetailsBlocked, true, 'old ordinary content is hidden while privacy is revalidated');
    detail.resolve([fresh]); await changing; await flush();
    assert.deepEqual(requested, [[7]]);
    assert.deepEqual(vm.state.clips, [], 'ordinary/current page excludes the newly private record');
    assert.equal(vm.editing.isPrivate, true); assert.equal(vm.privateRevealed, false);
    assert.equal(vm.clipDetailsBlocked, false);
    (fixture.mock as any).getClips = async () => [];
    await vm.privacyChanged(); await flush();
    assert.equal(vm.modal, ''); assert.equal(vm.editing, null); assert.equal(vm.clipContent, '', 'deleted details are released');
  } finally { fixture.cleanup(); }
});

test('late detail responses cannot replace another open record and failed checks keep content blocked', async () => {
  const fixture = await mount(); const { vm } = fixture;
  const clip = (id: number) => ({ id, title: '原创详情竞态', content: '原创只读正文', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0 });
  const pending = deferred<any[]>();
  (fixture.mock as any).getClips = () => pending.promise;
  try {
    vm.openClip(clip(1)); const changing = vm.privacyChanged();
    vm.openClip(clip(2)); pending.resolve([{ ...clip(1), isPrivate: true }]); await changing; await flush();
    assert.equal(vm.editing.id, 2); assert.equal(vm.clipDetailsBlocked, false);
    (fixture.mock as any).getClips = async () => { throw new Error('固定详情读取失败'); };
    await vm.privacyChanged(); await flush();
    assert.equal(vm.editing.id, 2); assert.equal(vm.clipDetailsBlocked, true);
    assert.notEqual(vm.clipDetailsError, '', 'a failed read offers feedback without reopening stale content');
    (fixture.mock as any).getClips = async () => [clip(2)];
    await vm.privacyChanged(); await flush();
    assert.equal(vm.clipDetailsBlocked, false);
  } finally { fixture.cleanup(); }
});


test('settings quit preserves pending work and drafts, then exits when all settings are saved', async () => {
  const f = await mount();
  try {
    f.vm.openSettings(); f.vm.settingsTab = 'habits';
    f.vm.appearanceSettings = { canRelease: true };
    f.vm.startupSettings = { canRelease: true };
    f.vm.settingsDraft.layout = 'list';
    await f.vm.quitFromSettings(); assert.equal(f.stopped.quit, 0);
    assert.match(f.vm.toast, /保存|save/i);
    f.vm.settingsDraft.layout = f.vm.state.settings.layout;
    f.vm.startupBusy = true;
    await f.vm.quitFromSettings(); assert.equal(f.stopped.quit, 0);
    f.vm.startupBusy = false;
    f.vm.startupSettings.canRelease = false;
    await f.vm.quitFromSettings(); assert.equal(f.stopped.quit, 0);
    f.vm.startupSettings.canRelease = true;
    await f.vm.quitFromSettings(); assert.equal(f.stopped.quit, 1);
  } finally { f.cleanup(); }
});


test('direct share tab blocks navigation and window release until sharing is fully complete', async () => {
  const f = await mount(), { vm, calls } = f;
  try {
    vm.openSettings(); vm.appearanceSettings = { canRelease: true };
    vm.changeSettingsTab('share');
    assert.equal(vm.settingsTab, 'share');
    await f.close(401);
    assert.deepEqual(calls.at(-1), { release: false, requestId: 401 }, 'unknown share panel is protected');
    vm.productShare = { canRelease: true }; await flush();
    assert.deepEqual(calls.at(-1), { release: true, requestId: 401 });

    vm.productShare.canRelease = false; vm.shareBusy = true;
    vm.changeSettingsTab('help');
    assert.equal(vm.settingsTab, 'share', 'in-flight sharing must not be unmounted');
    await f.close(402);
    assert.deepEqual(calls.at(-1), { release: false, requestId: 402 });
    vm.shareBusy = false; await flush();
    assert.deepEqual(calls.at(-1), { release: false, requestId: 402 });
    vm.productShare.canRelease = true; await flush();
    assert.deepEqual(calls.at(-1), { release: true, requestId: 402 });
    vm.changeSettingsTab('help'); vm.updateSettings = { canRelease: true };
    vm.productShare = undefined;
    await f.close(403);
    assert.deepEqual(calls.at(-1), { release: true, requestId: 403 }, 'help no longer depends on an unmounted share panel');
  } finally { f.cleanup(); }
});

test('quick copy hides the window and returns focus unless the preference is off', async () => {
  const fixture = await mount(); const { vm, calls, focusCalls, copiedIds } = fixture;
  try {
    const clip = { id: 7, content: '原创快选样例', title: '', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0 };
    await vm.copy(clip); await flush();
    assert.deepEqual(copiedIds, [7]); assert.equal(calls.length, 0, 'a plain button copy keeps the window open');
    vm.query = '原创';
    await vm.copy(clip, true); await flush();
    assert.deepEqual(copiedIds, [7, 7]); assert.deepEqual(focusCalls.at(-1), true); assert.equal(vm.query, '', 'search resets for the next summon');
    const before = calls.length;
    vm.state.settings.hideAfterCopy = false;
    await vm.copy(clip, true); await flush();
    assert.equal(calls.length, before, 'turning the preference off keeps the window open');
  } finally { fixture.cleanup(); }
});
