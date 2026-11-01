import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import type { HandwritingAssets, HandwritingOptions } from '../src/types.ts';

type Node = { type: string; text: string; props: Record<string, any>; children: Node[]; parent?: Node; addEventListener: () => void; removeEventListener: () => void; getRootNode: () => { activeElement: null } };
function node(type: string, text = ''): Node { return { type, text, props: {}, children: [], addEventListener() {}, removeEventListener() {}, getRootNode() { return Object.assign(Object.create(globalThis.Document.prototype), { activeElement: null }); } }; }
const renderer = createRenderer<Node, Node>({
  createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
  setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.text = text; node.children = []; },
  patchProp: (node, key, _previous, value) => { node.props[key] = value; },
  insert: (child, parent, anchor) => { const previous = child.parent?.children.indexOf(child) ?? -1; if (previous >= 0) child.parent!.children.splice(previous, 1); child.parent = parent; const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index >= 0 ? index : parent.children.length, 0, child); },
  remove: child => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); },
  parentNode: node => node.parent || null, nextSibling: node => node.parent?.children[(node.parent?.children.indexOf(node) ?? -1) + 1] || null,
});
function nodes(root: Node): Node[] { return [root, ...root.children.flatMap(nodes)]; }
function text(root: Node): string { return root.type === '#comment' ? '' : root.text + root.children.map(text).join(''); }
function button(root: Node, label: string) { const found = nodes(root).find(item => item.type === 'button' && (item.props['aria-label'] === label || text(item) === label)); assert(found, `missing ${label}`); return found; }
async function flush() { for (let i = 0; i < 15; i++) await Promise.resolve(); await nextTick(); }
// The catalog lives behind the font picker dialog so it no longer fills the panel.
async function openFonts(root: Node) { await button(root, '选择手写字体').props.onClick(); await flush(); }
function deferred<T>() { let resolve!: (value: T) => void, reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function moduleUrl(source: string) { return 'data:text/javascript;base64,' + Buffer.from(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText).toString('base64'); }
const catalog = (): HandwritingAssets => ({ available: true, path: 'builtin', error: null, backgrounds: [], fonts: ['one', 'two'].map((id, index) => ({ id, path: id, name: `原创字体${index + 1}`, author: index ? '来源于互联网' : '原创测试作者', tags: [index ? '手写体' : '展示体'], downloaded: false, bytes: 123456, previewUrl: `/handwriting-fonts/${id}.png`, license: 'OFL-1.1', sourceUrl: 'https://example.com/original-font' })) });
let sequence = 0;
async function mount(api: Record<string, unknown>, desktop = true) {
  const priorDocument = Object.getOwnPropertyDescriptor(globalThis, 'Document');
  if (!priorDocument) Object.defineProperty(globalThis, 'Document', { configurable: true, value: class {} });
  Object.assign(globalThis, { handwritingFontApi: api });
  const apiUrl = moduleUrl(`export const api=globalThis.handwritingFontApi; export const isDesktop=${desktop}; // ${sequence++}`);
  const { descriptor } = parse(fs.readFileSync(new URL('../src/components/HandwritingPanel.vue', import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'handwriting-font-test', inlineTemplate: true });
  const source = script.content.replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replaceAll("'../api'", `'${apiUrl}'`).replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`)
    .replaceAll("'../snippets'", `'${new URL('../src/snippets.ts', import.meta.url).href}'`).replaceAll("'../handwriting-preview'", `'${new URL('../src/handwriting-preview.ts', import.meta.url).href}'`)
    .replaceAll("'./UiSelect.vue'", "'data:text/javascript,export default {render(){return null}}'")
    .replaceAll("'./UiColor.vue'", "'data:text/javascript,export default {render(){return null}}'");
  const component = (await import(moduleUrl(source))).default;
  const root = node('root'); const app = renderer.createApp(component, { text: '原创固定样例，星光落在纸上。', title: '原创验收' }); app.mount(root); await flush();
  return { root, unmount() { app.unmount(); if (priorDocument) Object.defineProperty(globalThis, 'Document', priorDocument); else Reflect.deleteProperty(globalThis, 'Document'); } };
}

test('font previews do not download fonts; only the selected verified font starts local rendering', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const assets = catalog(), downloaded: string[] = [], rendered: HandwritingOptions[] = [], saved: unknown[][] = [];
  const install = deferred<HandwritingAssets>();
  const fixture = await mount({ handwritingAssets: async () => assets, downloadHandwritingFont(id: string) { downloaded.push(id); return install.promise; }, async renderHandwriting(options: HandwritingOptions) { rendered.push(options); return 'original-png'; }, async saveExport(...args: unknown[]) { saved.push(args); return true; } });
  try {
    t.mock.timers.tick(1000); await flush();
    assert.deepEqual(downloaded, []); assert.deepEqual(rendered, []);
    assert.equal(nodes(fixture.root).filter(item => item.type === 'article').length, 0, 'the catalog stays closed until the picker opens');
    await openFonts(fixture.root);
    assert.equal(nodes(fixture.root).filter(item => item.type === 'img' && item.props.src.startsWith('/handwriting-fonts/')).length, 2);
    assert.match(text(fixture.root), /安装包不含完整字体/);
    const cards = nodes(fixture.root).filter(item => item.type === 'article');
    assert.match(text(cards[0]!), /作者：原创测试作者/);
    assert.match(text(cards[1]!), /作者：来源于互联网/);
    assert.match(text(cards[1]!), /手写体/);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, true);
    const downloading = button(fixture.root, '下载 原创字体2').props.onClick(); await flush();
    assert.deepEqual(downloaded, ['two']); assert.match(text(fixture.root), /正在下载并验证/);
    assert.equal(button(fixture.root, '下载 原创字体1').props.disabled, true);
    await button(fixture.root, '下载 原创字体1').props.onClick(); assert.deepEqual(downloaded, ['two'], 'busy handler also blocks duplicate requests');
    const installed = catalog(); installed.fonts[1]!.downloaded = true;
    install.resolve(installed); await downloading; await flush();
    t.mock.timers.tick(350); await flush();
    assert.equal(rendered.length, 1); assert.equal(rendered[0]!.font, 'two');
    assert.equal(button(fixture.root, '使用 原创字体2').props['aria-pressed'], true);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, false);
    await button(fixture.root, '使用 原创字体2').props.onClick(); await flush(); t.mock.timers.tick(350); await flush();
    assert.deepEqual(downloaded, ['two'], 'selecting installed font must work offline');
    await button(fixture.root, '导出月光笺 PNG').props.onClick(); await flush(); assert.equal(saved[0]?.[0], 'original-png');
  } finally { fixture.unmount(); }
});

test('desktop font downloads report real progress inside the picker', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const install = deferred<HandwritingAssets>();
  let emit: ((progress: { id: string; received: number; total: number }) => void) | undefined;
  let released = 0;
  const fixture = await mount({
    handwritingAssets: async () => catalog(),
    downloadHandwritingFont() { return install.promise; },
    async onHandwritingFontProgress(callback: (progress: { id: string; received: number; total: number }) => void) { emit = callback; return () => { released++; }; },
    async renderHandwriting() { return 'unused'; },
  });
  try {
    await openFonts(fixture.root);
    const bar = () => nodes(fixture.root).find(item => item.props.role === 'progressbar');
    assert.equal(bar(), undefined, 'no progress bar before a download starts');
    const downloading = button(fixture.root, '下载 原创字体2').props.onClick(); await flush();
    assert(bar(), 'a progress bar appears while the download runs');
    emit!({ id: 'two', received: 61_728, total: 123_456 }); await flush();
    assert.equal(bar()!.props['aria-valuenow'], 50);
    assert.match(text(bar()!), /50%/);
    assert.match(text(fixture.root), /正在下载并验证… 50%/);
    const installed = catalog(); installed.fonts[1]!.downloaded = true;
    install.resolve(installed); await downloading; await flush();
    assert.equal(released, 1, 'the progress subscription is released once the download settles');
    assert.equal(bar(), undefined);
  } finally { fixture.unmount(); }
});

test('failed font download remains retryable and completion after unmount never renders', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jobs: ReturnType<typeof deferred<HandwritingAssets>>[] = [], rendered: HandwritingOptions[] = [];
  const fixture = await mount({ handwritingAssets: async () => catalog(), downloadHandwritingFont() { const job = deferred<HandwritingAssets>(); jobs.push(job); return job.promise; }, async renderHandwriting(options: HandwritingOptions) { rendered.push(options); return 'unused'; } });
  await openFonts(fixture.root);
  const failed = button(fixture.root, '下载 原创字体1').props.onClick(); await flush();
  jobs[0]!.reject(new Error('原创测试：网络不可用')); await failed; await flush();
  assert.match(text(fixture.root), /网络不可用/); assert.equal(button(fixture.root, '下载 原创字体1').props.disabled, false);
  assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, true);
  const late = button(fixture.root, '下载 原创字体1').props.onClick(); await flush(); fixture.unmount();
  const installed = catalog(); installed.fonts[0]!.downloaded = true;
  jobs[1]!.resolve(installed); await late; await flush(); t.mock.timers.tick(5000); await flush();
  assert.deepEqual(rendered, []);
});

test('browser font catalog is visibly preview-only and cannot download or render', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let downloads = 0;
  const fixture = await mount({ handwritingAssets: async () => catalog(), async downloadHandwritingFont() { downloads++; } }, false);
  try {
    assert.match(text(fixture.root), /浏览器仅供预览/);
    await openFonts(fixture.root);
    assert.match(text(nodes(fixture.root).find(item => item.type === 'dialog')!), /浏览器仅供预览/);
    assert.match(text(button(fixture.root, '下载 原创字体1')), /仅桌面版可下载/);
    assert.equal(button(fixture.root, '下载 原创字体1').props.disabled, true);
    await button(fixture.root, '下载 原创字体1').props.onClick(); await flush(); t.mock.timers.tick(5000); await flush();
    assert.equal(downloads, 0); assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, true);
  } finally { fixture.unmount(); }
});

test('paper picker installs only the chosen paper and switches the rendered snapshot after verification', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const assets = catalog(); assets.fonts[0]!.downloaded = true;
  assets.backgrounds = [{ id: 'sample', name: '原创图案纸', path: 'paper:sample', downloaded: false, bytes: 4567, previewUrl: '/handwriting-papers/sample.png', width: 724, height: 1024, ink: '#24324a', padding: 12 }];
  const install = deferred<HandwritingAssets>();
  const downloads: string[] = [], rendered: HandwritingOptions[] = [];
  const fixture = await mount({
    handwritingAssets: async () => assets,
    async onHandwritingPaperProgress() { return () => {}; },
    downloadHandwritingPaper(id: string) { downloads.push(id); return install.promise; },
    async renderHandwriting(options: HandwritingOptions) { rendered.push(options); return 'original-rendered-png'; },
  });
  try {
    t.mock.timers.tick(350); await flush();
    assert.deepEqual(downloads, []);
    assert.equal(rendered.at(-1)!.background, 'solid:#f8f1e3:1200:1600');
    await button(fixture.root, '选择纸张').props.onClick(); await flush();
    assert.deepEqual(downloads, [], 'opening thumbnails never downloads originals');
    const job = button(fixture.root, '使用纸张 原创图案纸').props.onClick(); await flush();
    assert.deepEqual(downloads, ['sample']);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, true);
    assert.equal(rendered.at(-1)!.background, 'solid:#f8f1e3:1200:1600');
    const installed = structuredClone(assets); installed.backgrounds[0]!.downloaded = true;
    install.resolve(installed); await job; await flush(); t.mock.timers.tick(350); await flush();
    assert.equal(rendered.at(-1)!.background, 'paper:sample');
    assert.equal(rendered.at(-1)!.padding, 12);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, false, text(fixture.root));
    await button(fixture.root, '选择纸张').props.onClick(); await flush();
    await button(fixture.root, '使用纸张 原创图案纸').props.onClick(); await flush();
    assert.deepEqual(downloads, ['sample'], 'downloaded paper can be selected offline');
  } finally { fixture.unmount(); }
});

test('name, download and handwriting filters combine without changing the current offline preview or export', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const assets = catalog(); assets.fonts[0]!.downloaded = true;
  let downloads = 0;
  const rendered: HandwritingOptions[] = [], saved: unknown[][] = [];
  const fixture = await mount({ handwritingAssets: async () => assets, async downloadHandwritingFont() { downloads++; }, async renderHandwriting(options: HandwritingOptions) { rendered.push(options); return 'original-offline-preview'; }, async saveExport(...args: unknown[]) { saved.push(args); return true; } });
  try {
    t.mock.timers.tick(350); await flush();
    assert.equal(rendered.length, 1);
    assert.equal(rendered[0]!.font, 'one');
    await openFonts(fixture.root);
    await button(fixture.root, '已下载字体').props.onClick(); await flush();
    assert.match(text(fixture.root), /显示 1 \/ 2 款/);
    const search = nodes(fixture.root).find(item => item.type === 'input' && item.props.type === 'search');
    assert(search);
    Object.assign(search, { value: '原创字体2' });
    search.props['onUpdate:modelValue']('原创字体2'); await flush();
    assert.match(text(fixture.root), /没有找到匹配的字体/);
    await button(fixture.root, '全部字体').props.onClick(); await flush();
    assert.equal(button(fixture.root, '下载 原创字体2').props.disabled, false);
    assert.match(text(fixture.root), /正在使用：原创字体1/);
    t.mock.timers.tick(1000); await flush();
    assert.equal(downloads, 0);
    assert.equal(rendered.length, 1);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, false);
    Object.assign(search, { value: '' });
    search.props['onUpdate:modelValue'](''); await flush();
    assert.equal(button(fixture.root, '使用 原创字体1').props['aria-pressed'], true);
    await button(fixture.root, '手写体').props.onClick(); await flush();
    assert.equal(button(fixture.root, '手写体').props['aria-pressed'], true);
    assert.equal(nodes(fixture.root).filter(item => item.type === 'article').length, 1);
    assert.equal(button(fixture.root, '下载 原创字体2').props.disabled, false);
    await button(fixture.root, '已下载字体').props.onClick(); await flush();
    assert.match(text(fixture.root), /当前手写体筛选下没有匹配的字体/);
    assert.doesNotMatch(text(fixture.root), /还没有下载字体/);
    Object.assign(search, { value: '原创字体2' });
    search.props['onUpdate:modelValue']('原创字体2'); await flush();
    assert.equal(nodes(fixture.root).filter(item => item.type === 'article').length, 0);
    await button(fixture.root, '全部字体').props.onClick(); await flush();
    assert.equal(nodes(fixture.root).filter(item => item.type === 'article').length, 1);
    assert.equal(button(fixture.root, '下载 原创字体2').props.disabled, false);
    Object.assign(search, { value: '原创字体1' });
    search.props['onUpdate:modelValue']('原创字体1'); await flush();
    assert.match(text(fixture.root), /当前手写体筛选下没有匹配的字体/);
    assert.match(text(fixture.root), /正在使用：原创字体1/);
    assert.equal(button(fixture.root, '导出月光笺 PNG').props.disabled, false);
    await button(fixture.root, '导出月光笺 PNG').props.onClick(); await flush();
    assert.equal(saved[0]?.[0], 'original-offline-preview');
    await button(fixture.root, '手写体').props.onClick(); await flush();
    assert.equal(button(fixture.root, '使用 原创字体1').props['aria-pressed'], true);
    t.mock.timers.tick(1000); await flush();
    assert.equal(downloads, 0);
    assert.equal(rendered.length, 1);
  } finally { fixture.unmount(); }
});
