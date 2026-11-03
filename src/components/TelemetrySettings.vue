<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, watch } from 'vue';
import { errorText, t } from '../i18n';
import { clearTelemetryQueue, flushTelemetry, getTelemetryState, initialTelemetryState, onTelemetryStateChanged, resetTelemetryIdentity, setTelemetryMode, telemetryDesktopAvailable, usageEventNames } from '../telemetry-api';
import type { TelemetryMode, TelemetryState, UsageEvent } from '../telemetry-api';

const props = defineProps<{ disabled?: boolean }>();
const emit = defineEmits<{ busy: [value: boolean] }>();
const state = ref(initialTelemetryState());
const loaded = ref(!telemetryDesktopAvailable);
const working = ref(false);
const error = ref('');
const identityNotice = ref('');
const retryChoice = ref<TelemetryMode | null>(null);
let disposed = false;
let stop: (() => void) | undefined;
const canRelease = computed(() => loaded.value && !working.value && retryChoice.value === null);
defineExpose({ canRelease });
watch(working, value => emit('busy', value), { flush: 'sync' });
const labels: Record<UsageEvent, string> = {
  app_started: '启动应用', studio_opened: '打开星绘坊', card_exported: '导出图片卡片',
  backup_exported: '导出备份', backup_imported: '导入备份', update_check_completed: '完成更新检查', font_downloaded: '下载字体',
  update_download_completed: '更新包下载完成', update_install_started: '开始安装更新', app_active: '当日使用',
};
// The three persisted modes are the only choices; per-installation reporting is never implicit.
const modeOptions: ReadonlyArray<{ value: TelemetryMode; name: string; detail: string }> = [
  { value: 'off', name: '不发送统计', detail: '不记录也不发送任何使用事件。' },
  { value: 'session', name: '每次运行随机编号', detail: '默认方式：每次启动换一个随机编号，无法跨启动关联。' },
  { value: 'installation', name: '本机固定编号', detail: '使用保存在本机的随机编号，用于估算活跃安装数；可随时重置。' },
];
// Render only the known schema even if an unexpected native payload is received.
const pending = computed(() => usageEventNames.flatMap(name => {
  const count = state.value.queuedEvents.find(event => event.name === name)?.count;
  return Number.isInteger(count) && count! > 0 && count! <= 100 ? [{ name, count: count! }] : [];
}));
const result = computed(() => ({
  never: '', sent: '', failed: '',
  disabled: '统计已关闭，待发送事件已清空。', unconfigured: '接收端未配置，当前不会收集或发送使用统计。',
}[state.value.lastResult]));
// Consent is the chosen mode; `state.enabled` additionally reports whether sending can
// actually happen (a local identity failure keeps the mode but stops delivery).
const enabled = computed(() => state.value.mode !== 'off');
const modeDisabled = computed(() => !loaded.value || props.disabled || working.value
  || retryChoice.value !== null || !telemetryDesktopAvailable);
const sendable = computed(() => state.value.enabled && state.value.configured && pending.value.length > 0);
function accept(next: TelemetryState) {
  if (!disposed && next.revision >= state.value.revision) state.value = next;
}
async function read() {
  if (!telemetryDesktopAvailable || disposed || working.value) return;
  working.value = true; error.value = '';
  try {
    if (!stop) {
      const unsubscribe = await onTelemetryStateChanged(accept);
      if (disposed) { unsubscribe(); return; }
      stop = unsubscribe;
    }
    accept(await getTelemetryState());
    if (!disposed) loaded.value = true;
  } catch (e) { if (!disposed) error.value = errorText(e); }
  finally { working.value = false; }
}
async function perform(action: () => Promise<TelemetryState>, silent = false) {
  if (!loaded.value || props.disabled || working.value || !telemetryDesktopAvailable) return;
  working.value = true; error.value = ''; identityNotice.value = '';
  try { accept(await action()); } catch (e) { if (!disposed && !silent) error.value = errorText(e); }
  finally { working.value = false; }
}
function choose(mode: TelemetryMode) {
  if (retryChoice.value !== null || modeDisabled.value || (mode === state.value.mode && !state.value.identityError)) return;
  // An unconfigured receiver can never be switched on, but it can always be switched off.
  if (mode !== 'off' && !state.value.configured) return;
  return saveChoice(mode);
}
function saveChoice(mode: TelemetryMode) {
  return perform(async () => {
    try { const next = await setTelemetryMode(mode); retryChoice.value = null; return next; }
    catch (e) { retryChoice.value = mode; throw e; }
  });
}
function resetIdentity() {
  if (state.value.mode !== 'installation' || modeDisabled.value) return;
  return perform(async () => {
    const next = await resetTelemetryIdentity();
    identityNotice.value = '统计标识已重置，后续统计将使用新的编号。';
    return next;
  });
}
function send() {
  if (!sendable.value) return;
  return perform(flushTelemetry, true);
}
onMounted(read);
onBeforeUnmount(() => { disposed = true; stop?.(); emit('busy', false); });
</script>

