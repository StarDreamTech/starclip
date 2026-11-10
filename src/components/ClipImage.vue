<script setup lang="ts">
import { errorText, t } from '../i18n';
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { api } from '../api';
import Icon from './Icon.vue';

const props = withDefaults(defineProps<{ id: number; alt: string; thumbnail?: boolean; position?: number }>(), { thumbnail: true, position: 0 });
const root = ref<HTMLElement>();
const source = ref('');
const error = ref('');
const visible = ref(false);
const loading = ref(false);
let observer: IntersectionObserver | undefined;
let generation = 0;
async function load() {
  const request = ++generation;
  source.value = ''; error.value = ''; loading.value = visible.value;
  if (!visible.value) return;
  try {
    const data = await api.imageData(props.id, props.thumbnail, props.position);
    if (request !== generation) return;
    if (!data.startsWith('data:image/png;base64,')) throw new Error(t('图片格式不可用'));
    source.value = data;
  } catch (reason) { if (request === generation) error.value = errorText(reason); }
  finally { if (request === generation) loading.value = false; }
}
function invalidImage() { source.value = ''; error.value = t('图片无法显示，请重试。'); }
watch([() => props.id, () => props.thumbnail, () => props.position, visible], load);
onMounted(() => {
  if (typeof IntersectionObserver === 'undefined') { visible.value = true; return; }
  observer = new IntersectionObserver(entries => { visible.value = entries.some(entry => entry.isIntersecting); });
  if (root.value) observer.observe(root.value);
});
onBeforeUnmount(() => { generation++; observer?.disconnect(); });
</script>

<template>
  <div ref="root" :class="['clip-image', { 'clip-image-detail': !thumbnail }]" :aria-busy="loading">
    <img v-if="source" :src="source" :alt="alt" decoding="async" draggable="false" @error="invalidImage" />
    <div v-else class="clip-image-placeholder" :role="error ? 'status' : undefined">
      <Icon :name="error ? 'alert' : 'image'" :size="thumbnail ? 21 : 32" />
      <span>{{ error ? t('图片暂不可用') : loading ? t('正在读取图片…') : t('图片预览') }}</span>
      <span v-if="error && !thumbnail" class="clip-image-error">{{ error }}</span>
      <button v-if="error && !thumbnail" type="button" class="clip-image-retry" :title="error" @click.stop.prevent="load" @dblclick.stop>{{ t('重试') }}</button>
    </div>
  </div>
</template>

<style scoped>
.clip-image{width:100%;height:100px;overflow:hidden;border:1px solid var(--line-soft);border-radius:6px;background-color:var(--input);background-image:linear-gradient(45deg,var(--line-soft) 25%,transparent 25%),linear-gradient(-45deg,var(--line-soft) 25%,transparent 25%),linear-gradient(45deg,transparent 75%,var(--line-soft) 75%),linear-gradient(-45deg,transparent 75%,var(--line-soft) 75%);background-size:16px 16px;background-position:0 0,0 8px,8px -8px,-8px 0;display:grid;place-items:center;min-width:0;flex-shrink:0}
.clip-image img{display:block;width:100%;height:100%;min-height:0;object-fit:contain}.clip-image-placeholder{display:flex;align-items:center;justify-content:center;gap:7px;flex-wrap:wrap;padding:10px;font-size:10px;color:var(--muted);text-align:center;max-width:100%}.clip-image-placeholder>svg{color:var(--gold);flex-shrink:0}.clip-image-retry{border:1px solid var(--line);border-radius:5px;background:var(--panel);color:var(--text);padding:3px 8px;font-size:10px;cursor:pointer}.clip-image-retry:focus-visible{outline:2px solid var(--gold);outline-offset:2px}.clip-image-detail{height:min(240px,30vh);min-height:120px}.clip-image-detail .clip-image-placeholder{flex-direction:column;gap:10px;font-size:12px}.clip-image-error{font-size:10px;max-width:320px;overflow-wrap:anywhere;line-height:1.7}
</style>
