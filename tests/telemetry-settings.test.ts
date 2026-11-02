import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { compileScript, parse } from '@vue/compiler-sfc';
import { createRenderer, markRaw, nextTick } from 'vue';
import type { TelemetryMode, TelemetryState } from '../src/telemetry-api.ts';

// Exercise the real settings panel without a WebView, network or clipboard.
type TestNode = { type: string; text: string; props: Record<string, any>; children: TestNode[]; parent?: TestNode };
function node(type: string, text = ''): TestNode { return markRaw({ type, text, props: {}, children: [] }); }
const renderer = createRenderer<TestNode, TestNode>({
  createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
  setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.text = text; node.children = []; },
  patchProp: (node, key, _previous, value) => { node.props[key] = value; },
  insert: (child, parent, anchor) => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); child.parent = parent; const target = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(target >= 0 ? target : parent.children.length, 0, child); },
  remove: child => { const index = child.parent?.children.indexOf(child) ?? -1; if (index >= 0) child.parent!.children.splice(index, 1); },
  parentNode: node => node.parent || null, nextSibling: node => node.parent?.children[(node.parent?.children.indexOf(node) ?? -1) + 1] || null,
});
function nodes(root: TestNode): TestNode[] { return [root, ...root.children.flatMap(nodes)]; }
function text(root: TestNode): string { return root.type === '#comment' ? '' : root.text + root.children.map(text).join(''); }
function button(root: TestNode, label: string): TestNode { const found = nodes(root).find(item => item.type === 'button' && text(item) === label); assert(found, `missing button: ${label}`); return found; }
async function flush() { await new Promise(resolve => setImmediate(resolve)); await nextTick(); }
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
let moduleNumber = 0;
async function mount(mock: Record<string, unknown>, props: Record<string, unknown> = {}) {
  Object.assign(globalThis, { telemetryPanelMock: mock });
  const api = `const api = globalThis.telemetryPanelMock; export const { ${Object.keys(mock).filter(key => /^[A-Za-z_$][\w$]*$/.test(key)).join(', ')} } = api;`;
  const apiUrl = 'data:text/javascript;base64,' + Buffer.from(api + `\n// ${moduleNumber++}`).toString('base64');
  const source = fs.readFileSync(new URL('../src/components/TelemetrySettings.vue', import.meta.url), 'utf8');
  const { descriptor } = parse(source);
  const script = compileScript(descriptor, { id: 'telemetry-panel-test', inlineTemplate: true });
  const code = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText
    .replace(/from (['"])vue\1/g, `from '${import.meta.resolve('vue')}'`)
    .replaceAll("'../telemetry-api'", `'${apiUrl}'`)
    .replaceAll("'../i18n'", `'${new URL('../src/i18n/index.ts', import.meta.url).href}'`);
  const component = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  const root = node('root'); const app = renderer.createApp(component.default, props);
  const exposed = app.mount(root) as unknown as { canRelease: boolean }; await flush();
  return { root, app, exposed };
}

async function loadApi(desktop: boolean) {
  Object.assign(globalThis, { window: desktop ? { __TAURI_INTERNALS__: {} } : {} });
  const source = fs.readFileSync(new URL('../src/telemetry-api.ts', import.meta.url), 'utf8')
    .replaceAll("import('@tauri-apps/api/core')", 'Promise.resolve(globalThis.telemetryTestCore)')
    .replaceAll("import('@tauri-apps/api/event')", 'Promise.resolve(globalThis.telemetryTestEvents)');
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  return import('data:text/javascript;base64,' + Buffer.from(code + `\n// module ${moduleNumber++}`).toString('base64'));
}

const initialState = (): TelemetryState => ({
  revision: 0, mode: 'off', enabled: false, configured: false, endpointHost: null, queuedEvents: [],
  lastResult: 'never', identityError: null, identityPresent: false,
});
const eventNames = ['app_started', 'studio_opened', 'card_exported', 'backup_exported', 'backup_imported', 'update_check_completed', 'update_download_completed', 'update_install_started', 'font_downloaded', 'app_active'];
function nativeMock(initial: TelemetryState = initialState()) {
  let state = { ...initial };
  let receive: (value: TelemetryState) => void = () => {};
  const calls: string[] = [];
  const mock = {
    calls, telemetryDesktopAvailable: true, usageEventNames: eventNames, initialTelemetryState: initialState,
    async getTelemetryState() { calls.push('read'); return { ...state }; },
    async onTelemetryStateChanged(callback: typeof receive) { calls.push('listen'); receive = callback; return () => { calls.push('dispose'); }; },
    send(value: TelemetryState) { state = value; receive(value); },
    async setTelemetryMode(mode: TelemetryMode) {
      calls.push(`mode:${mode}`);
      const installation = mode === 'installation';
      state = {
        ...state, revision: state.revision + 1, mode, enabled: mode !== 'off',
        queuedEvents: mode === 'off' ? [] : state.queuedEvents, lastResult: mode === 'off' ? 'disabled' : 'never',
        identityPresent: installation, identityError: null,
      };
      return { ...state };
    },
    async resetTelemetryIdentity() {
      calls.push('reset');
      state = { ...state, revision: state.revision + 1, identityPresent: true, identityError: null, queuedEvents: [] };
      return { ...state };
    },
    async flushTelemetry() { calls.push('send'); state = { ...state, revision: state.revision + 1, queuedEvents: [], lastResult: 'sent' }; return { ...state }; },
    async clearTelemetryQueue() { calls.push('clear'); state = { ...state, revision: state.revision + 1, queuedEvents: [] }; return { ...state }; },
  };
  return mock;
}
function telemetrySwitch(root: TestNode): TestNode {
  const found = nodes(root).find(item => item.props.role === 'switch' && item.props['aria-label'] === '帮助改进星笺');
  assert(found, 'missing usage consent switch'); return found;
}
function modeInput(root: TestNode, mode: TelemetryMode): TestNode {
  const found = nodes(root).find(item => item.type === 'input' && item.props.value === mode);
  assert(found, `missing mode option: ${mode}`); return found;
}

test('an unconfigured receiver cannot enable or send despite the default-on policy', async () => {
  const mock = nativeMock(); const { root, app } = await mount(mock);
  assert.deepEqual(mock.calls, ['listen', 'read'], 'opening settings only reads local state');
  assert.equal(telemetrySwitch(root).props['aria-checked'], false);
  assert.equal(telemetrySwitch(root).props.disabled, true);
  assert.match(text(root), /默认开启 · 随时切换 · 独立即时保存/);
  assert.match(text(root), /接收端未配置，当前不会收集或发送使用统计/);
  assert.equal(button(root, '发送待发统计').props.disabled, true);
  // Only "send nothing" stays selectable while the relay is unconfigured.
  assert.equal(modeInput(root, 'off').props.disabled, false);
  assert.equal(modeInput(root, 'session').props.disabled, true);
  assert.equal(modeInput(root, 'installation').props.disabled, true);
  await telemetrySwitch(root).props.onClick(); await modeInput(root, 'session').props.onChange();
  await button(root, '发送待发统计').props.onClick(); await flush();
  assert.deepEqual(mock.calls, ['listen', 'read']);
  app.unmount(); assert.equal(mock.calls.at(-1), 'dispose');
});

test('configured native preferences show the saved mode without changing it', async () => {
  for (const mode of ['off', 'session', 'installation'] as const) {
    const mock = nativeMock({
      ...initialState(), mode, enabled: mode !== 'off', configured: true, endpointHost: 'fixture.example',
      identityPresent: mode === 'installation',
    });
    const { root, app } = await mount(mock);
    assert.equal(telemetrySwitch(root).props['aria-checked'], mode !== 'off');
    assert.equal(telemetrySwitch(root).props.disabled, false);
    assert.equal(modeInput(root, mode).props.checked, true);
    for (const other of ['off', 'session', 'installation'] as const) {
      assert.equal(modeInput(root, other).props.checked, other === mode);
      assert.equal(modeInput(root, other).props.disabled, false);
    }
    assert.match(text(root), /默认开启 · 随时切换 · 独立即时保存/);
    assert.deepEqual(mock.calls, ['listen', 'read'], 'mounting must not change the mode or flush telemetry');
    app.unmount();
  }
});

test('statistics deliveries and network failures stay silent, including an IPC rejection', async () => {
  const mock = nativeMock({ ...initialState(), mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'app_started', count: 1 }] });
  let rejectSend = false;
  const send = mock.flushTelemetry;
  mock.flushTelemetry = async () => { if (rejectSend) throw new Error('FIXTURE_NETWORK_FAILURE'); return send(); };
  const { root, app } = await mount(mock);
  assert.match(text(root), /发送给 Google Analytics 4；/);
  assert.doesNotMatch(text(root), /Microsoft|Application Insights|两个服务|免费版/);
  await button(root, '发送待发统计').props.onClick(); await flush();
  assert.doesNotMatch(text(root), /使用统计已发送|暂时未能发送统计/);
  assert.doesNotMatch(text(root), /Microsoft|Application Insights|两个服务|报表|入库/);
  mock.send({ ...initialState(), revision: 2, mode: 'session', enabled: true, configured: true, lastResult: 'failed' }); await flush();
  assert.doesNotMatch(text(root), /使用统计已发送|暂时未能发送统计/);
  assert.deepEqual(mock.calls, ['listen', 'read', 'send']);
  mock.send({ ...initialState(), revision: 3, mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'studio_opened', count: 1 }] }); await flush();
  rejectSend = true;
  await button(root, '发送待发统计').props.onClick(); await flush();
  assert.doesNotMatch(text(root), /FIXTURE_NETWORK_FAILURE/);
  assert.equal(nodes(root).some(item => item.props.role === 'alert'), false);
  assert.match(text(root), /最多保留 72 小时、每种 100 次/);
  assert.match(text(root), /不收集序列号、电脑名称、用户名或系统账户标识/);
  assert.doesNotMatch(text(root), /永久设备标识/);
  app.unmount();
});

