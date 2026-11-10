import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, h, nextTick, reactive } from 'vue';

type Node = { type: string; text: string; props: Record<string, any>; children: Node[]; parent?: Node };
const node = (type: string, text = ''): Node => ({ type, text, props: {}, children: [] });
const renderer = createRenderer<Node, Node>({
  createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
  setText: (n, text) => { n.text = text }, setElementText: (n, text) => { n.text = text; n.children = [] },
  patchProp: (n, key, _previous, value) => { n.props[key] = value },
  insert: (child, parent, anchor) => { child.parent = parent; const i = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(i < 0 ? parent.children.length : i, 0, child) },
  remove: child => { const i = child.parent?.children.indexOf(child) ?? -1; if (i >= 0) child.parent!.children.splice(i, 1) },
  parentNode: n => n.parent || null, nextSibling: n => n.parent?.children[(n.parent?.children.indexOf(n) ?? -1) + 1] || null,
});
const nodes = (n: Node): Node[] => [n, ...n.children.flatMap(nodes)];
const text = (n: Node): string => n.type === '#comment' ? '' : n.text + n.children.map(text).join('');
const url = (s: string) => 'data:text/javascript;base64,' + Buffer.from(ts.transpileModule(s, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText).toString('base64');
const flush = async () => { await new Promise(resolve => setImmediate(resolve)); await nextTick() };

test('batch UI requires private consent, retains original order, invalidates prepared drag and handles cancellation/failure', async context => {
  const writes: any[] = [];
  let privateImage = true, fail = false;
  Object.assign(globalThis, { imageBatchMock: {
    isDesktop: true,
    api: {
      async getClips(ids: number[]) { return ids.map(id => ({ id, kind: 'image', isPrivate: privateImage && id === 3 })) },
      async copyImageFiles(ids: number[], allow: boolean) { if (fail) throw new Error('original fixed failure'); writes.push(['copy', ids, allow]) },
      async prepareImageTransfer(ids: number[], allow: boolean) { writes.push(['prepare', ids, allow]); return 'original-token' },
      async dragImageFiles(token: string) { writes.push(['drag', token]); return false },
      async createBundle(ids: number[], text: string, title: string, allow: boolean) { writes.push(['collect',ids,text,title,allow]);return {id:20,kind:'bundle'}; },
      async exportImageFiles(ids: number[], allow: boolean) { writes.push(['export', ids, allow]); return null },
    },
  } });
  const descriptor = parse(fs.readFileSync(new URL('../src/components/ImageBatchBar.vue', import.meta.url), 'utf8')).descriptor;
  const source = compileScript(descriptor, { id: 'image-batch-test', inlineTemplate: true }).content
    .replaceAll("from 'vue'", `from '${import.meta.resolve('vue')}'`).replaceAll('from "vue"', `from '${import.meta.resolve('vue')}'`)
    .replace("import { api, isDesktop } from '../api'", 'const { api, isDesktop } = globalThis.imageBatchMock')
    .replace("'../privacy'", `'${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replace("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = (await import(url(source))).default;
  const root = node('root');
  const props = reactive({ ids: [3, 1], disabled: false, pageIds: [3, 1] });
  const mounted = renderer.createApp({ render: () => h(component, props) });
  context.after(() => { mounted.unmount(); Reflect.deleteProperty(globalThis, 'imageBatchMock') });
  mounted.mount(root);
  const button = (label: string) => { const b = nodes(root).find(n => n.type === 'button' && text(n) === label); assert(b, label); return b };
  await button('复制为图片文件').props.onClick(); await flush();
  assert.deepEqual(writes, [], 'private images cannot transfer before explicit consent');
  const checkbox = nodes(root).find(n => n.type === 'input' && n.props.type === 'checkbox'); assert(checkbox);
  checkbox.props.onChange({ target: { checked: true } }); await flush();
  await button('复制为图片文件').props.onClick(); await flush();
  assert.deepEqual(writes.pop(), ['copy', [3, 1], true]);
  await button('准备拖出').props.onClick(); await flush();
  assert.deepEqual(writes.pop(), ['prepare', [3, 1], true]);
  await button('拖出图片').props.onMousedown({ button: 0, preventDefault() {} }); await flush();
  assert.deepEqual(writes.pop(), ['drag', 'original-token']);
  assert(text(root).includes('已取消拖出'));
  props.ids = [1]; privateImage = false; await flush();
  assert(!nodes(root).some(n => n.type === 'input' && n.props.type === 'checkbox'));
  assert(button('准备拖出'));
  await button('导出到文件夹').props.onClick(); await flush();
  assert.deepEqual(writes.pop(), ['export', [1], false]);
  assert(text(root).includes('已取消导出'));
  const caption=nodes(root).find(n=>n.type==='textarea')!;
  caption.props.onInput({target:{value:'Original caption'}});
  const title=nodes(root).find(n=>n.type==='input'&&n.props.type!=='checkbox')!;
  title.props.onInput({target:{value:'Original album'}});await flush();
  await button('保存图集并星标收藏').props.onClick();await flush();
  assert.deepEqual(writes.pop(),['collect',[1],'Original caption','Original album',false]);
  fail = true;
  await button('复制为图片文件').props.onClick(); await flush();
  assert(text(root).includes('original fixed failure'));
  assert.equal(button('复制为图片文件').props.disabled, false);
});
