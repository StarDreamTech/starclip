import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, mkdir, readFile, writeFile, rm, access, copyFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { bumpVersion, checkVersion, stableVersion } from '../scripts/version.mjs';
import { assembleRelease, collectArtifacts, manifestDownloadRepository, releaseNames, releaseNotes, signatureEnvelope } from '../scripts/release-manifest.mjs';

async function temporary(t) {
  const root = await mkdtemp(join(tmpdir(), 'starclip-release-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

async function versionFixture(t) {
  const root = await temporary(t);
  await mkdir(join(root, 'src-tauri'));
  const files = {
    'package.json': JSON.stringify({ name: 'starclip', version: '0.1.0' }),
    'package-lock.json': JSON.stringify({ version: '0.1.0', packages: { '': { version: '0.1.0' }, 'node_modules/example': { version: '9.0.0' } } }),
    'src-tauri/tauri.conf.json': JSON.stringify({ version: '0.1.0', identifier: 'com.starclip.desktop' }),
    'src-tauri/Cargo.toml': '[package]\nname = "starclip"\nversion = "0.1.0"\n\n[dependencies]\nexample = "9.0.0"\n',
    'src-tauri/Cargo.lock': 'version = 4\n\n[[package]]\nname = "example"\nversion = "9.0.0"\n\n[[package]]\nname = "starclip"\nversion = "0.1.0"\ndependencies = ["example"]\n',
  };
  for (const [path, text] of Object.entries(files)) await writeFile(join(root, path), text);
  return root;
}

test('版本校验读取所有 npm/Cargo/Tauri 版本，并检查发布 tag', async t => {
  const root = await versionFixture(t);
  assert.equal(await checkVersion(root, 'v0.1.0'), '0.1.0');
  await assert.rejects(checkVersion(root, 'v0.2.0'), /tag/);
  const lock = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
  lock.packages[''].version = '0.0.9';
  await writeFile(join(root, 'package-lock.json'), JSON.stringify(lock));
  await assert.rejects(checkVersion(root), /packages.*不一致/);
});

test('版本不一致时 bump 拒绝写入，避免掩盖旧错误', async t => {
  const root = await versionFixture(t);
  await writeFile(join(root, 'src-tauri/tauri.conf.json'), '{"version":"0.0.9"}');
  await assert.rejects(bumpVersion('0.2.0', root), /不一致/);
  assert.equal(JSON.parse(await readFile(join(root, 'package.json'), 'utf8')).version, '0.1.0');
});

test('升级一次同步所有版本，保留依赖版本与应用标识', async t => {
  const root = await versionFixture(t);
  await bumpVersion('0.2.0', root);
  assert.equal(await checkVersion(root, 'v0.2.0'), '0.2.0');
  const lock = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
  assert.equal(lock.packages['node_modules/example'].version, '9.0.0');
  assert.match(await readFile(join(root, 'src-tauri/Cargo.lock'), 'utf8'), /name = "example"\nversion = "9.0.0"/);
  assert.equal(JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8')).identifier, 'com.starclip.desktop');
});

test('只允许递增的稳定版本', async t => {
  for (const version of ['v0.2.0', '01.2.0', '0.2', '0.2.0-rc.1', '0.2.0+build']) assert.throws(() => stableVersion(version));
  const root = await versionFixture(t);
  await assert.rejects(bumpVersion('0.1.0', root), /高于/);
  await assert.rejects(bumpVersion('0.0.9', root), /高于/);
  await bumpVersion('0.10.0', root);
  assert.equal(await checkVersion(root), '0.10.0');
});

test('从指定版本提取更新说明，不混入其他版本', () => {
  const notes = '# 更新记录\n\n## [0.2.0] - 2026-10-26\n\n新版内容\n\n## [0.1.0]\n旧版内容\n';
  assert.equal(releaseNotes(notes, '0.2.0'), '新版内容');
  assert.equal(releaseNotes(notes, '0.1.0'), '旧版内容');
  assert.throws(() => releaseNotes(notes, '0.3.0'), /缺少/);
});

function signingFixture() {
  // Synthetic format fixtures; no real signing key or user data is used.
  const key = Buffer.alloc(42), signed = Buffer.alloc(74);
  key.write('Ed'); signed.write('ED'); key.fill(7, 2, 10); signed.fill(7, 2, 10);
  return {
    pubkey: Buffer.from(`untrusted comment: fixture\tversion:0.2.0\n${key.toString('base64')}\n`).toString('base64'),
    signature: Buffer.from(`untrusted comment: fixture\tversion:0.2.0\n${signed.toString('base64')}\ntrusted comment: fixture\tversion:0.2.0\n${Buffer.alloc(64).toString('base64')}\n`).toString('base64'),
  };
}

async function releaseFixture(t) {
  const root = await temporary(t), artifactsDir = join(root, 'artifacts'), outputDir = join(root, 'output');
  await mkdir(artifactsDir);
  const names = releaseNames('0.2.0'), { pubkey, signature } = signingFixture();
  for (const name of [names.mac, names.dmg, names.windows]) await writeFile(join(artifactsDir, name), 'original fixed test artifact');
  for (const name of [names.mac, names.windows]) await writeFile(join(artifactsDir, name + '.sig'), signature + '\n');
  return { artifactsDir, outputDir, version: '0.2.0', tag: 'v0.2.0', repository: 'StarDreamTech/starclip', pubkey, notes: '测试更新说明', publishedAt: '2026-10-26T01:00:00Z' };
}

test('只在两个平台验签后输出完整 manifest，并保留签名内容与版本下载地址', async t => {
  const options = await releaseFixture(t), verified = [];
  const manifest = await assembleRelease({ ...options, verify: async path => { verified.push(path); } });
  assert.equal(verified.length, 2);
  assert.deepEqual(Object.keys(manifest.platforms), ['darwin-aarch64', 'windows-x86_64']);
  assert.equal(manifest.version, '0.2.0');
  assert.equal(manifest.platforms['windows-x86_64'].signature, signingFixture().signature);
  assert.equal(manifest.platforms['darwin-aarch64'].url, 'https://github.com/StarDreamTech/starclip/releases/download/v0.2.0/Starclip_0.2.0_aarch64.app.tar.gz');
  assert.deepEqual(JSON.parse(await readFile(join(options.outputDir, 'latest.json'), 'utf8')), manifest);
  const sums = await readFile(join(options.outputDir, 'SHA256SUMS'), 'utf8');
  assert.equal(sums.trim().split('\n').length, 7);
  assert.match(sums, /^[a-f0-9]{64}  Starclip_0.2.0_aarch64.app.tar.gz/m);
});

function signingFixtureFor(version) {
  const key = Buffer.alloc(42), signed = Buffer.alloc(74);
  key.write('Ed'); signed.write('ED'); key.fill(7, 2, 10); signed.fill(7, 2, 10);
  return {
    pubkey: Buffer.from(`untrusted comment: fixture\tversion:${version}\n${key.toString('base64')}\n`).toString('base64'),
    signature: Buffer.from(`untrusted comment: fixture\tversion:${version}\n${signed.toString('base64')}\ntrusted comment: fixture\tversion:${version}\n${Buffer.alloc(64).toString('base64')}\n`).toString('base64'),
  };
}

test('0.2.6 清单保留旧下载前缀，其他版本使用组织仓库', async t => {
  assert.equal(manifestDownloadRepository('0.2.6', 'StarDreamTech/starclip'), 'cpython666/starclip');
  assert.equal(manifestDownloadRepository('0.2.7', 'StarDreamTech/starclip'), 'StarDreamTech/starclip');
  const root = await temporary(t), artifactsDir = join(root, 'artifacts'), outputDir = join(root, 'output');
  await mkdir(artifactsDir);
  const names = releaseNames('0.2.6'), { pubkey, signature } = signingFixtureFor('0.2.6');
  for (const name of [names.mac, names.dmg, names.windows]) await writeFile(join(artifactsDir, name), 'original fixed test artifact');
  for (const name of [names.mac, names.windows]) await writeFile(join(artifactsDir, name + '.sig'), signature + '\n');
  const manifest = await assembleRelease({ artifactsDir, outputDir, version: '0.2.6', tag: 'v0.2.6', repository: 'StarDreamTech/starclip', pubkey, notes: '测试更新说明', publishedAt: '2026-11-11T01:00:00Z', verify: async () => {} });
  assert.equal(manifest.platforms['darwin-aarch64'].url, 'https://github.com/cpython666/starclip/releases/download/v0.2.6/Starclip_0.2.6_aarch64.app.tar.gz');
  assert.equal(manifest.platforms['windows-x86_64'].url, 'https://github.com/cpython666/starclip/releases/download/v0.2.6/Starclip_0.2.6_x64-setup.exe');
});

test('缺少一个平台时拒绝生成 latest.json', async t => {
  const options = await releaseFixture(t);
  await rm(join(options.artifactsDir, releaseNames('0.2.0').windows));
  await assert.rejects(assembleRelease({ ...options, verify: async () => {} }), /必须存在且唯一/);
  await assert.rejects(access(join(options.outputDir, 'latest.json')));
});

test('同名重复构建产物不能被静默选中', async t => {
  const options = await releaseFixture(t), name = releaseNames('0.2.0').mac;
  await mkdir(join(options.artifactsDir, 'duplicate'));
  await copyFile(join(options.artifactsDir, name), join(options.artifactsDir, 'duplicate', name));
  await assert.rejects(assembleRelease({ ...options, verify: async () => {} }), /实际 2/);
});

test('任何平台验签失败都不生成可发布 manifest', async t => {
  const options = await releaseFixture(t);
  await assert.rejects(assembleRelease({ ...options, verify: async path => {
    if (path.endsWith('.exe')) throw new Error('签名无效');
  } }), /签名无效/);
  await assert.rejects(access(join(options.outputDir, 'latest.json')));
});

test('发布版本、时间、密钥不一致或签名无效时均拒绝', async t => {
  const options = await releaseFixture(t), verify = async () => {};
  await assert.rejects(assembleRelease({ ...options, tag: 'v0.1.0', verify }), /tag/);
  await assert.rejects(assembleRelease({ ...options, publishedAt: '2026-10-26', verify }), /RFC 3339/);
  const wrongKey = Buffer.from(options.pubkey, 'base64').toString('utf8').split('\n');
  const bytes = Buffer.from(wrongKey[1], 'base64'); bytes[2] = 9; wrongKey[1] = bytes.toString('base64');
  assert.throws(() => signatureEnvelope(signingFixture().signature, Buffer.from(wrongKey.join('\n')).toString('base64')), /密钥/);
  assert.throws(() => signatureEnvelope('not a signature', options.pubkey), /Base64/);
  assert.throws(() => signatureEnvelope(signingFixture().signature, options.pubkey, '0.3.0'), /绑定的版本/);
});

test('收集平台产物时必须同时存在安装包与官方签名文件', async t => {
  const root = await temporary(t), source = join(root, 'source'), output = join(root, 'output');
  await mkdir(source);
  await writeFile(join(source, 'Starclip_0.2.0_x64-setup.exe'), 'fixed exe');
  await assert.rejects(collectArtifacts(source, output, 'windows-x86_64', '0.2.0'), /ENOENT/);
  await writeFile(join(source, 'Starclip_0.2.0_x64-setup.exe.sig'), signingFixture().signature);
  assert.deepEqual(await collectArtifacts(source, output, 'windows-x86_64', '0.2.0'), ['Starclip_0.2.0_x64-setup.exe', 'Starclip_0.2.0_x64-setup.exe.sig']);
});
