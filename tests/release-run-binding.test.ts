import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, symlink, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  assertAssetMatch, assertPublishedRelease, assertReleaseRun, bindReleaseRun, decideCommitMatch,
  expectedAssetNames, hashDirectory, isAllowedDiffPath, publishedDigests, selectCompleteArtifact,
  verifiedMatchLog, verifyPublishedAssets,
} from '../scripts/release-run-binding.mjs';

const TAG = 'a'.repeat(40);
const HEAD = 'b'.repeat(40);
const RUN_ID = 43050585288;
const VERSION = '0.2.6';
const CI_FILES = [
  '.github/workflows/release.yml',
  'docs/releases.md',
  '.agents/skills/starclip/SKILL.md',
  '.agents/skills/starclip/references/development.md',
];

function compare(files, overrides = {}) {
  return {
    status: 'ahead', ahead_by: 1, behind_by: 0, merge_base_commit: { sha: TAG },
    files: files.map(filename => ({ filename, status: 'modified', previous_filename: null })),
    ...overrides,
  };
}

test('精确相同的提交不需要 compare', () => {
  assert.deepEqual(decideCommitMatch({ tagSha: TAG, runHeadSha: TAG }), { mode: 'exact', files: [] });
  assert.equal(isAllowedDiffPath('.github/workflows/release.yml'), true);
  assert.equal(isAllowedDiffPath('docs/../../src/main.rs'), false);
  assert.equal(isAllowedDiffPath('.github/../src/main.rs'), false);
  for (const path of ['src/App.vue', 'src-tauri/src/main.rs', 'scripts/release-manifest.mjs', 'package.json', 'CHANGELOG.md', 'README.md', 'public/starclip.svg']) {
    assert.equal(isAllowedDiffPath(path), false);
  }
});

test('只改 CI 与说明的祖先提交可以放行，应用源码不行', () => {
  const decision = decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare(CI_FILES) });
  assert.equal(decision.mode, 'ci-only');
  assert.deepEqual(decision.files, CI_FILES);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare([...CI_FILES, 'src/App.vue']) }), /应用源码/);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare(['CHANGELOG.md']) }), /未允许/);
  assert.throws(() => decideCommitMatch({
    tagSha: TAG, runHeadSha: HEAD,
    compare: compare(['.github/workflows/release.yml'], { files: [{ filename: '.github/workflows/release.yml', previous_filename: 'src/old.ts', status: 'renamed' }] }),
  }), /src\/old.ts/);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare(CI_FILES, { status: 'diverged', behind_by: 1 }) }), /祖先/);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare(CI_FILES, { merge_base_commit: { sha: HEAD } }) }), /祖先/);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare([]) }), /没有文件差异/);
  assert.throws(() => decideCommitMatch({ tagSha: TAG, runHeadSha: HEAD, compare: compare(Array.from({ length: 300 }, (_, index) => `.github/file-${index}`)) }), /上限/);
});

function assetBodies() {
  const names = expectedAssetNames(VERSION);
  const bodies = Object.fromEntries(names.packages.map(name => [name, `original fixed release asset ${name}\n`]));
  bodies.SHA256SUMS = `${names.packages.map(name => `${createHash('sha256').update(bodies[name]).digest('hex')}  ${name}`).join('\n')}\n`;
  const assetDigests = Object.fromEntries(Object.entries(bodies).map(([name, body]) => [name, createHash('sha256').update(body).digest('hex')]));
  const releaseAssets = Object.entries(assetDigests).map(([name, digest]) => ({ name, id: 100 + Object.keys(assetDigests).indexOf(name), state: 'uploaded', digest: `sha256:${digest}` }));
  const publishedChecksums = Object.fromEntries(bodies.SHA256SUMS.trim().split('\n').map(line => {
    const [digest, name] = line.split('  ');
    return [name, digest];
  }));
  return { bodies, assetDigests, releaseAssets, publishedChecksums };
}

function binding(mode, digests, publishedChecksums) {
  return {
    repository: 'StarDreamTech/starclip', tag: `v${VERSION}`, version: VERSION,
    tagSha: TAG, runHeadSha: mode === 'exact' ? TAG : HEAD, runId: RUN_ID, artifactId: 77,
    mode, files: mode === 'exact' ? [] : CI_FILES, releaseId: 9, assetDigests: digests, publishedChecksums,
  };
}

