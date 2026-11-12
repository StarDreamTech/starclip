#!/usr/bin/env node
// Bind updater acceptance to a successful release.yml run without requiring
// the run head to be the tag commit. A descendant is accepted only when the
// compare API shows no application, script, lockfile or changelog changes.
// The packages taken from that run must still hash to the published Release
// assets and to SHA256SUMS. This file never changes a release or a tag.
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { appendFileSync } from 'node:fs';
import { lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseNames } from './release-manifest.mjs';
import { stableVersion } from './version.mjs';

export const REPOSITORY = 'StarDreamTech/starclip';
export const RELEASE_WORKFLOW = '.github/workflows/release.yml';
// `.github/` is the CI tree the acceptance rule allows. `docs/` and
// `.agents/` are not desktop build inputs; the published 0.2.6 run edits
// them in the same commit as the minisign install. Anything else is rejected.
export const ALLOWED_DIFF_PREFIXES = ['.github/', 'docs/', '.agents/'];
const SHA = /^[a-f0-9]{40}$/;
const TAG = /^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/;

export function isAllowedDiffPath(file) {
  if (typeof file !== 'string' || file.length === 0 || file.length > 512) return false;
  if (file.includes('\\') || file.startsWith('/') || file.includes('\0')) return false;
  const parts = file.split('/');
  if (parts.some(part => part === '' || part === '.' || part === '..')) return false;
  return ALLOWED_DIFF_PREFIXES.some(prefix => file.startsWith(prefix));
}

export function decideCommitMatch({ tagSha, runHeadSha, compare }) {
  if (!SHA.test(tagSha || '') || !SHA.test(runHeadSha || '')) throw new Error('tag 或 release run 的提交号无效');
  if (tagSha === runHeadSha) return { mode: 'exact', files: [] };
  if (!compare || typeof compare !== 'object') throw new Error('提交不同，但没有 compare API 结果');
  if (compare.status !== 'ahead' || compare.behind_by !== 0 || !(compare.ahead_by >= 1) || compare.merge_base_commit?.sha !== tagSha) {
    throw new Error('tag 提交不是 release run head 的祖先');
  }
  const files = compare.files;
  if (!Array.isArray(files)) throw new Error('compare API 没有返回文件列表');
  if (files.length === 0) throw new Error('提交不同，但 compare 没有文件差异');
  if (files.length >= 300) throw new Error('compare 文件列表达到上限，无法确认差异不包含应用源码');
  const paths = [];
  for (const file of files) {
    if (!file || typeof file.filename !== 'string') throw new Error('compare 文件条目无效');
    const names = [file.filename];
    if (file.previous_filename) names.push(file.previous_filename);
    for (const name of names) {
      if (!isAllowedDiffPath(name)) throw new Error(`差异涉及应用源码或其他未允许的文件：${name}`);
    }
    paths.push(file.filename);
  }
  return { mode: 'ci-only', files: paths };
}

export function assertReleaseRun(run, repository = REPOSITORY) {
  if (repository !== REPOSITORY) throw new Error('只能核对 StarDreamTech/starclip 的 release run');
  if (!run || run.path !== RELEASE_WORKFLOW || run.status !== 'completed' || run.conclusion !== 'success' || run.head_repository?.full_name !== REPOSITORY) {
    throw new Error('release run 不是本仓库 release.yml 的成功运行');
  }
  if (!SHA.test(run.head_sha || '')) throw new Error('release run 的 head_sha 无效');
  return run;
}

export function expectedAssetNames(version) {
  const names = releaseNames(stableVersion(version));
  // Installers and signatures are the packages acceptance actually runs.
  // latest.json and release notes can be edited on the published Release
  // after upload; their bytes are not the installed package.
  const installers = [names.mac, `${names.mac}.sig`, names.dmg, names.windows, `${names.windows}.sig`];
  const packages = [...installers, 'latest.json', 'release-notes.md'];
  return { installers, packages, all: [...packages, 'SHA256SUMS'] };
}

function sameNames(left, right) {
  return [...left].sort().join('\0') === [...right].sort().join('\0');
}

