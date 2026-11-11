import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { assertReleaseUnchanged, parseChecksums, prepareRelease, renderCask, updateCask, verifyDownload } from '../scripts/homebrew-release.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
function asset(tag, name, body, id) {
  return { id, name, size: Buffer.byteLength(body), state: 'uploaded', updated_at: '2026-10-26T01:00:00Z', digest: `sha256:${hash(body)}`, browser_download_url: `https://github.com/StarDreamTech/starclip/releases/download/${tag}/${name}` };
}
function releaseFixture(version = '0.2.0') {
  const tag = `v${version}`, name = `Starclip_${version}_aarch64.dmg`;
  const body = 'Original fixed artifact: stars over the local clipboard.\n';
  const sums = `${hash(body)}  ${name}\n`;
  const release = {
    id: 123, tag_name: tag, draft: false, prerelease: false, published_at: '2026-10-26T01:00:00Z',
    html_url: `https://github.com/StarDreamTech/starclip/releases/tag/${tag}`,
    assets: [asset(tag, name, body, 101), asset(tag, 'SHA256SUMS', sums, 102)],
  };
  return { body, sums, release };
}
async function files(t, version = '0.2.0') {
  const root = await mkdtemp(join(tmpdir(), 'starclip-homebrew-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const fixture = releaseFixture(version), dmg = join(root, fixture.release.assets[0].name), sums = join(root, 'SHA256SUMS');
  await writeFile(dmg, fixture.body); await writeFile(sums, fixture.sums);
  const plan = prepareRelease(fixture.release, fixture.release);
  return { ...fixture, root, dmg, checksumPath: sums, plan };
}

test('Homebrew 只准备指定仓库的最新公开稳定版本，草稿、预发布、旧版和暂无 latest 均跳过', () => {
  const { release } = releaseFixture();
  const plan = prepareRelease(release, release);
  assert.equal(plan.skip, false);
  assert.equal(plan.version, '0.2.0');
  assert.equal(plan.dmgName, 'Starclip_0.2.0_aarch64.dmg');
  assert.equal(prepareRelease({ draft: true }, null).skip, true);
  assert.equal(prepareRelease({ prerelease: true }, null).skip, true);
  assert.equal(prepareRelease(release, null).skip, true);
  assert.equal(prepareRelease(release, { ...release, id: 999 }).skip, true);
  assert.equal(prepareRelease(release, { ...release, tag_name: 'v0.3.0' }).skip, true);
});

test('Homebrew 拒绝错误仓库、注入 tag、缺失或重复及未完成的资产', () => {
  const { release } = releaseFixture();
  for (const mutate of [
    value => { value.html_url = value.html_url.replace('StarDreamTech', 'someone'); },
    value => { value.tag_name = 'v0.2.0;echo injected'; },
    value => { value.assets = value.assets.slice(1); },
    value => { value.assets.push(value.assets[0]); },
    value => { value.assets[0].state = 'new'; },
    value => { value.assets[0].size = 0; },
    value => { value.assets[0].browser_download_url += '?redirect=elsewhere'; },
    value => { value.assets[0].browser_download_url = value.assets[0].browser_download_url.replace('/v0.2.0/', '/v0.1.0/'); },
    value => { value.assets[0].digest = 'sha256:not-a-hash'; },
    value => { value.published_at = null; },
  ]) {
    const changed = structuredClone(release); mutate(changed);
    assert.throws(() => prepareRelease(changed, changed));
  }
});

test('Homebrew 真实文件以流式 SHA-256、大小、清单与 GitHub digest 共同核验', async t => {
  const fixture = await files(t);
  const evidence = await verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg);
  assert.equal(evidence.sha256, hash(fixture.body));
  assert.equal(evidence.checksumsSha256, hash(fixture.sums));
  assert.equal(evidence.dmg.id, 101);
  assert.equal(evidence.tag, 'v0.2.0');
  const withoutDigest = structuredClone(fixture.plan); withoutDigest.dmg.digest = null; withoutDigest.checksums.digest = null;
  assert.equal((await verifyDownload(withoutDigest, fixture.checksumPath, fixture.dmg)).sha256, hash(fixture.body));
});

test('Homebrew 拒绝损坏包、大小变化、错误 GitHub 摘要和缺失的精确清单条目', async t => {
  const fixture = await files(t);
  await writeFile(fixture.dmg, fixture.body.replace('stars', 'moons'));
  await assert.rejects(verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg), /SHA-256/);
  await writeFile(fixture.dmg, fixture.body + 'changed');
  await assert.rejects(verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg), /大小/);
  await writeFile(fixture.dmg, fixture.body);
  const wrongDigest = structuredClone(fixture.plan); wrongDigest.dmg.digest = `sha256:${'0'.repeat(64)}`;
  await assert.rejects(verifyDownload(wrongDigest, fixture.checksumPath, fixture.dmg), /GitHub digest/);
  const wrongSums = fixture.sums.replace('aarch64', 'aarch65');
  await writeFile(fixture.checksumPath, wrongSums);
  await assert.rejects(verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg), /精确匹配/);
});

