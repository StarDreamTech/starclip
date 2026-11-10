import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

let sequence = 0;
async function loadApi(desktop = false) {
  Object.assign(globalThis, { window: desktop ? { __TAURI_INTERNALS__: {} } : {} });
  const mock = 'data:text/javascript,export const invoke=(...args)=>globalThis.favoriteInvoke(...args);';
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replaceAll("'@tauri-apps/api/core'", `'${mock}'`)
    .replaceAll("'./privacy'", `'${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replaceAll("'./snippets'", `'${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replaceAll("'./i18n/index.ts'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  return (await import('data:text/javascript;base64,' + Buffer.from(code + `\n// ${sequence++}`).toString('base64'))).api;
}

test('favorite order covers all pages, supports both edges and pins without changing capture timestamps', async () => {
  const api = await loadApi();
  const clips = Array.from({ length: 130 }, (_, index) => ({ id: index + 100, content: `Original favorite fixture ${index}`, title: `Original favorite ${index}`, kind: 'text', language: null, pinned: true, keepForever: false, groupId: null, createdAt: index, updatedAt: index, copyCount: index % 9 }));
  await api.importBackup(JSON.stringify({ format: 'starclip', version: 5, clips, groups: [] }));
  const all = async (sort = 'manual') => {
    const result = [];
    for (let page = 1; page <= 3; page++) result.push(...(await api.getState({ filter: 'pinned', sort, page })).clips);
    return result;
  };
  const original = await all(), last = original.at(-1);
  await api.moveFavorite(last.id, original[0].id, true);
  assert.equal((await all())[0].id, last.id, 'reordering an off-page favorite works');
  await api.moveFavorite(last.id, null, false);
  assert.equal((await all()).at(-1).id, last.id);
  await api.setFavoriteTop(last.id, true);
  for (const sort of ['manual', 'copies', 'created', 'updated']) assert.equal((await all(sort))[0].id, last.id);
  assert.equal((await api.getClips([last.id]))[0].updatedAt, last.updatedAt);
  const other = (await all())[1];
  await assert.rejects(api.moveFavorite(other.id, last.id, true), /分别排序/);
  const plain = await api.addClip('Original unstarred fixture', '', null);
  await assert.rejects(api.setFavoriteTop(plain.id, true), /星标收藏/);
  const before = await api.getState();
  const invalid = { ...clips[0], favoritePosition: -1 };
  await assert.rejects(api.importBackup(JSON.stringify({ format: 'starclip', version: 5, clips: [invalid], groups: [] })), /收藏排序无效/);
  assert.deepEqual(await api.getState(), before);
});

test('native favorite actions use main-window commands with explicit edge and pin arguments', async () => {
  const calls: unknown[] = [];
  Object.assign(globalThis, { favoriteInvoke: async (...args: unknown[]) => { calls.push(args); } });
  const api = await loadApi(true);
  await api.setFavoriteTop(7, true);
  await api.moveFavorite(7, 9, false);
  await api.moveFavorite(7, null, true);
  assert.deepEqual(calls, [
    ['set_favorite_top', { id: 7, onTop: true }],
    ['move_favorite', { id: 7, target: 9, before: false }],
    ['move_favorite', { id: 7, target: null, before: true }],
  ]);
});