export function publishedDigests(releaseAssets, version) {
  const expected = expectedAssetNames(version);
  if (!Array.isArray(releaseAssets)) throw new Error('已发布 Release 没有资产列表');
  const digests = {};
  for (const asset of releaseAssets) {
    if (!asset || typeof asset.name !== 'string' || asset.state !== 'uploaded' || Object.hasOwn(digests, asset.name)) {
      throw new Error(`已发布资产缺失、未上传完成或重名：${asset?.name || '未命名'}`);
    }
    const match = /^sha256:([a-fA-F0-9]{64})$/.exec(String(asset.digest || ''));
    if (!match) throw new Error(`已发布资产缺少 sha256：${asset.name}`);
    digests[asset.name] = match[1].toLowerCase();
  }
  if (!sameNames(Object.keys(digests), expected.all)) throw new Error('已发布资产与完整发布集不一致');
  return digests;
}

export function assertPublishedRelease(release, tag) {
  if (!TAG.test(tag || '') || !release || release.draft !== false || release.prerelease !== false || release.tag_name !== tag) {
    throw new Error('没有与 tag 对应的已发布正式 Release');
  }
  if (!Number.isSafeInteger(release.id) || release.id <= 0) throw new Error('已发布 Release 编号无效');
  return release.assets;
}

export function parseChecksums(text) {
  if (typeof text !== 'string' || !text.endsWith('\n') || text.includes('\r')) throw new Error('SHA256SUMS 必须是以 LF 结尾的文本');
  const lines = text.slice(0, -1).split('\n');
  if (lines.length === 0 || lines.some(line => line === '')) throw new Error('SHA256SUMS 为空或含有空行');
  const hashes = {};
  for (const line of lines) {
    const match = /^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$/.exec(line);
    if (!match || Object.hasOwn(hashes, match[2])) throw new Error('SHA256SUMS 条目无效或重复');
    hashes[match[2]] = match[1];
  }
  return hashes;
}

export function assertAssetMatch({ version, artifactHashes, assetDigests, publishedChecksums, checksumsText }) {
  const expected = expectedAssetNames(version);
  const artifactChecksums = parseChecksums(checksumsText);
  if (!artifactHashes || !assetDigests || !publishedChecksums) throw new Error('缺少产物或已发布资产摘要');
  if (!sameNames(Object.keys(artifactChecksums), expected.packages)) throw new Error('release run 的 SHA256SUMS 覆盖的文件与完整发布集不一致');
  if (!sameNames(Object.keys(publishedChecksums), expected.packages)) throw new Error('已发布 SHA256SUMS 覆盖的文件与完整发布集不一致');
  if (!sameNames(Object.keys(assetDigests), expected.all)) throw new Error('已发布资产与完整发布集不一致');
  if (!sameNames(Object.keys(artifactHashes), expected.all)) throw new Error('release run 产物文件集与已发布资产不一致');
  for (const [name, digest] of Object.entries(artifactChecksums)) {
    if (artifactHashes[name] !== digest) throw new Error(`release run 的 SHA256SUMS 与产物不一致：${name}`);
  }
  for (const name of expected.installers) {
    for (const digest of [artifactHashes[name], assetDigests[name], artifactChecksums[name], publishedChecksums[name]]) {
      if (!/^[a-f0-9]{64}$/.test(digest || '')) throw new Error(`被测包摘要无效：${name}`);
    }
    if (artifactHashes[name] !== assetDigests[name] || artifactHashes[name] !== artifactChecksums[name] || artifactHashes[name] !== publishedChecksums[name]) {
      throw new Error(`被测包 sha256 与已发布 Release 资产或 SHA256SUMS 不一致：${name}（产物 ${artifactHashes[name]}，运行内 SHA256SUMS ${artifactChecksums[name]}，已发布 SHA256SUMS ${publishedChecksums[name]}，已发布资产 ${assetDigests[name]}）`);
    }
  }
}

