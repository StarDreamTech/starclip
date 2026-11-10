<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Clip } from '../types'
import { t } from '../i18n'
import ClipImage from './ClipImage.vue'
import Icon from './Icon.vue'

const props = defineProps<{ clip: Clip; initialPosition?: number }>()
const emit = defineEmits<{ close: [] }>()
const dialog = ref<HTMLDialogElement>()
const stage = ref<HTMLElement>()
const position = ref(props.initialPosition ?? 0)
const zoom = ref<number | null>(null)
const viewport = ref({ width: 0, height: 0 })
const dragging = ref(false)
const pictures = computed(() => props.clip.kind === 'bundle' ? props.clip.images ?? [] : props.clip.image ? [props.clip.image] : [])
const count = computed(() => pictures.value.length)
const picture = computed(() => pictures.value[position.value])
const fit = computed(() => Math.min(1, Math.max(1, viewport.value.width - 48) / (picture.value?.width || 1), Math.max(1, viewport.value.height - 48) / (picture.value?.height || 1)))
const scale = computed(() => zoom.value ?? fit.value)
const width = computed(() => Math.max(1, Math.round((picture.value?.width || 1) * scale.value)))
const height = computed(() => Math.max(1, Math.round((picture.value?.height || 1) * scale.value)))
let observer: ResizeObserver | undefined
let returnFocus: HTMLElement | null = null
let pointer: { id: number; x: number; y: number; left: number; top: number } | null = null

function measure() {
  if (stage.value) viewport.value = { width: stage.value.clientWidth, height: stage.value.clientHeight }
}
async function resize(value: number | null) {
  const element = stage.value
  const centerX = element ? (element.scrollLeft + element.clientWidth / 2) / Math.max(element.clientWidth, width.value + 48) : .5
  const centerY = element ? (element.scrollTop + element.clientHeight / 2) / Math.max(element.clientHeight, height.value + 48) : .5
  zoom.value = value === null ? null : Math.max(.01, Math.min(4, value))
  await nextTick()
  if (element) {
    element.scrollLeft = centerX * Math.max(element.clientWidth, width.value + 48) - element.clientWidth / 2
    element.scrollTop = centerY * Math.max(element.clientHeight, height.value + 48) - element.clientHeight / 2
  }
}
function changeZoom(factor: number) { void resize(scale.value * factor) }
function move(delta: number) {
  position.value = Math.max(0, Math.min(count.value - 1, position.value + delta))
}
watch([() => props.clip.id, count], () => { position.value = Math.max(0, Math.min(count.value - 1, props.initialPosition ?? 0)) })
watch(position, () => { zoom.value = null; stage.value?.scrollTo({ left: 0, top: 0 }) })
function keyboard(event: KeyboardEvent) {
  if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return
  if (event.key === 'Escape') emit('close')
  else if (event.key === 'ArrowLeft') move(-1)
  else if (event.key === 'ArrowRight') move(1)
  else if (event.key === '+' || event.key === '=') changeZoom(1.25)
  else if (event.key === '-') changeZoom(.8)
  else if (event.key === '0') void resize(null)
  else if (event.key === '1') void resize(1)
  else return
  event.preventDefault()
}
function wheel(event: WheelEvent) {
  if (!event.ctrlKey && !event.metaKey) return
  event.preventDefault()
  changeZoom(event.deltaY < 0 ? 1.1 : 1 / 1.1)
}
function startDrag(event: PointerEvent) {
  if (event.button !== 0 || !stage.value || (width.value + 48 <= viewport.value.width && height.value + 48 <= viewport.value.height)) return
  event.preventDefault()
  pointer = { id: event.pointerId, x: event.clientX, y: event.clientY, left: stage.value.scrollLeft, top: stage.value.scrollTop }
  stage.value.setPointerCapture(event.pointerId)
  dragging.value = true
}
function drag(event: PointerEvent) {
  if (!pointer || pointer.id !== event.pointerId || !stage.value) return
  stage.value.scrollLeft = pointer.left + pointer.x - event.clientX
  stage.value.scrollTop = pointer.top + pointer.y - event.clientY
}
function endDrag() { pointer = null; dragging.value = false }
onMounted(() => {
  returnFocus = document.activeElement as HTMLElement | null
  dialog.value?.showModal()
  measure()
  if (typeof ResizeObserver !== 'undefined' && stage.value) { observer = new ResizeObserver(measure); observer.observe(stage.value) }
  window.addEventListener('resize', measure)
})
onBeforeUnmount(() => {
  observer?.disconnect(); window.removeEventListener('resize', measure)
  dialog.value?.close()
  if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true })
})
</script>

