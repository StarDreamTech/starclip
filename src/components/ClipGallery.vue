<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { Clip } from '../types'
import { t } from '../i18n'
import ClipImage from './ClipImage.vue'
import Icon from './Icon.vue'

const props = defineProps<{ clip: Clip; detail?: boolean }>()
const emit = defineEmits<{ view: [position: number] }>()
const position = ref(0)
const count = computed(() => props.clip.images?.length ?? 0)
const displayed = computed(() => props.detail ? count.value : Math.min(9, count.value))
watch(() => props.clip.id, () => { position.value = 0 })
</script>

<template>
  <div class="clip-gallery">
    <template v-if="detail && count">
      <div class="gallery-preview"><ClipImage :id="clip.id" :position="position" :thumbnail="false" :alt="t('第 {index} 张图片', { index: position + 1 })" /><button type="button" class="image-expand" :aria-label="t('全屏查看图片')" @click="emit('view', position)"><Icon name="expand" :size="16" />{{ t('全屏查看') }}</button></div>
      <div class="gallery-navigation">
        <button type="button" :disabled="position === 0" @click="position--">{{ t('上一张') }}</button>
        <span>{{ position + 1 }} / {{ count }}</span>
        <button type="button" :disabled="position + 1 >= count" @click="position++">{{ t('下一张') }}</button>
      </div>
    </template>
    <div :class="['gallery-grid', { 'gallery-detail': detail, 'gallery-pair': count === 2 || count === 4 }]">
      <component :is="detail ? 'button' : 'div'" v-for="index in displayed" :key="index" class="gallery-cell" :type="detail ? 'button' : undefined" :aria-label="detail ? t('第 {index} 张图片', { index }) : undefined" :aria-pressed="detail ? position === index - 1 : undefined" @click="detail && (position = index - 1)">
        <ClipImage :id="clip.id" :position="index - 1" :alt="t('第 {index} 张图片', { index })" />
        <span v-if="!detail && index === 9 && count > 9" class="gallery-more">+{{ count - 9 }}</span>
      </component>
    </div>
    <p class="gallery-count">{{ t('{count} 张图片', { count }) }}</p>
  </div>
</template>

<style scoped>
.gallery-preview{position:relative}.image-expand{position:absolute;right:10px;bottom:10px;display:flex;align-items:center;gap:6px;padding:7px 10px;border:1px solid var(--line);border-radius:7px;background:var(--panel);color:var(--text);box-shadow:0 2px 10px #0002;font-size:11px}.image-expand:hover{border-color:var(--gold);color:var(--gold)}
.clip-gallery{min-width:0;width:100%}.gallery-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:4px;max-width:320px}.gallery-pair{grid-template-columns:repeat(2,minmax(0,1fr))}.gallery-cell{position:relative;min-width:0;border:0;padding:0;background:none;border-radius:5px;overflow:hidden;aspect-ratio:1}.gallery-cell :deep(.clip-image){height:100%;border:0;border-radius:0}.gallery-cell :deep(img){object-fit:cover}.gallery-cell :deep(.clip-image-placeholder){font-size:8px;padding:3px}.gallery-more{position:absolute;inset:0;display:grid;place-items:center;background:#1119;color:white;font-size:22px}.gallery-count{font-size:10px;color:var(--muted);margin:6px 0 0}.gallery-detail{grid-template-columns:repeat(6,minmax(0,1fr));max-width:none;max-height:145px;overflow:auto}.gallery-detail button{cursor:pointer;border:2px solid transparent}.gallery-detail button[aria-pressed=true]{border-color:var(--gold)}.gallery-detail button:focus-visible{outline:2px solid var(--gold);outline-offset:-3px}.gallery-navigation{display:flex;align-items:center;justify-content:center;gap:15px;margin:8px 0;color:var(--muted);font-size:11px}.gallery-navigation button{background:var(--input);color:var(--text);border:1px solid var(--line);border-radius:5px;padding:5px 10px;cursor:pointer}.gallery-navigation button:disabled{opacity:.4;cursor:default}
</style>
