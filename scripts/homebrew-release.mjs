import { createReadStream } from 'node:fs';
import { lstat, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableVersion } from './version.mjs';

export const repository = 'StarDreamTech/starclip';
const releaseRoot = `https://github.com/${repository}/releases`;
const shaPattern = /^[a-f0-9]{64}$/;
function positiveInteger(value) { return Number.isSafeInteger(value) && value > 0; }
function timestamp(value) { return typeof value === 'string' && /^\d{4}-\d\d-\d\dT/.test(value) && Number.isFinite(Date.parse(value)); }
function versionFromTag(tag) {
  if (typeof tag !== 'string' || !tag.startsWith('v')) throw new Error('Release tag 必须是 vX.Y.Z。');
  return stableVersion(tag.slice(1));
}
function assetSnapshot(release, name) {
  const assets = release.assets?.filter(asset => asset.name === name);
  if (!assets || assets.length !== 1) throw new Error(`${name} 必须存在且唯一。`);
  const asset = assets[0], url = `${releaseRoot}/download/${release.tag_name}/${name}`;
  if (!positiveInteger(asset.id) || !positiveInteger(asset.size) || asset.state !== 'uploaded'
      || asset.browser_download_url !== url || !timestamp(asset.updated_at)) throw new Error(`${name} 资产元数据无效。`);
  if (asset.digest != null && !/^sha256:[a-f0-9]{64}$/.test(asset.digest)) throw new Error(`${name} 的 GitHub digest 无效。`);
  return { id: asset.id, name, size: asset.size, url, digest: asset.digest ?? null, updatedAt: asset.updated_at };
}

export function prepareRelease(release, latest) {
  if (!release || typeof release !== 'object') throw new Error('缺少 Release 元数据。');
  if (release.draft === true || release.prerelease === true) return { skip: true, reason: '草稿或预发布版不更新稳定 Cask。' };
  const version = versionFromTag(release.tag_name), tag = `v${version}`;
  if (release.draft !== false || release.prerelease !== false || !positiveInteger(release.id)
      || !timestamp(release.published_at) || release.html_url !== `${releaseRoot}/tag/${tag}`) throw new Error('Release 必须是指定仓库已公开的稳定版本。');
  if (!latest || release.id !== latest.id || tag !== latest.tag_name) return { skip: true, reason: '只同步当前最新稳定 Release。' };
  if (latest.draft !== false || latest.prerelease !== false || latest.html_url !== release.html_url) throw new Error('最新稳定 Release 元数据不一致。');
  const dmgName = `Starclip_${version}_aarch64.dmg`;
  return {
    skip: false, repository, version, tag, dmgName, releaseId: release.id,
    releasePublishedAt: release.published_at,
    dmg: assetSnapshot(release, dmgName), checksums: assetSnapshot(release, 'SHA256SUMS'),
  };
}

function validatePlan(plan) {
  const version = versionFromTag(plan?.tag);
  if (plan.skip !== false || plan.repository !== repository || plan.version !== version
      || plan.dmgName !== `Starclip_${version}_aarch64.dmg` || !positiveInteger(plan.releaseId)
      || !timestamp(plan.releasePublishedAt)) throw new Error('发布验证计划无效。');
  for (const [asset, name] of [[plan.dmg, plan.dmgName], [plan.checksums, 'SHA256SUMS']]) {
    if (!asset || asset.name !== name || !positiveInteger(asset.id) || !positiveInteger(asset.size)
        || asset.url !== `${releaseRoot}/download/${plan.tag}/${name}` || !timestamp(asset.updatedAt)
        || (asset.digest != null && !/^sha256:[a-f0-9]{64}$/.test(asset.digest))) throw new Error('发布资产快照无效。');
  }
}
function validateEvidence(evidence) {
  validatePlan(evidence);
  if (!shaPattern.test(evidence.sha256 ?? '') || !shaPattern.test(evidence.checksumsSha256 ?? '')) throw new Error('缺少已验证的文件摘要。');
  for (const [asset, digest] of [[evidence.dmg, evidence.sha256], [evidence.checksums, evidence.checksumsSha256]]) {
    if (asset.digest != null && asset.digest !== `sha256:${digest}`) throw new Error('验证摘要与 GitHub digest 不一致。');
  }
}

export function parseChecksums(text) {
  const entries = new Map();
  for (const line of text.split(/\r?\n/).filter(line => line.length)) {
    const match = line.match(/^([a-f0-9]{64}) [ *]([A-Za-z0-9][A-Za-z0-9._-]*)$/);
    if (!match) throw new Error('SHA256SUMS 格式或文件名无效；不接受目录路径。');
    if (entries.has(match[2])) throw new Error(`SHA256SUMS 存在重复文件：${match[2]}。`);
    entries.set(match[2], match[1]);
  }
  return entries;
}
async function hashFile(path, expectedSize) {
  const info = await lstat(path);
  if (!info.isFile() || info.size !== expectedSize) throw new Error('下载文件类型或大小与 Release 不一致。');
  const hash = createHash('sha256');
  let bytes = 0;
  for await (const chunk of createReadStream(path)) { hash.update(chunk); bytes += chunk.length; }
  if (bytes !== expectedSize) throw new Error('下载文件在验证中发生变化。');
  return hash.digest('hex');
}
export async function verifyDownload(plan, checksumPath, dmgPath) {
  validatePlan(plan);
  if (plan.checksums.size > 1024 * 1024) throw new Error('SHA256SUMS 超过 1 MiB。');
  const checksumsSha256 = await hashFile(checksumPath, plan.checksums.size);
  const entries = parseChecksums(await readFile(checksumPath, 'utf8'));
  const expected = entries.get(plan.dmgName);
  if (!expected) throw new Error('SHA256SUMS 缺少精确匹配的 DMG 条目。');
  const sha256 = await hashFile(dmgPath, plan.dmg.size);
  if (sha256 !== expected) throw new Error('DMG 的 SHA-256 与 SHA256SUMS 不一致。');
  const evidence = { ...plan, sha256, checksumsSha256 };
  validateEvidence(evidence);
  return evidence;
}