test('被测包必须同时匹配已发布资产和 SHA256SUMS', async t => {
  const { bodies, assetDigests, releaseAssets, publishedChecksums } = assetBodies();
  const root = await mkdtemp(join(tmpdir(), 'starclip-binding-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [name, body] of Object.entries(bodies)) await writeFile(join(root, name), body);
  const hashes = await hashDirectory(root);
  const match = { version: VERSION, artifactHashes: hashes, assetDigests, publishedChecksums, checksumsText: bodies.SHA256SUMS };
  assertAssetMatch(match);
  const exactLog = await verifyPublishedAssets(binding('exact', assetDigests, publishedChecksums), root);
  assert.match(exactLog, /匹配方式：精确匹配/);
  assert.match(exactLog, /已发布 Release 资产及 SHA256SUMS 一致/);
  const ciLog = verifiedMatchLog(binding('ci-only', assetDigests, publishedChecksums));
  assert.match(ciLog, /匹配方式：仅 CI 文件差异 \+ 资产 sha256 一致/);
  assert.match(ciLog, /docs\/releases.md/);
  assert.match(ciLog, /Starclip_0\.2\.6_x64-setup\.exe/);

  const notesChanged = { ...hashes, 'latest.json': '0'.repeat(64) };
  const notesChecksum = bodies.SHA256SUMS.replace(hashes['latest.json'], notesChanged['latest.json']);
  assert.throws(() => assertAssetMatch({ ...match, artifactHashes: notesChanged }), /latest.json/);
  assertAssetMatch({ ...match, artifactHashes: notesChanged, checksumsText: notesChecksum });
  const installer = 'Starclip_0.2.6_x64-setup.exe';
  assert.throws(() => assertAssetMatch({ ...match, artifactHashes: { ...hashes, [installer]: '0'.repeat(64) } }), new RegExp(installer));
  const wrongPublished = { ...publishedChecksums, [installer]: '1'.repeat(64) };
  assert.throws(() => assertAssetMatch({ ...match, publishedChecksums: wrongPublished }), /已发布 SHA256SUMS/);
  assert.throws(() => publishedDigests(releaseAssets.map(asset => asset.name === 'SHA256SUMS' ? { ...asset, digest: null } : asset), VERSION), /缺少 sha256/);
  assert.throws(() => publishedDigests([...releaseAssets, { name: 'extra.bin', state: 'uploaded', digest: `sha256:${'1'.repeat(64)}` }], VERSION), /完整发布集/);
  await symlink(join(root, 'latest.json'), join(root, 'linked.json'));
  await assert.rejects(hashDirectory(root), /非普通文件/);
});

test('绑定只接受本仓库成功的 release.yml，以及已发布正式资产', async t => {
  const { bodies, releaseAssets, assetDigests, publishedChecksums } = assetBodies();
  const run = { path: '.github/workflows/release.yml', status: 'completed', conclusion: 'success', head_sha: HEAD, head_repository: { full_name: 'StarDreamTech/starclip' } };
  assertReleaseRun(run);
  assert.throws(() => assertReleaseRun({ ...run, conclusion: 'failure' }), /成功运行/);
  assert.throws(() => assertReleaseRun({ ...run, path: '.github/workflows/verify.yml' }), /成功运行/);
  assert.throws(() => assertReleaseRun({ ...run, head_repository: { full_name: 'other/starclip' } }), /成功运行/);
  assert.throws(() => assertPublishedRelease({ id: 9, draft: true, prerelease: false, tag_name: `v${VERSION}`, assets: releaseAssets }, `v${VERSION}`), /已发布正式 Release/);
  assert.throws(() => assertPublishedRelease({ id: 9, draft: false, prerelease: true, tag_name: `v${VERSION}`, assets: releaseAssets }, `v${VERSION}`), /已发布正式 Release/);
  const artifact = { id: 77, name: `complete-release-${VERSION}`, expired: false, workflow_run: { id: RUN_ID, head_sha: HEAD } };
  assert.equal(selectCompleteArtifact([artifact], { version: VERSION, runId: RUN_ID, runHeadSha: HEAD }), 77);
  assert.throws(() => selectCompleteArtifact([{ ...artifact, workflow_run: { id: RUN_ID, head_sha: TAG } }], { version: VERSION, runId: RUN_ID, runHeadSha: HEAD }), /不一致/);
  assert.throws(() => selectCompleteArtifact([{ ...artifact, id: 78 }, artifact], { version: VERSION, runId: RUN_ID, runHeadSha: HEAD }), /不一致/);
  assert.throws(() => selectCompleteArtifact([{ ...artifact, expired: true }], { version: VERSION, runId: RUN_ID, runHeadSha: HEAD }), /过期/);

  const calls = [];
  const github = async path => {
    calls.push(path);
    if (path.includes('/artifacts?')) {
      const page = new URL(path, 'https://github.com').searchParams.get('page');
      if (page === '1') return { total_count: 2, artifacts: [{ id: 1, name: 'other', expired: false, workflow_run: { id: RUN_ID, head_sha: HEAD } }] };
      return { total_count: 2, artifacts: [artifact] };
    }
    if (path.includes('/compare/')) return compare(CI_FILES);
    if (path.includes('/releases/tags/')) return { id: 9, draft: false, prerelease: false, tag_name: `v${VERSION}`, assets: releaseAssets };
    if (path.includes('/commits/')) return { sha: TAG };
    if (path.includes('/actions/runs/')) return run;
    throw new Error(path);
  };
  const readPublishedChecksums = async () => bodies.SHA256SUMS;
  const bound = await bindReleaseRun({ tag: `v${VERSION}`, runId: String(RUN_ID), github, readPublishedChecksums });
  assert.equal(bound.mode, 'ci-only');
  assert.equal(bound.tagSha, TAG);
  assert.equal(bound.runHeadSha, HEAD);
  assert.equal(bound.artifactId, 77);
  assert.deepEqual(bound.assetDigests, assetDigests);
  assert.deepEqual(bound.publishedChecksums, publishedChecksums);
  assert.equal(calls.filter(path => path.includes('/compare/')).length, 1);
  const output = await mkdtemp(join(tmpdir(), 'starclip-binding-cli-'));
  t.after(() => rm(output, { recursive: true, force: true }));
  const directory = join(output, 'assets');
  await mkdir(directory);
  for (const [name, body] of Object.entries(bodies)) await writeFile(join(directory, name), body);
  const bindingPath = join(output, 'binding.json');
  await writeFile(bindingPath, `${JSON.stringify(binding('exact', assetDigests, publishedChecksums))}\n`);
  const passed = spawnSync(process.execPath, ['scripts/release-run-binding.mjs', 'verify-assets', '--binding', bindingPath, '--assets', directory], { encoding: 'utf8' });
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /匹配方式：精确匹配/);
  await writeFile(join(directory, 'Starclip_0.2.6_x64-setup.exe'), 'changed installer\n');
  const failed = spawnSync(process.execPath, ['scripts/release-run-binding.mjs', 'verify-assets', '--binding', bindingPath, '--assets', directory], { encoding: 'utf8' });
  assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /不一致：Starclip_0\.2\.6_x64-setup\.exe/);
  const exactCalls = [];
  const exactGithub = async path => {
    exactCalls.push(path);
    if (path.includes('/compare/')) throw new Error('exact match must not call compare');
    if (path.includes('/artifacts?')) return { total_count: 1, artifacts: [{ ...artifact, workflow_run: { id: RUN_ID, head_sha: TAG } }] };
    if (path.includes('/releases/tags/')) return { id: 9, draft: false, prerelease: false, tag_name: `v${VERSION}`, assets: releaseAssets };
    if (path.includes('/commits/')) return { sha: TAG };
    if (path.includes('/actions/runs/')) return { ...run, head_sha: TAG };
    throw new Error(path);
  };
  const exact = await bindReleaseRun({ tag: `v${VERSION}`, runId: RUN_ID, github: exactGithub, readPublishedChecksums });
  assert.equal(exact.mode, 'exact');
  assert.equal(exactCalls.some(path => path.includes('/compare/')), false);
  const sourceCalls = [];
  await assert.rejects(bindReleaseRun({
    tag: `v${VERSION}`, runId: RUN_ID, github: async path => {
      sourceCalls.push(path);
      if (path.includes('/compare/')) return compare(['src-tauri/src/main.rs']);
      return github(path);
    }, readPublishedChecksums,
  }), /src-tauri\/src\/main.rs/);
});