export function assertBinding(binding) {
  if (!binding || typeof binding !== 'object') throw new Error('发布绑定无效');
  if (binding.repository !== REPOSITORY) throw new Error('发布绑定不属于本仓库');
  const version = stableVersion(binding.version);
  if (binding.tag !== `v${version}`) throw new Error('发布绑定的 tag 与版本不一致');
  if (!SHA.test(binding.tagSha || '') || !SHA.test(binding.runHeadSha || '')) throw new Error('发布绑定的提交号无效');
  if (!Number.isSafeInteger(binding.runId) || binding.runId <= 0 || !Number.isSafeInteger(binding.artifactId) || binding.artifactId <= 0) {
    throw new Error('发布绑定的运行或产物编号无效');
  }
  if (binding.mode === 'exact') {
    if (binding.tagSha !== binding.runHeadSha || !Array.isArray(binding.files) || binding.files.length !== 0) {
      throw new Error('精确匹配的绑定记录了额外差异');
    }
  } else if (binding.mode === 'ci-only') {
    if (binding.tagSha === binding.runHeadSha || !Array.isArray(binding.files) || binding.files.length === 0 || binding.files.length >= 300) {
      throw new Error('仅 CI 文件差异的绑定无效');
    }
    for (const file of binding.files) {
      if (!isAllowedDiffPath(file)) throw new Error(`绑定中的差异文件未允许：${file}`);
    }
  } else {
    throw new Error('未知的提交匹配方式');
  }
  const assetDigests = publishedDigests(Object.entries(binding.assetDigests || {}).map(([name, digest]) => ({ name, state: 'uploaded', digest: `sha256:${digest}` })), version);
  const expected = expectedAssetNames(version);
  if (!sameNames(Object.keys(binding.publishedChecksums || {}), expected.packages)) throw new Error('发布绑定的 SHA256SUMS 不完整');
  for (const name of expected.installers) {
    if (binding.publishedChecksums[name] !== assetDigests[name]) throw new Error(`发布绑定里被测包与已发布资产摘要不一致：${name}`);
  }
  return binding;
}

export function commitRelationLog(binding) {
  assertBinding(binding);
  if (binding.mode === 'exact') {
    return `提交关系：精确匹配。release run head_sha 等于 tag 提交 ${binding.tagSha}。资产 sha256 将在下载产物后核对。`;
  }
  return `提交关系：tag 提交 ${binding.tagSha} 是 release run ${binding.runHeadSha} 的祖先，compare 差异未涉及应用源码。差异文件：${binding.files.join(', ')}。资产 sha256 将在下载产物后核对。`;
}

export function verifiedMatchLog(binding) {
  assertBinding(binding);
  const checked = expectedAssetNames(binding.version).installers.join(', ');
  if (binding.mode === 'exact') {
    return `匹配方式：精确匹配。被测包 sha256 与已发布 Release 资产及 SHA256SUMS 一致。tag 提交与 release run head 都是 ${binding.tagSha}。已核对：${checked}。`;
  }
  return `匹配方式：仅 CI 文件差异 + 资产 sha256 一致。被测包 sha256 与已发布 Release 资产及 SHA256SUMS 一致。tag ${binding.tagSha} 是 release run ${binding.runHeadSha} 的祖先。差异文件：${binding.files.join(', ')}。已核对：${checked}。`;
}

export function selectCompleteArtifact(artifacts, { version, runId, runHeadSha }) {
  stableVersion(version);
  if (!Number.isSafeInteger(runId) || runId <= 0 || !SHA.test(runHeadSha || '')) throw new Error('release run 编号或提交号无效');
  const name = `complete-release-${version}`;
  const matches = (Array.isArray(artifacts) ? artifacts : []).filter(artifact => artifact && artifact.name === name && artifact.expired === false
    && artifact.workflow_run?.id === runId && artifact.workflow_run?.head_sha === runHeadSha);
  if (matches.length !== 1 || !Number.isSafeInteger(matches[0].id) || matches[0].id <= 0) {
    throw new Error('complete-release 产物缺失、过期或与这次 release run 不一致');
  }
  return matches[0].id;
}

