<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { errorText, formatDate, formatNumber, t } from '../i18n';
import { activityBars, barRatio, clearClipStats, clipStatsDesktopAvailable, dayDate, formatBytes, getClipStats, kindRows, statKinds, statRanges, totals } from '../clip-stats';
import type { ClipStats, StatBar, StatKind } from '../clip-stats';

const props = defineProps<{ disabled?: boolean; revision?: number }>();
const emit = defineEmits<{ busy: [value: boolean] }>();
const days = ref<number>(30);
const stats = ref<ClipStats | null>(null);
const loading = ref(false);
const clearing = ref(false);
const confirmClear = ref(false);
const error = ref('');
const active = ref<StatBar | null>(null);
let request = 0;
let disposed = false;
const canRelease = computed(() => !clearing.value && !confirmClear.value);
defineExpose({ canRelease });
watch(clearing, value => emit('busy', value), { flush: 'sync' });

const kindNames: Record<StatKind, string> = { text: '文本', link: '链接', code: '代码', color: '色彩', image: '图片', file: '文件' };
const weekdayNames = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];
const formatDay = (day: number) => formatDate(dayDate(day), { month: 'numeric', day: 'numeric', timeZone: 'UTC' });
const summary = computed(() => stats.value ? totals(stats.value) : null);
const bars = computed(() => stats.value ? activityBars(stats.value, formatDay) : []);
const barMax = computed(() => Math.max(0, ...bars.value.map(bar => bar.captures)));
const kinds = computed(() => stats.value ? kindRows(stats.value) : []);
const kindMax = computed(() => Math.max(0, ...kinds.value.map(row => row.captures)));
const hourMax = computed(() => Math.max(0, ...(stats.value?.hours ?? [])));
const weekdayMax = computed(() => Math.max(0, ...(stats.value?.weekdays ?? [])));
const peakHour = computed(() => hourMax.value > 0 ? stats.value!.hours.indexOf(hourMax.value) : null);
const weekly = computed(() => (stats.value?.days ?? 0) > 90);
const shown = computed(() => active.value ?? bars.value[bars.value.length - 1] ?? null);
const library = computed(() => stats.value?.library ?? null);
const sizeText = (kind: StatKind, size: number) => kind === 'image' ? formatBytes(size) : t('{count} 字', { count: formatNumber(size) });
const barLabel = (bar: StatBar) => t('{date}：采集 {captures} 次，复制回 {reuse} 次', { date: bar.label, captures: bar.captures, reuse: bar.reuse });
const kindBreakdown = (bar: StatBar) => statKinds.filter(kind => bar.byKind[kind] > 0).map(kind => `${t(kindNames[kind])} ${bar.byKind[kind]}`).join(t('、'));

async function load() {
  const id = ++request;
  loading.value = true; error.value = '';
  try {
    const next = await getClipStats(days.value);
    if (!disposed && id === request) { stats.value = next; active.value = null; }
  } catch (e) { if (!disposed && id === request) error.value = errorText(e); }
  finally { if (id === request) loading.value = false; }
}
async function clear() {
  if (!confirmClear.value) { confirmClear.value = true; return; }
  if (props.disabled || clearing.value) return;
  clearing.value = true; error.value = '';
  try { await clearClipStats(); confirmClear.value = false; }
  catch (e) { if (!disposed) error.value = errorText(e); }
  finally { clearing.value = false; }
  if (!disposed) await load();
}
watch(days, load);
// New captures arrive as change events; coalesce bursts into one read.
let refreshTimer: ReturnType<typeof setTimeout> | undefined;
watch(() => props.revision, () => { clearTimeout(refreshTimer); refreshTimer = setTimeout(() => { if (!clearing.value) void load(); }, 800); });
onMounted(load);
onBeforeUnmount(() => { disposed = true; request++; clearTimeout(refreshTimer); emit('busy', false); });
</script>

