import test from 'node:test';
import type { TestContext } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import type { StarPetPreferences } from '../src/starpet-api.ts';

type Node = { type: string; text: string; props: Record<string, any>; children: Node[]; parent?: Node };
const node = (type: string, text = ''): Node => ({ type, text, props: {}, children: [] });
const renderer = createRenderer<Node, Node>({
  createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
  setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.text = text; node.children = []; },
  patchProp: (node, key, _previous, value) => { node.props[key] = value; },
  insert: (child, parent, anchor) => { const previous = child.parent?.children.indexOf(child) ?? -1; if (previous >= 0) child.parent!.children.splice(previous, 1); child.parent = parent; const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index >= 0 ? index : parent.children.length, 0, child); },
  remove: child => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); },
  parentNode: node => node.parent || null, nextSibling: node => node.parent?.children[(node.parent?.children.indexOf(node) ?? -1) + 1] || null,
});
const nodes = (root: Node): Node[] => [root, ...root.children.flatMap(nodes)];
const text = (root: Node): string => root.type === '#comment' ? '' : root.text + root.children.map(text).join('');
function button(root: Node, label: string) { const found = nodes(root).find(item => item.type === 'button' && (item.props['aria-label'] === label || text(item) === label)); assert(found, `missing ${label}`); return found; }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
function moduleUrl(source: string) { return 'data:text/javascript;base64,' + Buffer.from(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText).toString('base64'); }

