<script setup lang="ts">
import { errorText, formatNumber, t } from '../i18n';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { api, isDesktop } from '../api';
import { safeFilename } from '../snippets';
import { createHandwritingPreview } from '../handwriting-preview';
import type { HandwritingPreviewState, HandwritingSnapshot } from '../handwriting-preview';
import type { HandwritingAssets, HandwritingFontProgress, HandwritingOptions, HandwritingPaper } from '../types';
import UiColor from './UiColor.vue';

const props = defineProps<{ text: string; title: string }>();
const assets = ref<HandwritingAssets>({ available: false, path: null, fonts: [], backgrounds: [], error: null });
const loading = ref(true);
const operation = ref<'download' | 'export' | null>(null);
const downloadingFont = ref('');
const downloadProgress = ref<HandwritingFontProgress | null>(null);
const renderState = ref<HandwritingPreviewState>('idle');
const error = ref('');
const renderError = ref('');
const notice = ref('');
const font = ref('');
const fontSearch = ref('');
const fontFilter = ref<'all' | 'downloaded'>('all');
const handwritingOnly = ref(false);
const fontPickerOpen = ref(false);
const background = ref('solid:#f8f1e3:1200:1600');
const fontSize = ref(30);
const ink = ref('#24324a');
const padding = ref(100);
const preview = ref('');
const renderedKey = ref('');
let disposed = false;
const downloadedFonts = computed(() => assets.value.fonts.filter(item => item.downloaded));
const currentFont = computed(() => downloadedFonts.value.find(item => item.path === font.value));
const visibleFonts = computed(() => {
  const query = fontSearch.value.trim().toLowerCase();
  return assets.value.fonts.filter(item => (fontFilter.value === 'all' || item.downloaded)
    && (!handwritingOnly.value || item.tags.includes('手写体'))
    && (!query || item.name.toLowerCase().includes(query) || t(item.name).toLowerCase().includes(query)));
});
const emptyFontMessage = computed(() => fontFilter.value === 'downloaded' && !downloadedFonts.value.length
  ? '还没有下载字体，切换到全部字体选择喜欢的一款。'
  : handwritingOnly.value ? '当前手写体筛选下没有匹配的字体，试试其他名称、下载状态或关闭手写体筛选。'
    : '没有找到匹配的字体，试试其他名称或切换到全部字体。');