<template>
  <section class="clip-stats" :aria-label="t('星迹统计')" :aria-busy="loading">
    <div class="stats-head">
      <div>
        <h3>{{ t('星迹统计') }}</h3>
        <p>{{ t('在本机统计每次采集的类型、次数与时间分布。只保存计数，不保存正文、标题或链接，也不会上传。') }}</p>
      </div>
      <div class="stats-range" role="radiogroup" :aria-label="t('统计范围')">
        <button v-for="range in statRanges" :key="range" type="button" role="radio" :aria-checked="days === range" :class="{ on: days === range }" :disabled="loading && !stats" @click="days = range">{{ t('{count} 天', { count: range }) }}</button>
      </div>
    </div>
    <p v-if="!clipStatsDesktopAvailable" class="stats-note">{{ t('浏览器预览使用原创示例数据，桌面版统计本机真实采集次数。') }}</p>
    <p v-if="error" role="alert" class="stats-error">{{ error }} <button type="button" class="button subtle" @click="load">{{ t('重新读取') }}</button></p>

    <template v-if="stats && summary">
      <div class="stats-tiles">
        <div><span>{{ t('采集次数') }}</span><strong>{{ formatNumber(summary.captures) }}</strong><small>{{ t('日均 {value}', { value: formatNumber(summary.perDay, { maximumFractionDigits: 1 }) }) }}</small></div>
        <div><span>{{ t('新内容') }}</span><strong>{{ formatNumber(summary.new) }}</strong><small>{{ t('再次复制 {count} 次', { count: formatNumber(summary.repeat) }) }}</small></div>
        <div><span>{{ t('从星笺复制回') }}</span><strong>{{ formatNumber(summary.reuse) }}</strong><small>{{ t('活跃 {count} 天', { count: summary.activeDays }) }}</small></div>
        <div><span>{{ t('最忙的一天') }}</span><strong>{{ summary.busiest ? formatDay(summary.busiest.day) : '—' }}</strong><small>{{ summary.busiest ? t('{count} 次', { count: summary.busiest.new + summary.busiest.repeat }) : t('暂无记录') }}</small></div>
      </div>

      <div v-if="!summary.captures && !summary.reuse" class="stats-empty">
        <p>{{ t('这段时间还没有采集记录。开启记录后，复制的文字、链接、代码和图片会按类型计入这里。') }}</p>
      </div>

      <template v-else>
        <h4>{{ weekly ? t('每周采集') : t('每日采集') }}</h4>
        <p class="stats-readout" aria-live="polite"><template v-if="shown"><strong>{{ shown.label }}</strong> · {{ t('采集 {count} 次', { count: shown.captures }) }}<template v-if="kindBreakdown(shown)"> · {{ kindBreakdown(shown) }}</template> · {{ t('复制回 {count} 次', { count: shown.reuse }) }}</template></p>
        <div class="stats-bars" :class="{ dense: bars.length > 31 }" role="img" :aria-label="t('采集次数柱状图，详细数据见下方表格')" @pointerleave="active = null">
          <div v-for="bar in bars" :key="bar.key" class="stats-bar" :class="{ on: shown?.key === bar.key }" :title="barLabel(bar)" @pointerenter="active = bar">
            <span :style="{ height: `${barRatio(bar.captures, barMax) * 100}%` }" />
          </div>
        </div>
        <div class="stats-axis"><span>{{ bars[0]?.label }}</span><span>{{ bars[bars.length - 1]?.label }}</span></div>

        <h4>{{ t('类型分布') }}</h4>
        <ul class="stats-kinds">
          <li v-for="row in kinds" :key="row.kind">
            <span class="kind-name">{{ t(kindNames[row.kind]) }}</span>
            <span class="kind-track"><span :style="{ width: `${barRatio(row.captures, kindMax) * 100}%` }" /></span>
            <span class="kind-value">{{ formatNumber(row.captures) }} · {{ formatNumber(row.share * 100, { maximumFractionDigits: 0 }) }}%</span>
            <small>{{ t('新 {new} · 再次 {repeat} · 复制回 {reuse}', { new: row.new, repeat: row.repeat, reuse: row.reuse }) }} · {{ sizeText(row.kind, row.size) }}</small>
          </li>
        </ul>

        <div class="stats-split">
          <div>
            <h4>{{ t('一天中的时段') }}</h4>
            <div class="stats-mini hours" role="img" :aria-label="peakHour === null ? t('暂无时段数据') : t('最常在 {hour} 点采集', { hour: peakHour })">
              <span v-for="(value, hour) in stats.hours" :key="hour" :title="t('{hour} 点：{count} 次', { hour, count: value })" :style="{ height: `${barRatio(value, hourMax) * 100}%` }" />
            </div>
            <div class="stats-axis"><span>0</span><span>6</span><span>12</span><span>18</span><span>23</span></div>
          </div>
          <div>
            <h4>{{ t('一周中的分布') }}</h4>
            <div class="stats-mini week">
              <span v-for="(value, day) in stats.weekdays" :key="day" :title="t('{day}：{count} 次', { day: t(weekdayNames[day]), count: value })" :style="{ height: `${barRatio(value, weekdayMax) * 100}%` }" />
            </div>
            <div class="stats-axis week"><span v-for="name in weekdayNames" :key="name">{{ t(name) }}</span></div>
          </div>
        </div>

        <details class="stats-table">
          <summary>{{ t('查看明细表') }}</summary>
          <table>
            <thead><tr><th>{{ weekly ? t('周') : t('日期') }}</th><th>{{ t('采集') }}</th><th v-for="kind in statKinds" :key="kind">{{ t(kindNames[kind]) }}</th><th>{{ t('复制回') }}</th></tr></thead>
            <tbody><tr v-for="bar in [...bars].reverse()" :key="bar.key"><td>{{ bar.label }}</td><td>{{ bar.captures }}</td><td v-for="kind in statKinds" :key="kind">{{ bar.byKind[kind] }}</td><td>{{ bar.reuse }}</td></tr></tbody>
          </table>
        </details>
      </template>

      <template v-if="library">
        <h4>{{ t('当前星系') }}</h4>
        <p class="stats-caption">{{ t('现存记录的构成，随清理与删除变化；上方计数不受清理影响。') }}</p>
        <dl class="stats-library">
          <div><dt>{{ t('记录总数') }}</dt><dd>{{ formatNumber(library.total) }}</dd></div>
          <div v-for="kind in statKinds" :key="kind"><dt>{{ t(kindNames[kind]) }}</dt><dd>{{ formatNumber(library.kinds[kind] ?? 0) }}</dd></div>
          <div><dt>{{ t('星标') }}</dt><dd>{{ formatNumber(library.pinned) }}</dd></div>
          <div><dt>{{ t('永久保留') }}</dt><dd>{{ formatNumber(library.kept) }}</dd></div>
          <div><dt>{{ t('已分组') }}</dt><dd>{{ formatNumber(library.grouped) }}</dd></div>
          <div><dt>{{ t('隐私记录') }}</dt><dd>{{ formatNumber(library.private) }}</dd></div>
          <div><dt>{{ t('复制过的记录') }}</dt><dd>{{ t('{count} 条 · 共 {total} 次', { count: library.reusedClips, total: formatNumber(library.copyTotal) }) }}</dd></div>
          <div><dt>{{ t('图片占用') }}</dt><dd>{{ formatBytes(library.imageBytes) }}</dd></div>
        </dl>
        <template v-if="library.topHosts.length">
          <h4>{{ t('常见来源域名') }}</h4>
          <ol class="stats-hosts"><li v-for="item in library.topHosts" :key="item.host"><span>{{ item.host }}</span><span>{{ t('{count} 条', { count: item.count }) }}</span></li></ol>
          <p class="stats-caption">{{ t('隐私记录的域名不计入。') }}</p>
        </template>
      </template>

      <div class="stats-foot">
        <p>{{ stats.recordingSince === null ? t('尚无计数。计数从本版本开始记录，最多保留 400 天；导入备份、手动新建和保存代码不计入。') : t('自 {date} 起记录，最多保留 400 天；导入备份、手动新建和保存代码不计入。', { date: formatDate(stats.recordingSince, { year: 'numeric', month: 'numeric', day: 'numeric' }) }) }}</p>
        <div class="stats-actions">
          <button type="button" class="button subtle" :disabled="disabled || clearing || stats.recordingSince === null" @click="clear">{{ confirmClear ? t('确认清空统计') : t('清空统计') }}</button>
          <button v-if="confirmClear" type="button" class="button subtle" :disabled="clearing" @click="confirmClear = false">{{ t('取消') }}</button>
        </div>
        <p v-if="confirmClear" class="stats-caption">{{ t('只删除统计计数，不影响剪贴记录。') }}</p>
      </div>
    </template>
    <p v-else-if="loading" class="stats-caption">{{ t('正在读取统计…') }}</p>
  </section>