test('enabling waits for confirmation and a failed save keeps consent off', async () => {
  const mock = nativeMock({ ...initialState(), configured: true, endpointHost: 'fixture.example' });
  const saving = deferred<TelemetryState>(); const busy: boolean[] = [];
  let attempts = 0;
  mock.setTelemetryMode = async mode => {
    mock.calls.push(`mode:${mode}`);
    return ++attempts === 1 ? saving.promise : { ...initialState(), revision: 1, mode, enabled: true, configured: true };
  };
  const { root, app } = await mount(mock, { onBusy: (value: boolean) => busy.push(value) });
  assert.equal(telemetrySwitch(root).props.disabled, false);
  const toggle = telemetrySwitch(root).props.onClick(); await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], false, 'consent is not optimistically enabled');
  assert.equal(telemetrySwitch(root).props.disabled, true); assert.equal(busy.at(-1), true);
  await telemetrySwitch(root).props.onClick(); assert.equal(mock.calls.filter(call => call === 'mode:session').length, 1);
  saving.reject(new Error('固定测试：统计偏好保存失败')); await toggle; await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], false);
  assert.equal(telemetrySwitch(root).props.disabled, true); assert.equal(busy.at(-1), false);
  assert.match(text(root), /统计偏好保存失败/); assert(!mock.calls.includes('send'));
  await button(root, '重试保存统计选择').props.onClick(); await flush();
  assert.deepEqual(mock.calls.filter(call => call.startsWith('mode:')), ['mode:session', 'mode:session']);
  assert.equal(telemetrySwitch(root).props.disabled, false);
  assert.equal(telemetrySwitch(root).props['aria-checked'], true);
  app.unmount();
});

