<script setup lang="ts">
import { errorText, formatDate, t } from '../i18n';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { getUpdatePreferences, initialUpdatePreferences, initialUpdateStatus, onUpdatePreferencesChanged, pendingDataDirectory, restartAfterUpdate, setAutoUpdateCheck, updateDesktopAvailable, watchStarclipUpdates } from '../update-api';
import type { UpdatePreferences } from '../update-api';
import { downloadProgress, fileSize, updateInProgress, updatePhaseText } from '../update-state';

const props = withDefaults(defineProps<{ disabled?: boolean; unsavedChanges?: boolean }>(), { disabled: false, unsavedChanges: false });
const emit = defineEmits<{ busy: [value: boolean]; 'restart-required': [] }>();
const status = ref(initialUpdateStatus());
const preferences = ref<UpdatePreferences>(initialUpdatePreferences());
const statusLoaded = ref(false);
const preferencesLoaded = ref(false);
const pendingLoaded = ref(false);
const pendingPath = ref<string | null>(null);
const operation = ref<'load' | 'check' | 'download' | 'install' | 'restart' | null>(null);
const preferencesOperation = ref(false);
const error = ref('');
const notice = ref('');
const installVersion = ref<string | null>(null);
const installDialog = ref<HTMLDialogElement>();
let disposed = false;
const monitor = watchStarclipUpdates(value => { status.value = value; statusLoaded.value = true; });
let stopPreferences: (() => void) | undefined;
const needsRestart = computed(() => !!pendingPath.value || ['installed', 'restart-required'].includes(status.value.phase));
const working = computed(() => operation.value !== null || preferencesOperation.value || updateInProgress(status.value));
const blocked = computed(() => props.disabled || working.value || needsRestart.value);
const progress = computed(() => downloadProgress(status.value));
const canDownload = computed(() => !!status.value.available && ['available', 'error'].includes(status.value.phase));
const canInstall = computed(() => !!status.value.available && status.value.phase === 'ready');
const canRelease = computed(() => (!updateDesktopAvailable || (statusLoaded.value && pendingLoaded.value))
  && (!updateDesktopAvailable || preferencesLoaded.value) && !working.value && !preferencesOperation.value && !needsRestart.value && !installVersion.value);
watch(() => working.value || needsRestart.value, value => emit('busy', value), { immediate: true, flush: 'sync' });
watch(needsRestart, value => { if (value) emit('restart-required'); }, { flush: 'sync' });
watch([() => status.value.phase, () => status.value.available?.version], ([phase, version]) => {
  if (phase === 'ready' && version) {
    installVersion.value = version;
  } else dismissInstall();
});
watch(installVersion, async version => {
  if (!version) return;
  await nextTick();
  if (!disposed && installVersion.value === version && !installDialog.value?.open) installDialog.value?.showModal();
});

