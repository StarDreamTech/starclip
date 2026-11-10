import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import type { UpdateStatus } from '../src/update-api.ts';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
const base: UpdateStatus = { revision: 0, currentVersion: '0.2.0', configured: true, phase: 'idle', downloaded: 0 };
let moduleNumber = 0;
async function loadApi(desktop = true) {
  Object.assign(globalThis, { window: desktop ? { __TAURI_INTERNALS__: {} } : {} });
  const source = fs.readFileSync(new URL('../src/update-api.ts', import.meta.url), 'utf8')
    .replace(/import packageInfo[^\n]+/, "const packageInfo = { version: '0.2.0' };")
    .replaceAll("import('@tauri-apps/api/core')", 'Promise.resolve(globalThis.updateTestCore)')
    .replaceAll("import('@tauri-apps/api/event')", 'Promise.resolve(globalThis.updateTestEvents)');
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  return import('data:text/javascript;base64,' + Buffer.from(code + `\n// module ${moduleNumber++}`).toString('base64'));
}

test('update events precede commands; old reads cannot overwrite progress; failed checks can retry', async () => {
  const pendingRead = deferred<UpdateStatus>();
  let event: ((value: { payload: UpdateStatus }) => void) | undefined;
  let stopped = 0, failCheck = true;
  const commands: string[] = [];
  Object.assign(globalThis, {
    updateTestEvents: { async listen(name: string, callback: typeof event) { assert.equal(name, 'starclip-update-status'); event = callback; return () => { stopped++; event = undefined; }; } },
    updateTestCore: { async invoke(command: string, args?: Record<string, unknown>) {
      assert(event, 'subscribe before any native command'); commands.push(command);
      if (command === 'get_update_status') return pendingRead.promise;
      if (command === 'check_starclip_update') {
        if (failCheck) { failCheck = false; throw new Error('测试网络不可用'); }
        return { ...base, revision: 6, phase: 'up-to-date' };
      }
      assert.deepEqual(args, { expectedVersion: '0.3.0' });
      if (command === 'download_starclip_update') {
        event({ payload: { ...base, revision: 7, phase: 'downloading', downloaded: 30, total: 100 } });
        event({ payload: { ...base, revision: 8, phase: 'ready', downloaded: 100, total: 100 } });
        return { ...base, revision: 7, phase: 'downloading', downloaded: 30, total: 100 };
      }
      assert.equal(command, 'install_starclip_update');
      event({ payload: { ...base, revision: 9, phase: 'backing-up', downloaded: 100, total: 100 } });
      return { ...base, revision: 10, phase: 'installed' };
    } },
  });
  const api = await loadApi();
  const seen: UpdateStatus[] = [];
  const watcher = api.watchStarclipUpdates((state: UpdateStatus) => seen.push(state));
  const reading = watcher.read();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(commands, ['get_update_status'], 'opening settings must only read local state');
  event!({ payload: { ...base, revision: 5, phase: 'available', available: { version: '0.3.0', notes: '<script>固定测试文本</script>' } } });
  pendingRead.resolve(base); await reading;
  assert.equal(seen.length, 1); assert.equal(seen[0].phase, 'available');
  await assert.rejects(watcher.check(), /测试网络不可用/);
  await watcher.check(); assert.equal(seen.at(-1)?.phase, 'up-to-date');
  await watcher.download('0.3.0');
  assert.equal(seen.at(-1)?.phase, 'ready', 'late download response cannot undo the verified ready event');
  assert(!commands.includes('install_starclip_update'), 'download must not install');
  await watcher.install('0.3.0');
  assert.deepEqual(seen.slice(-4).map(item => item.phase), ['downloading', 'ready', 'backing-up', 'installed']);
  watcher.dispose(); watcher.dispose(); assert.equal(stopped, 1);
});