test('confirmed disable clears the displayed pending queue without flushing it', async () => {
  const mock = nativeMock({ ...initialState(), revision: 3, mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'card_exported', count: 2 }] });
  const { root, app } = await mount(mock);
  assert.match(text(root), /导出图片卡片 · 2/);
  await telemetrySwitch(root).props.onClick(); await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], false);
  assert.match(text(root), /没有待发送事件/); assert.match(text(root), /统计已关闭，待发送事件已清空/);
  assert.equal(button(root, '发送待发统计').props.disabled, true);
  assert.deepEqual(mock.calls, ['listen', 'read', 'mode:off']);
  app.unmount();
});

test('retrying a failed consent write preserves the disable choice after runtime collection already stopped', async () => {
  const mock = nativeMock({ ...initialState(), revision: 4, mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'studio_opened', count: 2 }] });
  const save = mock.setTelemetryMode; let attempts = 0;
  mock.setTelemetryMode = async mode => {
    if (++attempts > 1) return save(mode);
    mock.calls.push(`mode:${mode}`);
    mock.send({ ...initialState(), revision: 5, mode: 'off', enabled: false, configured: true, lastResult: 'disabled' });
    throw new Error('固定测试：本次已停止，但偏好保存失败，重启前请重试。');
  };
  const { root, app, exposed } = await mount(mock);
  assert.equal(exposed.canRelease, true);
  await telemetrySwitch(root).props.onClick(); await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], false);
  assert.match(text(root), /没有待发送事件/); assert.match(text(root), /本次已停止，但偏好保存失败/);
  assert.equal(exposed.canRelease, false, 'an unsaved consent choice keeps the settings state alive');
  assert.equal(telemetrySwitch(root).props.disabled, true);
  await telemetrySwitch(root).props.onClick(); await flush();
  assert.deepEqual(mock.calls.filter(call => call.startsWith('mode:')), ['mode:off']);
  await button(root, '重试保存统计选择').props.onClick(); await flush();
  assert.deepEqual(mock.calls.filter(call => call.startsWith('mode:')), ['mode:off', 'mode:off']);
  assert.equal(telemetrySwitch(root).props['aria-checked'], false);
  assert.equal(exposed.canRelease, true); assert.doesNotMatch(text(root), /重试保存统计选择|偏好保存失败/);
  assert(!mock.calls.includes('send')); app.unmount();
});