function acceptPreferences(value: UpdatePreferences) {
  if (disposed || value.revision < preferences.value.revision) return;
  preferences.value = value; preferencesLoaded.value = true;
}
function message(reason: unknown) { return errorText(reason); }
function date(value: number | string) {
  if (typeof value === 'number' && value <= 0) return t('时间未知');
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? t('日期未提供') : formatDate(parsed, { dateStyle: 'medium', timeStyle: 'short' });
}
async function perform(name: NonNullable<typeof operation.value>, action: () => Promise<void>) {
  if (disposed || !updateDesktopAvailable || props.disabled || working.value || (needsRestart.value && name !== 'restart')) return;
  operation.value = name; error.value = ''; notice.value = '';
  try { await action(); } catch (reason) { if (!disposed) error.value = message(reason); }
  finally { if (!disposed) operation.value = null; }
}
function refresh() {
  return perform('load', async () => {
    const results = await Promise.allSettled([monitor.read(), getUpdatePreferences(), pendingDataDirectory()]);
    if (disposed) return;
    const [update, preference, pending] = results;
    const errors: string[] = [];
    if (update.status === 'rejected') errors.push(t('更新状态读取失败：{error}', { error: message(update.reason) }));
    if (preference.status === 'fulfilled') acceptPreferences(preference.value);
    else errors.push(t('更新偏好读取失败：{error}', { error: message(preference.reason) }));
    if (pending.status === 'fulfilled') { pendingPath.value = pending.value; pendingLoaded.value = true; }
    else errors.push(t('数据位置读取失败：{error}', { error: message(pending.reason) }));
    error.value = errors.join('\n');
  });
}
async function toggleAutomaticChecks() {
  if (disposed || !updateDesktopAvailable || blocked.value || !preferencesLoaded.value) return;
  const next = !preferences.value.autoCheck;
  preferencesOperation.value = true; error.value = '';
  try { acceptPreferences(await setAutoUpdateCheck(next)); }
  catch (reason) { if (!disposed) error.value = message(reason); }
  finally { if (!disposed) preferencesOperation.value = false; }
}
function check() { return perform('check', () => monitor.check()); }
function download() {
  const version = status.value.available?.version;
  if (!version || !canDownload.value) return;
  return perform('download', async () => {
    try { await monitor.download(version); }
    catch (reason) {
      try { await monitor.read(); } catch { /* Preserve the download error. */ }
      throw reason;
    }
  });
}
function confirmInstall() {
  if (blocked.value || !canInstall.value) return;
  installVersion.value = status.value.available!.version;
}
function dismissInstall() {
  installDialog.value?.close();
  installVersion.value = null;
}
function install() {
  const approvedVersion = installVersion.value;
  if (!approvedVersion || props.unsavedChanges || !canInstall.value || status.value.available?.version !== approvedVersion) return;
  return perform('install', async () => {
    dismissInstall();
    try { await monitor.install(approvedVersion); }
    catch (reason) {
      try { await monitor.read(); } catch { /* Preserve the installation error. */ }
      if (status.value.phase !== 'installed') throw reason;
    }
    if (!disposed && status.value.phase === 'installed') await restartInstalled();
  });
}
async function restartInstalled() {
  notice.value = t('正在重启星笺…');
  try { await restartAfterUpdate(); }
  catch (reason) { throw new Error(t('自动重启未完成，请再次点击「重启星笺」。{error}', { error: message(reason) })); }
}
function restart() { return perform('restart', restartInstalled); }
defineExpose({ refresh, canRelease });
onMounted(async () => {
  if (!updateDesktopAvailable) { preferencesLoaded.value = true; return; }
  try {
    const stop = await onUpdatePreferencesChanged(acceptPreferences);
    if (disposed) { stop(); return; }
    stopPreferences = stop;
  } catch { /* refresh below reports the actionable native state */ }
  if (!disposed) void refresh();
});
onBeforeUnmount(() => { disposed = true; installDialog.value?.close(); stopPreferences?.(); monitor.dispose(); emit('busy', false); });
</script>

