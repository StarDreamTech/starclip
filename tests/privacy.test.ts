import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { analyzeDemoClip, matchesMetadata, pendingClip, privacyReasonExplanation, protectedClip } from '../src/privacy.ts';
import type { Clip } from '../src/types.ts';
const clip = (content: string, extra: Partial<Clip> = {}): Clip => ({ id: 1, title: '', content, kind: 'text', language: null, pinned: false, keepForever: false, groupId: null, createdAt: 1, updatedAt: 1, copyCount: 0, ...extra });

test('images remain visible without OCR while manual protection and named rule evidence survive', () => {
  const original = clip('api_key=internal-image-identifier', { kind: 'image', isPrivate: true, privacyReasons: ['图片尚未进行文字识别'] });
  const image = analyzeDemoClip(original, { revision: 1, rules: [] });
  assert.equal(protectedClip(image), false);
  assert.equal(pendingClip(image, 1), true);
  assert.equal(matchesMetadata(image, [], '', 'private', 1), false);
  assert.equal(protectedClip(analyzeDemoClip({ ...original, privacyManual: true }, { revision: 1, rules: [] })), true);
  const protectedImage = analyzeDemoClip({ ...image, title: '原创内部资料' }, { revision: 2, rules: [{ id: 'work', name: '内部资料', enabled: true, condition: 'keyword', value: '内部', tag: null, markPrivate: true }] });
  assert.deepEqual(protectedImage.privacyReasons, ['自定义规则：内部资料']);
  assert.equal(protectedClip(protectedImage), true);
  assert.match(privacyReasonExplanation('自定义规则：内部资料'), /关键词或域名/);
  assert.match(privacyReasonExplanation('私钥标记'), /PRIVATE KEY/);
});

test('source filters use metadata, combine source OR with tag/privacy AND, and stale scans stay unavailable', () => {
  const item = clip('original text', { linkSources: ['wechat', 'bilibili'], linkHosts: ['mp.weixin.qq.com'], tags: ['工作'], scanVersion: 2 });
  assert.equal(matchesMetadata(item, ['x', 'wechat'], '工作', 'ordinary', 2), true);
  assert.equal(matchesMetadata(item, ['x'], '工作', 'all', 2), false);
  assert.equal(matchesMetadata(item, ['host:mp.weixin.qq.com'], '设计', 'all', 2), false);
  assert.equal(matchesMetadata(item, [], '', 'pending', 3), true);
  assert.equal(pendingClip(clip('old fixture'), 1), true);
  assert.equal(protectedClip({ ...item, privacyManual: true, isPrivate: false }), true);
  assert.equal(protectedClip({ ...item, privacyReasons: ['规则命中'], isPrivate: false }), true);
});

test('browser source demonstration rejects misleading hosts and keeps rule protection independent of tags', () => {
  const config = { revision: 1, rules: [{ id: 'demo', name: '原创工作规则', enabled: true, condition: 'domain' as const, value: 'linux.do', tag: '工作', markPrivate: true }] };
  const malicious = analyzeDemoClip(clip('https://x.com.evil.example/a https://evil.example/?next=https://x.com https://x.com@evil.example/'), config);
  assert.deepEqual(malicious.linkSources, []);
  assert.equal(malicious.isPrivate, true, 'URL username is sensitive even when host is another website');
  const item = analyzeDemoClip(clip('https://linux.do/a https://t.co/example https://www.bilibili.com/video/example'), config);
  assert.deepEqual(item.linkSources, ['linuxdo', 'x-short', 'bilibili']);
  assert.deepEqual(item.tags, ['工作']); assert.deepEqual(item.manualTags, []);
  assert.equal(item.isPrivate, true);
  const rescanned = analyzeDemoClip(item, { revision: 2, rules: [] });
  assert.equal(rescanned.isPrivate, true); assert.deepEqual(rescanned.tags, [], 'automatic tags recompute without removing old protection');
  assert.equal(analyzeDemoClip(clip('https://sub.linux.do/a'), config).isPrivate, false, 'custom domains are exact');
});

test('browser privacy changes, scan tokens and v3 backup import cannot erase manual protection or activate imported rules', async () => {
  Object.assign(globalThis, { window: {} });
  const source = fs.readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
    .replace("from './snippets'", `from '${new URL('../src/snippets.ts', import.meta.url).href}'`)
    .replace("from './i18n/index.ts'", `from '${new URL('../src/i18n/index.ts', import.meta.url).href}'`)
    .replace("from './privacy'", `from '${new URL('../src/privacy.ts', import.meta.url).href}'`);
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  const { api } = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  for (const image of (await api.getState()).clips.filter((item: Clip) => item.kind === 'image')) await api.deleteClip(image.id);
  const item = await api.addClip('原创固定资料甲', '原创资料', null);
  await api.setClipPrivacy(item.id, true);
  await api.setClipTags(item.id, ['手动标签']);
  await assert.rejects(api.setClipPrivacy(item.id, false), /复核/);
  const original = await api.privacyRules();
  await api.savePrivacyRules({ ...original, rules: [{ id: 'work', name: '原创自动规则', enabled: true, condition: 'keyword', value: '原创固定', tag: '自动标签', markPrivate: true }] });
  await assert.rejects(api.savePrivacyRules(original), /变更/);
  const stale = await api.previewPrivacyRescan(); await api.addClip('预览后新增的原创内容', '', null);
  await assert.rejects(api.applyPrivacyRescan(stale.token), /重新预览/);
  const fresh = await api.previewPrivacyRescan(); await api.applyPrivacyRescan(fresh.token);
  const after = (await api.getState()).clips.find((entry: Clip) => entry.id === item.id);
  assert.deepEqual(after.manualTags, ['手动标签']); assert.deepEqual(after.tags, ['手动标签', '自动标签']);
  await api.setClipPrivacy(item.id, false, true);
  assert.equal((await api.getState()).clips.find((entry: Clip) => entry.id === item.id).isPrivate, true, 'active rules still protect after local review');
  await api.setClipPrivacy(item.id, true);
  const backup = JSON.parse(await api.exportBackup()); assert.equal(backup.version, 4);
  for (const entry of backup.clips) { entry.privacyManual = false; entry.isPrivate = false; entry.privacyReasons = []; }
  await api.importBackup(JSON.stringify(backup));
  const restored = (await api.getState()).clips.find((entry: Clip) => entry.id === item.id);
  assert.equal(restored.privacyManual, true);
  const rules = await api.privacyRules(); assert.equal(rules.rules.length, 2); assert.equal(rules.rules[0].enabled, true); assert.equal(rules.rules[1].enabled, false); assert.notEqual(rules.rules[0].id, rules.rules[1].id);
});
