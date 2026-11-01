<script setup lang="ts">
import { errorText, t } from '../i18n';
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { Clip } from '../types'
import { api } from '../api'
import { normalizeTags, originHost, pendingClip, privacyReasonExplanation, protectedClip, sourceNames } from '../privacy'
const props = defineProps<{ clip: Clip; revision: number; disabled?: boolean }>()
const emit = defineEmits<{ busy: [value: boolean]; changed: [] }>()
const tags = ref(''), busy = ref(false), reviewed = ref(false), error = ref(''), status = ref('')
const sourcesLabel = computed(() => (props.clip.linkSources ?? []).map(source => t(sourceNames[source] ?? source)).join(t('、')) || t('其他网站'))
const manualTags = computed(() => props.clip.manualTags ?? props.clip.tags ?? [])
const appTags = computed(() => props.clip.appTags ?? [])
const automaticTags = computed(() => (props.clip.tags ?? []).filter(tag => !manualTags.value.includes(tag) && !appTags.value.includes(tag)))
const isPrivate = computed(() => protectedClip(props.clip))
let disposed = false
watch(() => [props.clip.id, props.clip.tags], () => { tags.value = manualTags.value.join(t('，')); reviewed.value = false }, { immediate: true })
async function run(action: () => Promise<void>, message: string) {
  if (busy.value || props.disabled) return
  busy.value = true; emit('busy', true); error.value = ''; status.value = ''
  try { await action(); if (!disposed) { status.value = message; reviewed.value = false; emit('changed') } }
  catch (cause) { if (!disposed) error.value = errorText(cause) }
  finally { if (!disposed) { busy.value = false; emit('busy', false) } }
}
function saveTags() { void run(() => api.setClipTags(props.clip.id, normalizeTags(tags.value)), t('标签已保存，不会改变 AI 授权。')) }
function protect() { void run(() => api.setClipPrivacy(props.clip.id, true), t('已标记为隐私。')) }
function release() { void run(() => api.setClipPrivacy(props.clip.id, false, true), t('已完成复核。仍命中有效规则的内容会继续受保护。')) }
onBeforeUnmount(() => { disposed = true; emit('busy', false) })
</script>
<template>
  <fieldset class="clip-privacy" :disabled="busy || disabled">
    <h3>{{ t('标签、来源与隐私') }}</h3>
    <p v-if="clip.linkHosts?.length">{{ t('来源：') }}{{ sourcesLabel }}<br /><small>{{ clip.linkHosts.join(' · ') }}</small></p>
    <p v-if="appTags.length" class="clip-origin">{{ t('复制自：') }}{{ appTags.join(t('、')) }}</p>
    <p v-if="clip.originUrl" class="clip-origin">{{ t('来源页面：') }}<small :title="clip.originUrl">{{ originHost(clip) || clip.originUrl }}</small><br /><small class="clip-origin-url">{{ clip.originUrl }}</small></p>
    <label class="privacy-tags-label">{{ t('普通标签') }}<input @keydown.enter.prevent v-model="tags" :aria-label="t('普通标签')" :placeholder="t('用逗号分隔，例如 工作，设计')" maxlength="1340" /></label>
    <div class="privacy-buttons"><button type="button" class="button subtle" :disabled="JSON.stringify(normalizeTags(tags)) === JSON.stringify(manualTags)" @click="saveTags">{{ t('保存标签') }}</button></div>
    <p v-if="automaticTags.length" class="field-note">{{ t('自动标签：') }}{{ automaticTags.join('、') }}{{ t('（修改规则后重扫可更新）') }}</p>
    <div class="privacy-state"><strong>{{ isPrivate ? t('隐私 · 不向 AI 提供') : clip.kind === 'image' ? t('图片未识别文字 · 不向 AI 提供') : pendingClip(clip, revision) ? t('待扫描 · 不向 AI 提供') : t('普通记录 · 需单独授权才供 AI 读取') }}</strong><p v-if="clip.privacyManual">{{ t('手动标记为隐私') }}</p><div v-if="clip.privacyReasons?.length" class="privacy-match-reasons"><strong>{{ t('保护依据与命中规则') }}</strong><ul><li v-for="reason in clip.privacyReasons" :key="reason"><strong>{{ reason.startsWith('自定义规则：') ? t('自定义规则：{name}', { name: reason.slice(6) }) : t(reason) }}</strong><p>{{ t(privacyReasonExplanation(reason)) }}</p></li></ul></div><p v-if="clip.kind === 'image' && !isPrivate" class="field-note">{{ t('图片未做文字识别，不会因为图片像素自动判为隐私；如需保护，可手动标记。') }}</p></div>
    <button v-if="!isPrivate" type="button" class="button subtle" @click="protect">{{ t('标记为隐私') }}</button>
    <template v-else><label class="privacy-review"><input v-model="reviewed" type="checkbox" />{{ t('我已在本机复核，申请解除保护') }}</label><button type="button" class="button subtle" :disabled="!reviewed" @click="release">{{ t('复核并尝试解除') }}</button></template>
    <p class="field-note">{{ t('隐私状态独立即时保存；普通标签不会授予 AI 权限。完整备份包含隐私数据，折叠显示不是加密。') }}</p>
    <p v-if="error" class="privacy-error" role="alert">{{ error }}</p><p v-if="status" class="privacy-status" role="status">{{ status }}</p>
  </fieldset>
</template>