test('per-installation reporting is only reachable through its own explicit choice', async () => {
  const mock = nativeMock({ ...initialState(), revision: 2, configured: true, endpointHost: 'fixture.example' });
  const { root, app } = await mount(mock);
  assert.doesNotMatch(text(root), /本机统计标识已保存|重置统计标识/);
  // Turning the master switch on selects the default per-run identity, never the persistent one.
  await telemetrySwitch(root).props.onClick(); await flush();
  assert.deepEqual(mock.calls, ['listen', 'read', 'mode:session']);
  assert.equal(modeInput(root, 'session').props.checked, true);
  await modeInput(root, 'installation').props.onChange(); await flush();
  assert.equal(mock.calls.at(-1), 'mode:installation');
  assert.equal(modeInput(root, 'installation').props.checked, true);
  assert.match(text(root), /本机统计标识已保存，只用于区分安装次数/);
  assert.match(text(root), /用于估算活跃安装数/);
  await button(root, '重置统计标识').props.onClick(); await flush();
  assert.equal(mock.calls.at(-1), 'reset');
  assert.match(text(root), /统计标识已重置，后续统计将使用新的编号/);
  // Selecting the mode that is already active never writes again.
  await modeInput(root, 'installation').props.onChange(); await flush();
  assert.equal(mock.calls.at(-1), 'reset');
  app.unmount();
});

test('a local identity failure is shown and stops the per-installation mode', async () => {
  const mock = nativeMock({
    ...initialState(), revision: 6, mode: 'installation', enabled: false, configured: true,
    identityError: '统计标识未能保存，已停止按安装统计。', identityPresent: false,
    queuedEvents: [{ name: 'app_started', count: 1 }],
  });
  const { root, app } = await mount(mock);
  const alerts = nodes(root).filter(item => item.props.role === 'alert').map(text);
  assert.deepEqual(alerts, ['统计标识未能保存，已停止按安装统计。']);
  assert.doesNotMatch(text(root), /本机统计标识已保存/);
  // Consent stays on, but the backend reports that nothing can be delivered.
  assert.equal(telemetrySwitch(root).props['aria-checked'], true);
  assert.equal(button(root, '发送待发统计').props.disabled, true);
  assert.equal(button(root, '重置统计标识').props.disabled, false);
  app.unmount();
});

