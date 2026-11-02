import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, h, nextTick, ref } from 'vue';
import type { Clip } from '../src/types.ts';

type Node = { type: string; tagName: string; text: string; props: Record<string, any>; children: Node[]; parent?: Node; getRootNode: () => unknown; addEventListener: (...args: any[]) => void; removeEventListener: (...args: any[]) => void };
function node(type: string, text = ''): Node { return { type, tagName: type.toUpperCase(), text, props: {}, children: [], getRootNode() { return { activeElement: null }; }, addEventListener() {}, removeEventListener() {} }; }
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
function button(root: Node, label: string) { const found = nodes(root).find(item => item.type === 'button' && text(item) === label); assert(found, `missing ${label}`); return found; }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
function moduleUrl(source: string) { return 'data:text/javascript;base64,' + Buffer.from(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText).toString('base64'); }
let sequence = 0;
async function component(name: string, mock: Record<string, unknown>) {
  Object.assign(globalThis, { Document: class {}, ShadowRoot: class {}, privacyPanelApi: mock, document: { activeElement: null } });
  const apiUrl = moduleUrl(`export const api=globalThis.privacyPanelApi; export const aiApi=api; export const isDesktop=true; // ${sequence++}`);
  const helperUrl = moduleUrl(fs.readFileSync(new URL('../src/privacy.ts', import.meta.url), 'utf8'));
  const { descriptor } = parse(fs.readFileSync(new URL(`../src/components/${name}.vue`, import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'privacy-panel-test', inlineTemplate: true });
  const source = script.content.replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replaceAll("'../api'", `'${apiUrl}'`).replaceAll("'../ai-api'", `'${apiUrl}'`).replaceAll("'../privacy'", `'${helperUrl}'`)
    .replaceAll("'./HistoryPagination.vue'", "'data:text/javascript,export default {render(){return null}}'").replaceAll("'./UiSelect.vue'", "'data:text/javascript,export default {render(){return null}}'").replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`).replaceAll("'./i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  return (await import(moduleUrl(source))).default;
}

test('privacy panel explains named rule evidence and does not call an unrecognized image private', async () => {
  const item = ref<Clip>({ id: 1, title: '原创截图', content: 'image:fixture', kind: 'image', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0, scanVersion: 0, privacyReasons: [] });
  const Component = await component('ClipPrivacy', {});
  const root = node('root'); const app = renderer.createApp({ setup: () => () => h(Component, { clip: item.value, revision: 2 }) });
  app.mount(root); await flush();
  try {
    assert.match(text(root), /图片未识别文字 · 不向 AI 提供/);
    assert.match(text(root), /标记为隐私/);
    assert.doesNotMatch(text(root), /隐私 · 不向 AI 提供|复核并尝试解除/);
    item.value = { ...item.value, isPrivate: true, privacyReasons: ['私钥标记', '自定义规则：原创团队资料'] }; await flush();
    assert.match(text(root), /保护依据与命中规则/);
    assert.match(text(root), /PRIVATE KEY 私钥区块标记/);
    assert.match(text(root), /自定义规则：原创团队资料/);
    assert.match(text(root), /关键词或域名条件/);
    assert.equal(button(root, '复核并尝试解除').props.disabled, true);
  } finally { app.unmount(); }
});

test('rules panel preserves drafts after failed save, normalizes empty tags, and applies only an explicit successful scan preview', async () => {
  const rules = { revision: 1, rules: [{ id: 'one', name: '原创规则', enabled: true, condition: 'keyword', value: '原创', tag: '资料', markPrivate: true }] };
  let rejectSave!: (error: Error) => void; const calls: any[] = [], busy: boolean[] = [];
  const Component = await component('PrivacyRules', {
    async privacyRules() { return structuredClone(rules); },
    savePrivacyRules(value: unknown) { calls.push(value); return new Promise((_resolve, reject) => { rejectSave = reject; }); },
    async previewPrivacyRescan() { calls.push('preview'); return { token: 'fixture-token', total: 4, privateCount: 2, newlyPrivateCount: 1, changedCount: 2 }; },
    async applyPrivacyRescan(token: string) { calls.push(token); return { total: 4, privateCount: 2 }; },
  });
  const root = node('root'); const app = renderer.createApp(Component, { revision: 1, onBusy: (value: boolean) => busy.push(value) }); app.mount(root); await flush();
  nodes(root).find(item => item.props['aria-label'] === '规则 1 普通标签')!.props['onUpdate:modelValue'](' '); await flush();
  const saving = button(root, '保存规则').props.onClick(); await flush(); assert.equal(calls[0].rules[0].tag, null); assert.equal(busy.at(-1), true);
  await button(root, '保存规则').props.onClick(); assert.equal(calls.length, 1, 'duplicate saves blocked');
  rejectSave(new Error('原创测试：规则版本已变更')); await saving; await flush();
  assert.match(text(root), /规则版本已变更/); assert.match(text(root), /有未保存的规则草稿/); assert.equal(button(root, '预览历史扫描').props.disabled, true);
  await button(root, '重新读取').props.onClick(); await flush();
  await button(root, '预览历史扫描').props.onClick(); await flush(); assert.deepEqual(calls.slice(1), ['preview']);
  assert.match(text(root), /新增隐私 1 条/); await button(root, '确认扫描历史').props.onClick(); await flush();
  assert.deepEqual(calls.slice(1), ['preview', 'fixture-token']); assert.match(text(root), /已扫描 4 条/); app.unmount();
});

test('AI panel excludes protected, unscanned and image records, requires preview, and retains selection on native rejection', async () => {
  const base = { title: '原创普通资料', content: '原创正文', kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0, scanVersion: 2 } as Clip;
  const clips = ref<Clip[]>([{ ...base, id: 1 }, { ...base, id: 2, isPrivate: true, title: '不可显示私密标题' }, { ...base, id: 3, scanVersion: 1 }, { ...base, id: 4, kind: 'image' }, { ...base, id: 5, kind: 'file', title: '不可授权的文件路径' }]);
  const calls: any[] = []; let reject!: (error: Error) => void;
  const Component = await component('AiSettings', {
    async state() { return { enabled: true, available: true, connections: [], audit: [], limits: { maxRecords: 500, maxResults: 50, maxHours: 168 } }; },
    create(request: unknown) { calls.push(request); return new Promise((_resolve, failure) => { reject = failure; }); },
  });
  const root = node('root'); const app = renderer.createApp({ setup: () => () => h(Component, { clips: clips.value, groups: [], revision: 2 }) }); app.mount(root); await flush();
  assert.doesNotMatch(text(root), /不可显示私密标题|不可授权的文件路径/); assert.match(text(root), /隐私、图片、文件与待扫描记录不会出现在可授权列表/);
  await button(root, '选择当前结果').props.onClick(); await flush();
  assert.equal(calls.length, 0); await button(root, '预览本次授权').props.onClick(); await flush();
  assert.equal(calls.length, 0); assert.match(text(root), /开放 1 条记录/);
  assert.equal(nodes(root).some(item => item.type === 'pre'), false, 'collapsed records do not mount full content');
  const details = nodes(root).find(item => item.type === 'details')!;
  details.props.onToggle({ target: { open: true } }); await flush();
  assert.equal(nodes(root).find(item => item.type === 'pre')?.text, base.content);
  details.props.onToggle({ target: { open: false } }); await flush();
  assert.equal(nodes(root).some(item => item.type === 'pre'), false, 'closing a record releases its content node');
  const creating = button(root, '确认创建连接').props.onClick(); await flush();
  assert.deepEqual(calls[0].clipIds, [1]); assert.deepEqual(calls[0].fields, ['title', 'content']); assert.equal(calls[0].allowPetControl, false);
  await button(root, '确认创建连接').props.onClick(); assert.equal(calls.length, 1);
  reject(new Error('原创测试：记录已标为隐私')); await creating; await flush();
  assert.match(text(root), /记录已标为隐私/); assert.match(text(root), /开放 1 条记录/);
  clips.value = clips.value.map(clip => ({ ...clip, isPrivate: true })); await flush();
  assert.equal(nodes(root).some(item => item.type === 'button' && text(item) === '确认创建连接'), false, 'state change invalidates previously reviewed grant');
  assert.match(text(root), /已选 0/); app.unmount();
});


test('AI panel grants pet-only control without records or fields and previews it separately', async () => {
  const calls: any[] = [];
  const initial = { enabled: true, available: true, connections: [], audit: [], limits: { maxRecords: 500, maxResults: 50, maxHours: 168 } };
  const Component = await component('AiSettings', {
    async state() { return initial; },
    async create(request: unknown) { calls.push(request); return { state: initial, config: { mcp: {}, cli: 'fixture ai status', skill: 'original fixture' } }; },
  });
  const root = node('root'); const app = renderer.createApp(Component, { clips: [], groups: [], revision: 0 }); app.mount(root); await flush();
  try {
    assert.equal(button(root, '预览本次授权').props.disabled, true);
    const permission = nodes(root).find(item => item.props['aria-label'] === '允许星伴动作与消息')!;
    permission.props['onUpdate:modelValue'](true); await flush();
    assert.equal(button(root, '预览本次授权').props.disabled, false);
    await button(root, '预览本次授权').props.onClick(); await flush();
    assert.match(text(root), /不开放剪贴历史/); assert.match(text(root), /星伴动作与消息：允许/);
    await button(root, '确认创建连接').props.onClick(); await flush();
    assert.deepEqual(calls[0], { name: '我的 AI 助手', clipIds: [], fields: [], maxResults: 20, expiresHours: 24, allowPetControl: true });
    assert.equal(button(root, '预览本次授权').props.disabled, true, 'new connections do not inherit pet permission');
  } finally { app.unmount(); }
});

test('file details state their AI exclusion even when the path is an ordinary reviewed record', async () => {
  const Component = await component('ClipPrivacy', {});
  const clip = { id: 100, title: '原创文件', content: '/original-fixture/note.txt', kind: 'file', scanVersion: 1, tags: [], manualTags: [], privacyReasons: [] } as unknown as Clip;
  const root = node('root'); const app = renderer.createApp(Component, { clip, revision: 1 }); app.mount(root); await flush();
  try {
    assert.match(text(root), /文件记录 · 不向 AI 提供/);
    assert.doesNotMatch(text(root), /普通记录 · 需单独授权才供 AI 读取/);
  } finally { app.unmount(); }
});