</template>

<style scoped>
.clip-stats{min-width:0}.clip-stats h3{font-size:13px;color:var(--gold);margin:0 0 8px}.clip-stats h4{font-size:12px;color:var(--text);margin:22px 0 8px}
.clip-stats p{font-size:12px;line-height:1.8;color:var(--muted);margin:0;overflow-wrap:anywhere}
.stats-head{display:flex;gap:14px;justify-content:space-between;align-items:flex-start;flex-wrap:wrap}.stats-head>div:first-child{flex:1 1 260px}
.stats-range{display:flex;border:1px solid var(--line);border-radius:9px;overflow:hidden}.stats-range button{border:0;background:transparent;color:var(--muted);font:inherit;font-size:12px;padding:6px 10px;cursor:pointer}.stats-range button+button{border-left:1px solid var(--line)}.stats-range button.on{background:var(--gold-dim);color:var(--gold);font-weight:600}.stats-range button:focus-visible{outline:2px solid var(--primary);outline-offset:-2px}
.stats-note,.stats-empty{margin-top:14px!important;padding:12px;background:var(--input);border:1px solid var(--line);border-radius:9px}.stats-empty{margin-top:16px}
.clip-stats .stats-error{color:var(--danger);margin-top:12px}
.stats-tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(130px,1fr));gap:10px;margin-top:16px}.stats-tiles>div{background:var(--input);border:1px solid var(--line-soft);border-radius:10px;padding:10px 12px;display:flex;flex-direction:column;gap:2px;min-width:0}.stats-tiles span,.stats-tiles small{font-size:11px;color:var(--muted)}.stats-tiles strong{font-size:20px;color:var(--text);font-variant-numeric:tabular-nums}
.clip-stats .stats-readout{min-height:22px;color:var(--text)}
.stats-bars{display:flex;align-items:flex-end;gap:2px;height:120px;border-bottom:1px solid var(--line);padding-top:4px}.stats-bar{flex:1 1 0;min-width:0;height:100%;display:flex;align-items:flex-end;cursor:default}.stats-bar>span{display:block;width:100%;background:var(--primary);opacity:.72;border-radius:4px 4px 0 0}.stats-bar.on>span{opacity:1}.stats-bars.dense{gap:1px}.stats-bars.dense .stats-bar>span{border-radius:2px 2px 0 0}
.stats-axis{display:flex;justify-content:space-between;font-size:10px;color:var(--faint);margin-top:4px;font-variant-numeric:tabular-nums}.stats-axis.week span{flex:1;text-align:center}
.stats-kinds{list-style:none;margin:0;padding:0;display:grid;gap:10px}.stats-kinds li{display:grid;grid-template-columns:44px 1fr auto;gap:4px 10px;align-items:center;font-size:12px}.stats-kinds small{grid-column:2/4;font-size:11px;color:var(--faint)}.kind-name{color:var(--text)}.kind-value{color:var(--muted);font-variant-numeric:tabular-nums}.kind-track{height:8px;background:var(--line-soft);border-radius:4px;overflow:hidden}.kind-track>span{display:block;height:100%;background:var(--primary);border-radius:4px}
.stats-split{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:18px}
.stats-mini{display:flex;align-items:flex-end;gap:2px;height:64px;border-bottom:1px solid var(--line)}.stats-mini>span{flex:1;background:var(--primary);opacity:.72;border-radius:3px 3px 0 0}.stats-mini.week{gap:6px}
.stats-table summary{font-size:12px;cursor:pointer;color:var(--text);padding:14px 0 8px}.stats-table{overflow-x:auto}.stats-table table{border-collapse:collapse;font-size:11px;width:100%;font-variant-numeric:tabular-nums}.stats-table th,.stats-table td{padding:4px 6px;text-align:right;border-bottom:1px solid var(--line-soft);white-space:nowrap}.stats-table th:first-child,.stats-table td:first-child{text-align:left}.stats-table th{color:var(--muted);font-weight:600}.stats-table td{color:var(--text)}
.clip-stats .stats-caption{font-size:11px;color:var(--faint)}
.stats-library{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:6px 16px;margin:10px 0 0}.stats-library>div{display:flex;justify-content:space-between;gap:8px;font-size:12px;border-bottom:1px dashed var(--line-soft);padding:4px 0}.stats-library dt{color:var(--muted)}.stats-library dd{margin:0;color:var(--text);font-variant-numeric:tabular-nums}
.stats-hosts{margin:0;padding:0 0 0 18px;font-size:12px;color:var(--text)}.stats-hosts li{padding:2px 0}.stats-hosts li span:first-child{overflow-wrap:anywhere}.stats-hosts li span:last-child{color:var(--muted);margin-left:8px}
.stats-foot{border-top:1px solid var(--line);margin-top:24px;padding-top:16px}.stats-actions{display:flex;gap:10px;flex-wrap:wrap;margin:10px 0 6px}
</style>