<template>
  <dialog ref="dialog" class="image-viewer" :aria-label="t('图片全屏查看')" @cancel.prevent="emit('close')" @keydown.stop="keyboard">
    <div class="image-viewer-shell">
      <header class="image-viewer-toolbar">
        <div class="image-viewer-title"><Icon name="image" :size="18" /><strong>{{ clip.title || t('图片预览') }}</strong><span v-if="count > 1" aria-live="polite">{{ position + 1 }} / {{ count }}</span></div>
        <div class="image-viewer-zoom" :aria-label="t('图片缩放')">
          <button type="button" :aria-label="t('缩小图片')" :title="t('缩小图片')" :disabled="scale <= .01" @click="changeZoom(.8)"><Icon name="minus" :size="17" /></button>
          <output aria-live="polite">{{ Math.round(scale * 100) }}%</output>
          <button type="button" :aria-label="t('放大图片')" :title="t('放大图片')" :disabled="scale >= 4" @click="changeZoom(1.25)"><Icon name="plus" :size="17" /></button>
          <button type="button" :aria-pressed="zoom === null" @click="resize(null)">{{ t('适应窗口') }}</button>
          <button type="button" :aria-pressed="zoom === 1" @click="resize(1)">{{ t('原始大小') }}</button>
        </div>
        <button type="button" class="image-viewer-close" :aria-label="t('关闭图片查看')" :title="t('关闭图片查看')" autofocus @click="emit('close')"><Icon name="close" :size="21" /></button>
      </header>
      <div ref="stage" :class="['image-viewer-stage', { dragging }]" @wheel="wheel" @pointerdown="startDrag" @pointermove="drag" @pointerup="endDrag" @pointercancel="endDrag" @lostpointercapture="endDrag">
        <div class="image-viewer-canvas" :style="{ width: `${width + 48}px`, height: `${height + 48}px` }">
          <ClipImage :id="clip.id" class="image-viewer-picture" :position="position" :thumbnail="false" :alt="t('第 {index} 张图片', { index: position + 1 })" :style="{ width: `${width}px`, height: `${height}px` }" />
        </div>
      </div>
      <footer class="image-viewer-footer">
        <span>{{ picture ? `${picture.width} × ${picture.height}` : '' }}</span>
        <div v-if="count > 1" class="image-viewer-navigation"><button type="button" :disabled="position === 0" @click="move(-1)"><Icon name="chevron" class="image-viewer-previous" :size="15" />{{ t('上一张') }}</button><span>{{ position + 1 }} / {{ count }}</span><button type="button" :disabled="position + 1 >= count" @click="move(1)">{{ t('下一张') }}<Icon name="chevron" :size="15" /></button></div>
        <span class="image-viewer-hint">{{ t('滚动查看 · 拖动平移 · Ctrl/⌘ + 滚轮缩放 · Esc 关闭') }}</span>
      </footer>
    </div>
  </dialog>
</template>

<style scoped>
.image-viewer{position:fixed;inset:0;width:100vw;height:100vh;max-width:none;max-height:none;margin:0;padding:0;border:0;border-radius:0;background:var(--bg);color:var(--text)}
.image-viewer::backdrop{background:var(--backdrop)}
.image-viewer-shell{height:100%;display:flex;flex-direction:column}
.image-viewer-toolbar,.image-viewer-footer{display:flex;align-items:center;gap:12px;padding:12px 18px;background:var(--panel);flex-shrink:0}
.image-viewer-toolbar{border-bottom:1px solid var(--line);flex-wrap:wrap}
.image-viewer-title{display:flex;align-items:center;gap:10px;min-width:100px;flex:1}
.image-viewer-title strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:40vw;font-size:14px}
.image-viewer-title span{color:var(--muted);white-space:nowrap;font-size:12px}
.image-viewer-zoom{display:flex;align-items:center;gap:5px}
.image-viewer button{min-height:34px;min-width:34px;border:1px solid var(--line);border-radius:7px;background:var(--input);color:var(--text);padding:6px 10px}
.image-viewer button:hover:enabled{background:var(--hover);border-color:var(--gold)}
.image-viewer button[aria-pressed=true]{color:var(--gold);background:var(--gold-dim);border-color:var(--gold)}
.image-viewer button:disabled{opacity:.4;cursor:default}
.image-viewer-zoom output{width:52px;text-align:center;font-size:12px;font-variant-numeric:tabular-nums}
.image-viewer-close{margin-left:auto}
.image-viewer-stage{min-height:0;flex:1;overflow:auto;overscroll-behavior:contain;cursor:grab;user-select:none;touch-action:none}
.image-viewer-stage.dragging{cursor:grabbing}
.image-viewer-canvas{min-width:100%;min-height:100%;display:grid;place-items:center;padding:24px}
.image-viewer .image-viewer-picture{min-height:0;border-radius:2px;max-width:none}
.image-viewer-footer{justify-content:space-between;font-size:11px;color:var(--muted);border-top:1px solid var(--line)}
.image-viewer-navigation{display:flex;align-items:center;gap:12px}
.image-viewer-navigation button{display:flex;align-items:center;gap:6px;font-size:12px}
.image-viewer-previous{transform:rotate(180deg)}
.image-viewer-hint{text-align:right}
@media(max-width:760px){.image-viewer-toolbar{gap:7px;padding:10px}.image-viewer-title strong{max-width:180px}.image-viewer-zoom{order:3;width:100%;justify-content:center}.image-viewer-hint{display:none}.image-viewer-footer{padding:10px}}
</style>