const progressFont = computed(() => assets.value.fonts.find(item => item.id === downloadProgress.value?.id));
const progressPercent = computed(() => {
  const value = downloadProgress.value;
  return value && value.total > 0 ? Math.min(100, Math.round((value.received / value.total) * 100)) : 0;
});
const progressSize = computed(() => {
  const value = downloadProgress.value;
  if (!value) return '';
  return `${formatNumber(value.received / 1048576, { maximumFractionDigits: 1 })} / ${formatNumber(value.total / 1048576, { maximumFractionDigits: 1 })} MiB`;
});
function fontSizeLabel(bytes: number) { return `${formatNumber(bytes / (1024 * 1024), { maximumFractionDigits: 1 })} MiB`; }
const defaultPapers: HandwritingPaper[] = [
  { id: 'warm', name: '暖白信纸', path: 'solid:#f8f1e3:1200:1600', width: 1200, height: 1600, color: '#f8f1e3', previewUrl: null },
  { id: 'white', name: '纯白纸张', path: 'solid:#ffffff:1200:1600', width: 1200, height: 1600, color: '#ffffff', previewUrl: null },
];
const papers = computed(() => assets.value.backgrounds.length ? assets.value.backgrounds : defaultPapers);
const currentPaper = computed(() => papers.value.find(item => item.path === background.value) ?? papers.value[0]!);
// Papers range from 200 px to 1200 px wide, so keep the text in proportion when switching.
function choosePaper(paper: HandwritingPaper) {
  if (operation.value || paper.path === background.value) return;
  const ratio = paper.width / currentPaper.value.width;
  background.value = paper.path;
  fontSize.value = Math.min(72, Math.max(12, Math.round(fontSize.value * ratio)));
  padding.value = Math.min(160, Math.max(16, Math.round(padding.value * ratio / 4) * 4));
}
const options = computed<HandwritingOptions>(() => ({ text: props.text, font: font.value, background: background.value, fontSize: fontSize.value, ink: ink.value, padding: padding.value }));
const tooLong = computed(() => [...props.text].length > 10000);
const request = computed<HandwritingSnapshot | null>(() => {
  if (!isDesktop || loading.value || !assets.value.available
    || !props.text.trim() || tooLong.value || !downloadedFonts.value.some(item => item.path === font.value)) return null;
  return { key: JSON.stringify([assets.value.path, options.value]), options: { ...options.value } };
});
const stale = computed(() => !!preview.value && renderedKey.value !== request.value?.key);
const canExport = computed(() => !!request.value && !!preview.value && !stale.value && operation.value === null && renderState.value === 'idle');
const scheduler = createHandwritingPreview(api.renderHandwriting, {
  state: value => { renderState.value = value; },
  result: (png, key) => { preview.value = png; renderedKey.value = key; renderError.value = ''; },
  error: reason => { renderError.value = errorText(reason); },
});
watch(() => request.value?.key ?? null, () => {
  const value = request.value;
  if (!value) { preview.value = ''; renderedKey.value = ''; }
  renderError.value = ''; notice.value = '';
  scheduler.update(value);
}, { immediate: true, flush: 'sync' });
function setAssets(value: HandwritingAssets) {
  assets.value = value;
  if (!value.fonts.some(item => item.downloaded && item.path === font.value)) font.value = value.fonts.find(item => item.downloaded)?.path || '';
  if (!background.value.startsWith('solid:') && !value.backgrounds.some(item => item.path === background.value)) background.value = defaultPapers[0]!.path;
}
async function loadAssets() {
  if (disposed || operation.value) return;
  loading.value = true; error.value = '';
  try { const value = await api.handwritingAssets(); if (!disposed) setAssets(value); }
  catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) loading.value = false; }
}
onMounted(loadAssets);
onBeforeUnmount(() => { disposed = true; clearProgress(); scheduler.dispose(); });
let stopProgress: (() => void) | undefined;
function clearProgress() { stopProgress?.(); stopProgress = undefined; downloadProgress.value = null; }
async function selectFont(id: string) {
  if (disposed || operation.value || loading.value || !isDesktop) return;
  const selected = assets.value.fonts.find(item => item.id === id);
  if (!selected) return;
  error.value = ''; notice.value = '';
  if (selected.downloaded) { font.value = selected.path; fontPickerOpen.value = false; return; }
  operation.value = 'download'; downloadingFont.value = id;
  downloadProgress.value = { id, received: 0, total: selected.bytes };
  if (typeof api.onHandwritingFontProgress === 'function') {
    try {
      stopProgress = await api.onHandwritingFontProgress(progress => {
        if (!disposed && progress.id === id) downloadProgress.value = progress;
      });
    } catch { stopProgress = undefined; }
  }
  try {
    const value = await api.downloadHandwritingFont(id);
    if (disposed) return;
    setAssets(value);
    const installed = value.fonts.find(item => item.id === id && item.downloaded);
    if (!installed) throw new Error('字体尚未安装完成，请重试。');
    font.value = installed.path;
    fontPickerOpen.value = false;
    notice.value = t('字体已下载，可离线使用。');
  } catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) { operation.value = null; downloadingFont.value = ''; } clearProgress(); }
}
function retryPreview() {
  if (!request.value || renderState.value !== 'idle' || operation.value) return;
  renderError.value = ''; notice.value = '';
  scheduler.update(request.value, 0);
}
async function exportImage() {
  if (!canExport.value) return;
  const png = preview.value, filename = `${safeFilename(props.title || t('月光笺'))}.png`;
  operation.value = 'export'; error.value = ''; notice.value = '';
  try { if (await api.saveExport(png, 'png', filename) && !disposed) notice.value = t('月光笺已导出'); }
  catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) operation.value = null; }
}
</script>

