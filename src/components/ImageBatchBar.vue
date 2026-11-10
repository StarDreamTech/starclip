<script setup lang="ts">
import type { Clip } from '../types'
import { ref, watch } from 'vue'
import { api, isDesktop } from '../api'
import { protectedClip } from '../privacy'
import { errorText, t } from '../i18n'

const props = defineProps<{ ids: number[]; disabled: boolean; pageIds: number[] }>()
const emit = defineEmits<{ clear: []; done: []; selectPage: []; busy: [value: boolean]; created: [clip: Clip] }>()
const working = ref(false), message = ref(''), failed = ref(false)
const needsConsent = ref(false), consent = ref(false), token = ref('')
const caption = ref(''), title = ref('')
let generation = 0
watch(() => props.ids, () => { ++generation; token.value = ''; message.value = ''; needsConsent.value = false; consent.value = false }, { flush: 'sync' })
watch(consent, () => { token.value = '' })
async function act(action: 'copy' | 'export' | 'prepare' | 'collect') {
  if (working.value || props.disabled || !props.ids.length) return
  const request = generation, ids = [...props.ids]
  working.value = true; emit('busy', true); message.value = ''; failed.value = false
  try {
    if (!isDesktop) throw new Error(t('批量图片传出请使用桌面版。'))
    const clips = await api.getClips(ids)
    if (request !== generation) return
    if (clips.length !== ids.length || clips.some(c => !['image','bundle'].includes(c.kind))) throw new Error(t('所选图片已变化，请清空后重新选择。'))
    needsConsent.value = clips.some(protectedClip)
    if (needsConsent.value && !consent.value) return
    const allowPrivate = needsConsent.value && consent.value
    if (action === 'collect') {
      const clip = await api.createBundle(ids, caption.value, title.value, allowPrivate)
      if (request === generation) emit('created', clip)
    } else if (action === 'prepare') {
      const prepared = await api.prepareImageTransfer(ids, allowPrivate)
      if (request === generation) { token.value = prepared; message.value = t('已准备好，按住「拖出图片」移到桌面或文件夹。') }
    } else if (action === 'copy') {
      await api.copyImageFiles(ids, allowPrivate)
      message.value = t('已复制为独立 PNG 文件，可粘贴到文件夹；其他应用的接收方式由其自身决定。')
    } else {
      const path = await api.exportImageFiles(ids, allowPrivate)
      message.value = path ? t('图片已导出到：{path}', { path }) : t('已取消导出。')
    }
  } catch (error) { if (request === generation) { message.value = errorText(error); failed.value = true; token.value = '' } }
  finally { working.value = false; emit('busy', false) }
}
async function drag(event: MouseEvent) {
  if (event.button !== 0 || !token.value || working.value || props.disabled) return
  event.preventDefault()
  working.value = true; emit('busy', true); failed.value = false
  try { message.value = await api.dragImageFiles(token.value) ? t('拖出已交给目标应用，请检查文件是否已保存。') : t('已取消拖出。') }
  catch (error) { message.value = errorText(error); failed.value = true; token.value = '' }
  finally { working.value = false; emit('busy', false) }
}
</script>

<template>
  <section class="image-batch-bar" :aria-label="t('批量图片操作')" :aria-busy="working">
    <div class="image-batch-actions">
      <strong>{{ t('已选 {count} 条图片记录', { count: ids.length }) }}</strong>
      <button type="button" class="button subtle" :disabled="working || disabled || !pageIds.length" @click="emit('selectPage')">{{ t('全选本页图片') }}</button>
      <button type="button" class="button subtle" :disabled="working || disabled || !ids.length" @click="emit('clear')">{{ t('清空选择') }}</button>
      <button type="button" class="button subtle" :disabled="working || disabled || !ids.length" @click="act('copy')">{{ t('复制为图片文件') }}</button>
      <button type="button" class="button subtle" :disabled="working || disabled || !ids.length" @click="act('export')">{{ t('导出到文件夹') }}</button>
      <button v-if="!token" type="button" class="button primary" :disabled="working || disabled || !ids.length" @click="act('prepare')">{{ t('准备拖出') }}</button>
      <button v-else type="button" class="button primary image-drag-handle" :disabled="working || disabled" @mousedown="drag" @dragstart.prevent @click.prevent>{{ t('拖出图片') }}</button>
      <button type="button" class="button subtle" :disabled="working || disabled" @click="emit('done')">{{ t('完成') }}</button>
    </div>
    <details class="bundle-collect"><summary>{{ t('收藏为一条图文') }}</summary><label>{{ t('片段名称') }}<input :value="title" @input="title = ($event.target as HTMLInputElement).value" maxlength="200" :disabled="working || disabled" /></label><label>{{ t('配文') }}<textarea :value="caption" @input="caption = ($event.target as HTMLTextAreaElement).value" rows="3" maxlength="262144" :placeholder="t('可选：给这组图片写点文字')" :disabled="working || disabled" /></label><button type="button" class="button primary" :disabled="working || disabled || !ids.length" @click="act('collect')">{{ t('保存图集并星标收藏') }}</button><p>{{ t('图片按选择顺序组成一条记录，保留原记录；隐私保护随图集保留。') }}</p></details>
    <p>{{ t('最多 50 张、合计 100 MiB；保留独立 PNG 原图。Shift 点选可连选，跨页保留选择。') }}</p>
    <p>{{ t('复制和拖出会生成本地临时副本，24 小时后在下次操作时清理；长期保留请导出。') }}</p>
    <label v-if="needsConsent" class="image-batch-consent"><input type="checkbox" :checked="consent" :disabled="working" @change="consent = ($event.target as HTMLInputElement).checked" />{{ t('所选包含隐私图片。我确认组合或传出这些图片。') }}</label>
    <p v-if="working" role="status">{{ t('正在处理图片，请稍候…') }}</p>
    <p v-else-if="message" :class="{ 'error-message': failed }" role="status">{{ message }}</p>
  </section>
</template>

<style scoped>
.bundle-collect{margin:10px 0}.bundle-collect summary{color:var(--gold);cursor:pointer;padding:4px 0}.bundle-collect label{display:block;max-width:600px;margin:8px 0;color:var(--muted);font-size:11px}.bundle-collect input,.bundle-collect textarea{display:block;width:100%;box-sizing:border-box;background:var(--input);color:var(--text);border:1px solid var(--line);border-radius:5px;padding:8px;margin-top:5px;resize:vertical}
</style>