<template>
  <section class="telemetry-settings" :aria-label="t('可选使用统计')">
    <h3>{{ t('可选使用统计') }}</h3>
    <div class="setting-row">
      <div><strong>{{ t('帮助改进星笺') }}</strong><p>{{ t('默认开启 · 随时切换 · 独立即时保存') }}</p></div>
      <button type="button" role="switch" :class="['switch', { on: enabled }]" :aria-checked="enabled" :aria-label="t('帮助改进星笺')" :disabled="modeDisabled || (!state.configured && !enabled)" @click="choose(enabled ? 'off' : 'session')"><span /></button>
    </div>
    <p>{{ t('固定功能的使用次数发送给 Google Analytics 4；可随时关闭。') }}</p>
    <p>{{ t('使用统计用于改进功能与更新体验。只发送固定事件及次数、应用版本、系统与主版本、应用架构、机型代号或产品系列，以及随机统计编号。') }}</p>
    <p>{{ t('统计编号有两种：默认每次运行更换；选择「本机固定编号」后改用保存在本机的随机编号，用于估算活跃安装数。它不含硬件信息，可随时重置。') }}</p>
    <p>{{ t('旧版已开启的统计设置升级为每次运行随机编号，已关闭的保持关闭；如需按安装统计，请明确选择本机固定编号。') }}</p>
    <p>{{ t('本机固定编号模式会另发送已知的编号生成日期，用于留存分组；旧编号日期未知时不会补造。') }}</p>
    <p>{{ t('不收集序列号、电脑名称、用户名或系统账户标识。机型无法安全识别时只记录通用类别。') }}</p>
    <p>{{ t('不收集剪贴内容、搜索词、图片、标题、链接、文件路径，也不录屏。接收端会接触网络 IP，但本实现不记录或转发它；云服务仍按各自政策处理请求。') }}</p>
    <fieldset class="telemetry-modes">
      <legend>{{ t('统计编号方式') }}</legend>
      <label v-for="option in modeOptions" :key="option.value" :class="['telemetry-mode', { active: state.mode === option.value }]">
        <input type="radio" name="telemetry-mode" :value="option.value" :checked="state.mode === option.value" :aria-label="t(option.name)" :disabled="modeDisabled || (option.value !== 'off' && !state.configured)" @change="choose(option.value)">
        <span><strong>{{ t(option.name) }}</strong><em>{{ t(option.detail) }}</em></span>
      </label>
    </fieldset>
    <p v-if="state.cacheError" role="alert" class="telemetry-error">{{ t(state.cacheError) }}</p>
    <button v-if="state.cacheError" type="button" class="button subtle" :disabled="disabled || working" @click="perform(clearTelemetryQueue)">{{ t('重试清空统计缓存') }}</button>
    <p v-if="state.identityError" role="alert" class="telemetry-error">{{ t(state.identityError) }}</p>
    <button v-if="state.identityError" type="button" class="button subtle" :disabled="modeDisabled" @click="saveChoice(state.mode)">{{ t('重试统计标识处理') }}</button>
    <div v-if="state.mode === 'installation'" class="telemetry-identity">
      <p v-if="!state.identityError && state.identityPresent">{{ t('本机统计标识已保存，只用于区分安装次数。') }}</p>
      <button type="button" class="button subtle" :disabled="modeDisabled || !state.configured" @click="resetIdentity">{{ t('重置统计标识') }}</button>
      <p v-if="identityNotice" role="status">{{ t(identityNotice) }}</p>
    </div>
    <p v-if="!telemetryDesktopAvailable" class="telemetry-status">{{ t('使用统计设置仅在桌面版可用。') }}</p>
    <p v-else-if="loaded && !state.configured" class="telemetry-status">{{ t('接收端未配置，当前不会收集或发送使用统计。') }}</p>
    <p v-else-if="state.endpointHost">{{ t('接收端：{host}', { host: state.endpointHost }) }}</p>
    <details>
      <summary>{{ t('查看统计范围与待发送事件') }}</summary>
      <p>{{ t('统计范围：启动应用、打开星绘坊、导出图片卡片、导入与导出备份、完成更新检查、更新包下载完成、开始安装更新、下载字体；本机固定编号模式另含当日使用。') }}</p>
      <p>{{ t('统计在后台静默发送，失败不提示。断网时仅缓存固定次数，最多保留 72 小时、每种 100 次；自动逐步延长重试间隔，最长一小时。关闭统计会清空待发计数和本地缓存。') }}</p>
      <p>{{ t('缓存不含剪贴内容、设备信息或会话编号；升级版本时舍弃旧缓存。补报按收到时间统计，可能漏计或重复，不用于精确追踪个人。') }}</p>
      <p>{{ t('会话模式的待发计数仅归属本次运行，重启不跨会话补报。') }}</p>
      <p>{{ t('缓存记录固定次数、模式和归属摘要；身份切换或旧格式缓存不会跨编号补报。') }}</p>
      <p>{{ t('安装计数表示开始尝试，不代表安装成功；退出前未送达的计数可能丢失。') }}</p>
      <p>{{ t('本机固定编号模式下记录当日使用和每小时运行心跳；心跳不代表实际使用或使用时长。') }}</p>
      <ul v-if="pending.length"><li v-for="event in pending" :key="event.name">{{ t(labels[event.name]) }} · {{ event.count }}</li></ul>
      <p v-else>{{ t('没有待发送事件') }}</p>
      <div class="telemetry-actions">
        <button type="button" class="button subtle" :disabled="disabled || working || !sendable" @click="send">{{ t('发送待发统计') }}</button>
        <button type="button" class="button subtle" :disabled="disabled || working || !pending.length" @click="perform(clearTelemetryQueue)">{{ t('清空待发送事件') }}</button>
      </div>
      <p v-if="result" role="status">{{ t(result) }}</p>
    </details>
    <p v-if="error" role="alert" class="telemetry-error">{{ error }}</p>
    <button v-if="retryChoice !== null" type="button" class="button subtle" :disabled="disabled || working" @click="saveChoice(retryChoice)">{{ t('重试保存统计选择') }}</button>
    <button v-if="telemetryDesktopAvailable && !loaded" type="button" class="button subtle" :disabled="working" @click="read">{{ t('重新读取') }}</button>
  </section>