<template>
  <section class="handwriting-panel" :aria-label="t('月光笺 · 手写纸笺制作')">
    <div class="handwriting-layout">
      <div class="handwriting-side">
        <slot name="editor" />
        <section class="handwriting-settings" :aria-label="t('月光笺样式')">
          <div class="handwriting-heading"><strong>{{ t('借一纸月光，写几行心事。') }}</strong><span>{{ t('内置手写引擎 · 字体按需下载') }}</span></div>
          <p v-if="loading" class="handwriting-hint" role="status">{{ t('正在读取字体列表…') }}</p>
          <template v-else-if="assets.available">
            <div class="handwriting-field font-field">
              <span class="field-label">{{ t('字体') }}<small role="status">{{ t('显示 {visible} / {total} 款 · 已下载 {downloaded} 款', { visible: formatNumber(visibleFonts.length), total: formatNumber(assets.fonts.length), downloaded: formatNumber(downloadedFonts.length) }) }}</small></span>
              <button type="button" class="font-trigger" aria-haspopup="dialog" :aria-expanded="fontPickerOpen" :aria-label="t('选择手写字体')" @click="fontPickerOpen = !fontPickerOpen">
                <img v-if="currentFont" :src="currentFont.previewUrl" alt="" />
                <span class="font-trigger-name">{{ currentFont ? t(currentFont.name) : isDesktop ? t('选择并下载一款字体') : t('浏览字体预览') }}</span>
                <span class="font-trigger-caret" aria-hidden="true">▾</span>
              </button>
              <p v-if="!isDesktop" class="font-credit">{{ t('浏览器仅展示字体预览，请在桌面版下载字体并生成月光笺。') }}</p>
              <p v-else-if="currentFont" class="font-credit">{{ t('作者：{author}', { author: t(currentFont.author) }) }} · {{ t(currentFont.license) }} · <a :href="currentFont.sourceUrl" target="_blank" rel="noopener noreferrer">{{ t('字体来源') }}</a></p>
              <template v-if="fontPickerOpen">
                <div class="font-backdrop" aria-hidden="true" @click="fontPickerOpen = false" />
                <div class="font-popover" role="dialog" :aria-label="t('选择手写字体')" @keydown.esc.stop="fontPickerOpen = false">
                  <div class="font-tools">
                    <input v-model="fontSearch" type="search" :placeholder="t('输入字体名称')" :aria-label="t('搜索字体')" />
                    <div class="font-filters" role="group" :aria-label="t('字体筛选')"><button type="button" :aria-pressed="fontFilter === 'all'" @click="fontFilter = 'all'">{{ t('全部字体') }}</button><button type="button" :aria-pressed="fontFilter === 'downloaded'" @click="fontFilter = 'downloaded'">{{ t('已下载字体') }}</button><button type="button" :aria-pressed="handwritingOnly" @click="handwritingOnly = !handwritingOnly">{{ t('手写体') }}</button></div>
                  </div>
                  <p v-if="isDesktop" class="handwriting-hint">{{ t('安装包不含完整字体。先看预览，点击哪款才下载哪款；下载后可离线使用。') }}</p>
                  <div class="font-list" role="list">
                    <button v-for="item in visibleFonts" :key="item.id" type="button" role="listitem" class="font-row" :class="{ 'font-selected': item.downloaded && font === item.path }" :disabled="!!operation || !isDesktop" :aria-pressed="item.downloaded && font === item.path" :aria-label="item.downloaded ? t('使用 {name}', { name: t(item.name) }) : t('下载 {name}', { name: t(item.name) })" @click="selectFont(item.id)">
                      <img :src="item.previewUrl" :alt="t('{name} 字体预览', { name: t(item.name) })" loading="lazy" />
                      <span class="font-row-text"><strong>{{ t(item.name) }}</strong><small>{{ item.tags.map(tag => t(tag)).join(' · ') }} · {{ t('作者：{author}', { author: t(item.author) }) }}</small></span>
                      <span class="font-row-state">{{ downloadingFont === item.id ? downloadProgress ? `${t('正在下载并验证…')} ${progressPercent}%` : t('正在下载并验证…') : item.downloaded ? font === item.path ? t('使用中') : t('已下载') : fontSizeLabel(item.bytes) + ' ↓' }}</span>
                    </button>
                    <p v-if="!visibleFonts.length" class="font-empty">{{ t(emptyFontMessage) }}</p>
                  </div>
                  <div v-if="downloadProgress && operation === 'download'" class="font-progress" role="progressbar" :aria-valuenow="progressPercent" aria-valuemin="0" aria-valuemax="100" :aria-label="t('字体下载进度')"><div class="font-progress-track"><i :style="{ width: `${progressPercent}%` }" /></div><span role="status">{{ progressFont ? t(progressFont.name) : '' }} · {{ progressPercent }}% · {{ progressSize }}</span></div>
                </div>
              </template>
            </div>
            <div class="handwriting-field">
              <span class="field-label">{{ t('纸张') }}<small>{{ t(currentPaper.name) }} · {{ currentPaper.width }} × {{ currentPaper.height }}</small></span>
              <div class="paper-grid" role="group" :aria-label="t('手写纸张')">
                <button v-for="item in papers" :key="item.id" type="button" class="paper-option" :aria-pressed="background === item.path" :aria-label="t(item.name)" :title="`${t(item.name)} · ${item.width} × ${item.height}`" :disabled="!!operation" @click="choosePaper(item)">
                  <span class="paper-thumb" :style="item.color ? { backgroundColor: item.color } : undefined"><img v-if="item.previewUrl" :src="item.previewUrl" alt="" loading="lazy" /></span>
                  <span class="paper-name">{{ t(item.name).split(' · ')[0] }}</span><span class="paper-size">{{ item.width }}×{{ item.height }}</span>
                </button>
              </div>
            </div>
            <div class="handwriting-controls">
              <div class="handwriting-field"><span class="field-label">{{ t('墨色') }}</span><UiColor v-model="ink" :label="t('手写墨色')" :disabled="!!operation" /></div>
              <label class="handwriting-field"><span class="field-label">{{ t('字号') }}<small>{{ fontSize }} pt</small></span><input v-model.number="fontSize" type="range" min="12" max="72" step="1" :aria-label="t('手写字号')" /></label>
              <label class="handwriting-field"><span class="field-label">{{ t('留白') }}<small>{{ padding }} px</small></span><input v-model.number="padding" type="range" min="16" max="160" step="4" :aria-label="t('手写留白')" /></label>
            </div>
          </template>
          <div v-else class="handwriting-unavailable">
            <span>✧</span><h3>{{ t('字体列表暂不可用') }}</h3><p>{{ assets.error ? errorText(assets.error) : t('重新读取字体列表后，即可选择字体制作月光笺。') }}</p>
            <button type="button" :disabled="!!operation" @click="loadAssets">{{ t('重试读取字体') }}</button>
          </div>
        </section>
      </div>
      <section class="handwriting-stage" :aria-label="t('月光笺预览')">
        <div class="stage-label">{{ t('导出效果预览') }}<span>{{ currentPaper.width }} × {{ currentPaper.height }}</span></div>
        <div class="handwriting-preview" :aria-busy="renderState !== 'idle'">
          <img v-if="preview && request" :class="{ 'preview-outdated': stale }" :src="`data:image/png;base64,${preview}`" :alt="stale ? t('上一版月光笺，正在更新，暂不可导出') : t('当前月光笺手写预览，导出使用此图片的原始尺寸')" />
          <div v-else class="paper-placeholder"><span>✧</span><p>{{ tooLong ? t('请将正文缩短至 10,000 字以内。') : !text.trim() ? t('写下想留下的话，纸笺会自动出现。') : !font ? t('在左侧选择喜欢的字体，下载后即可开始制作。') : renderError ? t('这次未能生成预览，可以点击下方重试。') : t('正在为这几行文字，准备一纸月光…') }}</p></div>
          <span v-if="stale" class="preview-stale">{{ renderError ? t('预览尚未更新，请重试；当前图片不可导出') : t('正在更新预览，当前图片暂不可导出') }}</span>
        </div>
        <p class="handwriting-hint">{{ t('正文与样式会在短暂停顿后自动预览。单页排版，超出纸张的文字会裁切，请检查预览后导出。') }}</p>
        <div class="handwriting-actions"><span class="auto-preview-state" role="status">{{ t('自动预览 ·') }} {{ renderState === 'rendering' ? t('正在更新…') : renderState === 'waiting' ? t('等待输入停顿…') : renderError ? t('生成失败') : preview && !stale ? t('已更新') : t('等待有效内容') }}</span><button v-if="renderError" type="button" class="render-button" :disabled="!request || renderState !== 'idle' || !!operation" @click="retryPreview">{{ t('重试预览') }}</button><button type="button" class="export-button" :disabled="!canExport" @click="exportImage">{{ operation === 'export' ? t('正在导出…') : t('导出月光笺 PNG') }}</button></div>
        <p class="handwriting-message" aria-live="polite"><span v-if="renderError || error" role="alert">{{ renderError || error }}</span><span v-else-if="tooLong">{{ t('单张纸笺最多支持 10,000 字，请缩短内容。') }}</span><span v-else>{{ notice }}</span></p>
      </section>
    </div>
  </section>
