import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import { defaultAppearance } from '../src/appearance.ts';
import type { AppState, Clip } from '../src/types.ts';
import { setLocale } from '../src/i18n/index.ts';

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

// Browser demo: file records are references only, so the preview can never
// read a file, and the text fallbacks must stay explicit.
test('browser demo keeps file references separate from text and never fakes native access', async () => {
  Object.assign(globalThis, { window: {} });
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replace("from './privacy'", `from '${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replace("from './snippets'", `from '${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replace("from './i18n/index.ts'", `from '${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const { api } = await import(moduleUrl(source));
  const initial = await api.getState();
  const file = initial.clips.find((clip: Clip) => clip.kind === 'file');
  assert(file, 'the demo history shows one file record');
  assert.equal(file.entries.length, 2);
  assert.deepEqual(file.entries.map((entry: { entryKind: string }) => entry.entryKind), ['file', 'directory']);
  assert.equal(file.content, file.entries.map((entry: { path: string }) => entry.path).join('\n'));

  const filtered = await api.getState({ query: '', filter: 'file' });
  assert.equal(filtered.history.total, 1);
  assert.equal(filtered.history.counts.file, 1);
  assert.equal((await api.getState({ query: '封面稿' })).history.total, 1, 'search covers the file name');
  assert.equal((await api.getState({ query: '月白' })).history.total, 1, 'search covers the folder path');
  assert.equal((await api.getState({ query: '', aiEligible: true })).clips.some((clip: Clip) => clip.kind === 'file'), false);

  const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  try {
    const writes: string[] = [];
    Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { clipboard: { writeText: async (value: string) => { writes.push(value); } } } });
    await api.copyClipPaths(file.id);
    assert.deepEqual(writes, [file.entries.map((entry: { path: string }) => entry.path).join('\n')]);
    assert.equal((await api.getState()).clips.find((clip: Clip) => clip.id === file.id).copyCount, 1);
  } finally {
    if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor);
    else Reflect.deleteProperty(globalThis, 'navigator');
  }
  // The preview has no file manager and no disk access, and says so.
  await assert.rejects(api.revealClipFile(file.entries[0].path), /桌面版/);
  assert.deepEqual(await api.checkClipFiles(file.id), []);
  await assert.rejects(api.checkClipFiles(initial.clips[0].id), /只有文件记录/);
  await assert.rejects(api.copyClipPaths(initial.clips[0].id), /只有文件记录/);
  await assert.rejects(api.saveCode(file.id, '', file.content, 'text', null), /文件记录/);
  assert.equal((await api.previewCleanup({ minBytes: 0, olderThanDays: 0, kind: 'file', includeGrouped: true })).matchedCount, 1);
});

// Exercise the real App setup so the file branches are wired to the api layer.
const renderer = createRenderer<object, object>({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  setText() {}, setElementText() {}, patchProp() {}, insert() {}, remove() {},
  parentNode: () => null, nextSibling: () => null,
});
let sequence = 0;
const fileClip: Clip = {
  id: 7, content: '/原创/目录/封面 稿.sketch\n/原创/目录/素材', title: '', kind: 'file', language: null,
  pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0,
  entries: [{ path: '/原创/目录/封面 稿.sketch', entryKind: 'file' }, { path: '/原创/目录/素材', entryKind: 'directory' }],
};

