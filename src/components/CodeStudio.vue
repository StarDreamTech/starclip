<script setup lang="ts">
import { errorText, t } from '../i18n';
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue';
import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import json from 'highlight.js/lib/languages/json';
import xml from 'highlight.js/lib/languages/xml';
import css from 'highlight.js/lib/languages/css';
import bash from 'highlight.js/lib/languages/bash';
import sql from 'highlight.js/lib/languages/sql';
import markdown from 'highlight.js/lib/languages/markdown';
import go from 'highlight.js/lib/languages/go';
import java from 'highlight.js/lib/languages/java';
import HandwritingPanel from './HandwritingPanel.vue';
import UiSelect from './UiSelect.vue';
import UiColor from './UiColor.vue';
import type { Clip, Group } from '../types';
import { api } from '../api';
import { languages, formatJson, markdownCode, safeFilename } from '../snippets';
import { defaultCodeCardStyle, renderCodeCard } from '../code-card';

Object.entries({ javascript, typescript, python, rust, json, html: xml, css, bash, sql, markdown, go, java }).forEach(([name, definition]) => hljs.registerLanguage(name, definition));
const props = withDefaults(defineProps<{ clip: Clip | null; groups: Group[]; initialMode?: 'code' | 'handwriting' }>(), { initialMode: 'code' });
const emit = defineEmits<{ saved: []; close: [] }>();
const dialog = ref<HTMLDialogElement>();
const mode = ref<'code' | 'handwriting'>(props.initialMode);
const title = ref(props.clip?.title || (props.initialMode === 'handwriting' ? t('写给星空的信') : t('一页星码')));
// Switching modes shares the user's original text; neither tab replaces or rewrites it.
const content = ref(props.clip?.content ?? (props.initialMode === 'handwriting' ? t('让散落的灵感，\n在这里连成星座。') : 'fn main() {\n    println!("Hello, stars. ✦");\n}'));
const language = ref(props.clip?.language || (props.clip || props.initialMode === 'handwriting' ? 'plaintext' : 'rust'));
const groupId = ref<number | null>(props.clip?.groupId ?? null);
const languageOptions = computed(() => languages.map(([value, label]) => ({ value, label: t(label) })));
const groupOptions = computed(() => [{ value: null, label: t('未分组') }, ...props.groups.map(group => ({ value: group.id, label: group.name }))]);
const codeThemeOptions = computed(() => [{ value: 'dark', label: t('深空 · 深色') }, { value: 'light', label: t('晨光 · 浅色') }] as const);
const paletteOptions = computed(() => [{ value: 'night', label: t('深空蓝') }, { value: 'nebula', label: t('星云紫') }, { value: 'solid', label: t('自选纯色') }] as const);
const widthOptions = computed(() => [{ value: 'auto', label: t('自适应正文') }, { value: 'fixed', label: t('固定宽度 · 长行软换行') }] as const);
const windowOptions = computed(() => [{ value: 'mac', label: t('macOS 三色圆点') }, { value: 'minimal', label: t('简洁标题') }] as const);
const style = reactive({ ...defaultCodeCardStyle, watermark: t(defaultCodeCardStyle.watermark) });
const busy = ref(false);
const error = ref('');
const notice = ref('');
const preview = ref<{ url: string; width: number; height: number } | null>(null);
const previewError = ref('');
let previewTimer: ReturnType<typeof setTimeout> | undefined;
const modifier = navigator.platform.toLowerCase().includes('mac') ? '⌘' : 'Ctrl';
const highlighted = computed(() => {
  if (content.value.length > 100000 || language.value === 'plaintext' || !hljs.getLanguage(language.value)) return escape(content.value);
  return hljs.highlight(content.value, { language: language.value, ignoreIllegals: true }).value;
});
function escape(text: string) { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'); }
function makePreview() {
  clearTimeout(previewTimer);
  if (mode.value !== 'code') return null;
  try {
    const result = renderCodeCard(title.value, content.value, language.value, highlighted.value, style);
    preview.value = result; previewError.value = '';
    return result;
  } catch (e) { preview.value = null; previewError.value = errorText(e); return null; }
}
function schedulePreview() { clearTimeout(previewTimer); previewTimer = setTimeout(makePreview, 160); }
watch([title, content, language, mode], schedulePreview);
watch(style, schedulePreview);
watch(mode, () => { error.value = ''; notice.value = ''; });
onMounted(async () => { await nextTick(); dialog.value?.showModal(); makePreview(); });
onUnmounted(() => clearTimeout(previewTimer));
async function action(fn: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true; error.value = ''; notice.value = '';
  try { await fn(); } catch (e) { error.value = errorText(e); } finally { busy.value = false; }
}
function save() {
  if (mode.value !== 'code' || !content.value.trim()) return;
  return action(async () => { await api.saveCode(props.clip?.id ?? null, title.value, content.value, language.value, groupId.value); emit('saved'); emit('close'); });
}
function format() { try { content.value = formatJson(content.value); error.value = ''; } catch { error.value = t('JSON 格式有误，请检查后再格式化。'); } }
function exportMarkdown() {
  return action(async () => { if (await api.saveExport(markdownCode(title.value, content.value, language.value), 'md', `${safeFilename(title.value)}.md`)) notice.value = t('Markdown 已导出'); });
}
function insertTab(event: KeyboardEvent) {
  if (event.key !== 'Tab' || mode.value !== 'code') return;
  event.preventDefault();
  const field = event.target as HTMLTextAreaElement;
  const start = field.selectionStart, end = field.selectionEnd;
  content.value = content.value.slice(0, start) + '  ' + content.value.slice(end);
  nextTick(() => { field.selectionStart = field.selectionEnd = start + 2; });
}
function exportImage() {
  return action(async () => {
    const image = makePreview();
    if (!image) throw new Error(previewError.value || t('暂时无法生成卡片'));
    if (await api.saveExport(image.url.split(',')[1]!, 'png', `${safeFilename(title.value)}.png`)) notice.value = t('星码卡已导出，与当前预览一致');
  });
}
</script>

<template>
  <dialog ref="dialog" class="code-studio" :class="{ 'is-handwriting': mode === 'handwriting' }" @cancel.prevent="!busy && emit('close')" @click="event => { if (!busy && event.target === dialog) emit('close'); }" @keydown.meta.s.prevent="save" @keydown.ctrl.s.prevent="save">
    <header class="studio-topbar"><div class="studio-heading"><div><span class="studio-eyebrow">A LITTLE PIECE OF YOUR UNIVERSE</span><h2>{{ t('星绘坊') }} <span>✦</span></h2><p>{{ t('让代码有星光，让文字有温度。') }}</p></div><button class="studio-close" :aria-label="t('关闭星绘坊')" :disabled="busy" @click="emit('close')">×</button></div>
    <div class="studio-tabs" role="tablist" :aria-label="t('卡片类型')"><button id="code-tab" role="tab" :aria-selected="mode === 'code'" aria-controls="studio-workspace" :class="{ active: mode === 'code' }" :disabled="busy" @click="mode = 'code'"><span aria-hidden="true">〈/〉</span>{{ t('星码卡') }}<em>{{ t('代码卡片') }}</em></button><button id="handwriting-tab" role="tab" :aria-selected="mode === 'handwriting'" aria-controls="studio-workspace" :class="{ active: mode === 'handwriting' }" :disabled="busy" @click="mode = 'handwriting'"><span aria-hidden="true">✎</span>{{ t('月光笺') }}<em>{{ t('手写纸笺') }}</em></button><small>{{ t('切换样式，正文保留') }}</small></div>
    </header>
    <div id="studio-workspace" role="tabpanel" :aria-labelledby="mode === 'code' ? 'code-tab' : 'handwriting-tab'">
      <div v-if="mode === 'code'" class="studio-meta"><label>{{ t('标题') }}<input v-model="title" maxlength="200" :aria-label="t('卡片标题')" /></label><div class="studio-field"><span>{{ t('语言') }}</span><UiSelect v-model="language" :options="languageOptions" :label="t('代码语言')" :disabled="busy" /></div><div class="studio-field"><span>{{ t('分组') }}</span><UiSelect v-model="groupId" :options="groupOptions" :label="t('代码分组')" :disabled="busy" /></div></div>
      <div v-if="mode === 'code'" class="studio-panels">
        <section class="studio-editor"><div class="studio-panel-label">{{ mode === 'code' ? t('编辑代码') : t('纸笺正文') }}<button v-if="mode === 'code' && language === 'json'" @click="format">{{ t('格式化 JSON') }}</button><span v-else>{{ content.split('\n').length }} {{ t('行 ·') }} {{ [...content].length }} {{ t('字符') }}</span></div><textarea v-model="content" :aria-label="mode === 'code' ? t('代码内容') : t('纸笺正文')"  spellcheck="false" autocomplete="off" autocapitalize="off" @keydown="insertTab" /><p class="editor-note">{{ mode === 'code' ? t('Tab 插入两个空格 · 原文只在本机处理') : t('切换卡片类型不会改写正文，也不会保存为代码。') }}</p></section>
        <section v-if="mode === 'code'" class="studio-preview"><div class="studio-panel-label">{{ t('导出效果预览') }} <span v-if="preview">{{ preview.width }} × {{ preview.height }}</span></div><div class="canvas-preview"><img v-if="preview" :src="preview.url" :alt="t('{title}，星码卡导出预览', { title })" /><div v-else class="preview-placeholder"><span>✦</span><p>{{ previewError || t('正在点亮这张卡片…') }}</p></div></div><p class="preview-note">{{ style.widthMode === 'fixed' ? t('长行软换行，原文与行号保留 · PNG 以 2 倍分辨率生成') : t('预览即导出 · PNG 以 2 倍分辨率生成') }}</p></section>

      </div>
      <HandwritingPanel v-if="mode === 'handwriting'" :text="content" :title="title">
        <template #editor>
          <label class="handwriting-title-field">{{ t('标题') }}<input v-model="title" maxlength="200" :aria-label="t('卡片标题')" /></label>
          <section class="studio-editor handwriting-editor"><div class="studio-panel-label">{{ t('纸笺正文') }}<span>{{ content.split('\n').length }} {{ t('行 ·') }} {{ [...content].length }} {{ t('字符') }}</span></div><textarea v-model="content" :aria-label="t('纸笺正文')" class="handwriting-text" spellcheck="false" autocomplete="off" autocapitalize="off" /><p class="editor-note">{{ t('正文与样式自动预览，原文只在本机处理。') }}</p></section>
        </template>
      </HandwritingPanel>
      <section v-if="mode === 'code'" class="card-controls" :aria-label="t('代码卡片样式')">
        <div class="control-heading"><h3>{{ t('装点这张卡片') }}</h3><button class="reset-style" @click="Object.assign(style, defaultCodeCardStyle)">{{ t('恢复默认') }}</button></div>
        <div class="control-grid">
          <div class="control-field"><span>{{ t('代码配色') }}</span><UiSelect v-model="style.codeTheme" :options="codeThemeOptions" :label="t('代码配色')" :disabled="busy" /></div>
          <div class="control-field"><span>{{ t('外层背景') }}</span><UiSelect v-model="style.palette" :options="paletteOptions" :label="t('卡片背景')" :disabled="busy" /></div>
          <div v-if="style.palette === 'solid'" class="control-field"><span>{{ t('背景颜色') }}</span><UiColor v-model="style.background" :label="t('自选背景颜色')" :disabled="busy" /></div>
          <div class="control-field"><span>{{ t('卡片宽度') }}</span><UiSelect v-model="style.widthMode" :options="widthOptions" :label="t('卡片宽度模式')" :disabled="busy" /></div>
          <label v-if="style.widthMode === 'fixed'">{{ t('代码窗宽度') }} <span>{{ style.fixedWidth }} px</span><input v-model.number="style.fixedWidth" type="range" min="320" max="960" step="16" :aria-label="t('代码窗宽度')" /></label>
          <label>{{ t('横向留白') }} <span>{{ style.paddingX }} px</span><input v-model.number="style.paddingX" type="range" min="16" max="96" step="4" :aria-label="t('横向留白')" /></label>
          <label>{{ t('纵向留白') }} <span>{{ style.paddingY }} px</span><input v-model.number="style.paddingY" type="range" min="16" max="96" step="4" :aria-label="t('纵向留白')" /></label>
          <div class="control-field"><span>{{ t('窗头') }}</span><UiSelect v-model="style.windowStyle" :options="windowOptions" :label="t('窗头样式')" :disabled="busy" /></div>
          <div class="control-checks"><label><input v-model="style.lineNumbers" type="checkbox" />{{ t('显示行号') }}</label><label><input v-model="style.shadow" type="checkbox" />{{ t('卡片阴影') }}</label></div>
          <label class="watermark-control">{{ t('水印') }} <span>{{ t('留空即隐藏') }}</span><input v-model="style.watermark" maxlength="80" :aria-label="t('卡片水印')" :placeholder="t('写下一句属于你的话')" /></label>
          <label>{{ t('水印不透明度') }} <span>{{ Math.round(style.watermarkOpacity * 100) }}%</span><input v-model.number="style.watermarkOpacity" type="range" min="0" max="1" step="0.05" :aria-label="t('水印不透明度')" :disabled="!style.watermark.trim()" /></label>
        </div>
      </section>
    </div>
    <div v-if="mode === 'code'" class="studio-message" aria-live="polite"><span v-if="error" class="studio-error">{{ error }}</span><span v-else-if="notice">{{ notice }}</span><span v-else-if="previewError" class="studio-error">{{ previewError }}</span><span v-else>{{ t('本地保存 · 不上传代码 · 新建片段收入星码集，并自动星标') }}</span></div>
    <footer v-if="mode === 'code'" class="studio-footer"><div class="studio-export"><button :disabled="busy || !content.trim()" @click="exportMarkdown">{{ t('导出 Markdown') }}</button><button :disabled="busy || !content.trim() || !!previewError" @click="exportImage">{{ t('导出星码卡 PNG') }}</button></div><button class="studio-save" :disabled="busy || !content.trim()" @click="save">{{ busy ? t('处理中…') : props.clip ? t('保存修改') : t('收入星码集') }} <kbd>{{ modifier }} S</kbd></button></footer>
  </dialog>
</template>

<style scoped>
.code-studio{width:min(1150px,calc(100vw - 48px));max-height:calc(100vh - 48px);box-sizing:border-box;border:1px solid var(--line);border-radius:18px;padding:27px;background:var(--panel);color:var(--text);box-shadow:var(--shadow);overflow:auto}.code-studio::backdrop{background:var(--backdrop);backdrop-filter:blur(7px)}.studio-heading{display:flex;align-items:flex-start;justify-content:space-between;margin-bottom:21px}.studio-eyebrow{font:9px/1.5 monospace;letter-spacing:2px;color:var(--muted)}.studio-heading h2{font-size:24px;margin:8px 0;font-weight:500}.studio-heading h2 span{color:var(--gold);margin-left:10px}.studio-heading p{font-size:11px;color:var(--muted);margin:0}.studio-close{font-size:25px!important;background:transparent!important;border:0!important;color:var(--muted)!important;padding:0 8px!important}.code-studio input:not([type=range]):not([type=checkbox]){box-sizing:border-box;border:1px solid var(--line);background:var(--input);color:var(--text);border-radius:7px;padding:9px;font-size:11px;min-width:0}.code-studio button{font-size:11px;background:var(--input);border:1px solid var(--line);color:var(--text);border-radius:7px;padding:9px 12px;cursor:pointer}.code-studio button:hover{background:var(--hover)}.code-studio button:disabled{opacity:.4;cursor:default}.code-studio :focus-visible{outline:2px solid var(--gold);outline-offset:3px}.studio-tabs{display:flex;align-items:center;gap:7px;border-bottom:1px solid var(--line);padding-bottom:15px;margin-bottom:18px}.studio-tabs button{background:transparent;border-color:transparent;color:var(--muted);display:flex;align-items:center;gap:7px}.studio-tabs button>span{font-size:15px;color:var(--muted)}.studio-tabs em{font-style:normal;font-size:9px;color:var(--muted)}.studio-tabs button.active{background:var(--gold-dim);border-color:var(--line);color:var(--gold)}.studio-tabs button.active>span{color:var(--gold)}.studio-tabs small{margin-left:auto;font-size:9px;color:var(--faint);letter-spacing:.4px}.studio-meta{display:grid;grid-template-columns:2fr 1fr 1fr;gap:14px;margin-bottom:17px}.studio-meta label,.studio-meta .studio-field{font-size:10px;color:var(--muted);display:flex;flex-direction:column;gap:7px}.studio-meta.handwriting-meta{grid-template-columns:1fr 1fr;align-items:center}.studio-meta p{color:var(--muted);font-size:10px;line-height:1.8;margin:10px 0 0}.studio-panels{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:16px}.studio-editor,.studio-preview{min-width:0;border:1px solid var(--line);border-radius:10px;overflow:hidden}.studio-panel-label{display:flex;align-items:center;justify-content:space-between;height:40px;padding:0 14px;font-size:10px;color:var(--muted);border-bottom:1px solid var(--line);gap:10px}.studio-panel-label button{font-size:10px;padding:3px 6px}.studio-panel-label span{font-family:monospace;color:var(--faint);font-size:9px;white-space:nowrap}.studio-editor{display:flex;flex-direction:column;background:var(--input)}.studio-editor textarea{display:block;resize:vertical;min-height:230px;height:290px;flex:1;width:100%;box-sizing:border-box;background:var(--input);color:var(--text);border:0;border-radius:0;padding:17px;font:12px/1.8 Menlo,Consolas,"PingFang SC",monospace;tab-size:2;white-space:pre;outline:none}.studio-editor textarea.handwriting-text{white-space:pre-wrap;font:14px/2.1 "PingFang SC","Microsoft YaHei",sans-serif;min-height:350px}.editor-note{font-size:9px;line-height:1.6;color:var(--faint);padding:9px 14px;border-top:1px solid var(--line);margin:0}.studio-preview{background:var(--input);display:flex;flex-direction:column}.canvas-preview{padding:17px;max-height:333px;min-height:230px;overflow:auto;flex:1;display:flex;align-items:flex-start;justify-content:center;background-image:radial-gradient(var(--line) .7px,transparent .7px);background-size:12px 12px}.canvas-preview img{display:block;width:100%;height:auto;box-shadow:var(--shadow)}.preview-placeholder{align-self:center;max-width:340px;display:flex;align-items:center;flex-direction:column;padding:18px;color:var(--muted);text-align:center;gap:15px}.preview-placeholder>span{font-size:28px;color:var(--gold)}.preview-placeholder p{font-size:11px;line-height:1.8}.preview-note{font-size:9px;color:var(--faint);padding:9px 14px;border-top:1px solid var(--line);margin:0}.handwriting-preview{min-width:0}.handwriting-workspace{align-items:start}.handwriting-workspace .studio-editor{position:sticky;top:0}.card-controls{border:1px solid var(--line);border-radius:10px;margin-top:17px;padding:14px 16px;background:var(--bg)}.control-heading{display:flex;align-items:center;justify-content:space-between;margin-bottom:14px}.control-heading h3{font-size:10px;font-weight:500;color:var(--gold)}.code-studio .reset-style{border:0;background:none;padding:0;font-size:9px;color:var(--gold)}.control-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:15px 18px}.control-grid>label,.control-grid>.control-field{display:flex;flex-wrap:wrap;align-items:center;font-size:9px;color:var(--muted);gap:7px;align-content:start}.control-grid>label>span{font-size:8px;color:var(--faint);margin-left:auto}.control-grid>.control-field>.ui-select,.control-grid>.control-field>.ui-color,.control-grid>label>input{width:100%}.control-grid input[type=range]:disabled{opacity:.35}.control-checks{display:flex;flex-direction:column;justify-content:center;gap:12px;min-height:50px}.control-checks label{display:flex;align-items:center;gap:7px;color:var(--muted);font-size:10px;cursor:pointer}.control-checks input{margin:0}.control-grid .watermark-control{grid-column:span 2}.studio-message{min-height:17px;padding-top:14px;font-size:10px;color:var(--muted);line-height:1.6}.studio-error{color:var(--danger)}.studio-footer{display:flex;justify-content:space-between;align-items:center;gap:10px;margin-top:14px}.studio-export{display:flex;gap:8px}.code-studio .studio-save{background:var(--primary);color:var(--on-accent);border-color:var(--primary);font-weight:600}.studio-save kbd{font:9px monospace;margin-left:10px;opacity:.6;border-color:var(--line);color:inherit}@media(max-width:900px){.code-studio{padding:22px;width:calc(100vw - 32px);max-height:calc(100vh - 32px)}.studio-panels{grid-template-columns:1fr}.studio-editor textarea{height:215px;min-height:180px}.studio-editor textarea.handwriting-text{height:230px;min-height:180px}.canvas-preview{max-height:420px;min-height:220px;padding:22px}.canvas-preview img{max-width:700px}.control-grid{grid-template-columns:repeat(3,minmax(0,1fr))}.handwriting-workspace .studio-editor{position:static}.studio-meta.handwriting-meta{grid-template-columns:1fr}.studio-meta.handwriting-meta p{margin:0;font-size:10px}.handwriting-preview{width:100%}}@media(max-width:600px){.code-studio{padding:17px;width:calc(100vw - 18px);max-height:calc(100vh - 18px);border-radius:12px}.studio-heading h2{font-size:22px}.studio-eyebrow{font-size:8px;letter-spacing:1px}.studio-meta{grid-template-columns:1fr 1fr;gap:12px}.studio-meta>label:first-child{grid-column:1/-1}.studio-tabs small{display:none}.studio-tabs button{padding:8px 10px;font-size:10px}.control-grid{grid-template-columns:repeat(2,minmax(0,1fr));gap:14px}.studio-footer{flex-wrap:wrap;justify-content:flex-end}.studio-export{flex:1;flex-wrap:wrap}.studio-export button{font-size:10px;padding:8px 9px}.studio-save kbd{display:none}.canvas-preview{padding:12px}.card-controls{padding:13px}.studio-tabs{margin-bottom:15px}}

