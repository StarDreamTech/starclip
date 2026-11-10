<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { formatNumber, t } from '../i18n'
import type { HistorySummary } from '../types'
import Icon from './Icon.vue'
const props = defineProps<{ history: HistorySummary; disabled?: boolean }>()
const emit = defineEmits<{ page: [value: number] }>()
const pages = computed(() => Math.max(1, Math.ceil(props.history.total / props.history.pageSize)))
const jump = ref('')
const jumpPage = computed(() => /^[1-9]\d*$/.test(jump.value) ? Number(jump.value) : 0)
const validJump = computed(() => Number.isSafeInteger(jumpPage.value) && jumpPage.value >= 1 && jumpPage.value <= pages.value)
const buttons = computed(() => {
  if (pages.value <= 7) return Array.from({ length: pages.value }, (_, index) => index + 1)
  const start = Math.max(2, Math.min(props.history.page - 1, pages.value - 3))
  const end = start + 2
  const result: Array<number | 'previous' | 'next'> = [1]
  if (start > 2) result.push(start === 3 ? 2 : 'previous')
  for (let page = start; page <= end; page++) result.push(page)
  if (end < pages.value - 1) result.push(end === pages.value - 2 ? pages.value - 1 : 'next')
  result.push(pages.value)
  return result
})
watch([() => props.history.page, pages], () => { jump.value = '' })
function go(page: number) {
  if (props.disabled || !Number.isInteger(page) || page < 1 || page > pages.value || page === props.history.page) return
  emit('page', page)
}
</script>
<template>
  <nav class="history-pagination" :aria-label="t('历史分页')">
    <div><span>{{ t('第 {page} / {pages} 页 · 共 {count} 条', { page: formatNumber(history.page), pages: formatNumber(pages), count: formatNumber(history.total) }) }}</span><small>{{ t('每页 {count} 条，只加载当前页', { count: history.pageSize }) }}</small></div>
    <div class="history-page-controls">
      <div class="history-page-buttons">
        <button type="button" class="history-page-arrow" :aria-label="t('上一页')" :title="t('上一页')" :disabled="disabled || history.page <= 1" @click="go(history.page - 1)"><Icon name="chevron" :size="14" class="previous-icon" /></button>
        <button v-for="item in buttons" :key="item" type="button" :class="['history-page-number', { current: item === history.page }]" :disabled="disabled" :aria-current="item === history.page ? 'page' : undefined" :aria-label="typeof item === 'number' ? t('第 {page} 页', { page: item }) : item === 'previous' ? t('往前跳 5 页') : t('往后跳 5 页')" @click="go(typeof item === 'number' ? item : Math.min(pages, Math.max(1, history.page + (item === 'previous' ? -5 : 5))))">{{ typeof item === 'number' ? formatNumber(item) : '…' }}</button>
        <button type="button" class="history-page-arrow" :aria-label="t('下一页')" :title="t('下一页')" :disabled="disabled || history.page >= pages" @click="go(history.page + 1)"><Icon name="chevron" :size="14" /></button>
      </div>
      <label class="history-page-jump">{{ t('前往') }}<input v-model="jump" type="text" inputmode="numeric" pattern="[0-9]*" maxlength="10" :placeholder="String(history.page)" :aria-label="t('跳转页码')" :title="t('请输入 1 至 {pages} 的页码', { pages })" :aria-invalid="!!jump && !validJump" :disabled="disabled || pages <= 1" @keydown.enter.stop.prevent="validJump && go(jumpPage)" />{{ t('页') }}</label>
      <button type="button" class="history-page-submit" :disabled="disabled || !validJump || jumpPage === history.page" @click="go(jumpPage)">{{ t('跳转') }}</button>
    </div>
  </nav>
</template>
<style scoped>
.history-pagination { display: flex; align-items: center; justify-content: space-between; flex-wrap:wrap; gap: 12px; margin: 16px 0; color: var(--muted); font-size: 12px; }
.history-pagination small { display: block; margin-top: 4px; font-size: 11px; }
.history-page-controls,.history-page-buttons,.history-page-jump { display:flex;align-items:center;gap:5px; }
.history-page-controls { flex-wrap:wrap;gap:8px; }
.history-page-buttons button,.history-page-submit { min-width:32px;min-height:32px;padding:5px 8px;border:1px solid transparent;border-radius:7px;background:var(--input);color:var(--muted);font-variant-numeric:tabular-nums; }
.history-page-buttons button:hover:enabled,.history-page-submit:hover:enabled { border-color:var(--line);color:var(--text);background:var(--hover); }
.history-page-buttons .current { background:var(--gold-dim);color:var(--gold);border-color:var(--gold);font-weight:600; }
.history-page-buttons button:disabled,.history-page-submit:disabled { cursor:default;opacity:.4; }
.history-page-arrow { display:grid;place-items:center; }
.previous-icon { transform:rotate(180deg); }
.history-page-jump { margin-left:5px;white-space:nowrap; }
.history-page-jump input { width:48px;min-height:32px;padding:5px;text-align:center;border-radius:7px;font-size:12px; }
.history-page-jump input[aria-invalid=true] { border-color:var(--danger); }
.history-page-submit { border-color:var(--line); }
@media(max-width:620px) { .history-page-controls{width:100%}.history-page-buttons{gap:3px}.history-page-buttons button{min-width:28px;padding:5px}.history-page-jump{margin-left:0} }
</style>