export function assertReleaseUnchanged(evidence, release, latest) {
  validateEvidence(evidence);
  const current = prepareRelease(release, latest);
  if (current.skip) throw new Error(`发布状态已变化：${current.reason}`);
  for (const key of ['releaseId', 'releasePublishedAt', 'version', 'tag', 'dmgName', 'dmg', 'checksums']) {
    if (JSON.stringify(current[key]) !== JSON.stringify(evidence[key])) throw new Error(`验证后 Release 或资产已变化：${key}。请重新验证。`);
  }
}

export function renderCask(version, sha256) {
  stableVersion(version);
  if (!shaPattern.test(sha256)) throw new Error('Cask SHA-256 无效。');
  return `cask "starclip" do
  version "${version}"
  sha256 "${sha256}"

  url "${releaseRoot}/download/v#{version}/Starclip_#{version}_aarch64.dmg"
  name "Starclip"
  name "星笺"
  desc "Local-first clipboard history and code snippet manager"
  homepage "https://github.com/${repository}"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on :macos

  app "Starclip.app"

  caveats <<~EOS
    See the release notes for notarization status and installation instructions:
    ${releaseRoot}/tag/v#{version}
  EOS
end
`;
}
function currentCask(text) {
  const version = [...text.matchAll(/^  version "([^"\r\n]+)"$/gm)];
  const sha = [...text.matchAll(/^  sha256 "([a-f0-9]{64})"$/gm)];
  if ([...text.matchAll(/^cask "starclip" do$/gm)].length !== 1 || version.length !== 1 || sha.length !== 1) throw new Error('现有 Cask 的名称、版本或 SHA-256 不唯一。');
  const parts = version[0][1].match(/^((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))(?:-(alpha|beta|rc)\.(?:0|[1-9]\d*))?$/);
  if (!parts) throw new Error('现有 Cask 版本格式不受支持。');
  return { version: parts[1], prerelease: Boolean(parts[2]), sha256: sha[0][1] };
}
export async function updateCask(evidence, path) {
  validateEvidence(evidence);
  const current = currentCask(await readFile(path, 'utf8'));
  const wanted = evidence.version.split('.').map(BigInt), existing = current.version.split('.').map(BigInt);
  const difference = wanted.findIndex((part, index) => part !== existing[index]);
  if (difference >= 0 && wanted[difference] < existing[difference]) return { changed: false, reason: 'Tap 已有更新版本，不降级。' };
  if (difference < 0 && !current.prerelease) {
    if (current.sha256 !== evidence.sha256) throw new Error('同版本安装包 SHA-256 已变化，拒绝覆盖已发布 Cask。');
    return { changed: false, version: evidence.version, sha256: evidence.sha256 };
  }
  const temporary = `${path}.homebrew-${process.pid}.tmp`;
  try {
    await writeFile(temporary, renderCask(evidence.version, evidence.sha256), { flag: 'wx' });
    await rename(temporary, path);
  } finally { await rm(temporary, { force: true }); }
  return { changed: true, version: evidence.version, sha256: evidence.sha256 };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const json = async path => JSON.parse(await readFile(path, 'utf8'));
  try {
    const [command, ...args] = process.argv.slice(2);
    let result;
    if (command === 'prepare' && args.length === 3) {
      const plan = prepareRelease(await json(args[0]), await json(args[1]));
      await writeFile(args[2], JSON.stringify(plan, null, 2) + '\n');
      result = plan.skip ? plan : { skip: false, version: plan.version, tag: plan.tag, dmgName: plan.dmgName };
    } else if (command === 'verify' && args.length === 4) {
      const evidence = await verifyDownload(await json(args[0]), args[1], args[2]);
      await writeFile(args[3], JSON.stringify(evidence, null, 2) + '\n');
      result = { version: evidence.version, tag: evidence.tag, dmgName: evidence.dmgName, sha256: evidence.sha256 };
    } else if (command === 'cask' && args.length === 2) result = await updateCask(await json(args[0]), args[1]);
    else if (command === 'recheck' && args.length === 3) {
      assertReleaseUnchanged(await json(args[0]), await json(args[1]), await json(args[2]));
      result = { unchanged: true };
    } else throw new Error('用法：homebrew-release.mjs prepare <release.json> <latest.json> <plan.json> | verify <plan.json> <SHA256SUMS> <dmg> <evidence.json> | cask <evidence.json> <Cask路径> | recheck <evidence.json> <release.json> <latest.json>');
    console.log(JSON.stringify(result));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