test('late listener registration and in-flight responses are ignored after unmount', async () => {
  const listening = deferred<() => void>();
  let calls = 0, stopped = 0;
  Object.assign(globalThis, {
    updateTestEvents: { listen: () => listening.promise },
    updateTestCore: { async invoke() { calls++; return base; } },
  });
  const api = await loadApi();
  const seen: UpdateStatus[] = [];
  const watcher = api.watchStarclipUpdates((state: UpdateStatus) => seen.push(state));
  const reading = watcher.read(); watcher.dispose(); listening.resolve(() => { stopped++; }); await reading;
  assert.equal(stopped, 1); assert.equal(calls, 0); assert.deepEqual(seen, []);

  const response = deferred<UpdateStatus>();
  Object.assign(globalThis, {
    updateTestEvents: { async listen() { return () => { stopped++; }; } },
    updateTestCore: { async invoke() { calls++; return response.promise; } },
  });
  const second = api.watchStarclipUpdates((state: UpdateStatus) => seen.push(state));
  const check = second.check(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 1); second.dispose(); response.resolve({ ...base, revision: 2, phase: 'up-to-date' }); await check;
  assert.deepEqual(seen, []); assert.equal(stopped, 2);
});

test('subscription errors allow reconnecting; browser mode never invokes native updates or manufactures a release', async () => {
  let listens = 0, calls = 0;
  Object.assign(globalThis, {
    updateTestEvents: { async listen() { if (++listens === 1) throw new Error('测试订阅失败'); return () => {}; } },
    updateTestCore: { async invoke() { calls++; return base; } },
  });
  const api = await loadApi();
  const watcher = api.watchStarclipUpdates(() => {});
  await assert.rejects(watcher.read(), /测试订阅失败/); await watcher.read(); watcher.dispose();
  assert.equal(listens, 2); assert.equal(calls, 1);
  const browser = await loadApi(false);
  const seen: UpdateStatus[] = [];
  const demo = browser.watchStarclipUpdates((state: UpdateStatus) => seen.push(state));
  await demo.read(); await demo.check(); await demo.download('0.3.0'); await demo.install('0.3.0');
  assert.deepEqual(seen, []); assert.equal(calls, 1); assert.equal(listens, 2);
  assert.equal(browser.initialUpdateStatus().phase, 'idle');
  await assert.rejects(browser.setAutoUpdateCheck(true), /桌面版/);
  demo.dispose();
});


test('update preferences use dedicated native commands and the browser stays offline', async () => {
  const preferences = { revision: 3, autoCheck: true, skippedVersion: null, lastCheckAt: 1760000000000 };
  const commands: unknown[] = []; let stops = 0;
  Object.assign(globalThis, {
    updateTestEvents: { async listen(name: string) { assert.equal(name, 'starclip-update-preferences'); return () => { stops++; }; } },
    updateTestCore: { async invoke(command: string, args?: Record<string, unknown>) { commands.push([command, args]); return preferences; } },
  });
  const api = await loadApi();
  assert.deepEqual(await api.getUpdatePreferences(), preferences);
  await api.setAutoUpdateCheck(false); await api.skipUpdateVersion('0.3.0');
  const stop = await api.onUpdatePreferencesChanged(() => {}); stop(); assert.equal(stops, 1);
  assert.deepEqual(commands, [['get_update_preferences', undefined], ['set_auto_update_check', { enabled: false }], ['skip_update_version', { version: '0.3.0' }]]);
  const browser = await loadApi(false);
  assert.equal((await browser.getUpdatePreferences()).autoCheck, true);
  (await browser.onUpdatePreferencesChanged(() => assert.fail('no synthetic preference event')))();
  await assert.rejects(browser.setAutoUpdateCheck(false), /桌面版/);
  await assert.rejects(browser.skipUpdateVersion('0.3.0'), /桌面版/);
  assert.equal(commands.length, 3); assert.equal(stops, 1);
});


test('manual recovery commands are absent from the frontend API and window permissions', async () => {
  const api = await loadApi();
  for (const name of ['listRecoveryPoints', 'createRecoveryPoint', 'restoreRecoveryPoint']) assert.equal(name in api, false);
  const permissions = JSON.parse(fs.readFileSync(new URL('../src-tauri/permissions/windows.json', import.meta.url), 'utf8'));
  for (const permission of permissions.permission) {
    for (const command of ['list_recovery_points', 'create_recovery_point', 'restore_recovery_point']) assert(!permission.commands.allow.includes(command));
  }
});