<template>
  <section class="update-settings" aria-labelledby="software-update-heading" :aria-busy="working">
    <div class="update-heading"><h3 id="software-update-heading">{{ t('软件更新') }}</h3><div class="update-heading-actions"><span class="update-version">{{ t('当前版本') }} {{ status.currentVersion }}</span><button v-if="updateDesktopAvailable" type="button" class="update-refresh" :disabled="blocked" @click="refresh">{{ t('刷新状态') }}</button></div></div>
    <div v-if="!updateDesktopAvailable" class="update-card">
      <strong>{{ t('请在星笺桌面版检查更新') }}</strong>
      <p>{{ t('浏览器演示没有已安装的软件，不会检查、下载或安装桌面更新。') }}</p>
    </div>
    <template v-else>
      <p v-if="!statusLoaded" class="update-hint" role="status">{{ operation === 'load' ? t('正在读取本机状态…') : t('尚未读取更新状态。') }}</p>
      <p v-else class="update-hint" :class="{ 'update-error': ['error', 'restart-required'].includes(status.phase) }" :role="['error', 'restart-required'].includes(status.phase) ? 'alert' : 'status'">{{ updatePhaseText(status) }}</p>
      <p class="update-hint">{{ t('开启后在启动时检查，此后每 24 小时最多自动检查一次；仅连接 GitHub 获取更新信息，不上传剪贴内容。') }}</p>
      <div class="update-preference-row">
        <div><strong>{{ t('自动检查稳定版更新') }}</strong><p>{{ t('立即保存，可随时关闭；当前仅提供稳定版更新，预览通道尚未开放。') }}</p></div>
        <button type="button" class="switch" :class="{ on: preferences.autoCheck }" role="switch" :aria-checked="preferences.autoCheck" :aria-label="t('自动检查稳定版更新')" :disabled="preferencesOperation || blocked || !preferencesLoaded" @click="toggleAutomaticChecks"><span /></button>
      </div>
      <p class="update-hint">{{ t('也可随时手动检查。更新只提醒，不会自动下载或安装；确认安装时会自动备份数据，再安装并重启，历史、偏好和星伴成长保留在本机。') }}</p>
      <div v-if="status.available" class="update-card update-release">
        <div class="update-release-heading"><strong>{{ t('新版本') }} {{ status.available.version }}</strong><span v-if="status.available.date">{{ date(status.available.date) }}</span></div>
        <p v-if="status.available.notes" class="update-notes">{{ status.available.notes }}</p>
        <p v-else>{{ t('此版本未提供更新说明。') }}</p>
      </div>
      <div v-if="status.phase === 'downloading'" class="update-download" role="status">
        <progress :aria-label="t('更新下载进度')" :value="progress" max="100"></progress>
        <span>{{ fileSize(status.downloaded) }}<template v-if="status.total"> / {{ fileSize(status.total) }}</template><template v-if="progress !== undefined"> · {{ Math.floor(progress) }}%</template></span>
      </div>
      <div v-if="!needsRestart" class="update-actions">
        <button type="button" class="update-button" :disabled="blocked || (statusLoaded && !status.configured)" @click="statusLoaded ? check() : refresh()">{{ operation === 'check' || status.phase === 'checking' ? t('正在检查…') : statusLoaded ? t('检查更新') : t('重新读取状态') }}</button>
        <button v-if="canDownload" type="button" class="update-button update-primary" :disabled="blocked" @click="download">{{ status.phase === 'error' ? t('重新下载更新') : t('下载更新') }}</button>
        <button v-if="canInstall" type="button" class="update-button update-primary" :disabled="blocked" @click="confirmInstall">{{ t('安装并重启') }}</button>
      </div>
      <div v-if="needsRestart" class="update-confirmation" role="status">
        <strong>{{ pendingPath ? t('数据位置已变更，重启后继续') : status.phase === 'installed' ? t('新版已安装，重启后继续') : t('安装未完成，请先重启星笺') }}</strong>
        <p>{{ t('当前编辑已暂停，请重启星笺后继续使用。') }}</p>
        <details v-if="pendingPath"><summary>{{ t('下次启动的数据位置') }}</summary><code>{{ pendingPath }}</code></details>
        <button type="button" class="update-button update-primary" :disabled="operation !== null || disabled" @click="restart">{{ operation === 'restart' ? t('正在重启…') : t('重启星笺') }}</button>
      </div>
      <dialog v-if="installVersion && !needsRestart" ref="installDialog" class="update-ready-dialog" aria-labelledby="update-ready-heading" aria-describedby="update-ready-description" @cancel.prevent.stop="dismissInstall" @close.stop @keydown.stop @click="event => { if (event.target === installDialog) dismissInstall() }">
        <div class="update-ready-content">
          <div class="update-ready-heading"><h3 id="update-ready-heading">{{ t('新版本') }} {{ installVersion }} {{ t('已准备就绪') }}</h3><button type="button" class="update-ready-close" :aria-label="t('稍后安装更新')" @click="dismissInstall">×</button></div>
          <p id="update-ready-description">{{ t('更新包已下载并通过验证。安装前会自动备份当前数据，随后暂时关闭星笺，完成后自动打开。') }}</p>
          <p v-if="unsavedChanges" class="update-warning">{{ t('有尚未保存的偏好，请先选择「稍后再说」并保存，再安装更新。') }}</p>
          <div class="update-actions update-confirm-actions"><button type="button" class="update-button" autofocus @click="dismissInstall">{{ t('稍后再说') }}</button><button type="button" class="update-button update-primary" :disabled="blocked || unsavedChanges" @click="install">{{ t('安装并重启') }}</button></div>
        </div>
      </dialog>
      <p v-if="error" class="update-error" role="alert">{{ error }}</p>
      <p v-else-if="notice" class="update-feedback" role="status">{{ notice }}</p>
    </template>
  </section>