test('failed identity deletion remains visible in off and session modes and retries the same choice', async () => {
  for (const mode of ['off', 'session'] as const) {
    const mock = nativeMock({ ...initialState(), configured: true, mode, enabled: mode !== 'off',
      identityError: '统计标识未能删除；请检查目录写入权限后重试。' });
    const { root, app } = await mount(mock);
    assert.match(text(root), /统计标识未能删除/);
    assert(button(root, '重试统计标识处理'));
    await button(root, '重试统计标识处理').props.onClick(); await flush();
    assert(mock.calls.includes(`mode:${mode}`));
    app.unmount();
  }
});

test('off mode shows a failed cache revocation until an explicit retry succeeds', async () => {
  const state={ ...initialState(), configured: true, cacheError: '本次待发统计已清空，但本地缓存未能删除；请恢复目录写入权限后重试，重启可能恢复旧计数。' };
  const mock=nativeMock(state);
  mock.clearTelemetryQueue=async () => { mock.calls.push('clear'); const next={...state,revision:1,cacheError:null};mock.send(next);return next; };
  const {root,app}=await mount(mock);
  assert.match(text(root),/本地缓存未能删除/);
  await button(root,'重试清空统计缓存').props.onClick();await flush();
  assert(mock.calls.includes('clear'));assert.doesNotMatch(text(root),/本地缓存未能删除/);app.unmount();
});

test('event preview renders only fixed labels and bounded counts, never arbitrary payload fields', async () => {
  const mock = nativeMock({
    ...initialState(), configured: true, mode: 'session', enabled: true,
    queuedEvents: [
      { name: 'card_exported', count: 3, payload: 'FIXTURE_PRIVATE_PAYLOAD', content: '<img src=x>' },
      { name: 'update_download_completed', count: 2 },
      { name: 'update_install_started', count: 1 },
      { name: 'FIXTURE_UNKNOWN_EVENT', count: 7 },
      { name: 'studio_opened', count: 'FIXTURE_TEXT_COUNT' },
      { name: 'backup_imported', count: -1 },
      { name: 'font_downloaded', count: 101 },
    ],
  } as unknown as TelemetryState);
  const { root, app } = await mount(mock);
  assert.deepEqual(nodes(root).filter(item => item.type === 'li').map(text), ['导出图片卡片 · 3', '更新包下载完成 · 2', '开始安装更新 · 1']);
  assert.match(text(root), /统计在后台静默发送，失败不提示/);
  assert.match(text(root), /安装计数表示开始尝试，不代表安装成功；退出前未送达的计数可能丢失/);
  assert.match(text(root), /本机固定编号模式下记录当日使用和每小时运行心跳/);
  assert.doesNotMatch(text(root), /FIXTURE_|<img src=x>/);
  assert.equal(nodes(root).some(item => item.type === 'img'), false);
  assert(!mock.calls.includes('send'), 'preview does not send events'); app.unmount();
});

test('a newer event wins over both a delayed initial read and a delayed preference response', async () => {
  const mock = nativeMock(); const read = deferred<TelemetryState>();
  mock.getTelemetryState = async () => { mock.calls.push('read'); return read.promise; };
  const { root, app } = await mount(mock);
  mock.send({ ...initialState(), revision: 5, mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'studio_opened', count: 4 }] });
  read.resolve({ ...initialState(), revision: 2 }); await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], true); assert.match(text(root), /打开星绘坊 · 4/);
  const saved = deferred<TelemetryState>();
  mock.setTelemetryMode = async mode => { mock.calls.push(`mode:${mode}`); return saved.promise; };
  const disabling = telemetrySwitch(root).props.onClick(); await flush();
  mock.send({ ...initialState(), revision: 7, mode: 'off', enabled: false, configured: true, lastResult: 'disabled' });
  saved.resolve({ ...initialState(), revision: 6, mode: 'session', enabled: true, configured: true, queuedEvents: [{ name: 'studio_opened', count: 9 }] });
  await disabling; await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], false); assert.match(text(root), /没有待发送事件/);
  assert.doesNotMatch(text(root), /打开星绘坊 · 9/); app.unmount();
});

