<script setup lang="ts">
import { errorText, formatDate, formatNumber, t } from '../i18n';
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { api, isDesktop } from '../api';
import type { CleanupFilter, CleanupPreview, CleanupResult } from '../types';
import Icon from './Icon.vue';
import UiSelect from './UiSelect.vue';

const sizeUnits = [{ value: 'KiB', label: 'KiB' }, { value: 'MiB', label: 'MiB' }] as const;
const ageChoices = computed(() => [{ value: '0', label: t('不限时间') }, ...[7, 30, 90].map(value => ({ value: String(value), label: t('{count} 天前', { count: value }) })), { value: 'custom', label: t('自定义天数') }]);
const kindChoices = computed(() => [{ value: 'all', label: t('全部类型') }, { value: 'text', label: t('文本') }, { value: 'link', label: t('链接') }, { value: 'code', label: t('代码') }, { value: 'color', label: t('色彩') }, { value: 'image', label: t('图片') }, { value: 'file', label: t('文件') }] as const);
const emit = defineEmits<{ busy: [value: boolean]; changed: [] }>();
const size = ref<string | number>('0');
const sizeUnit = ref<'KiB' | 'MiB'>('KiB');
const age = ref('30');
const customDays = ref<string | number>('30');
const kind = ref<CleanupFilter['kind']>('all');
const includeGrouped = ref(false);
const compact = ref(true);
const confirmed = ref(false);
const operation = ref<'preview' | 'execute' | null>(null);
const preview = ref<CleanupPreview | null>(null);
const result = ref<CleanupResult | null>(null);
const error = ref('');
const stale = ref('');
const previewRegion = ref<HTMLElement>();
const resultRegion = ref<HTMLElement>();
const busy = computed(() => operation.value !== null);
const kindNames = computed<Record<CleanupFilter['kind'], string>>(() => ({ all: t('全部类型'), text: t('文本'), link: t('链接'), code: t('代码'), color: t('色彩'), image: t('图片'), file: t('文件') }));
const dateLabel = (date: Date) => formatDate(date, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
const sizeBytes = computed(() => Math.floor(Number(size.value) * (sizeUnit.value === 'MiB' ? 1048576 : 1024)));
const days = computed(() => Number(age.value === 'custom' ? customDays.value : age.value));
const validation = computed(() => {
  if (!String(size.value).trim() || !Number.isFinite(Number(size.value)) || Number(size.value) < 0 || !Number.isSafeInteger(sizeBytes.value)) return t('请输入有效的非负大小，换算后不能超过安全整数字节范围。');
  if (Number(size.value) > 0 && sizeBytes.value < 1) return t('大小阈值至少为 1 字节，或填写 0 表示不限。');
  if ((age.value === 'custom' && !String(customDays.value).trim()) || !Number.isInteger(days.value) || days.value < 0 || days.value > 36500) return t('保留天数筛选请输入 0–36500 的整数，0 表示不限。');
  return '';
});
const filter = computed<CleanupFilter>(() => ({ minBytes: sizeBytes.value, olderThanDays: days.value, kind: kind.value, includeGrouped: includeGrouped.value }));
const filterKey = computed(() => JSON.stringify([size.value, sizeUnit.value, age.value, customDays.value, kind.value, includeGrouped.value]));
let preparedKey = '';
let disposed = false;
function discardPreview() {
  if (preview.value) void api.cancelCleanup(preview.value.token).catch(() => {});
  preview.value = null;
}
onBeforeUnmount(() => { disposed = true; discardPreview(); });
const items = computed(() => preview.value?.items.slice(0, 100) || []);
const canExecute = computed(() => !busy.value && !validation.value && confirmed.value && !!preview.value && preparedKey === filterKey.value && (preview.value.matchedCount > 0 || compact.value));
watch(busy, value => emit('busy', value), { immediate: true, flush: 'sync' });
watch(filterKey, () => {
  if (preview.value || operation.value === 'preview') stale.value = t('筛选条件已改变，请重新预览。');
  discardPreview(); preparedKey = ''; confirmed.value = false; result.value = null; error.value = '';
}, { flush: 'sync' });
watch(compact, () => { confirmed.value = false; }, { flush: 'sync' });

function bytes(value: number) {
  if (!Number.isFinite(value) || value < 0) return t('暂不可用');
  if (value < 1024) return `${Math.round(value)} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let amount = value / 1024, unit = 0;
  while (amount >= 1024 && unit < units.length - 1) { amount /= 1024; unit++; }
  return `${formatNumber(amount, { maximumFractionDigits: 2 })} ${units[unit]}`;
}
function updatedAt(value: number) { const date = new Date(value); return Number.isNaN(date.valueOf()) ? t('时间不可用') : dateLabel(date); }
function message(reason: unknown) { return errorText(reason); }
function preset(value: 'large' | 'old' | 'all') {
  if (busy.value) return;
  size.value = value === 'large' ? '32' : '0'; sizeUnit.value = 'KiB';
  age.value = value === 'old' ? '30' : '0'; customDays.value = '30';
  kind.value = 'all'; includeGrouped.value = false;
}
async function requestPreview() {
  if (busy.value || validation.value) return;
  const key = filterKey.value, selected = { ...filter.value };
  operation.value = 'preview'; discardPreview(); result.value = null; confirmed.value = false; preparedKey = ''; error.value = ''; stale.value = '';
  try {
    const value = await api.previewCleanup(selected);
    if (disposed || key !== filterKey.value) { void api.cancelCleanup(value.token).catch(() => {}); stale.value = t('筛选条件已改变，请重新预览。'); return; }
    preview.value = value; preparedKey = key;
    await nextTick(); previewRegion.value?.focus();
  } catch (reason) { error.value = message(reason); stale.value = t('尚未取得有效预览，请检查条件后重新预览。'); }
  finally { operation.value = null; }
}
async function execute() {
  if (!canExecute.value || !preview.value) return;
  const token = preview.value.token, shouldCompact = compact.value;
  operation.value = 'execute'; error.value = ''; stale.value = ''; result.value = null;
  try {
    result.value = await api.executeCleanup(token, shouldCompact);
    await nextTick(); resultRegion.value?.focus();
  } catch (reason) {
    error.value = message(reason);
    stale.value = t('请重新预览，核对当前记录后再操作。');
  } finally {
    preview.value = null; preparedKey = ''; confirmed.value = false;
    emit('changed'); operation.value = null;
  }
}
</script>

<template>
  <section class="cleanup-panel" :aria-label="t('星尘整理筛选与预览')" :aria-busy="busy">
    <p class="cleanup-intro">{{ t('整理不再需要的片段，留出一点空间。') }}<strong>{{ t('星标与「永久保留」记录始终受保护。') }}</strong></p>
    <p v-if="!isDesktop" class="cleanup-demo"><Icon name="alert" :size="14" />{{ t('浏览器演示只整理内存中的示例，没有实际数据库文件；物理空间释放始终为 0。') }}</p>
    <fieldset class="cleanup-filters" :disabled="busy">
      <legend>{{ t('同时满足以下条件的记录才会入选') }}</legend>
      <div class="cleanup-presets" :aria-label="t('快速筛选')">
        <button type="button" @click="preset('large')">{{ t('大内容 · > 32 KiB') }}</button>
        <button type="button" @click="preset('old')">{{ t('30 天前') }}</button>
        <button type="button" @click="preset('all')">{{ t('全部未保护') }}</button>
      </div>
      <p class="cleanup-small">{{ t('预设会重置其他筛选，并默认排除已分组记录。') }}</p>
      <div class="cleanup-filter-grid">
        <div class="cleanup-field"><span>{{ t('内容大小大于') }}</span>
          <span class="cleanup-size-input"><input v-model="size" type="number" min="0" step="any" inputmode="decimal" :aria-label="t('内容大小阈值')" /><UiSelect v-model="sizeUnit" :options="sizeUnits" :label="t('内容大小单位')" :disabled="busy" /></span>
          <small v-if="!validation">{{ sizeBytes === 0 ? t('0 表示不限大小') : t('严格大于 {size} 字节', { size: formatNumber(sizeBytes) }) }}</small>
        </div>
        <div class="cleanup-field"><span>{{ t('最近更新早于') }}</span>
          <UiSelect v-model="age" :options="ageChoices" :label="t('最近更新早于天数')" :disabled="busy" />
          <input v-if="age === 'custom'" v-model="customDays" type="number" min="0" max="36500" step="1" inputmode="numeric" :aria-label="t('自定义清理天数')" :placeholder="t('0 表示不限')" />
          <small>{{ t('按最近更新时间计算，0 表示不限') }}</small>
        </div>
        <div class="cleanup-field"><span>{{ t('内容类型') }}</span>
          <UiSelect v-model="kind" :options="kindChoices" :label="t('清理内容类型')" :disabled="busy" />
        </div>
      </div>
      <label class="cleanup-check"><input v-model="includeGrouped" type="checkbox" />{{ t('包含已分组记录') }}<span>{{ t('仍跳过星标与永久保留') }}</span></label>
    </fieldset>
    <p v-if="validation" class="cleanup-error" role="alert">{{ validation }}</p>
    <div class="cleanup-preview-action"><span>{{ t('先预览，再决定是否删除。') }}</span><button type="button" class="cleanup-button" :disabled="busy || !!validation" @click="requestPreview">{{ operation === 'preview' ? t('正在计算…') : preview ? t('重新预览') : t('预览整理范围') }}</button></div>

    <section v-if="preview" ref="previewRegion" class="cleanup-preview" tabindex="-1" :aria-label="t('清理预览')">
      <div class="cleanup-counts"><span>{{ t('全部') }} <strong>{{ preview.totalCount }}</strong> {{ t('条') }}</span><span><Icon name="shield" :size="13" />{{ t('受保护') }} <strong>{{ preview.protectedCount }}</strong> {{ t('条') }}</span><span>{{ t('待删除') }} <strong>{{ preview.matchedCount }}</strong> {{ t('条') }}</span></div>
      <p class="cleanup-small">{{ t('保护数量为全库星标或永久保留记录，重复保护只计一次；默认排除的分组记录不计入这一数字。') }}</p>
      <dl class="cleanup-metrics">
        <div><dt>{{ t('待删除内容') }}</dt><dd :title="t('{size} 字节', { size: formatNumber(preview.contentBytes) })">{{ bytes(preview.contentBytes) }}</dd></div>
        <div><dt>{{ t('当前数据库文件') }}</dt><dd :title="t('{size} 字节', { size: formatNumber(preview.databaseBytes) })">{{ bytes(preview.databaseBytes) }}</dd></div>
        <div><dt>{{ t('已有可复用空间') }}</dt><dd :title="t('{size} 字节', { size: formatNumber(preview.reusableBytes) })">{{ bytes(preview.reusableBytes) }}</dd></div>
      </dl>
      <p class="cleanup-estimate">{{ t('文字与标题按 UTF-8 字节统计，图片按原图 PNG 字节加标题统计，不计缩略图。这不是预计可释放的磁盘空间；索引、缩略图和空闲页会影响数据库大小，实际缩减以执行结果为准。') }}</p>
      <div v-if="items.length" class="cleanup-table-wrap" role="region" :aria-label="t('待清理记录明细')" tabindex="0">
        <table><thead><tr><th scope="col">{{ t('片段标题') }}</th><th scope="col">{{ t('类型') }}</th><th scope="col">{{ t('内容占用') }}</th><th scope="col">{{ t('最近更新') }}</th></tr></thead><tbody><tr v-for="item in items" :key="item.id"><td class="cleanup-title">{{ item.title || t('未命名片段') }}</td><td>{{ kindNames[item.kind] }}</td><td :title="t('{size} 字节', { size: formatNumber(item.bytes) })">{{ bytes(item.bytes) }}</td><td>{{ updatedAt(item.updatedAt) }}</td></tr></tbody></table>
      </div>
      <p v-if="preview.truncated || preview.matchedCount > items.length" class="cleanup-truncated">{{ t('列出前 {shown} 条，确认后将清理全部 {total} 条匹配记录。', { shown: items.length, total: preview.matchedCount }) }}</p>
      <p v-else-if="preview.matchedCount === 0" class="cleanup-empty">{{ t('没有符合条件的可删除记录。仍可单独整理数据库中的可复用空间。') }}</p>
      <label class="cleanup-check cleanup-compact"><input v-model="compact" type="checkbox" :disabled="busy" />{{ preview.matchedCount ? t('同时整理数据库文件') : t('仅整理数据库文件') }}</label>
      <p class="cleanup-small">{{ t('整理不影响保留的记录；可能需要片刻，完成前请勿关闭应用。') }}</p>
      <label v-if="preview.matchedCount > 0 || compact" class="cleanup-confirm"><input v-model="confirmed" type="checkbox" :disabled="busy" /><span>{{ preview.matchedCount > 0 ? t('我确认永久删除预览中的 {count} 条记录，操作无法撤回。', { count: preview.matchedCount }) : t('我确认仅整理数据库文件，不删除任何记录。') }}</span></label>
      <div class="cleanup-final-action"><span v-if="preview.matchedCount">{{ t('需要留档时，请先返回设置导出备份。') }}</span><span v-else-if="!compact">{{ t('没有需要执行的操作。') }}</span><button type="button" class="cleanup-button cleanup-execute" :disabled="!canExecute" @click="execute">{{ operation === 'execute' ? t('正在整理…') : preview.matchedCount ? t('确认清理 {count} 条', { count: preview.matchedCount }) : t('确认整理数据库') }}</button></div>
    </section>

    <section v-if="result" ref="resultRegion" class="cleanup-result" :class="{ 'has-warning': result.warning }" tabindex="-1" :aria-label="t('本次整理结果')">
      <h3><Icon :name="result.warning ? 'alert' : 'check'" :size="16" />{{ t('本次整理结果') }}</h3>
      <dl class="cleanup-result-metrics"><div><dt>{{ t('已删除记录') }}</dt><dd>{{ result.deletedCount }} {{ t('条') }}</dd></div><div><dt>{{ t('删除的内容') }}</dt><dd>{{ bytes(result.contentBytes) }}</dd></div><div><dt>{{ t('数据库文件实际缩减') }}</dt><dd>{{ bytes(result.releasedBytes) }}</dd></div></dl>
      <p class="cleanup-small">{{ t('数据库文件：') }}{{ bytes(result.databaseBytesBefore) }} → {{ bytes(result.databaseBytesAfter) }}{{ !isDesktop ? t(' · 浏览器没有实际文件') : '' }}</p>
      <p v-if="result.warning" class="cleanup-warning" role="status">{{ t(result.warning) }}<br />{{ t('以上删除数量已生效。') }}<span v-if="isDesktop">{{ t('可重新预览后再决定是否整理数据库。') }}</span></p>
      <p v-else class="cleanup-success" role="status">{{ result.deletedCount ? t('选中的记录已清理，保护记录仍保留。') : t('未删除任何记录。') }}</p>
    </section>
    <p v-if="error" class="cleanup-error" role="alert">{{ error }}</p>
    <p v-if="stale" class="cleanup-stale" role="status">{{ stale }}</p>
  </section>
</template>

<style scoped>
.cleanup-panel{color:var(--text);min-width:0}.cleanup-intro{font-size:12px;line-height:1.8;color:var(--muted);margin:0 0 14px}.cleanup-intro strong{display:block;font-weight:500;color:var(--text)}.cleanup-demo{display:flex;gap:8px;align-items:flex-start;padding:11px 12px;border:1px solid var(--line);border-radius:8px;background:var(--input);color:var(--muted);font-size:11px;line-height:1.7;margin:0 0 15px}.cleanup-demo svg{flex-shrink:0;margin-top:3px}
.cleanup-filters{padding:13px 14px 12px;border:1px solid var(--line);border-radius:9px;min-width:0;margin:0}.cleanup-filters legend{font-size:11px;color:var(--gold);padding:0 6px}.cleanup-presets{display:flex;flex-wrap:wrap;gap:7px;margin-bottom:8px}.cleanup-presets button{background:var(--input);border:1px solid var(--line);border-radius:6px;padding:6px 9px;color:var(--muted);font-size:10px}.cleanup-presets button:hover{color:var(--text);border-color:var(--muted)}.cleanup-filter-grid{display:grid;grid-template-columns:1.2fr 1fr 1fr;gap:13px;margin:14px 0}.cleanup-panel .cleanup-field{display:flex;flex-direction:column;gap:7px;color:var(--muted);font-size:10px;margin:0;min-width:0}.cleanup-field input{width:100%;font-size:11px;padding:8px;background:var(--input);color:var(--text);border:1px solid var(--line);border-radius:6px;min-width:0}.cleanup-size-input{display:flex;gap:6px}.cleanup-size-input input{flex:1;width:70px}.cleanup-size-input .ui-select{width:80px;flex-shrink:0}.cleanup-field small{font-size:9px;color:var(--faint);line-height:1.6}.cleanup-panel .cleanup-check{display:flex;align-items:center;gap:8px;font-size:11px;color:var(--text);margin:10px 0 0;line-height:1.6}.cleanup-check span{font-size:9px;color:var(--muted)}.cleanup-panel input[type=checkbox]{width:15px;height:15px;margin:0;flex-shrink:0;}
.cleanup-small{font-size:10px;line-height:1.7;color:var(--muted);margin:6px 0 0}.cleanup-preview-action,.cleanup-final-action{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:15px}.cleanup-preview-action>span,.cleanup-final-action>span{font-size:10px;color:var(--muted);line-height:1.6}.cleanup-button{padding:9px 13px;border:1px solid var(--gold);border-radius:7px;color:var(--gold);background:var(--input);font-size:11px;line-height:1.5;white-space:nowrap}.cleanup-button:hover{background:var(--panel)}.cleanup-panel button:disabled{opacity:.45;cursor:default}.cleanup-panel fieldset:disabled{opacity:.65}.cleanup-preview{margin-top:18px;padding-top:17px;border-top:1px solid var(--line)}.cleanup-counts{display:flex;flex-wrap:wrap;gap:10px 24px;color:var(--muted);font-size:11px}.cleanup-counts>span{display:inline-flex;align-items:center;gap:5px}.cleanup-counts strong{font-size:14px;font-weight:500;color:var(--text)}.cleanup-counts svg{color:var(--mint)}.cleanup-metrics{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px;padding:12px;margin:13px 0 0;border:1px solid var(--line);border-radius:8px;background:var(--input)}.cleanup-metrics dt,.cleanup-result-metrics dt{font-size:10px;color:var(--muted);line-height:1.6}.cleanup-metrics dd,.cleanup-result-metrics dd{font-size:15px;color:var(--text);margin:6px 0 0;font-variant-numeric:tabular-nums}.cleanup-estimate{font-size:10px;line-height:1.8;color:var(--muted);margin:10px 0 12px}
.cleanup-table-wrap{max-height:240px;overflow:auto;border:1px solid var(--line);border-radius:7px}.cleanup-table-wrap table{width:100%;min-width:450px;border-collapse:collapse;font-size:10px;text-align:left}.cleanup-table-wrap th{position:sticky;top:0;background:var(--panel);color:var(--muted);font-weight:500;border-bottom:1px solid var(--line);z-index:1}.cleanup-table-wrap th,.cleanup-table-wrap td{padding:9px 10px;line-height:1.6;vertical-align:top}.cleanup-table-wrap td{border-bottom:1px solid var(--line);color:var(--muted);white-space:nowrap}.cleanup-table-wrap tr:last-child td{border-bottom:0}.cleanup-table-wrap td.cleanup-title{white-space:pre-wrap;overflow-wrap:anywhere;min-width:140px;max-width:260px;color:var(--text)}.cleanup-truncated,.cleanup-empty{font-size:10px;line-height:1.8;color:var(--gold);margin:10px 0}.cleanup-panel .cleanup-compact{margin-top:15px}.cleanup-panel .cleanup-confirm{display:flex;gap:9px;align-items:flex-start;padding:11px 12px;margin:13px 0 0;border:1px solid var(--line);border-radius:7px;background:var(--input);font-size:11px;line-height:1.8;color:var(--text)}.cleanup-confirm input{margin-top:3px!important}.cleanup-execute{color:var(--danger);border-color:var(--danger)}
.cleanup-result{margin-top:18px;padding:14px;border:1px solid var(--line);border-radius:9px;background:var(--input)}.cleanup-result h3{display:flex;align-items:center;gap:8px;font-size:12px;font-weight:500;color:var(--mint);margin:0}.cleanup-result-metrics{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px;margin:14px 0 10px}.cleanup-warning,.cleanup-error,.cleanup-stale,.cleanup-success{font-size:11px;line-height:1.8;white-space:pre-wrap;overflow-wrap:anywhere;margin:12px 0 0}.cleanup-warning,.cleanup-stale{color:var(--gold)}.cleanup-error{color:var(--danger)}.cleanup-success{color:var(--mint)}.cleanup-panel :focus-visible{outline:2px solid var(--gold);outline-offset:3px}
.cleanup-result.has-warning h3{color:var(--gold)}
@media(max-width:650px){.cleanup-filter-grid{grid-template-columns:1fr 1fr}.cleanup-filter-grid .cleanup-field:last-child{grid-column:1/-1}.cleanup-metrics,.cleanup-result-metrics{grid-template-columns:1fr 1fr}.cleanup-preview-action,.cleanup-final-action{align-items:flex-start}.cleanup-check{flex-wrap:wrap}.cleanup-check span{width:100%;padding-left:23px}.cleanup-counts{gap:8px 16px}}
</style>
