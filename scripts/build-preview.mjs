// Build an isolated local macOS app and DMG. No GitHub or Homebrew operations.
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'darwin') throw new Error('此命令仅用于 macOS 本地预览。');
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const base = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const config = {
  productName: 'Starclip Preview',
  identifier: 'com.starclip.preview',
  app: { windows: base.app.windows.map(window => ({ ...window, title: '星笺 · 本地预览' })) },
  bundle: { createUpdaterArtifacts: false, macOS: { signingIdentity: '-' } },
  plugins: { updater: { endpoints: [] } },
};
const env = { ...process.env, STARCLIP_TELEMETRY_ENDPOINT: '', CARGO_TARGET_DIR: join(root, 'src-tauri/target') };
// Local previews must never inherit signing/notarization or updater credentials.
for (const key of Object.keys(env)) {
  if (key.startsWith('APPLE_') || key.startsWith('TAURI_SIGNING_') || key === 'CARGO_BUILD_TARGET') delete env[key];
}
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const output = join(root, 'artifacts/preview-local.noindex', stamp);
mkdirSync(output, { recursive: true });
writeFileSync(join(output, 'preview-config.json'), JSON.stringify(config, null, 2) + '\n');
console.log('构建独立预览：com.starclip.preview；线上统计未配置；无更新源。');
execFileSync(join(root, 'node_modules/.bin/tauri'), [
  'build', '--ci', '--bundles', 'app,dmg', '--config', JSON.stringify(config),
], { cwd: root, env, stdio: 'inherit' });
const bundles = join(env.CARGO_TARGET_DIR, 'release/bundle');
const app = join(output, 'Starclip Preview.app');
// Keep the app under .noindex instead of leaving a second searchable build copy.
renameSync(join(bundles, 'macos/Starclip Preview.app'), app);
const diskImage = `Starclip Preview_${base.version}_${process.arch === 'arm64' ? 'aarch64' : 'x64'}.dmg`;
const dmg = join(output, diskImage);
cpSync(join(bundles, 'dmg', diskImage), dmg);
writeFileSync(join(output, 'preview-info.json'), JSON.stringify({
  builtAt: new Date().toISOString(), sourceVersion: base.version,
  identifier: config.identifier, app, dmg, telemetryConfigured: false,
  updaterEndpoints: [], signing: 'local ad-hoc', published: false,
}, null, 2) + '\n');
console.log(`\n本地预览应用：${app}\n本地预览安装包：${dmg}`);