</template>

<style scoped>
.handwriting-panel{min-width:0;color:var(--text)}
.handwriting-panel button{box-sizing:border-box;border:1px solid var(--line);border-radius:7px;color:var(--text);background:var(--input);font:12px system-ui,sans-serif;padding:7px 10px;cursor:pointer}
.handwriting-panel button:disabled{opacity:.45;cursor:default}
.handwriting-panel :focus-visible{outline:2px solid var(--primary);outline-offset:2px}
.handwriting-layout{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:16px;align-items:stretch}
.handwriting-side{display:flex;flex-direction:column;gap:14px;min-width:0}
.handwriting-settings,.handwriting-stage{border:1px solid var(--line);border-radius:10px;background:var(--bg);min-width:0}
.handwriting-settings{padding:14px;display:flex;flex-direction:column;gap:14px}
.handwriting-heading{display:flex;justify-content:space-between;align-items:baseline;gap:4px 12px;flex-wrap:wrap}
.handwriting-heading strong{font-size:13px;font-weight:500;color:var(--gold)}
.handwriting-heading span{font-size:10px;color:var(--muted)}
.handwriting-field{position:relative;display:flex;flex-direction:column;gap:7px;min-width:0;font-size:11px;color:var(--muted)}
.field-label{display:flex;justify-content:space-between;align-items:baseline;gap:8px;font-size:11px;color:var(--muted)}
.field-label small{font-size:10px;color:var(--faint);font-family:inherit;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.font-trigger{display:flex;align-items:center;gap:10px;width:100%;min-height:42px;text-align:left}
.handwriting-panel .font-trigger{padding:5px 10px}
.font-trigger img{height:30px;width:96px;object-fit:contain;border-radius:4px;background:#f8f1e3;flex:none}
.font-trigger-name{flex:1;min-width:0;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.font-trigger-caret{color:var(--muted);font-size:11px}
.font-credit{margin:0;font-size:10px;line-height:1.6;color:var(--faint);overflow-wrap:anywhere}
.font-credit a{color:var(--gold)}
.font-backdrop{position:fixed;inset:0;z-index:20}
.font-popover{position:absolute;z-index:21;top:68px;left:0;right:0;display:flex;flex-direction:column;gap:8px;padding:10px;border:1px solid var(--line);border-radius:10px;background:var(--panel);box-shadow:var(--shadow)}
.font-tools{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.font-tools input{flex:1 1 140px;min-width:0;box-sizing:border-box;border:1px solid var(--line);border-radius:7px;padding:7px 9px;background:var(--input);color:var(--text);font:12px system-ui,sans-serif}
.font-filters{display:flex;gap:5px}
.handwriting-panel .font-filters button{padding:5px 8px;font-size:11px}
.font-filters button[aria-pressed=true]{border-color:var(--gold);color:var(--gold)}
.font-popover .handwriting-hint{margin:0}
.font-list{display:flex;flex-direction:column;gap:4px;max-height:300px;overflow-y:auto;overscroll-behavior:contain;padding-right:2px}
.handwriting-panel .font-row{display:flex;align-items:center;gap:10px;width:100%;padding:5px 8px;text-align:left;border-color:transparent;background:transparent}
.handwriting-panel .font-row:not(:disabled):hover{background:var(--hover)}
.handwriting-panel .font-row:disabled{opacity:1;cursor:default}
.handwriting-panel .font-row.font-selected{border-color:var(--gold);background:var(--gold-dim)}
.font-row img{height:28px;width:90px;object-fit:contain;border-radius:4px;background:#f8f1e3;flex:none}
.font-row-text{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.font-row-text strong{font-size:12px;font-weight:500;color:var(--text);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.font-row-text small{font-size:10px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.font-row-state{font-size:10px;color:var(--muted);white-space:nowrap}
.font-selected .font-row-state{color:var(--gold)}
.font-progress{display:flex;flex-direction:column;gap:6px;font-size:10px;color:var(--muted)}.font-progress-track{height:6px;border:1px solid var(--line);border-radius:99px;background:var(--input);overflow:hidden}.font-progress-track i{display:block;height:100%;border-radius:99px;background:var(--gold);transition:width .18s ease}
.font-empty{margin:0;padding:16px 8px;text-align:center;color:var(--muted);font-size:11px;line-height:1.7}
.paper-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(58px,1fr));gap:8px}
.handwriting-panel .paper-option{display:flex;flex-direction:column;align-items:center;gap:4px;padding:5px 3px;min-width:0;background:transparent;border-color:transparent}
.handwriting-panel .paper-option:not(:disabled):hover{background:var(--hover)}
.handwriting-panel .paper-option[aria-pressed=true]{border-color:var(--gold);background:var(--gold-dim)}
.paper-thumb{width:42px;height:56px;border-radius:3px;border:1px solid var(--line);overflow:hidden;display:flex;align-items:center;justify-content:center;background-color:#fff;background-image:linear-gradient(45deg,#ddd 25%,transparent 25%,transparent 75%,#ddd 75%),linear-gradient(45deg,#ddd 25%,transparent 25%,transparent 75%,#ddd 75%);background-size:8px 8px;background-position:0 0,4px 4px}
.paper-thumb[style]{background-image:none}
.paper-thumb img{width:100%;height:100%;object-fit:contain}
.paper-name{max-width:100%;font-size:10px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.paper-size{font-size:9px;line-height:1;color:var(--faint);font-family:monospace}
.paper-option[aria-pressed=true] .paper-name{color:var(--gold)}
.handwriting-controls{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px}
.handwriting-controls .ui-color{width:100%;min-width:0}
.handwriting-controls input[type=range]{width:100%;margin:6px 0 0}
.handwriting-stage{display:flex;flex-direction:column;background:var(--input);overflow:hidden}
.stage-label{display:flex;align-items:center;justify-content:space-between;height:40px;padding:0 14px;font-size:10px;color:var(--muted);border-bottom:1px solid var(--line)}
.stage-label span{font-family:monospace;color:var(--faint);font-size:9px}
.handwriting-preview{position:relative;flex:1;min-height:360px;max-height:640px;overflow:auto;display:flex;align-items:flex-start;justify-content:center;padding:16px;background-image:radial-gradient(var(--line) .7px,transparent .7px);background-size:12px 12px}
.handwriting-preview img{display:block;width:100%;height:auto;max-width:440px;box-shadow:var(--shadow)}
.handwriting-preview img.preview-outdated{opacity:.5}
.paper-placeholder{align-self:center;padding:34px 20px;text-align:center;font-size:12px;line-height:1.9;color:var(--muted)}
.paper-placeholder span{font-size:32px;color:var(--gold)}
.preview-stale{position:absolute;top:8px;left:8px;right:8px;text-align:center;border-radius:5px;background:var(--panel);border:1px solid var(--line);color:var(--gold);font-size:11px;padding:8px}
.handwriting-hint{font-size:10px;line-height:1.7;color:var(--muted);margin:0}
.handwriting-stage>.handwriting-hint{padding:9px 14px 0;border-top:1px solid var(--line)}
.handwriting-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap;padding:10px 14px 0}
.auto-preview-state{font-size:10px;color:var(--muted);margin-right:auto;line-height:1.7}
.handwriting-panel .render-button,.handwriting-panel .export-button:not(:disabled){background:var(--primary);color:var(--on-accent);border-color:var(--primary)}
.handwriting-panel .handwriting-actions button{padding:9px 12px}
.handwriting-unavailable{min-height:220px;display:flex;flex-direction:column;justify-content:center;align-items:center;text-align:center;padding:16px;gap:10px}
.handwriting-unavailable>span{font-size:38px;color:var(--gold)}
.handwriting-unavailable h3{font-size:17px;margin:0;color:var(--gold)}
.handwriting-unavailable p{font-size:12px;line-height:1.9;color:var(--muted);margin:0}
.handwriting-message{font-size:11px;color:var(--gold);line-height:1.6;min-height:1em;margin:0;padding:8px 14px 12px}
@media(max-width:900px){.handwriting-layout{grid-template-columns:1fr}.handwriting-preview{min-height:260px;max-height:480px}}
@media(max-width:600px){.handwriting-controls{grid-template-columns:1fr 1fr}.handwriting-controls>.handwriting-field:first-child{grid-column:1/-1}}
</style>