let sequence = 0;
async function mount(context: TestContext, options: {
  initial?: Partial<StarPetPreferences>;
  write?: (patch: Partial<StarPetPreferences>, current: StarPetPreferences) => Promise<StarPetPreferences>;
} = {}) {
  const savedGlobal = Object.getOwnPropertyDescriptor(globalThis, 'starPetSettingsMock');
  let app: { unmount(): void } | undefined;
  let preferences: StarPetPreferences = { enabled: true, mood: 'sad', color: '#FFD66E', size: 100, ...options.initial };
  let changed = (_value: StarPetPreferences) => {};
  const writes: Partial<StarPetPreferences>[] = [];
  context.after(() => {
    app?.unmount();
    if (savedGlobal) Object.defineProperty(globalThis, 'starPetSettingsMock', savedGlobal); else Reflect.deleteProperty(globalThis, 'starPetSettingsMock');
  });
  Object.assign(globalThis, { starPetSettingsMock: {
    starPetDesktopAvailable: true,
    async getStarPetPreferences() { return { ...preferences }; },
    async onStarPetChanged(callback: typeof changed) { changed = callback; return () => { changed = () => {}; }; },
    async setStarPetPreferences(patch: Partial<StarPetPreferences>) {
      writes.push({ ...patch });
      if (options.write) return options.write(patch, { ...preferences });
      preferences = { ...preferences, ...patch };
      changed({ ...preferences });
      return { ...preferences };
    },
  } });
  const api = moduleUrl(`export const { starPetDesktopAvailable, getStarPetPreferences, onStarPetChanged, setStarPetPreferences } = globalThis.starPetSettingsMock; // ${sequence++}`);
  const empty = moduleUrl('export default {render(){return null}}');
  const pet = moduleUrl(`import { h } from '${import.meta.resolve('vue')}'; export default {props:['mood'],render(){return h('pet-fixture',{...this.$props})}}`);
  const select = moduleUrl(`import { h } from '${import.meta.resolve('vue')}'; export default {props:['modelValue','options'],emits:['update:modelValue'],render(){return h('select-fixture',{...this.$props,change:value=>this.$emit('update:modelValue',value)})}}`);
  const { descriptor } = parse(fs.readFileSync(new URL('../src/components/StarPetSettings.vue', import.meta.url), 'utf8'));
  let source = compileScript(descriptor, { id: 'starpet-settings-test', inlineTemplate: true }).content
    .replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replaceAll("'../starpet-api'", `'${api}'`).replaceAll("'./StarPet.vue'", `'${pet}'`).replaceAll("'./UiSelect.vue'", `'${select}'`);
  for (const path of ['starpet', 'starpet-growth', 'starpet-size', 'i18n/index']) source = source.replaceAll(`'../${path === 'i18n/index' ? 'i18n' : path}'`, `'${new URL(`../src/${path}.ts`, import.meta.url).href}'`);
  for (const path of ['Icon', 'UiColor', 'StarPetGrowth']) source = source.replaceAll(`'./${path}.vue'`, `'${empty}'`);
  const component = await import(moduleUrl(source));
  const root = node('root');
  const mounted = renderer.createApp(component.default); app = mounted; mounted.mount(root); await flush();
  return {
    root, writes,
    current: () => ({ ...preferences }),
    stage: () => nodes(root).find(item => item.type === 'pet-fixture')!.props.mood,
    choices: () => nodes(root).filter(item => item.type === 'button' && item.props['aria-label']?.startsWith('选择')),
    other: () => nodes(root).find(item => item.type === 'input' && item.props.type === 'checkbox')!,
    category: () => nodes(root).find(item => item.type === 'select-fixture')!,
    changed(patch: Partial<StarPetPreferences>) { preferences = { ...preferences, ...patch }; changed({ ...preferences }); },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('manual mood choices save immediately as a mood-only patch and other windows keep the gallery in sync', async context => {
  const initial: StarPetPreferences = { enabled: true, mood: 'love', color: '#AABBCC', size: 140 };
  const fixture = await mount(context, { initial });
  assert.equal(fixture.stage(), 'love'); assert.deepEqual(fixture.writes, []);
  await button(fixture.root, '选择有点好奇').props.onClick(); await flush();
  assert.deepEqual(fixture.writes, [{ mood: 'curious' }]);
  assert.deepEqual(fixture.current(), { ...initial, mood: 'curious' });
  assert.equal(fixture.stage(), 'curious');
  assert.equal(button(fixture.root, '选择有点好奇').props['aria-pressed'], true);
  assert.match(text(fixture.root), /表情已保存/);
  fixture.changed({ mood: 'reading' }); await flush();
  assert.equal(fixture.stage(), 'reading');
  assert.equal(button(fixture.root, '选择有点好奇').props['aria-pressed'], false);
  assert.deepEqual(fixture.writes, [{ mood: 'curious' }], 'an incoming window event never writes back');
});

test('choosing a mood while the desktop pet is off saves it without enabling the pet', async context => {
  const initial: StarPetPreferences = { enabled: false, mood: 'idle', color: '#AABBCC', size: 70 };
  const fixture = await mount(context, { initial });
  await button(fixture.root, '选择有点好奇').props.onClick(); await flush();
  assert.deepEqual(fixture.writes, [{ mood: 'curious' }]);
  assert.deepEqual(fixture.current(), { ...initial, mood: 'curious' });
  assert.equal(fixture.stage(), 'curious');
  assert.ok(button(fixture.root, '让星芽住在桌面'));
  await button(fixture.root, '选择有点好奇').props.onClick(); await flush();
  assert.equal(fixture.writes.length, 1, 'clicking the saved mood needs no duplicate write');
});

test('a failed mood save returns to the latest saved mood even after another field changes', async context => {
  const pending = deferred<StarPetPreferences>();
  const fixture = await mount(context, { initial: { mood: 'love' }, write: () => pending.promise });
  const clicking = button(fixture.root, '选择有点好奇').props.onClick(); await flush();
  assert.equal(fixture.stage(), 'curious');
  assert.equal(button(fixture.root, '选择有点好奇').props.disabled, true);
  fixture.changed({ color: '#AABBCC', size: 140 }); await flush();
  assert.equal(fixture.stage(), 'curious', 'a color/size event does not apply a different mood');
  pending.reject(new Error('original fixed mood write failure')); await clicking; await flush();
  assert.equal(fixture.stage(), 'love', 'the failed candidate is no longer selected');
  assert.equal(button(fixture.root, '选择有点好奇').props['aria-pressed'], false);
  assert.equal(button(fixture.root, '选择有点好奇').props.disabled, false);
  assert.deepEqual(fixture.current(), { enabled: true, mood: 'love', color: '#AABBCC', size: 140 });
  assert.match(text(fixture.root), /表情未能保存/);
});

test('newer preference events win over delayed save responses without rolling back another window', async context => {
  const initial: StarPetPreferences = { enabled: true, mood: 'love', color: '#FFD66E', size: 100 };
  const pending = deferred<StarPetPreferences>();
  const fixture = await mount(context, { initial, write: () => pending.promise });
  const clicking = button(fixture.root, '选择有点好奇').props.onClick(); await flush();
  fixture.changed({ enabled: false, mood: 'reading', color: '#AABBCC', size: 70 }); await flush();
  assert.equal(fixture.stage(), 'reading', 'a newer explicit mood is accepted during the pending save');
  pending.resolve({ ...initial, mood: 'curious' }); await clicking; await flush();
  assert.equal(fixture.stage(), 'reading', 'the late response cannot replace the newer selected mood');
  assert.deepEqual(fixture.current(), { enabled: false, mood: 'reading', color: '#AABBCC', size: 70 });
  assert.ok(button(fixture.root, '让星芽住在桌面'));
  assert.deepEqual(fixture.writes, [{ mood: 'curious' }]);
});

test('gallery starts with everyday moods, other expressions require a choice and automatic tours remain positive previews', async context => {
  context.mock.timers.enable({ apis: ['setInterval'] });
  const fixture = await mount(context);
  context.after(() => context.mock.timers.reset());
  const { root, writes, choices, stage, other, category } = fixture;
  assert.equal(choices().length, 26); assert.equal(stage(), 'sad', 'explicit saved mood is not reset on opening settings');
  assert.equal(choices().some(item => item.props['aria-label'] === '选择眼泪汪汪'), false);
  assert.deepEqual(writes, []);
  other().props.onChange({ target: { checked: true } }); await flush();
  assert.equal(choices().length, 32);
  await button(root, '选择眼泪汪汪').props.onClick(); await flush();
  assert.equal(stage(), 'crying');
  assert.equal(fixture.current().mood, 'crying', 'manual negative choice is saved explicitly');
  assert.deepEqual(writes, [{ mood: 'crying' }]);
  other().props.onChange({ target: { checked: false } }); await flush();
  assert.equal(choices().length, 26); assert.equal(stage(), 'crying', 'hiding the optional catalog does not change a chosen mood');
  category().props.change('daily'); await flush();
  assert.equal(stage(), 'idle', 'category browsing only changes the local preview');
  assert.equal(fixture.current().mood, 'crying');
  other().props.onChange({ target: { checked: true } }); await flush();
  category().props.change('all'); await flush();
  await button(root, '表情巡游').props.onClick(); await flush();
  const beforeEvent = stage();
  fixture.changed({ mood: 'thinking' }); await flush();
  assert.equal(stage(), beforeEvent, 'a background preference event does not interrupt the active tour');
  const encountered = new Set<string>();
  for (let step = 0; step < 52; step++) {
    assert.ok(!['puzzled', 'sad', 'crying', 'angry', 'nervous', 'error'].includes(stage()), stage());
    encountered.add(stage()); context.mock.timers.tick(2600); await flush();
  }
  assert.equal(encountered.size, 26, 'tour excludes other moods even when optional tiles are shown');
  category().props.change('emotion'); await flush();
  assert.equal(choices().length, 12, 'all optional moods remain selectable in their category');
  for (let step = 0; step < 14; step++) {
    assert.ok(['happy', 'delighted', 'love', 'shy', 'surprised', 'proud', 'relieved'].includes(stage()), stage());
    context.mock.timers.tick(2600); await flush();
  }
  assert.deepEqual(writes, [{ mood: 'crying' }], 'tour and categories never persist their temporary previews');
  assert.equal(fixture.current().mood, 'thinking');
});