export async function listWorkflowRunArtifacts(github, owner, repo, runId) {
  const artifacts = [];
  for (let page = 1; page <= 20; page += 1) {
    const data = await github(`/repos/${owner}/${repo}/actions/runs/${runId}/artifacts?per_page=100&page=${page}`);
    const pageItems = data?.artifacts;
    if (!Array.isArray(pageItems) || !Number.isSafeInteger(data.total_count) || data.total_count < 0) throw new Error('无法列出 release run 产物');
    artifacts.push(...pageItems);
    if (artifacts.length >= data.total_count || pageItems.length === 0) {
      if (artifacts.length !== data.total_count) throw new Error('release run 产物列表不完整');
      return artifacts;
    }
  }
  throw new Error('release run 产物列表过长');
}

export async function bindReleaseRun({ tag, runId, repository = REPOSITORY, github, readPublishedChecksums }) {
  if (repository !== REPOSITORY) throw new Error('只能核对 StarDreamTech/starclip 的 release run');
  if (!TAG.test(tag || '')) throw new Error('发布 tag 无效');
  const version = stableVersion(tag.slice(1));
  const runText = String(runId ?? '');
  if (!/^[1-9]\d*$/.test(runText)) throw new Error('release run 编号无效');
  const numericRunId = Number(runText);
  if (!Number.isSafeInteger(numericRunId)) throw new Error('release run 编号无效');
  if (typeof github !== 'function') throw new Error('缺少 GitHub API');
  const [owner, repo] = repository.split('/');
  const encodedTag = encodeURIComponent(tag);
  const [run, commit, release] = await Promise.all([
    github(`/repos/${owner}/${repo}/actions/runs/${numericRunId}`),
    github(`/repos/${owner}/${repo}/commits/${encodedTag}`),
    github(`/repos/${owner}/${repo}/releases/tags/${encodedTag}`),
  ]);
  assertReleaseRun(run, repository);
  const tagSha = commit?.sha;
  if (!SHA.test(tagSha || '')) throw new Error('tag 没有解析成提交');
  const assets = assertPublishedRelease(release, tag);
  const compare = run.head_sha === tagSha ? null : await github(`/repos/${owner}/${repo}/compare/${tagSha}...${run.head_sha}`);
  const decision = decideCommitMatch({ tagSha, runHeadSha: run.head_sha, compare });
  const artifacts = await listWorkflowRunArtifacts(github, owner, repo, numericRunId);
  const artifactId = selectCompleteArtifact(artifacts, { version, runId: numericRunId, runHeadSha: run.head_sha });
  const assetDigests = publishedDigests(assets, version);
  const sumsAsset = assets.find(asset => asset?.name === 'SHA256SUMS');
  if (typeof readPublishedChecksums !== 'function') throw new Error('缺少已发布 SHA256SUMS');
  const checksumsText = await readPublishedChecksums(sumsAsset);
  if (typeof checksumsText !== 'string' || createHash('sha256').update(checksumsText).digest('hex') !== assetDigests.SHA256SUMS) {
    throw new Error('下载到的 SHA256SUMS 与已发布资产摘要不一致');
  }
  const publishedChecksums = parseChecksums(checksumsText);
  const binding = {
    repository, tag, version, tagSha, runId: numericRunId, runHeadSha: run.head_sha, artifactId,
    mode: decision.mode, files: decision.files, releaseId: release.id, assetDigests, publishedChecksums,
  };
  assertBinding(binding);
  return binding;
}

function hashFile(path) {
  return new Promise((resolveHash, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(path);
    stream.on('error', reject);
    stream.on('data', chunk => hash.update(chunk));
    stream.on('end', () => resolveHash(hash.digest('hex')));
  });
}

export async function hashDirectory(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const hashes = {};
  for (const entry of entries) {
    if (entry.name === '.' || entry.name === '..' || Object.hasOwn(hashes, entry.name)) throw new Error(`发布产物文件名无效：${entry.name}`);
    const path = join(directory, entry.name);
    const info = await lstat(path);
    if (entry.isSymbolicLink() || info.isSymbolicLink() || !info.isFile()) throw new Error(`发布产物目录含有非普通文件：${entry.name}`);
    hashes[entry.name] = await hashFile(path);
  }
  return hashes;
}

