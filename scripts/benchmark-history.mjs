// Explicit original-data benchmark; never starts the app or opens a user profile.
// Usage: node scripts/benchmark-history.mjs [--phase baseline|optimized|both]
//        [--baseline-ref 2c64c48] [--sizes 10000,50000,100000] [--iterations 20]
//        [--output-dir /absolute/ignored-artifact-directory] [--stage compile|measure|all]
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const options = { phase: 'both', stage: 'all', 'baseline-ref': '2c64c48', sizes: '10000,50000,100000', iterations: '20', 'output-dir': join(root, 'artifacts/history-query-20261102.noindex') };
for (let i = 2; i < process.argv.length; i += 2) {
  const key = process.argv[i].replace(/^--/, '');
  if (!(key in options) || process.argv[i + 1] === undefined) throw new Error(`Unknown or missing argument: ${process.argv[i]}`);
  options[key] = process.argv[i + 1];
}
if (!['baseline', 'optimized', 'both'].includes(options.phase) || !['compile', 'measure', 'all'].includes(options.stage)
  || !/^\d+(,\d+)*$/.test(options.sizes) || options.sizes.split(',').some(n => +n < 100 || +n > 100000)
  || !/^\d+$/.test(options.iterations) || +options.iterations < 2 || +options.iterations > 100
  || !isAbsolute(options['output-dir'])) throw new Error('Invalid phase, fixture sizes, sample count or output directory');
const output = resolve(options['output-dir']);
// Keep worktrees/build products under this repo's ignored artifacts namespace.
if (!output.startsWith(join(root, 'artifacts') + sep) || !output.includes('.noindex')) throw new Error('Use an absolute .noindex directory under this repository artifacts/');
mkdirSync(output, { recursive: true });
const git = (args, cwd = root) => execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
const baseline = git(['rev-parse', '--verify', `${options['baseline-ref']}^{commit}`]);
const harness = readFileSync(join(root, 'src-tauri/src/store/query_bench.rs'));
const harnessSha256 = createHash('sha256').update(harness).digest('hex');
const metadata = { baselineCommit: baseline, optimizedCommit: git(['rev-parse', 'HEAD']), harnessSha256,
  sizes: options.sizes, iterations: +options.iterations, rustc: execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim(),
  scope: 'release no-default-features actual Store::history_page with deterministic original SQLite fixtures; not GUI/IPC or mutex contention' };
const baselineSource = join(output, 'baseline-source.noindex');
if (options.phase !== 'optimized') {
  if (!existsSync(baselineSource)) execFileSync('git', ['worktree', 'add', '--detach', baselineSource, baseline], { cwd: root, stdio: 'inherit' });
  if (git(['rev-parse', 'HEAD'], baselineSource) !== baseline) throw new Error('Existing benchmark worktree is not the pinned baseline');
  const changed = execFileSync('git', ['status', '--porcelain', '--untracked-files=all'], { cwd: baselineSource, encoding: 'utf8' }).trimEnd().split('\n').filter(Boolean).map(line => line.slice(3));
  if (changed.some(path => !['src-tauri/src/store/mod.rs', 'src-tauri/src/store/query_bench.rs'].includes(path))) throw new Error('Unexpected baseline worktree edits; refusing to overwrite');
  writeFileSync(join(baselineSource, 'src-tauri/src/store/query_bench.rs'), harness);
  const module = join(baselineSource, 'src-tauri/src/store/mod.rs');
  const original = execFileSync('git', ['show', `${baseline}:src-tauri/src/store/mod.rs`], { cwd: root, encoding: 'utf8' });
  const injected = original.includes('mod query_bench;') ? original : original + '\n#[cfg(test)]\nmod query_bench;\n';
  if (![original, injected].includes(readFileSync(module, 'utf8'))) throw new Error('Baseline mod.rs differs from the exact original plus the benchmark declaration');
  writeFileSync(module, injected);
}

