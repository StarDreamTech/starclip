<script setup lang="ts">
import { errorText, formatNumber, t } from '../i18n';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { api, isDesktop } from '../api';
import { safeFilename } from '../snippets';
import { createHandwritingPreview } from '../handwriting-preview';
import type { HandwritingPreviewState, HandwritingSnapshot } from '../handwriting-preview';
import type { HandwritingAssets, HandwritingFontProgress, HandwritingOptions, HandwritingPaper } from '../types';
import UiColor from './UiColor.vue';

const props = defineProps<{ text: string; title: string }>();
const assets = ref<HandwritingAssets>({ available: false, path: null, fonts: [], backgrounds: [], error: null });
const loading = ref(true);
const operation = ref<'download' | 'paper' | 'export' | null>(null);
const downloadingFont = ref('');
const downloadProgress = ref<HandwritingFontProgress | null>(null);
const fontOpen = ref(false);
const fontDialog = ref<HTMLDialogElement | null>(null);
const paperOpen = ref(false);
const paperDialog = ref<HTMLDialogElement | null>(null);
const renderState = ref<HandwritingPreviewState>('idle');
const error = ref('');
const renderError = ref('');
const notice = ref('');
const font = ref('');
const fontSearch = ref('');
const fontFilter = ref<'all' | 'downloaded'>('all');
const handwritingOnly = ref(false);
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
const paperOptions = computed<HandwritingPaper[]>(() => [
  ...[{ id: 'warm', name: '暖白信纸', color: '#f8f1e3' }, { id: 'white', name: '纯白纸张', color: '#ffffff' }].map(item => ({
    id: item.id, name: item.name, path: `solid:${item.color}:1200:1600`, downloaded: true,
    bytes: 0, previewUrl: '', width: 1200, height: 1600, ink: '#24324a', padding: 100,
  })), ...assets.value.backgrounds,
]);
const currentPaper = computed(() => paperOptions.value.find(item => item.path === background.value) ?? paperOptions.value[0]!);
const progressAsset = computed(() => operation.value === 'paper' ? paperOptions.value.find(item => item.id === downloadProgress.value?.id) : progressFont.value);
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
  if (!background.value.startsWith('solid:') && !value.backgrounds.some(item => item.path === background.value)) background.value = 'solid:#f8f1e3:1200:1600';
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
function openFontDialog() {
  fontOpen.value = true;
  nextTick(() => {
    const element = fontDialog.value;
    if (element && !element.open && typeof element.showModal === 'function') {
      try { element.showModal(); } catch { /* keep the inline fallback */ }
    }
  });
}
function closeFontDialog() {
  const element = fontDialog.value;
  if (element?.open && typeof element.close === 'function') element.close();
  fontOpen.value = false;
}
function openPaperDialog() {
  paperOpen.value = true;
  nextTick(() => { const element = paperDialog.value; if (element && !element.open && typeof element.showModal === 'function') element.showModal(); });
}
function closePaperDialog() {
  if (paperDialog.value?.open && typeof paperDialog.value.close === 'function') paperDialog.value.close();
  paperOpen.value = false;
}
async function selectPaper(id: string) {
  if (disposed || operation.value || loading.value) return;
  let selected = paperOptions.value.find(item => item.id === id);
  if (!selected) return;
  error.value = ''; notice.value = '';
  if (!selected.downloaded) {
    if (!isDesktop) return;
    operation.value = 'paper';
    downloadProgress.value = { id, received: 0, total: selected.bytes };
    try {
      try { stopProgress = await api.onHandwritingPaperProgress(progress => { if (!disposed && progress.id === id) downloadProgress.value = progress; }); }
      catch { stopProgress = undefined; }
      if (disposed) return;
      const value = await api.downloadHandwritingPaper(id);
      if (disposed) return;
      setAssets(value);
      selected = paperOptions.value.find(item => item.id === id && item.downloaded);
      if (!selected) throw new Error('纸张尚未安装完成，请重试。');
    } catch (reason) { if (!disposed) error.value = errorText(reason); return; }
    finally { if (!disposed) operation.value = null; clearProgress(); }
  }
  background.value = selected.path;
  ink.value = selected.ink;
  padding.value = selected.padding;
  fontSize.value = Math.min(fontSize.value, selected.width < 400 ? 20 : 48);
  closePaperDialog();
}
async function selectFont(id: string) {
  if (disposed || operation.value || loading.value || !isDesktop) return;
  const selected = assets.value.fonts.find(item => item.id === id);
  if (!selected) return;
  error.value = ''; notice.value = '';
  if (selected.downloaded) { font.value = selected.path; return; }
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
    <div class="handwriting-workbench">
      <section class="handwriting-editor-pane">
        <slot name="editor" />
        <p v-if="loading" class="handwriting-hint" role="status">{{ t('正在读取字体列表…') }}</p>
        <div v-else-if="assets.available" class="handwriting-settings">
          <div class="picker-row">
            <div class="handwriting-field"><span>{{ t('字体') }}</span><button type="button" class="compact-picker" :aria-label="t('选择手写字体')" :disabled="!!operation" @click="openFontDialog"><strong>{{ currentFont ? t(currentFont.name) : t('选择字体') }}</strong><span aria-hidden="true">⌄</span></button></div>
            <div class="handwriting-field"><span>{{ t('纸张') }}</span><button type="button" class="compact-picker" :aria-label="t('选择纸张')" :disabled="!!operation" @click="openPaperDialog"><strong>{{ t(currentPaper.name) }}</strong><span aria-hidden="true">⌄</span></button></div>
          </div>
          <div class="detail-row">
            <div class="handwriting-field"><span>{{ t('墨色') }}</span><UiColor v-model="ink" :label="t('手写墨色')" :disabled="!!operation" /></div>
            <label class="handwriting-field slider-field"><span>{{ t('字号') }} <b>{{ fontSize }} pt</b></span><input v-model.number="fontSize" type="range" min="12" max="72" step="1" :aria-label="t('手写字号')" :disabled="!!operation" /></label>
            <label class="handwriting-field slider-field"><span>{{ t('留白') }} <b>{{ padding }} px</b></span><input v-model.number="padding" type="range" min="0" max="160" step="4" :aria-label="t('手写留白')" :disabled="!!operation" /></label>
          </div>
          <p class="handwriting-hint">{{ downloadedFonts.length ? t('已下载 {count} 款 · 可离线使用', { count: formatNumber(downloadedFonts.length) }) : t('字体按需下载，选中后保存在本机。') }} · {{ t('纸张 {count} 款', { count: paperOptions.length }) }}</p>
          <p class="handwriting-hint">{{ t('单页超出部分会裁切，请在导出前检查预览。') }}</p>
          <p v-if="!isDesktop" class="browser-note">{{ t('浏览器仅供预览，请在桌面版下载字体与纸张。') }}</p>
        </div>
        <div v-else class="handwriting-unavailable"><p>{{ assets.error || t('字体列表暂不可用') }}</p><button type="button" :disabled="!!operation" @click="loadAssets">{{ t('重试读取字体') }}</button></div>
      </section>
      <section class="handwriting-result-pane">
        <div class="paper-preview-heading"><strong>{{ t('纸笺预览') }}</strong><span>{{ currentPaper.width }} × {{ currentPaper.height }}</span></div>
        <div class="paper-stage" :aria-busy="renderState !== 'idle'">
          <img v-if="preview && request" :class="{ 'preview-outdated': stale }" :src="`data:image/png;base64,${preview}`" :alt="stale ? t('上一版月光笺，正在更新，暂不可导出') : t('当前月光笺手写预览，导出使用此图片的原始尺寸')" />
          <div v-else class="paper-placeholder"><span>✧</span><p>{{ tooLong ? t('请将正文缩短至 10,000 字以内。') : !text.trim() ? t('写下想留下的话，纸笺会自动出现。') : !font ? t('选择字体后，纸笺会自动出现在这里。') : renderError ? t('这次未能生成预览，可以点击下方重试。') : t('正在为这几行文字，准备一纸月光…') }}</p><img v-if="!isDesktop && currentPaper.previewUrl" class="paper-sample" :src="currentPaper.previewUrl" :alt="t('{name} 纸张预览', { name: t(currentPaper.name) })" /></div>
          <span v-if="stale" class="preview-stale">{{ renderError ? t('预览尚未更新，请重试；当前图片不可导出') : t('正在更新预览，当前图片暂不可导出') }}</span>
        </div>
        <p v-if="renderError || error" class="handwriting-message handwriting-error" role="alert">{{ renderError || error }}</p>
        <footer class="paper-preview-footer"><span role="status">{{ renderState === 'rendering' ? t('正在更新预览…') : renderState === 'waiting' ? t('等待更新预览…') : preview && !stale ? t('自动预览 · 已更新') : t('预览即导出') }}</span><button v-if="renderError && request" type="button" :disabled="!!operation || renderState !== 'idle'" @click="retryPreview">{{ t('重试预览') }}</button><button type="button" class="render-button" :disabled="!canExport" @click="exportImage">{{ operation === 'export' ? t('正在导出…') : t('导出月光笺 PNG') }}</button></footer>
        <p v-if="notice" class="handwriting-message" role="status">{{ notice }}</p>
      </section>
    </div>
      <dialog v-if="fontOpen" ref="fontDialog" class="font-dialog" :aria-label="t('选择手写字体')" @close="fontOpen = false" @cancel.prevent.stop="closeFontDialog" @click.self="closeFontDialog">
        <div class="font-dialog-heading"><div><strong>{{ t('选择手写字体') }}</strong><p>{{ t('安装包不含完整字体。先看预览，点击哪款才下载哪款；下载后可离线使用。') }}</p><p v-if="!isDesktop" class="browser-note">{{ t('浏览器仅供预览，请在桌面版下载字体与纸张。') }}</p></div><button type="button" class="font-dialog-close" :aria-label="t('关闭字体选择')" @click="closeFontDialog">×</button></div>
        <div class="font-tools">
          <label class="font-search"><span>{{ t('搜索字体') }}</span><input v-model="fontSearch" type="search" autofocus :placeholder="t('输入字体名称')" :aria-label="t('搜索字体')" /></label>
          <div class="font-filters" role="group" :aria-label="t('字体筛选')"><button type="button" :aria-pressed="fontFilter === 'all'" @click="fontFilter = 'all'">{{ t('全部字体') }}</button><button type="button" :aria-pressed="fontFilter === 'downloaded'" @click="fontFilter = 'downloaded'">{{ t('已下载字体') }}</button><button type="button" :aria-pressed="handwritingOnly" @click="handwritingOnly = !handwritingOnly">{{ t('手写体') }}</button></div>
        </div>
        <div class="font-summary"><span role="status">{{ t('显示 {visible} / {total} 款 · 已下载 {downloaded} 款', { visible: formatNumber(visibleFonts.length), total: formatNumber(assets.fonts.length), downloaded: formatNumber(downloadedFonts.length) }) }}</span><span v-if="currentFont">{{ t('正在使用：{name}', { name: t(currentFont.name) }) }}</span></div>
        <div class="font-gallery" role="region" tabindex="0" :aria-label="t('选择手写字体')">
          <article v-for="item in visibleFonts" :key="item.id" class="font-card" :class="{ 'font-selected': item.downloaded && font === item.path }">
            <img :src="item.previewUrl" :alt="t('{name} 字体预览', { name: t(item.name) })" loading="lazy" />
            <div class="font-card-heading"><strong>{{ t(item.name) }}</strong><span>{{ fontSizeLabel(item.bytes) }}</span></div>
            <div class="font-card-author">{{ t('作者：{author}', { author: t(item.author) }) }}</div>
            <div class="font-card-tags"><span v-for="tag in item.tags" :key="tag">{{ t(tag) }}</span></div>
            <div class="font-card-license"><span>{{ t(item.license) }}</span><a :href="item.sourceUrl" target="_blank" rel="noopener noreferrer">{{ t('字体来源') }}</a></div>
            <button type="button" :disabled="!!operation || !isDesktop" :aria-pressed="item.downloaded && font === item.path" :aria-label="item.downloaded ? t('使用 {name}', { name: t(item.name) }) : t('下载 {name}', { name: t(item.name) })" @click="selectFont(item.id)">{{ downloadingFont === item.id ? downloadProgress ? `${t('正在下载并验证…')} ${progressPercent}%` : t('正在下载并验证…') : item.downloaded ? font === item.path ? t('正在使用 · 已下载') : t('使用此字体 · 已下载') : !isDesktop ? t('仅桌面版可下载') : t('下载此字体') }}</button>
          </article>
          <p v-if="!visibleFonts.length" class="font-empty">{{ t(emptyFontMessage) }}</p>
        </div>
        <div v-if="downloadProgress && operation === 'download'" class="font-progress" role="progressbar" :aria-valuenow="progressPercent" aria-valuemin="0" aria-valuemax="100" :aria-label="t('字体下载进度')"><div class="font-progress-track"><i :style="{ width: `${progressPercent}%` }" /></div><span role="status">{{ progressFont ? t(progressFont.name) : '' }} · {{ progressPercent }}% · {{ progressSize }}</span></div>
        <p v-if="error" role="alert" class="handwriting-message handwriting-error">{{ error }}</p><footer class="font-dialog-footer"><span>{{ currentFont ? t('正在使用：{name}', { name: t(currentFont.name) }) : t('尚未选择字体') }}</span><button type="button" @click="closeFontDialog">{{ t('完成') }}</button></footer>
      </dialog>
    <dialog v-if="paperOpen" ref="paperDialog" class="font-dialog paper-dialog" :aria-label="t('选择纸张')" @close="paperOpen = false" @cancel.prevent.stop="closePaperDialog" @click.self="closePaperDialog">
      <header class="font-dialog-heading"><div><strong>{{ t('选择纸张') }}</strong><p>{{ t('原手写工具纸张全部保留，图案纸按需下载，纯色纸可直接使用。') }}</p><p v-if="!isDesktop" class="browser-note">{{ t('浏览器仅供预览，请在桌面版下载字体与纸张。') }}</p></div><button type="button" class="font-dialog-close" :aria-label="t('关闭纸张选择')" @click="closePaperDialog">×</button></header>
      <div class="paper-gallery">
        <button v-for="item in paperOptions" :key="item.id" type="button" class="paper-choice" :class="{ selected: item.path === background }" :aria-label="t('使用纸张 {name}', { name: t(item.name) })" :aria-pressed="item.path === background" :disabled="!!operation || (!isDesktop && !item.downloaded)" @click="selectPaper(item.id)">
          <div class="paper-thumb"><img v-if="item.previewUrl" :src="item.previewUrl" :alt="t('{name} 纸张预览', { name: t(item.name) })" loading="lazy" /><span v-else :style="{ background: item.path.split(':')[1] }"></span></div><strong>{{ t(item.name) }}</strong><small>{{ item.width }} × {{ item.height }}</small><em>{{ item.path === background ? t('正在使用') : item.downloaded ? t('可离线使用') : !isDesktop ? t('仅桌面版可下载') : t('下载并使用') }}</em>
        </button>
      </div>
      <div v-if="downloadProgress && operation === 'paper'" class="font-progress" role="progressbar" :aria-valuenow="progressPercent" aria-valuemin="0" aria-valuemax="100" :aria-label="t('纸张下载进度')"><div class="font-progress-track"><i :style="{ width: `${progressPercent}%` }" /></div><span>{{ progressAsset ? t(progressAsset.name) : '' }} · {{ progressPercent }}% · {{ progressSize }}</span></div>
      <p v-if="error" role="alert" class="handwriting-message handwriting-error">{{ error }}</p>
      <footer class="font-dialog-footer"><span>{{ t('保留原图比例与标识；导出包含原纸张图案。') }}</span><button type="button" @click="closePaperDialog">{{ t('完成') }}</button></footer>
    </dialog>
  </section>
</template>

<style scoped>
.handwriting-panel{min-width:0;min-height:0;color:var(--text)}
.handwriting-workbench{display:grid;grid-template-columns:minmax(0,.95fr) minmax(0,1.05fr);gap:18px;height:clamp(450px,calc(100dvh - 190px),760px);min-height:0}
.handwriting-editor-pane,.handwriting-result-pane{min-width:0;min-height:0;display:flex;flex-direction:column}
.handwriting-editor-pane{gap:12px}.handwriting-settings{padding-top:2px}
.picker-row{display:grid;grid-template-columns:1fr 1fr;gap:12px}.detail-row{display:grid;grid-template-columns:1fr 1fr 1fr;align-items:end;gap:14px;margin-top:12px}
.handwriting-field{display:flex;flex-direction:column;gap:6px;min-width:0;font-size:11px;color:var(--muted)}
.handwriting-field>span{display:flex;justify-content:space-between;gap:6px}.handwriting-field b{font-weight:400;color:var(--text);font-variant-numeric:tabular-nums}
.handwriting-panel button{box-sizing:border-box;border:1px solid var(--line);border-radius:7px;color:var(--text);background:var(--input);font:12px system-ui,sans-serif;padding:8px 10px;cursor:pointer}
.handwriting-panel button:disabled{opacity:.45;cursor:default}.handwriting-panel :focus-visible{outline:2px solid var(--primary);outline-offset:2px}
.compact-picker{width:100%;height:36px;display:flex;align-items:center;justify-content:space-between;gap:10px;text-align:left}.compact-picker strong{font-size:12px;font-weight:400;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.compact-picker>span{color:var(--muted)}
.slider-field input{height:34px;margin:0;width:100%;accent-color:var(--gold)}
.handwriting-hint,.browser-note,.handwriting-message{font-size:10px;line-height:1.6;color:var(--muted);margin:8px 0 0}.browser-note{color:var(--gold)}.handwriting-error{color:var(--danger)}
.handwriting-result-pane{border:1px solid var(--line);border-radius:10px;overflow:hidden;background:var(--input)}
.paper-preview-heading{display:flex;align-items:center;justify-content:space-between;padding:11px 14px;border-bottom:1px solid var(--line);font-size:11px;color:var(--muted)}.paper-preview-heading strong{font-weight:400}.paper-preview-heading>span{font:10px monospace;color:var(--faint)}
.paper-stage{position:relative;flex:1;min-height:0;display:flex;align-items:center;justify-content:center;padding:18px;overflow:hidden;background-image:radial-gradient(var(--line) .6px,transparent .6px);background-size:12px 12px}
.paper-stage>img{display:block;max-width:100%;max-height:100%;width:auto;height:auto;object-fit:contain;box-shadow:0 4px 20px #0003}.paper-stage>img.preview-outdated{opacity:.5}
.paper-placeholder{text-align:center;max-width:280px;font-size:12px;line-height:1.8;color:var(--muted)}.paper-placeholder>span{font-size:30px;color:var(--gold)}.paper-sample{display:block;width:auto;height:170px;max-width:100%;object-fit:contain;margin:16px auto 0}
.preview-stale{position:absolute;top:8px;left:8px;right:8px;padding:7px;background:var(--panel);border:1px solid var(--line);border-radius:6px;font-size:10px;text-align:center;color:var(--gold)}
.paper-preview-footer{display:flex;align-items:center;gap:8px;padding:10px 12px;border-top:1px solid var(--line);flex-wrap:wrap}.paper-preview-footer>span{font-size:10px;color:var(--muted);margin-right:auto}.render-button{background:var(--primary)!important;color:var(--on-accent)!important;border-color:var(--primary)!important}.handwriting-result-pane>.handwriting-message{padding:0 12px 8px}
.handwriting-unavailable{font-size:12px;color:var(--muted);padding:12px}
.font-dialog{width:min(760px,calc(100vw - 48px));max-height:calc(100dvh - 64px);box-sizing:border-box;border:1px solid var(--line);border-radius:14px;padding:18px;background:var(--panel);color:var(--text);box-shadow:var(--shadow);overflow:auto;overscroll-behavior:contain}
.font-dialog::backdrop{background:var(--backdrop);backdrop-filter:blur(5px)}.font-dialog-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;margin-bottom:14px}.font-dialog-heading strong{display:block;font-size:14px;font-weight:500;color:var(--gold)}.font-dialog-heading p{margin:6px 0 0;font-size:11px;line-height:1.7;color:var(--muted)}.font-dialog-heading p.browser-note{color:var(--gold)}.font-dialog-close{font-size:22px!important;line-height:1!important;background:transparent!important;border:0!important;color:var(--muted)!important;padding:0 6px!important}
.font-tools{display:flex;align-items:flex-end;gap:10px;flex-wrap:wrap}.font-search{display:flex;flex:1 1 150px;min-width:0;flex-direction:column;gap:6px;font-size:11px;color:var(--muted)}.font-search input{box-sizing:border-box;width:100%;min-width:0;border:1px solid var(--line);border-radius:7px;padding:9px 10px;background:var(--input);color:var(--text);font:12px system-ui,sans-serif}.font-filters{display:flex;gap:6px;flex-wrap:wrap}.handwriting-panel .font-filters button[aria-pressed=true]{border-color:var(--gold);color:var(--gold);background:var(--bg)}.font-summary{display:flex;justify-content:space-between;gap:6px;flex-wrap:wrap;margin-top:10px;font-size:10px;line-height:1.6;color:var(--muted)}
.font-gallery{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(155px,100%),1fr));align-content:start;gap:10px;margin:10px 0 12px;padding:3px;max-height:min(45vh,360px);overflow-y:auto;overflow-x:hidden;scrollbar-gutter:stable;overscroll-behavior-y:contain}
.font-card{min-width:0;border:1px solid var(--line);border-radius:8px;padding:9px;background:var(--input);display:flex;flex-direction:column;gap:8px}.font-card.font-selected{border-color:var(--gold)}.font-card>img{width:100%;aspect-ratio:3 / 1;object-fit:contain;border-radius:4px;background:#f8f1e3}.font-card-heading,.font-card-license{display:flex;justify-content:space-between;gap:6px;flex-wrap:wrap;align-items:baseline;font-size:10px;color:var(--muted)}.font-card-heading strong{font-size:12px;font-weight:500;color:var(--text)}.font-card-license a{color:var(--gold)}.font-card>button{width:100%;margin-top:auto;font-size:11px}.font-selected>button{color:var(--gold)}.font-card-author{font-size:10px;line-height:1.5;color:var(--muted);overflow-wrap:anywhere}.font-card-tags{display:flex;gap:5px;flex-wrap:wrap}.font-card-tags span{border:1px solid var(--line);border-radius:4px;padding:2px 5px;font-size:10px;color:var(--muted)}.font-empty{grid-column:1/-1;padding:16px;color:var(--muted);font-size:12px}
.font-progress{display:flex;flex-direction:column;gap:6px;margin:0 0 12px;font-size:10px;color:var(--muted)}.font-progress-track{height:6px;border:1px solid var(--line);border-radius:99px;background:var(--input);overflow:hidden}.font-progress-track i{display:block;height:100%;border-radius:99px;background:var(--gold);transition:width .18s ease}.font-dialog-footer{display:flex;align-items:center;justify-content:space-between;gap:10px;border-top:1px solid var(--line);padding-top:12px;font-size:10px;color:var(--muted)}
.paper-gallery{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;max-height:52vh;overflow:auto;padding:3px;margin-bottom:12px}.handwriting-panel .paper-choice{display:flex;flex-direction:column;gap:5px;padding:9px;min-width:0;text-align:left}.handwriting-panel .paper-choice.selected{border-color:var(--gold)}.paper-thumb{height:128px;display:flex;align-items:center;justify-content:center;width:100%;background-color:var(--bg);background-image:conic-gradient(#8e91981a 25%,transparent 0 50%,#8e91981a 0 75%,transparent 0);background-size:12px 12px;border-radius:5px;overflow:hidden}.paper-thumb img{max-width:100%;max-height:100%;object-fit:contain}.paper-thumb>span{display:block;width:85%;height:100%;box-shadow:0 2px 8px #0002}.paper-choice strong{font-size:11px;font-weight:400;line-height:1.5}.paper-choice small{font:9px monospace;color:var(--muted)}.paper-choice em{font-size:10px;font-style:normal;color:var(--gold)}
@media(max-width:800px){.handwriting-workbench{grid-template-columns:1fr;height:auto}.handwriting-result-pane{height:540px}.paper-stage{min-height:220px}.handwriting-editor-pane{min-height:400px}.paper-gallery{grid-template-columns:repeat(3,minmax(0,1fr))}}
@media(max-width:500px){.picker-row{gap:8px}.detail-row{gap:8px;grid-template-columns:1fr 1fr}.detail-row>.handwriting-field:first-child{grid-column:1/-1}.paper-gallery{grid-template-columns:repeat(2,minmax(0,1fr))}.handwriting-result-pane{height:480px}}
</style>