export async function verifyPublishedAssets(binding, assetsDirectory) {
  assertBinding(binding);
  const artifactHashes = await hashDirectory(assetsDirectory);
  let checksumsText;
  try {
    checksumsText = await readFile(join(assetsDirectory, 'SHA256SUMS'), 'utf8');
  } catch {
    throw new Error('release run 产物缺少 SHA256SUMS');
  }
  assertAssetMatch({ version: binding.version, artifactHashes, assetDigests: binding.assetDigests, publishedChecksums: binding.publishedChecksums, checksumsText });
  return verifiedMatchLog(binding);
}

function report(line) {
  console.log(line);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${line}\n`);
}

function writeOutputs(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  appendFileSync(process.env.GITHUB_OUTPUT, `${Object.entries(values).map(([key, value]) => `${key}=${value}`).join('\n')}\n`);
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let index = 0; index < rest.length; index += 1) {
    const key = rest[index];
    if (!key?.startsWith('--')) throw new Error(`未知参数：${key}`);
    const value = rest[index + 1];
    if (!value || value.startsWith('--')) throw new Error(`参数缺少值：${key}`);
    options[key.slice(2)] = value;
    index += 1;
  }
  return { command, options };
}

async function downloadPublishedChecksums(asset) {
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (!token || !Number.isSafeInteger(asset?.id) || asset.id <= 0) throw new Error('无法下载已发布 SHA256SUMS');
  if (!Number.isSafeInteger(asset.size) || asset.size <= 0 || asset.size > 1024 * 1024) throw new Error('已发布 SHA256SUMS 大小异常');
  const first = await fetch(`https://api.github.com/repos/${REPOSITORY}/releases/assets/${asset.id}`, {
    redirect: 'manual',
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: 'application/octet-stream',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'starclip-release-run-binding',
    },
  });
  if (![301, 302, 303, 307, 308].includes(first.status)) throw new Error(`GitHub API ${first.status}：无法下载已发布 SHA256SUMS`);
  const location = first.headers.get('location') || '';
  if (!location.startsWith('https://')) throw new Error('已发布 SHA256SUMS 的下载地址无效');
  const second = await fetch(location);
  if (!second.ok) throw new Error(`下载已发布 SHA256SUMS 失败：${second.status}`);
  return Buffer.from(await second.arrayBuffer()).toString('utf8');
}

async function github(path) {
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (!token) throw new Error('缺少 GH_TOKEN');
  const response = await fetch(`https://api.github.com${path}`, {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'starclip-release-run-binding',
    },
  });
  if (!response.ok) throw new Error(`GitHub API ${response.status}：${path.split('?')[0]}`);
  return response.json();
}

async function main() {
  const { command, options } = parseArgs(process.argv.slice(2));
  if (command === 'bind') {
    for (const name of ['tag', 'run-id', 'output']) if (!options[name]) throw new Error(`缺少 --${name}`);
    const binding = await bindReleaseRun({ tag: options.tag, runId: options['run-id'], github, readPublishedChecksums: downloadPublishedChecksums });
    await writeFile(options.output, `${JSON.stringify(binding, null, 2)}\n`);
    report(commitRelationLog(binding));
    writeOutputs({
      revision: binding.tagSha, 'head-sha': binding.tagSha, version: binding.version,
      artifact: String(binding.artifactId), 'artifact-id': String(binding.artifactId),
      'run-id': String(binding.runId), match: binding.mode, 'run-head': binding.runHeadSha,
    });
    return;
  }
  if (command === 'verify-assets') {
    for (const name of ['binding', 'assets']) if (!options[name]) throw new Error(`缺少 --${name}`);
    report(await verifyPublishedAssets(JSON.parse(await readFile(options.binding, 'utf8')), options.assets));
    return;
  }
  throw new Error('用法：release-run-binding.mjs bind --tag <vX.Y.Z> --run-id <id> --output <file> | verify-assets --binding <file> --assets <dir>');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