.code-studio.is-handwriting{width:min(1180px,calc(100vw - 48px));padding:22px;max-height:calc(100dvh - 36px);overflow:auto}
.is-handwriting .studio-topbar{display:flex;align-items:center;gap:24px;border-bottom:1px solid var(--line);padding-bottom:12px;margin-bottom:14px}
.is-handwriting .studio-heading{margin:0;flex:0 0 auto}.is-handwriting .studio-heading h2{font-size:19px;margin:0}.is-handwriting .studio-eyebrow,.is-handwriting .studio-heading p{display:none}.is-handwriting .studio-close{position:absolute;right:18px;top:18px}
.is-handwriting .studio-tabs{border:0;margin:0;padding:0 32px 0 0;flex:1}.is-handwriting .studio-tabs button{padding:8px 12px}.is-handwriting .studio-tabs>small{margin-left:auto;font-size:10px}
.handwriting-title-field{display:flex;flex-direction:column;gap:6px;color:var(--muted);font-size:11px}.handwriting-title-field input{width:100%;box-sizing:border-box;height:34px}
.handwriting-editor{flex:1;min-height:170px}.handwriting-editor textarea.handwriting-text{height:100%;min-height:100px;resize:none;padding:13px;font-size:14px;line-height:1.9}.handwriting-editor .studio-panel-label{height:34px;flex-shrink:0}.handwriting-editor .editor-note{font-size:10px}
@media(max-width:800px){.code-studio.is-handwriting{padding:16px;width:calc(100vw - 24px)}.is-handwriting .studio-topbar{gap:12px;align-items:flex-start}.is-handwriting .studio-heading h2{font-size:17px}.is-handwriting .studio-tabs{flex-wrap:wrap;padding-right:20px;gap:4px}.is-handwriting .studio-tabs>small{display:none}.is-handwriting .studio-tabs button{padding:7px 9px}.handwriting-editor textarea.handwriting-text{height:210px;min-height:180px}}
</style>
