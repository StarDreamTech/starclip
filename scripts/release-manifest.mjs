import { readdir, readFile, writeFile, copyFile, mkdir, mkdtemp, rm, stat } from 'node:fs/promises';
import { join, resolve, basename } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkVersion, stableVersion, projectRoot } from './version.mjs';

export function releaseNames(version) {
  stableVersion(version);
  return {
    mac: `Starclip_${version}_aarch64.app.tar.gz`,
    dmg: `Starclip_${version}_aarch64.dmg`,
    windows: `Starclip_${version}_x64-setup.exe`,
  };
}

async function filesUnder(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) files.push(...await filesUnder(join(directory, entry.name)));
    else if (entry.isFile()) files.push(join(directory, entry.name));
  }
  return files;
}

export async function collectArtifacts(source, destination, platform, version) {
  const names = releaseNames(version), files = await filesUnder(source);
  const patterns = platform === 'darwin-aarch64'
    ? [[/\.app\.tar\.gz$/, names.mac], [/\.dmg$/, names.dmg]]
    : platform === 'windows-x86_64' ? [[/\.exe$/, names.windows]] : null;
  if (!patterns) throw new Error(`不支持的发布平台：${platform}`);
  const copies = [];
  for (const [pattern, name] of patterns) {
    const matches = files.filter(path => pattern.test(path));
    if (matches.length !== 1) throw new Error(`${platform} 需要一个 ${pattern} 产物，实际 ${matches.length} 个。`);
    copies.push([matches[0], name]);
    if (name !== names.dmg) copies.push([matches[0] + '.sig', name + '.sig']);
  }
  // Validate the complete platform before copying; a failed build cannot look complete.
  for (const [path] of copies) if (!(await stat(path)).size) throw new Error(`产物为空：${basename(path)}`);
  await mkdir(destination, { recursive: true });
  for (const [path, name] of copies) await copyFile(path, join(destination, name));
  return copies.map(([, name]) => name);
}

function decodeBase64(value, label) {
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(value) || value.length % 4 !== 0) throw new Error(`${label} 不是有效的 Base64。`);
  const decoded = Buffer.from(value, 'base64');
  if (decoded.toString('base64') !== value) throw new Error(`${label} 编码无效。`);
  return decoded;
}

export function signatureEnvelope(signature, pubkey, version) {
  const keyText = decodeBase64(pubkey.trim(), '更新公钥').toString('utf8');
  const signatureText = decodeBase64(signature.trim(), '更新签名').toString('utf8');
  const keyLines = keyText.trim().split(/\r?\n/), signatureLines = signatureText.trim().split(/\r?\n/);
  if (keyLines.length !== 2 || !keyLines[0].startsWith('untrusted comment: ')
      || signatureLines.length !== 4 || !signatureLines[0].startsWith('untrusted comment: ')
      || !signatureLines[2].startsWith('trusted comment: ')) throw new Error('更新公钥或签名不是 minisign 格式。');
  const key = decodeBase64(keyLines[1], '公钥内容');
  const signed = decodeBase64(signatureLines[1], '签名内容');
  const globalSignature = decodeBase64(signatureLines[3], '完整签名');
  if (key.length !== 42 || signed.length !== 74 || globalSignature.length !== 64
      || key.subarray(0, 2).toString() !== 'Ed' || !['Ed', 'ED'].includes(signed.subarray(0, 2).toString())) throw new Error('更新公钥或签名长度/算法不正确。');
  if (!key.subarray(2, 10).equals(signed.subarray(2, 10))) throw new Error('签名使用的密钥与应用内置更新公钥不一致。');
  if (version && !signatureLines[2].split('\t').includes(`version:${version}`)) throw new Error('更新签名中绑定的版本与发布版本不一致。');
  return { keyText, signatureText };
}

export async function verifyMinisign(path, signature, pubkey) {
  const { keyText, signatureText } = signatureEnvelope(signature, pubkey);
  const temp = await mkdtemp(join(tmpdir(), 'starclip-signature-'));
  try {
    await writeFile(join(temp, 'key.pub'), keyText);
    await writeFile(join(temp, 'artifact.minisig'), signatureText);
    try {
      execFileSync('minisign', ['-V', '-p', join(temp, 'key.pub'), '-m', path, '-x', join(temp, 'artifact.minisig')], { stdio: 'pipe' });
    } catch {
      throw new Error(`${basename(path)} 签名核验失败（请确认 minisign 已安装、产物未损坏且密钥一致）。`);
    }
  } finally { await rm(temp, { recursive: true, force: true }); }
}