</template>

<style scoped>
.telemetry-settings{border-top:1px solid var(--line);padding-top:24px;margin-top:28px;min-width:0}.telemetry-settings h3{font-size:13px;color:var(--gold);margin:0 0 18px}.telemetry-settings p,.telemetry-settings li{font-size:12px;line-height:1.8;color:var(--muted);overflow-wrap:anywhere}.telemetry-settings summary{font-size:12px;cursor:pointer;color:var(--text);padding:8px 0}.telemetry-status{padding:12px;background:var(--input);border:1px solid var(--line);border-radius:9px}.telemetry-actions{display:flex;gap:10px;flex-wrap:wrap}.telemetry-settings .telemetry-error{color:var(--danger)}
.telemetry-modes{border:0;padding:0;margin:14px 0;display:grid;gap:8px;min-width:0}.telemetry-modes legend{font-size:12px;color:var(--muted);padding:0 0 6px}.telemetry-mode{display:flex;gap:9px;align-items:flex-start;padding:10px 12px;border:1px solid var(--line);border-radius:9px;cursor:pointer}.telemetry-mode.active{border-color:var(--gold)}.telemetry-mode span{display:grid;gap:2px;min-width:0}.telemetry-mode strong{font-size:12px;color:var(--text);font-weight:500}.telemetry-mode em{font-size:11px;color:var(--muted);font-style:normal;line-height:1.6}.telemetry-mode input{margin:2px 0 0}.telemetry-identity{display:grid;gap:8px;justify-items:start}
</style>