async function mountApp() {
  setLocale('zh-CN', false);
  const state: AppState = {
    onboardingCompleted: true, clips: [fileClip], groups: [],
    settings: { paused: true, pauseRevision: 0, uiLanguage: 'zh-CN', theme: 'dark', layout: 'grid', maxHistory: 0, retentionDays: 0, alwaysOnTop: false, showStartup: false, stickerPack: 'none', shortcut: 'CommandOrControl+Shift+V' },
    clipboardError: null, shortcutError: null, windowError: null,
  };
  const calls: { copiedPaths: number[]; checked: number[]; revealed: string[] } = { copiedPaths: [], checked: [], revealed: [] };
  const mock = {
    async getState() { return structuredClone(state); },
    async setNativeLanguage() {},
    async privacyRules() { return { revision: 1, rules: [] }; },
    async onChange() { return () => {}; },
    async hideWindow() { return { requestId: 1, pending: false }; },
    async getWindowCloseRequest() { return null; },
    async onWindowCloseRequested() { return () => {}; },
    async setTheme(theme: string) { return { ...state.settings, theme }; },
    async copyClipPaths(id: number) { calls.copiedPaths.push(id); },
    async checkClipFiles(id: number) { calls.checked.push(id); return ['/原创/目录/素材']; },
    async revealClipFile(path: string) { calls.revealed.push(path); },
    async copyClip() {},
  };
  const hub = { addEventListener() {}, removeEventListener() {}, count() { return 0; } };
  const restore = globals({
    window: { ...hub, matchMedia: () => ({ ...hub, matches: false }) },
    document: { documentElement: { dataset: {}, style: { setProperty() {}, removeProperty() {} } } },
    navigator: { platform: 'MacIntel' },
    fileClipMock: {
      api: mock, getAppearance: async () => defaultAppearance(),
      recordStudioOpened() {}, async quitStarclip() {}, async listen() { return () => {}; },
    },
  });
  const mockUrl = moduleUrl(`export const { api, getAppearance, listen, recordStudioOpened, quitStarclip } = globalThis.fileClipMock; export const isDesktop = true; export const appVersion = '0.0.0-test'; // ${sequence++}`);
  const stubUrl = moduleUrl('export default { render() { return null; } };');
  const { descriptor } = parse(fs.readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'file-clip-test' });
  const source = script.content.replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replace(/(['"])\.\/components\/[^'"]+\.vue\1/g, `'${stubUrl}'`)
    .replaceAll("'./api'", `'${mockUrl}'`).replaceAll("'./appearance-api'", `'${mockUrl}'`).replaceAll("'./update-api'", `'${mockUrl}'`)
    .replaceAll("'./startup-api'", `'${mockUrl}'`).replaceAll("'./telemetry-api'", `'${mockUrl}'`)
    .replaceAll("'@tauri-apps/api/event'", `'${mockUrl}'`)
    .replaceAll("'./appearance'", `'${new URL('../src/appearance.ts', import.meta.url).href}'`)
    .replaceAll("'./privacy'", `'${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replaceAll("'./theme'", `'${new URL('../src/theme.ts', import.meta.url).href}'`)
    .replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`)
    .replaceAll("'./i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = await import(moduleUrl(source));
  const app = renderer.createApp({ ...component.default, render() { return null; } });
  app.mount({}); await flush();
  const vm = app._instance!.setupState as Record<string, any>;
  return { vm, calls, cleanup() { app.unmount(); restore(); } };
}

test('file details list every path, check on demand and never reach the studio', async () => {
  const fixture = await mountApp();
  const { vm, calls } = fixture;
  try {
    assert.equal(vm.fileSummary(fileClip), '共 2 项');
    assert.equal(vm.fileName('/原创/目录/封面 稿.sketch'), '封面 稿.sketch');
    assert.equal(vm.fileFolder('/原创/目录/封面 稿.sketch'), '/原创/目录');
    assert.equal(vm.fileKindLabel(fileClip.entries[1]), '文件夹');
    assert.equal(vm.kindNames.file, '文件');
    assert.equal(vm.navItems.some((item: { key: string }) => item.key === 'file'), true);

    vm.openClip(fileClip);
    assert.equal(vm.modal, 'clip');
    assert.deepEqual(vm.missingFiles, []);
    assert.equal(vm.filesChecked, false);

    // Existence is only checked when the user asks for it.
    assert.deepEqual(calls.checked, []);
    await vm.checkFiles(fileClip);
    assert.deepEqual(calls.checked, [fileClip.id]);
    assert.deepEqual(vm.missingFiles, ['/原创/目录/素材']);
    assert.equal(vm.filesChecked, true);
    assert.equal(vm.fileBusy, false);

    await vm.copyFilePaths(fileClip);
    assert.deepEqual(calls.copiedPaths, [fileClip.id]);
    await vm.revealFile('/原创/目录/素材');
    assert.deepEqual(calls.revealed, ['/原创/目录/素材']);

    // Opening another record clears the previous missing-path marks.
    vm.openClip(fileClip);
    assert.deepEqual(vm.missingFiles, []);

    // A file record is never handed to the text/code studio.
    vm.modal = '';
    vm.openStudio(fileClip);
    assert.equal(vm.studioOpen, false);
    assert.match(vm.toast, /星绘坊用于文字和代码/);
  } finally { fixture.cleanup(); }
});