export function releaseNotes(changelog, version) {
  stableVersion(version);
  const escaped = version.replaceAll('.', '\\.');
  const notes = changelog.match(new RegExp(`^## \\[${escaped}\\](?:[^\\n]*)\\n([\\s\\S]*?)(?=^## |$(?![\\s\\S]))`, 'm'))?.[1].trim();
  if (!notes) throw new Error(`CHANGELOG.md 缺少 ${version} 发布说明。`);
  return notes;
}

export async function assembleRelease({ artifactsDir, outputDir, version, tag, repository, pubkey, notes, publishedAt = new Date().toISOString(), verify = verifyMinisign }) {
  stableVersion(version);
  if (tag !== `v${version}`) throw new Error('发布 tag 与应用版本不一致。');
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('GitHub 仓库名无效。');
  if (!notes?.trim()) throw new Error('发布说明不能为空。');
  if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)$/.test(publishedAt) || !Number.isFinite(Date.parse(publishedAt))) throw new Error('发布时间必须是 RFC 3339。');
  const names = releaseNames(version);
  const required = [names.mac, names.mac + '.sig', names.dmg, names.windows, names.windows + '.sig'];
  const files = await filesUnder(artifactsDir);
  const byName = new Map();
  for (const name of required) {
    const matches = files.filter(path => basename(path) === name);
    if (matches.length !== 1) throw new Error(`${name} 必须存在且唯一，实际 ${matches.length} 个。`);
    if (!(await stat(matches[0])).size) throw new Error(`${name} 不能为空。`);
    byName.set(name, matches[0]);
  }
  const platforms = {};
  for (const [platform, name] of [['darwin-aarch64', names.mac], ['windows-x86_64', names.windows]]) {
    const signature = (await readFile(byName.get(name + '.sig'), 'utf8')).trim();
    signatureEnvelope(signature, pubkey, version);
    await verify(byName.get(name), signature, pubkey);
    platforms[platform] = { signature, url: `https://github.com/${repository}/releases/download/${tag}/${name}` };
  }
  const manifest = { version, notes: notes.trim(), pub_date: publishedAt, platforms };
  // Neither latest.json nor a release directory exists until both signatures pass.
  await mkdir(outputDir, { recursive: true });
  for (const name of required) await copyFile(byName.get(name), join(outputDir, name));
  await writeFile(join(outputDir, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
  await writeFile(join(outputDir, 'release-notes.md'), notes.trim() + '\n');
  const checksums = [];
  for (const name of [...required, 'latest.json', 'release-notes.md']) {
    checksums.push(`${createHash('sha256').update(await readFile(join(outputDir, name))).digest('hex')}  ${name}`);
  }
  await writeFile(join(outputDir, 'SHA256SUMS'), checksums.join('\n') + '\n');
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [command, ...args] = process.argv.slice(2);
    if (command === 'collect' && args.length === 3) {
      console.log((await collectArtifacts(...args, await checkVersion())).join('\n'));
    } else if (command === 'assemble' && args.length === 4) {
      const [artifactsDir, outputDir, repository, tag] = args;
      const version = await checkVersion(projectRoot, tag);
      const config = JSON.parse(await readFile(join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
      const endpoint = `https://github.com/${repository}/releases/latest/download/latest.json`;
      if (!config.plugins?.updater?.endpoints?.includes(endpoint)) throw new Error('仓库与应用内置更新地址不一致。');
      const notes = releaseNotes(await readFile(join(projectRoot, 'CHANGELOG.md'), 'utf8'), version);
      await assembleRelease({ artifactsDir, outputDir, repository, tag, version, pubkey: config.plugins.updater.pubkey, notes });
      console.log(`已验证并汇总 ${tag} 的两个平台；等待创建草稿。`);
    } else throw new Error('用法：release-manifest.mjs collect <bundle目录> <输出目录> <平台> | assemble <产物目录> <输出目录> <owner/repo> <tag>');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
