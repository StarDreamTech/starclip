import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

test('legacy file backups keep path types, order and separate text identities without native I/O', async () => {
  Object.assign(globalThis, { window: {} });
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replace("from './privacy'", `from '${new URL('../src/privacy.ts', import.meta.url).href}'`)
    .replace("from './snippets'", `from '${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replace("from './i18n/index.ts'", `from '${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  const { api } = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  for (const clip of (await api.getState()).clips) if (clip.kind === 'image') await api.deleteClip(clip.id);
  const entries = [{ path: 'C:\\Original\\原创目录', entryKind: 'directory' }, { path: '/original-fixture/missing.txt', entryKind: 'unknown' }];
  const content = entries.map(entry => entry.path).join('\n');
  const base = { id: 900, content, title: '原创文件样例', kind: 'file', entries, language: null, pinned: true, keepForever: true, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 3 };
  const backup = (clips: unknown[], version = 4) => JSON.stringify({ format: 'starclip', version, exportedAt: 1, groups: [], clips });
  assert.equal(await api.importBackup(backup([base, { ...base, id: 901, kind: 'text', entries: undefined }])), 2);
  const history = await api.getState({ query: 'missing.txt' });
  assert.equal(history.history.total, 2);
  const file = history.clips.find((clip: { kind: string }) => clip.kind === 'file');
  assert.deepEqual(file.entries, entries);
  assert.equal((await api.getState({ filter: 'file' })).history.total, 1);
  assert.equal((await api.getState({ aiEligible: true })).clips.some((clip: { id: number }) => clip.id === file.id), false);
  await assert.rejects(api.saveCode(file.id, '', 'original code', 'plaintext', null));
  assert.equal(await api.importBackup(backup([base])), 0);
  const exported = JSON.parse(await api.exportBackup());
  assert.equal(exported.version, 4);
  assert.deepEqual(exported.clips.find((clip: { id: number }) => clip.id === file.id).entries, entries);

  for (const broken of [
    { ...base, entries: undefined }, { ...base, entries: [] }, { ...base, entries: [entries[1], entries[0]] },
    { ...base, content: 'relative.txt', entries: [{ path: 'relative.txt', entryKind: 'file' }] },
    { ...base, content: '/original-fixture/a', entries: [{ path: '/original-fixture/a', entryKind: 'guessed' }] },
    { ...base, entries, kind: 'text' },
  ]) {
    await assert.rejects(api.importBackup(backup([broken])));
    const after = JSON.parse(await api.exportBackup());
    assert.deepEqual({ ...after, exportedAt: 0 }, { ...exported, exportedAt: 0 }, 'invalid import leaves the entire backup unchanged');
  }
  const paths = JSON.parse(fs.readFileSync(new URL('./fixtures/file-paths.json', import.meta.url), 'utf8')) as Array<{ path: string; valid: boolean }>;
  for (const { path, valid } of paths) {
    const importing = api.importBackup(backup([{ ...base, content: path, entries: [{ path, entryKind: 'unknown' }] }]));
    if (valid) await assert.doesNotReject(importing, path); else await assert.rejects(importing, undefined, path);
  }
  for (const path of ['\\\\..\\share\\original.txt', '/' + 'a'.repeat(262143)]) {
    await assert.rejects(api.importBackup(backup([{ ...base, content: path, entries: [{ path, entryKind: 'unknown' }] }])));
  }
  const oldMain = { ...base, id: 902, content: '/original-fixture/old-main.txt', entries: undefined };
  assert.equal(await api.importBackup(backup([oldMain], 3)), 1);
  const old = (await api.getState({ filter: 'file', query: 'old-main.txt' })).clips[0];
  assert.deepEqual(old.entries, [{ path: oldMain.content, entryKind: 'unknown' }]);
});