test('Homebrew 校验清单拒绝重复条目、路径绕过、注入文件名与格式错误', () => {
  const line = `${'a'.repeat(64)}  Starclip_0.2.0_aarch64.dmg`;
  assert.equal(parseChecksums(line + '\n').size, 1);
  assert.throws(() => parseChecksums(line + '\n' + line), /重复/);
  for (const filename of ['../Starclip.dmg', './Starclip.dmg', '/tmp/Starclip.dmg', 'Starclip/../../file', 'Starclip\\..\\file', '$(echo hi).dmg']) {
    assert.throws(() => parseChecksums(`${'a'.repeat(64)}  ${filename}\n`), /格式或文件名/);
  }
  assert.throws(() => parseChecksums('no-checksum  file.dmg\n'));
  assert.throws(() => parseChecksums(line + '\nmalformed'));
});

test('Homebrew 推送前重查拒绝 latest 改变、撤回和任何验证后资产替换', async t => {
  const fixture = await files(t), evidence = await verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg);
  assert.doesNotThrow(() => assertReleaseUnchanged(evidence, fixture.release, fixture.release));
  assert.throws(() => assertReleaseUnchanged(evidence, fixture.release, null), /发布状态已变化/);
  for (const mutate of [
    value => { value.draft = true; },
    value => { value.prerelease = true; },
    value => { value.id++; },
    value => { value.published_at = '2026-10-26T02:00:00Z'; },
    value => { value.assets[0].id++; },
    value => { value.assets[0].size++; },
    value => { value.assets[0].updated_at = '2026-10-26T02:00:00Z'; },
    value => { value.assets[0].digest = `sha256:${'b'.repeat(64)}`; },
    value => { value.assets[1].id++; },
  ]) {
    const changed = structuredClone(fixture.release); mutate(changed);
    assert.throws(() => assertReleaseUnchanged(evidence, changed, changed));
  }
});

test('Homebrew 从 alpha Cask 过渡稳定版，写入固定模板并保持重跑幂等', async t => {
  const fixture = await files(t), evidence = await verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg);
  const path = join(fixture.root, 'starclip.rb');
  await writeFile(path, 'cask "starclip" do\n  version "0.1.0-alpha.2"\n  sha256 "' + 'a'.repeat(64) + '"\nend\n');
  assert.equal((await updateCask(evidence, path)).changed, true);
  const cask = await readFile(path, 'utf8');
  assert.match(cask, /version "0\.2\.0"/);
  assert.match(cask, new RegExp(`sha256 "${evidence.sha256}"`));
  assert.match(cask, /url "https:\/\/github\.com\/StarDreamTech\/starclip\/releases\/download\/v#\{version\}\/Starclip_#\{version\}_aarch64\.dmg"/);
  assert.match(cask, /homepage "https:\/\/stardreamtech\.github\.io\/projects\/starclip\/"/);
  assert.match(cask, /https:\/\/github\.com\/StarDreamTech\/starclip\/releases\/tag\/v#\{version\}/);
  assert.doesNotMatch(cask, /cpython666/);
  assert.match(cask, /depends_on arch: :arm64/);
  assert.match(cask, /app "Starclip\.app"/);
  assert.doesNotMatch(cask, /alpha|zap|auto_updates|system\(/);
  assert.equal((await updateCask(evidence, path)).changed, false);
  assert.equal(await readFile(path, 'utf8'), cask);
});

test('Homebrew 按数字比较版本，不降级且同版换包拒绝写入', async t => {
  const fixture = await files(t, '0.10.0'), evidence = await verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg);
  const path = join(fixture.root, 'starclip.rb');
  await writeFile(path, renderCask('0.9.0', 'a'.repeat(64)));
  assert.equal((await updateCask(evidence, path)).changed, true);
  const newer = renderCask('0.11.0', 'a'.repeat(64)); await writeFile(path, newer);
  assert.equal((await updateCask(evidence, path)).changed, false);
  assert.equal(await readFile(path, 'utf8'), newer);
  const replaced = renderCask('0.10.0', 'a'.repeat(64)); await writeFile(path, replaced);
  await assert.rejects(updateCask(evidence, path), /同版本/);
  assert.equal(await readFile(path, 'utf8'), replaced);
});

