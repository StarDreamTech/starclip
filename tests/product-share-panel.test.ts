import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, nextTick } from 'vue';
import { locale, setLocale, t } from '../src/i18n/index.ts';

test.afterEach(() => setLocale('zh-CN', false));

type Node = { type: string; text: string; props: Record<string, any>; children: Node[]; parent?: Node };
function node(type: string, text = ''): Node { return { type, text, props: {}, children: [] }; }
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
function preview(root: Node) { return nodes(root).find(item => item.type === 'figure')!.children.find(item => item.type === 'img')!; }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
let sequence = 0;
async function mount(mock: Record<string, unknown>, props: Record<string, unknown> = {}) {
  Object.assign(globalThis, { productSharePanelMock: mock });
  const api = 'const api=globalThis.productSharePanelMock; export const { copyProductShareImage, copyProductShareText, saveProductShareImage, shareImages, shareImageSize, shareImageSource, shareTexts }=api;';
  const apiUrl = 'data:text/javascript;base64,' + Buffer.from(api + `\n// ${sequence++}`).toString('base64');
  const { descriptor } = parse(fs.readFileSync(new URL('../src/components/ProductShare.vue', import.meta.url), 'utf8'));
  const script = compileScript(descriptor, { id: 'product-share-test', inlineTemplate: true });
  const code = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText
    .replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replace("import { api } from '../api';", 'const api=globalThis.productSharePanelMock;')
    .replaceAll("'../product-share'", `'${apiUrl}'`)
    .replaceAll("'./Icon.vue'", "'data:text/javascript,export default {render(){return null}}'").replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`).replaceAll("'./i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  const root = node('root'); const app = renderer.createApp(component.default, props); const vm = app.mount(root); await flush();
  return { root, app, vm };
}
function mockApi() {
  const calls: string[] = [];
  return {
    calls, shareTexts: ['zh-CN', 'en-US'].flatMap(locale => [
      { id: `short${locale === 'en-US' ? '-en' : ''}`, locale, label: '简短推荐', content: locale === 'en-US' ? 'Original short English copy' : '原创文案 <script>不会执行</script>' },
      { id: `detailed${locale === 'en-US' ? '-en' : ''}`, locale, label: '产品介绍', content: locale === 'en-US' ? 'Original detailed English copy' : '原创详细文案' },
    ]),
    shareImages: ['zh-CN', 'en-US'].flatMap(locale => ['overview', 'workflow', 'companion', 'creation', 'ai'].map((topic, i) => {
      const id = `${topic}${locale === 'en-US' ? '-en' : ''}`;
      return { id, locale, label: ['产品总览', '数据自主', '星伴陪伴', '创作图片', 'AI 授权'][i], description: '原创产品示例', path: `share/${id}.png`, filename: `${id}.png`, width: 1080, height: 1440, bytes: 102400 };
    })),
    shareImageSize: () => '100 KB', shareImageSource: (image: { path: string }) => `/${image.path}`,
    async favoriteShareImages(language: string) { calls.push(`favoriteImages:${language}`); }, async favoriteShareText(id: string) { calls.push(`favoriteText:${id}`); },
    async copyProductShareText(id: string) { calls.push(`text:${id}`); }, async copyProductShareImage(id: string) { calls.push(`image:${id}`); },
    async saveProductShareImage(id: string): Promise<{ kind: string; path?: string; filename?: string }> { calls.push(`save:${id}`); return { kind: 'cancelled' }; },
  };
}

test('sharing panel previews fixed materials, locks duplicate actions and reports only completed copy', async () => {
  const mock = mockApi(); const busy: boolean[] = []; let finish!: () => void;
  mock.copyProductShareImage = id => { mock.calls.push(`image:${id}`); return new Promise<void>(resolve => { finish = resolve; }); };
  const { root, app, vm } = await mount(mock, { onBusy: (value: boolean) => busy.push(value) });
  assert.deepEqual(mock.calls, []);
  assert.match(text(root), /<script>不会执行<\/script>/); assert.equal(nodes(root).some(item => item.type === 'script'), false);
  await button(root, '产品介绍').props.onClick(); await flush(); await button(root, '复制文案').props.onClick(); await flush();
  assert.deepEqual(mock.calls, ['text:detailed']); assert.match(text(root), /文案已复制/);
  const copiedNotice = nodes(root).find(item => item.props.role === 'status' && text(item).includes('文案已复制'))!;
  const textCopyButton = button(root, '复制文案');
  assert.equal(textCopyButton.parent, copiedNotice.parent);
  assert.equal(copiedNotice.parent!.children.indexOf(copiedNotice), textCopyButton.parent!.children.indexOf(textCopyButton) + 1,
    'text copy receipt must stay beside its button, above the tall image preview');
  await button(root, '星伴陪伴').props.onClick(); await flush();
  const copying = button(root, '复制图片').props.onClick(); await flush();
  assert.equal(busy.at(-1), true); assert.equal(vm.canRelease, false); assert.equal(button(root, '保存图片').props.disabled, true);
  assert.equal(button(root, 'English').props.disabled, true);
  await button(root, 'English').props.onClick(); await flush();
  assert.equal(preview(root).props.src, '/share/companion.png', 'pending work cannot change material language');
  await button(root, '正在复制…').props.onClick();
  assert.deepEqual(mock.calls, ['text:detailed', 'image:companion']);
  assert.doesNotMatch(text(root), /「星伴陪伴」已复制/);
  finish(); await copying; await flush(); assert.equal(busy.at(-1), false); assert.equal(vm.canRelease, true); assert.match(text(root), /「星伴陪伴」已复制/);
  app.unmount();
});

test('sharing panel distinguishes cancelled save, actual file path, browser download and copy failure', async () => {
  const mock = mockApi(); const { root, app } = await mount(mock);
  await button(root, '保存图片').props.onClick(); await flush(); assert.match(text(root), /已取消保存，没有写入文件/);
  mock.saveProductShareImage = async () => ({ kind: 'saved', path: '/synthetic-output/产品图.png' });
  // Exported API functions are bound when loaded; vary the same function through a new mount.
  app.unmount(); const saved = await mount(mock);
  await button(saved.root, '保存图片').props.onClick(); await flush(); assert.match(text(saved.root), /图片已保存：\/synthetic-output\/产品图.png/); saved.app.unmount();
  mock.saveProductShareImage = async () => ({ kind: 'downloaded', filename: 'overview.png' });
  mock.copyProductShareText = async () => { throw new Error('测试：剪贴板权限被拒绝'); };
  const download = await mount(mock);
  await button(download.root, '保存图片').props.onClick(); await flush(); assert.match(text(download.root), /已发起下载：overview.png/);
  await button(download.root, '复制文案').props.onClick(); await flush();
  assert.match(text(download.root), /剪贴板权限被拒绝/); assert.doesNotMatch(text(download.root), /文案已复制/);
  assert.equal(button(download.root, '复制文案').props.disabled, false); download.app.unmount();
});


test('share materials follow interface language by default and use the selected language for every action', async () => {
  const mock = mockApi(); const { root, app } = await mount(mock);
  try {
    assert.equal(button(root, '跟随界面').props['aria-pressed'], true);
    assert.equal(preview(root).props.src, '/share/overview.png');
    await button(root, '产品介绍').props.onClick();
    await button(root, '星伴陪伴').props.onClick(); await flush();

    setLocale('en-US', false); await flush();
    assert.match(text(root), /Original detailed English copy/);
    assert.equal(preview(root).props.src, '/share/companion-en.png', 'language change retains the selected subject');
    const imageOptions = nodes(root).find(item => item.props['aria-label'] === t('产品介绍图'))!;
    assert.equal(imageOptions.children.filter(item => item.type === 'button').length, 5);
    for (const label of ['复制文案', '复制图片', '保存图片']) {
      await button(root, t(label)).props.onClick(); await flush();
    }
    assert.deepEqual(mock.calls, ['text:detailed-en', 'image:companion-en', 'save:companion-en']);

    await button(root, '简体中文').props.onClick(); await flush();
    assert.equal(locale.value, 'en-US', 'material language does not change the interface');
    assert.match(text(root), /原创详细文案/);
    assert.equal(preview(root).props.src, '/share/companion.png');
    for (const label of ['复制文案', '复制图片', '保存图片']) {
      await button(root, t(label)).props.onClick(); await flush();
    }
    assert.deepEqual(mock.calls.slice(3), ['text:detailed', 'image:companion', 'save:companion']);

    setLocale('zh-CN', false); await flush();
    await button(root, 'English').props.onClick(); await flush();
    assert.match(text(root), /Original detailed English copy/);
    assert.equal(preview(root).props.src, '/share/companion-en.png');
    await button(root, '跟随界面').props.onClick(); await flush();
    assert.match(text(root), /原创详细文案/);
    assert.equal(preview(root).props.src, '/share/companion.png');
  } finally { app.unmount(); }
});


test('share favorites collect the selected text and all images in the chosen material language', async()=>{
  const mock=mockApi();const {root,app}=await mount(mock);
  await button(root,'English').props.onClick();await flush();
  await button(root,'产品介绍').props.onClick();await flush();
  await button(root,'收藏这段文案').props.onClick();await flush();
  assert.deepEqual(mock.calls,['favoriteText:detailed-en']);assert.match(text(root),/文案已加入星标收藏/);
  await button(root,'收藏本语言全部图片').props.onClick();await flush();
  assert.deepEqual(mock.calls,['favoriteText:detailed-en','favoriteImages:en-US']);assert.match(text(root),/已收藏为一条图集/);
  app.unmount();
});