function sourceSnapshot(source) {
  const files = ['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'rust-toolchain.toml'];
  const walk = relative => {
    for (const entry of readdirSync(join(source, relative), { withFileTypes: true })) {
      const path = `${relative}/${entry.name}`;
      if (entry.isDirectory()) walk(path); else if (entry.isFile() && path.endsWith('.rs')) files.push(path);
    }
  };
  walk('src-tauri/src');
  const hash = createHash('sha256');
  for (const file of files.sort()) hash.update(file + '\0').update(readFileSync(join(source, file)));
  return { sha256: hash.digest('hex'), baseCommit: git(['rev-parse', 'HEAD'], source), workingTree: git(['status', '--porcelain'], source) };
}
function run(label, source) {
  const snapshot = sourceSnapshot(source);
  const env = { ...process.env, CARGO_TARGET_DIR: join(output, `target-${label}.noindex`), STARCLIP_BENCH_LABEL: label,
    STARCLIP_BENCH_SIZES: options.sizes, STARCLIP_BENCH_ITERATIONS: options.iterations, STARCLIP_BENCH_OUTPUT: join(output, `${label}.json`) };
  delete env.STARCLIP_TELEMETRY_TEST_ENDPOINT;
  const manifest = join(output, `${label}-executable.json`);
  if (options.stage !== 'measure') {
    const args = ['test', '--locked', '--manifest-path', join(source, 'src-tauri/Cargo.toml'), '--release', '--no-default-features', '--lib', '--no-run', '--message-format=json'];
    console.log(`Compiling ${label} original-data benchmark.`);
    const compiled = spawnSync('cargo', args, { cwd: source, env, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
    writeFileSync(join(output, `${label}-compile.log`), (compiled.stderr ?? '') + '\n' + (compiled.stdout ?? ''));
    if (compiled.status !== 0) throw new Error(`${label} compile failed; see ${label}-compile.log`);
    const artifacts = compiled.stdout.split('\n').filter(Boolean).map(line => JSON.parse(line));
    const executable = artifacts.find(item => item.reason === 'compiler-artifact' && item.target.name === 'starclip_lib' && item.executable)?.executable;
    if (!executable || sourceSnapshot(source).sha256 !== snapshot.sha256) throw new Error('Missing test executable or source changed during compilation');
    writeFileSync(manifest, JSON.stringify({ executable, sourceSnapshot: snapshot, harnessSha256 }, null, 2) + '\n');
    console.log(`${label} benchmark compiled; source snapshot ${snapshot.sha256}.`);
  }
  if (options.stage === 'compile') return;
  const executable = JSON.parse(readFileSync(manifest, 'utf8'));
  if (executable.sourceSnapshot.sha256 !== snapshot.sha256 || executable.harnessSha256 !== harnessSha256) throw new Error('Source changed after compilation; compile again');
  const args = ['store::query_bench::original_history_queries', '--ignored', '--exact', '--nocapture', '--test-threads=1'];
  console.log(`Measuring ${label}; avoid other heavy builds while sampling.`);
  const startedAt = new Date().toISOString();
  const result = spawnSync(executable.executable, args, { cwd: source, env, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
  writeFileSync(join(output, `${label}.log`), (result.stdout ?? '') + (result.stderr ?? ''));
  if (result.status !== 0) throw new Error(`${label} benchmark failed; see ${join(output, `${label}.log`)}`);
  if (sourceSnapshot(source).sha256 !== snapshot.sha256) throw new Error('Source changed during sampling; discard this run');
  const report = JSON.parse(readFileSync(join(output, `${label}.json`), 'utf8'));
  report.provenance = { ...metadata, measuredBaseCommit: snapshot.baseCommit, sourceSnapshotSha256: snapshot.sha256,
    phase: label, startedAt, finishedAt: new Date().toISOString(), command: [executable.executable, ...args], workingTree: snapshot.workingTree };
  writeFileSync(join(output, `${label}.json`), JSON.stringify(report, null, 2) + '\n');
  console.log(`${label} completed: ${report.datasets.map(d => d.fixture.rows).join('/')} rows × ${report.iterations} samples.`);
}
if (options.phase !== 'optimized') run('baseline', baselineSource);
if (options.phase !== 'baseline') run('optimized', root);
if (options.stage !== 'compile' && existsSync(join(output, 'baseline.json')) && existsSync(join(output, 'optimized.json'))) {
  const before = JSON.parse(readFileSync(join(output, 'baseline.json'), 'utf8'));
  const after = JSON.parse(readFileSync(join(output, 'optimized.json'), 'utf8'));
  if (before.provenance.harnessSha256 !== after.provenance.harnessSha256 || before.iterations !== after.iterations) throw new Error('Only compare identical harness and sample counts');
  if (before.provenance.baselineCommit !== baseline || after.provenance.baselineCommit !== baseline || before.provenance.measuredBaseCommit !== baseline) throw new Error('Only compare the same pinned baseline commit');
  const sizes = report => report.datasets.map(dataset => dataset.fixture.rows).sort((a, b) => a - b);
  if (JSON.stringify(sizes(before)) !== JSON.stringify(sizes(after))) throw new Error('Complete dataset sizes differ');
  const datasets = after.datasets.map(next => {
    const previous = before.datasets.find(d => d.fixture.rows === next.fixture.rows);
    if (!previous) throw new Error('Dataset sizes differ');
    for (const key of ['firstResult', 'afterMutationResult']) {
      if (JSON.stringify(previous.coldStatisticsCache[key]) !== JSON.stringify(next.coldStatisticsCache[key])) throw new Error('Cold-query result mismatch');
    }
    return { rows: next.fixture.rows, coldStatisticsCache: { baselineFirstMs: previous.coldStatisticsCache.firstAfterSeedMs, optimizedFirstMs: next.coldStatisticsCache.firstAfterSeedMs,
      baselineAfterWriteMs: previous.coldStatisticsCache.firstAfterMutationMs, optimizedAfterWriteMs: next.coldStatisticsCache.firstAfterMutationMs },
      warmQueries: next.warmQueries.map(query => {
        const old = previous.warmQueries.find(q => q.name === query.name);
        if (!old || JSON.stringify(old.result) !== JSON.stringify(query.result)) throw new Error(`Query result mismatch: ${query.name}`);
        return { name: query.name, result: query.result, baselineMedianMs: old.fullHistoryPage.medianMs, optimizedMedianMs: query.fullHistoryPage.medianMs,
          baselineP95Ms: old.fullHistoryPage.p95Ms, optimizedP95Ms: query.fullHistoryPage.p95Ms, medianSpeedup: old.fullHistoryPage.medianMs / query.fullHistoryPage.medianMs };
      }) };
  });
  writeFileSync(join(output, 'comparison.json'), JSON.stringify({ baselineProvenance: before.provenance, optimizedProvenance: after.provenance, measuredAt: new Date().toISOString(), resultParity: true, datasets,
    limits: after.scope }, null, 2) + '\n');
  console.log(`Result parity passed. Comparison: ${join(output, 'comparison.json')}`);
}