test('Homebrew 无验证证据、重复 Cask 字段或异常版本均不写入', async t => {
  const fixture = await files(t), evidence = await verifyDownload(fixture.plan, fixture.checksumPath, fixture.dmg);
  const path = join(fixture.root, 'starclip.rb');
  for (const text of [
    renderCask('0.1.0', 'a'.repeat(64)).replace('  version "0.1.0"', '  version "0.1.0"\n  version "0.1.1"'),
    renderCask('0.1.0', 'a'.repeat(64)).replace('0.1.0', '0.1.0#{system("bad")}'),
    renderCask('0.1.0', 'a'.repeat(64)).replace('cask "starclip"', 'cask "other"'),
  ]) {
    await writeFile(path, text);
    await assert.rejects(updateCask(evidence, path));
    assert.equal(await readFile(path, 'utf8'), text);
  }
  const original = renderCask('0.1.0', 'a'.repeat(64)); await writeFile(path, original);
  await assert.rejects(updateCask(fixture.plan, path), /摘要/);
  assert.equal(await readFile(path, 'utf8'), original);
});

test('Homebrew CLI 从公开元数据、真实下载文件到 Cask 的本地完整流程', async t => {
  const fixture = await files(t), script = resolve('scripts/homebrew-release.mjs');
  const releasePath = join(fixture.root, 'release.json'), planPath = join(fixture.root, 'plan.json'), evidencePath = join(fixture.root, 'evidence.json'), caskPath = join(fixture.root, 'starclip.rb');
  await writeFile(releasePath, JSON.stringify(fixture.release));
  await writeFile(caskPath, renderCask('0.1.0', 'a'.repeat(64)));
  const run = (...args) => JSON.parse(execFileSync(process.execPath, [script, ...args], { encoding: 'utf8' }));
  assert.equal(run('prepare', releasePath, releasePath, planPath).skip, false);
  assert.equal(run('verify', planPath, fixture.checksumPath, fixture.dmg, evidencePath).sha256, hash(fixture.body));
  assert.equal(run('recheck', evidencePath, releasePath, releasePath).unchanged, true);
  assert.equal(run('cask', evidencePath, caskPath).changed, true);
  assert.equal(run('cask', evidencePath, caskPath).changed, false);
});

test('Homebrew 工作流把同一份 Cask 写入两个 Tap，组织密钥缺失时跳过', async () => {
  const workflow = await readFile(new URL('../.github/workflows/homebrew.yml', import.meta.url), 'utf8');
  assert.match(workflow, /github\.repository == 'StarDreamTech\/starclip'/);
  const personal = workflow.split(/^ {2}update-org-tap:/m)[0];
  const organization = workflow.split(/^ {2}update-org-tap:/m)[1];
  assert.ok(organization);
  assert.match(personal, /repository: cpython666\/homebrew-tap/);
  assert.match(personal, /secrets\.HOMEBREW_TAP_DEPLOY_KEY/);
  assert.match(personal, /Missing HOMEBREW_TAP_DEPLOY_KEY repository secret\./);
  assert.match(organization, /repository: StarDreamTech\/homebrew-tap/);
  assert.match(organization, /secrets\.HOMEBREW_ORG_TAP_DEPLOY_KEY/);
  assert.match(organization, /configured=false/);
  assert.match(organization, /steps\.org_key\.outputs\.configured == 'true'/);
  assert.doesNotMatch(organization, /exit 1/);
  assert.doesNotMatch(workflow, /needs:\s*(?:\[verify,\s*update-tap\]|update-tap)/);
});
