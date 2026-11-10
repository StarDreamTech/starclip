<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { toPng } from 'html-to-image';
import { api } from '../api';
import { safeFilename } from '../snippets';
import { t, errorText } from '../i18n';
import { renderMarkdown, checkMarkdownImageSize } from '../markdown-render';
import themeLicense from '../markdown-themes/LICENSE?raw';
import light from '../markdown-themes/drake-light.css?raw';
import purple from '../markdown-themes/drake-purple.css?raw';
import dark from '../markdown-themes/drake-dark.css?raw';
const props = defineProps<{ text: string; title: string }>();
const emit = defineEmits<{ 'update:text': [value: string]; busy: [value: boolean] }>();
const theme = ref('light');
const width = ref(720);
const size = ref(18);
const host = ref<HTMLDivElement>();
const busy = ref(false);
const error = ref('');
const notice = ref('');
const styles: Record<string, string> = { light, purple, dark };
const html = computed(() => { try { return renderMarkdown(props.text); } catch { return ''; } });
let article: HTMLElement | undefined;
function refresh() {
  if (!host.value) return;
  const root = host.value.shadowRoot ?? host.value.attachShadow({ mode: 'open' });
  root.replaceChildren();
  const style = document.createElement('style');
  style.textContent = styles[theme.value] + `
#write { box-sizing:border-box; width:${width.value}px; max-width:none; padding:40px; margin:0; font:${size.value}px/1.8 system-ui,sans-serif; color:var(--text-color); background:var(--bg-color); overflow-wrap:anywhere; }
#write * { max-height:none; } #write pre { white-space:pre-wrap; overflow-wrap:anywhere; background:var(--code-block-bg-color); color:var(--code-block-color); padding:16px; } #write code { font-family:monospace; } #write pre code {color:inherit;background:transparent;} #write table {width:100%; table-layout:fixed;} #write td,#write th {overflow-wrap:anywhere;} #write .md-link {color:var(--a-color);text-decoration:underline;}`;
  article = document.createElement('article'); article.id = 'write';
  const heading = document.createElement('h1'); heading.textContent = props.title;
  const body = document.createElement('div'); body.innerHTML = html.value;
  if (props.title.trim()) article.append(heading);
  article.append(body); root.append(style, article);
  error.value = ''; notice.value = '';
  try { renderMarkdown(props.text); checkMarkdownImageSize(width.value, article.scrollHeight); }
  catch (e) { error.value = errorText(e); }
}
watch([html, () => props.title, theme, width, size], () => nextTick(refresh));
onMounted(refresh);
async function exportImage() {
  if (busy.value || !props.text.trim()) return;
  busy.value = true; emit('busy', true); notice.value = '';
  try {
    await nextTick(); refresh();
    if (error.value || !article) return;
    const height = article.scrollHeight;
    checkMarkdownImageSize(width.value, height);
    const url = await toPng(article, { pixelRatio: 2, width: width.value, height, skipFonts: true });
    if (await api.saveExport(url.split(',')[1]!, 'png', `${safeFilename(props.title)}.png`)) notice.value = t('Markdown 图片已导出');
  } catch (e) { error.value = errorText(e); }
  finally { busy.value = false; emit('busy', false); }
}
</script>
<template>
  <section class="markdown-panel">
    <div class="controls">
      <div class="theme-options" role="group" :aria-label="t('文章主题')"><button v-for="choice in (['light', 'purple', 'dark'] as const)" :key="choice" :class="['theme-choice', choice]" :aria-pressed="theme === choice" :disabled="busy" @click="theme = choice">{{ t(choice === 'light' ? '素纸' : choice === 'purple' ? '星紫' : '深夜') }}</button></div>
      <label>{{ t('图片宽度') }} <select v-model.number="width" :disabled="busy"><option :value="540">540 px</option><option :value="720">720 px</option><option :value="960">960 px</option></select></label>
      <label>{{ t('字号') }} <input v-model.number="size" type="range" min="14" max="28" :disabled="busy" /> {{ size }}</label>
    <button class="export-image" :disabled="busy || !!error || !text.trim()" @click="exportImage">{{ busy ? t('处理中…') : t('导出 Markdown 图片') }}</button>
    </div>
    <div class="panels">
      <textarea :value="text" :disabled="busy" :aria-label="t('Markdown 正文')" spellcheck="false" @input="emit('update:text', ($event.target as HTMLTextAreaElement).value)" />
      <div class="preview" :aria-label="t('导出效果预览')"><div ref="host" /></div>
    </div>
    <p class="hint">{{ t('本地排版 · 支持标题、列表、引用、表格与代码块；图片显示替代文字，HTML 按原文显示。') }}</p>
    <p role="status">{{ error || notice }}</p>

    <p class="hint">{{ t('主题来源：Drake / MIT · 已适配星笺') }} · {{ theme === 'light' ? 'drake-light' : theme === 'purple' ? 'drake-purple' : 'drake-dark' }}</p>
    <details class="hint"><summary>{{ t('主题来源与许可') }}</summary><p>https://github.com/liangjingkanji/DrakeTyporaTheme</p><pre style="white-space:pre-wrap">{{ themeLicense }}</pre></details>
  </section>
</template>
<style scoped>
.theme-options{display:flex;gap:8px}.theme-choice{border:2px solid transparent}.theme-choice.light{background:#fafafa;color:#333}.theme-choice.purple{background:#f3eafa;color:#7952b3}.theme-choice.dark{background:#292929;color:#bdcbd8}.theme-choice[aria-pressed=true]{border-color:var(--gold);box-shadow:0 0 0 1px var(--gold)}.controls{display:flex;flex-wrap:wrap;gap:16px;align-items:center}.controls{margin-bottom:12px}.controls .export-image{margin-left:auto}.controls input[type=range]{width:100px}.controls label{white-space:nowrap;display:flex;gap:8px;align-items:center;font-size:12px}.controls select{background:var(--input);color:var(--text);padding:8px;border:1px solid var(--line);border-radius:6px}.panels{display:grid;grid-template-columns:minmax(200px,1fr) minmax(0,1.5fr);gap:16px}.panels textarea{box-sizing:border-box;width:100%;min-height:300px;resize:vertical;background:var(--input);color:var(--text);padding:16px;border:1px solid var(--line);border-radius:8px;font:14px/1.8 monospace}.preview{overflow:auto;max-height:400px;border:1px solid var(--line);border-radius:8px;background:var(--bg)}.hint{font-size:11px;color:var(--muted);line-height:1.7}button{padding:10px 16px;background:var(--primary);color:var(--on-accent);border:0;border-radius:8px}button:disabled{opacity:.5}@media(max-width:800px){.panels{grid-template-columns:1fr}}
</style>