</template>

<style scoped>
.update-settings{min-width:0;padding:16px 0;border-top:1px solid var(--line);color:var(--text)}
.update-ready-dialog{width:min(440px,calc(100vw - 40px));max-height:calc(100dvh - 40px);padding:0;border:1px solid var(--line);border-radius:18px;background:var(--panel);color:var(--text);box-shadow:0 24px 80px #0005;overflow:auto}.update-ready-dialog::backdrop{background:#080b1866;backdrop-filter:blur(3px)}.update-ready-content{padding:26px}.update-ready-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:16px}.update-ready-heading h3{margin:0;font-size:17px;line-height:1.6;font-weight:500;overflow-wrap:anywhere}.update-ready-close{flex-shrink:0;display:grid;place-items:center;width:28px;height:28px;border:0;border-radius:6px;color:var(--muted);background:transparent;font-size:25px;cursor:pointer}.update-ready-close:hover{background:var(--input);color:var(--text)}.update-ready-content>p{margin:16px 0;color:var(--muted);font-size:12px;line-height:1.9}.update-ready-content>.update-warning{color:var(--gold)}.update-ready-dialog .update-actions{margin-top:24px}.update-ready-dialog .update-button{font-size:12px;min-height:40px;padding:9px 16px}.update-ready-dialog .update-primary{flex:1}.update-ready-dialog :focus-visible{outline:2px solid var(--gold);outline-offset:3px}
.update-heading,.update-release-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.update-heading{margin-bottom:10px}.update-heading-actions{display:flex;align-items:center;gap:12px}.update-heading h3,.update-heading h4{margin:0;font-size:11px;font-weight:500;color:var(--gold)}.update-version,.update-release-heading>span{font-size:10px;color:var(--muted);font-variant-numeric:tabular-nums}.update-hint,.update-card p,.update-confirmation p{font-size:10px;line-height:1.8;color:var(--muted);margin:9px 0}.update-card,.update-confirmation{padding:12px;border:1px solid var(--line);border-radius:8px;background:var(--input);margin-top:12px;min-width:0}.update-card strong,.update-confirmation>strong{font-size:11px;font-weight:500}.update-card p:last-child{margin-bottom:0}.update-notes{white-space:pre-wrap;overflow-wrap:anywhere;max-height:200px;overflow-y:auto;user-select:text}.update-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}.update-button{display:inline-flex;align-items:center;justify-content:center;font-size:10px;line-height:1.5;padding:8px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--text);cursor:pointer}.update-button:hover{border-color:var(--muted);background:var(--input)}.update-primary{color:var(--gold);border-color:var(--gold)}.update-settings button:disabled{opacity:.45;cursor:default}.update-refresh{font-size:10px;color:var(--muted);padding:5px;border-radius:4px}.update-refresh:hover{background:var(--input);color:var(--text)}.update-confirmation{border-color:var(--gold);background:var(--panel)}.update-confirmation>strong{display:block;color:var(--gold)}.update-confirmation p strong{font-weight:500;color:var(--text)}.update-confirm-actions{justify-content:flex-end}.update-settings details{font-size:10px;line-height:1.8;color:var(--muted);margin-top:9px}.update-settings summary{cursor:pointer}.update-settings code{display:block;font:10px/1.8 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;white-space:pre-wrap;overflow-wrap:anywhere;user-select:text;margin-top:6px}.update-warning,.update-settings .update-warning{color:var(--gold)}.update-error,.update-settings .update-error,.update-feedback{font-size:10px;line-height:1.8;white-space:pre-wrap;overflow-wrap:anywhere;margin-top:12px}.update-error,.update-settings .update-error{color:var(--danger)}.update-feedback{color:var(--mint)}.update-download{display:flex;flex-direction:column;gap:7px;margin-top:12px}.update-download progress{width:100%;height:7px;accent-color:var(--gold)}.update-download span{font-size:10px;color:var(--muted);font-variant-numeric:tabular-nums}.update-settings :focus-visible{outline:2px solid var(--gold);outline-offset:3px}
@media(max-width:600px){.update-heading,.update-release-heading{align-items:flex-start;flex-wrap:wrap}.update-actions .update-button{flex:1}}
</style>
