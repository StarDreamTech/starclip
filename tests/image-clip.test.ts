import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

test('image demo keeps payload out of state, copies PNG, counts binary bytes and rejects lossy backups atomically', async () => {
  Object.assign(globalThis, { window: {} });
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replace("from './privacy'", `from '${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replace("from './snippets'", `from '${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replace("from './i18n/index.ts'", `from '${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  const { api } = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  const initial = await api.getState();
  const image = initial.clips.find((clip: { kind: string }) => clip.kind === 'image');
  assert(image);
  assert(!JSON.stringify(initial).includes('base64'), 'history must carry metadata only');
  const png = Buffer.from((await api.imageData(image.id, true)).split(',')[1], 'base64');
  assert.deepEqual([...png.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(png.readUInt32BE(16), image.image.width);
  assert.equal(png.readUInt32BE(20), image.image.height);
  assert.equal(png.length, image.image.bytes);
  assert.equal(await api.imageData(image.id, false), await api.imageData(image.id, true));
  await assert.rejects(api.imageData(initial.clips[0].id, false), /不是图片/);

  const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const clipboardItemDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'ClipboardItem');
  try {
    let textWrites = 0;
    const clipboard = { writeText: async () => { textWrites++; } };
    Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { clipboard } });
    await assert.rejects(api.copyClip(image.id), /不支持复制图片/);
    assert.equal(textWrites, 0, 'internal image identifiers must never be copied as text');
    assert.equal((await api.getState()).clips.find((clip: { id: number }) => clip.id === image.id).copyCount, 0);
    let written: Blob | undefined;
    class Item { data: Record<string, Blob>; constructor(data: Record<string, Blob>) { this.data = data; } }
    Object.defineProperty(globalThis, 'ClipboardItem', { configurable: true, value: Item });
    Object.assign(clipboard, { write: async (items: Item[]) => { written = items[0]!.data['image/png']; } });
    await api.copyClip(image.id);
    assert.equal(written?.type, 'image/png');
    assert.deepEqual(Buffer.from(await written!.arrayBuffer()), png);
    assert.equal((await api.getState()).clips.find((clip: { id: number }) => clip.id === image.id).copyCount, 1);
    Object.assign(clipboard, { write: async () => { throw new Error('permission denied'); } });
    await assert.rejects(api.copyClip(image.id), /允许剪贴板权限/);
    assert.equal((await api.getState()).clips.find((clip: { id: number }) => clip.id === image.id).copyCount, 1);
    assert.equal(textWrites, 0);
  } finally {
    if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor);
    else Reflect.deleteProperty(globalThis, 'navigator');
    if (clipboardItemDescriptor) Object.defineProperty(globalThis, 'ClipboardItem', clipboardItemDescriptor);
    else Reflect.deleteProperty(globalThis, 'ClipboardItem');
  }
  await assert.rejects(api.saveCode(image.id, '不能改成代码', image.content, 'text', null), /图片不能/);
  await api.updateClip(image.id, '图片的新名字', null, false);
  const preview = await api.previewCleanup({ minBytes: 0, olderThanDays: 0, kind: 'image', includeGrouped: false });
  const expectedBytes = png.length + Buffer.byteLength('图片的新名字');
  assert.equal(preview.matchedCount, 1);
  assert.equal(preview.contentBytes, expectedBytes, 'count original PNG and UTF-8 title, not the internal identifier');
  assert.equal((await api.previewCleanup({ minBytes: expectedBytes, olderThanDays: 0, kind: 'image', includeGrouped: false })).matchedCount, 0);
  await api.setKeepForever(image.id, true);
  assert.equal((await api.previewCleanup({ minBytes: 0, olderThanDays: 0, kind: 'image', includeGrouped: true })).matchedCount, 0);
  await assert.rejects(api.exportBackup(), /包含图片的备份/);

  const beforeImport = await api.getState();
  for (const backup of [
    { version: 2, clips: [], images: [] },
    { version: 1, clips: [image] },
    { version: 1, clips: [{ ...initial.clips[0], image: { width: 1, height: 1, bytes: 1 } }] },
    { version: 1, clips: [], images: [{ clipId: image.id, png: png.toString('base64') }] },
  ]) {
    await assert.rejects(api.importBackup(JSON.stringify({ format: 'starclip', groups: [{ id: 900, name: '不应写入', color: '#778899' }], ...backup })), /图片备份/);
    assert.deepEqual(await api.getState(), beforeImport, 'unsupported image backup must not partially import');
  }
  await api.setKeepForever(image.id, false);
  const cleanup = await api.previewCleanup({ minBytes: 0, olderThanDays: 0, kind: 'image', includeGrouped: false });
  const result = await api.executeCleanup(cleanup.token, false);
  assert.equal(result.deletedCount, 1);
  assert.equal(result.contentBytes, expectedBytes);
  await assert.rejects(api.imageData(image.id, false), /不存在/);
  assert.equal(JSON.parse(await api.exportBackup()).version, 5, 'text-only demo backup remains supported');
});
