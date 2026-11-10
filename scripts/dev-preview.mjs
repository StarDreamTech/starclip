// Run the native app with hot reload and a separate data identity.
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const config = {
  productName: 'Starclip Live Preview',
  identifier: 'com.starclip.livepreview',
  build: {
    beforeDevCommand: 'npm run build && npm run dev -- --port 1422',
    devUrl: 'http://127.0.0.1:1422',
  },
  bundle: { createUpdaterArtifacts: false },
  plugins: { updater: { endpoints: [] } },
};
const env = { ...process.env, STARCLIP_TELEMETRY_ENDPOINT: '' };
for (const key of Object.keys(env)) {
  if (key.startsWith('APPLE_') || key.startsWith('TAURI_SIGNING_') || key === 'CARGO_BUILD_TARGET') delete env[key];
}
if (process.argv.includes('--check')) {
  console.log(JSON.stringify(config, null, 2));
} else {
  console.log('星笺实时预览：独立数据、首启暂停，无线上统计或更新源。');
  console.log('界面修改自动刷新；Rust 修改自动重新编译。首次编译较慢，终端 Ctrl+C 结束预览。');
  const result = spawnSync(process.execPath, [join(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'dev', '--config', JSON.stringify(config)], { cwd: root, env, stdio: 'inherit' });
  if (result.error) console.error(result.error.message);
  process.exitCode = result.status ?? 1;
}