test('browser panel remains offline and disabled even when handlers are invoked directly', async () => {
  const mock = nativeMock(); mock.telemetryDesktopAvailable = false;
  const { root, app } = await mount(mock);
  assert.match(text(root), /使用统计设置仅在桌面版可用/);
  assert.equal(telemetrySwitch(root).props.disabled, true);
  await telemetrySwitch(root).props.onClick(); await modeInput(root, 'installation').props.onChange();
  await button(root, '发送待发统计').props.onClick(); await button(root, '清空待发送事件').props.onClick();
  await flush();
  assert.deepEqual(mock.calls, []); app.unmount(); assert.deepEqual(mock.calls, []);
});

test('late telemetry subscriptions are released without reading after unmount', async () => {
  const mock = nativeMock(); const listening = deferred<() => void>(); let stopped = 0;
  mock.onTelemetryStateChanged = () => listening.promise;
  const { app } = await mount(mock); app.unmount();
  listening.resolve(() => { stopped++; }); await flush();
  assert.equal(stopped, 1); assert.deepEqual(mock.calls, []);
});

test('retry after subscription failure reconnects before reading local telemetry state', async () => {
  const mock = nativeMock(); const subscribe = mock.onTelemetryStateChanged; let attempts = 0;
  mock.onTelemetryStateChanged = async callback => {
    if (++attempts === 1) throw new Error('固定测试：统计订阅失败');
    return subscribe(callback);
  };
  const { root, app } = await mount(mock);
  assert.match(text(root), /统计订阅失败/); assert.deepEqual(mock.calls, []);
  await button(root, '重新读取').props.onClick(); await flush();
  assert.equal(attempts, 2); assert.deepEqual(mock.calls, ['listen', 'read']);
  mock.send({ ...initialState(), revision: 3, mode: 'session', enabled: true, configured: true }); await flush();
  assert.equal(telemetrySwitch(root).props['aria-checked'], true); app.unmount();
});

test('actual browser API never invokes native IPC or subscribes, and studio events are ignored', async () => {
  let invokes = 0, subscriptions = 0;
  Object.assign(globalThis, {
    telemetryTestCore: { async invoke() { invokes++; throw new Error('unexpected native invoke'); } },
    telemetryTestEvents: { async listen() { subscriptions++; throw new Error('unexpected native subscription'); } },
  });
  const api = await loadApi(false);
  assert.equal(api.telemetryDesktopAvailable, false);
  assert.deepEqual(api.initialTelemetryState(), initialState());
  assert.deepEqual([...api.usageEventNames].sort(), [...eventNames].sort());
  // The heartbeat is a backend event and must never become a recordable UI action.
  assert.equal(api.usageEventNames.includes('app_running'), false);
  assert.deepEqual([...api.telemetryModes], ['off', 'session', 'installation']);
  await assert.rejects(api.getTelemetryState(), /桌面版/);
  await assert.rejects(api.setTelemetryMode('session'), /桌面版/);
  await assert.rejects(api.resetTelemetryIdentity(), /桌面版/);
  await assert.rejects(api.flushTelemetry(), /桌面版/);
  await assert.rejects(api.clearTelemetryQueue(), /桌面版/);
  (await api.onTelemetryStateChanged(() => assert.fail('browser event')))();
  api.recordStudioOpened(); await flush();
  assert.equal(invokes, 0); assert.equal(subscriptions, 0);
  delete (globalThis as { window?: unknown }).window;
});
