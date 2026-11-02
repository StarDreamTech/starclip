<script setup lang="ts">
import { errorText, formatDate, t } from '../i18n';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { AppState, Clip, Group, HistoryQuery, HistorySummary } from '../types'
import { aiApi } from '../ai-api'
import type { AiConfig, AiRequest, AiState } from '../ai-api'
import { pendingClip, protectedClip } from '../privacy'
import UiSelect from './UiSelect.vue'
import HistoryPagination from './HistoryPagination.vue'
const props = defineProps<{ clips: Clip[]; groups: Group[]; revision: number; loadHistory?: (query: HistoryQuery) => Promise<AppState>; loadClips?: (ids: number[]) => Promise<Clip[]>; disabled?: boolean }>()
const emit = defineEmits<{ busy: [value: boolean] }>()
const state = ref<AiState | null>(null), busy = ref(false), error = ref(''), status = ref('')
const search = ref(''), group = ref<number | null>(null), selected = ref<number[]>([]), name = ref(t('我的 AI 助手'))
const allowPetControl = ref(false)
const fields = ref(['title', 'content']), maxResults = ref(20), expiresHours = ref(24)
const preview = ref<AiRequest | null>(null), config = ref<AiConfig | null>(null), configName = ref('')
const expandedPreview = ref<number[]>([])
const records = ref<Clip[]>([]), history = ref<HistorySummary>(), recordPage = ref(1), recordLoading = ref(false)
const selectedRecords = ref<Clip[]>([]), recordReadFailed = ref(false)
let recordRequest = 0, recordTimer: ReturnType<typeof setTimeout> | undefined
const formKey = computed(() => JSON.stringify([name.value, fields.value, maxResults.value, expiresHours.value, selected.value, allowPetControl.value, search.value, group.value]))
const savedFormKey = ref(formKey.value)
const canRelease = computed(() => !!state.value && !busy.value && !recordLoading.value && !preview.value && !config.value && formKey.value === savedFormKey.value)
defineExpose({ canRelease })
const fieldChoices = computed(() => [{ key: 'title', label: t('标题') }, { key: 'content', label: t('正文') }, { key: 'tags', label: t('普通标签') }, { key: 'sources', label: t('来源域名') }])
const groupOptions = computed(() => [{ value: null as number | null, label: t('全部分组') }, ...props.groups.map(item => ({ value: item.id, label: item.name }))])
const eligible = computed(() => (props.loadHistory ? records.value : props.clips).filter(clip => !protectedClip(clip) && !pendingClip(clip, props.revision) && clip.kind !== 'image' && clip.kind !== 'file'))
const available = computed(() => (recordLoading.value || recordReadFailed.value) ? [] : props.loadHistory ? eligible.value : eligible.value.filter(clip => (group.value === null || clip.groupId === group.value) && (!search.value.trim() || `${clip.title}\n${clip.content}\n${clip.tags?.join(' ')}`.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase()))))
const chosen = computed(() => selectedRecords.value.filter(clip => selected.value.includes(clip.id)))
const previewClips = computed(() => preview.value ? selectedRecords.value.filter(clip => preview.value!.clipIds.includes(clip.id)) : [])
let disposed = false
watch(selected, ids => {
  const candidates = new Map([...eligible.value, ...selectedRecords.value].map(clip => [clip.id, clip]))
  selectedRecords.value = ids.flatMap(id => candidates.has(id) ? [candidates.get(id)!] : [])
}, { deep: true, flush: 'sync' })
async function loadRecords() {
  if (!props.loadHistory) return
  const request = ++recordRequest, selectedIds = [...selected.value]
  recordLoading.value = true; recordReadFailed.value = false; preview.value = null
  try {
    const [result, selectedDetails] = await Promise.all([
      props.loadHistory({ page: recordPage.value, filter: group.value === null ? 'all' : `group-${group.value}`, query: search.value, aiEligible: true }),
      props.loadClips && selectedIds.length ? props.loadClips(selectedIds) : Promise.resolve(null),
    ])
    if (disposed || request !== recordRequest) return
    records.value = result.clips; history.value = result.history; recordPage.value = result.history?.page ?? 1
    if (selectedDetails) {
      const requested = new Set(selectedIds)
      const current = new Map(selectedDetails.filter(clip => !protectedClip(clip) && !pendingClip(clip, props.revision) && clip.kind !== 'image' && clip.kind !== 'file').map(clip => [clip.id, clip]))
      selectedRecords.value = selectedRecords.value.flatMap(clip => !requested.has(clip.id) ? [clip] : current.has(clip.id) ? [current.get(clip.id)!] : [])
      selected.value = selected.value.filter(id => !requested.has(id) || current.has(id))
    }
  } catch (cause) { if (!disposed && request === recordRequest) { records.value = []; recordReadFailed.value = true; error.value = errorText(cause) } }
  finally { if (!disposed && request === recordRequest) recordLoading.value = false }
}
function changeRecordPage(page: number) { recordPage.value = page; void loadRecords() }
watch([search, group], () => {
  if (!props.loadHistory) return
  recordPage.value = 1; recordLoading.value = true; ++recordRequest; preview.value = null
  clearTimeout(recordTimer); recordTimer = setTimeout(() => { void loadRecords() }, 150)
}, { flush: 'sync' })
watch(() => props.revision, () => { selected.value = []; preview.value = null; void loadRecords() })
watch(() => props.clips, () => {
  if (props.loadHistory) void loadRecords()
  else selected.value = selected.value.filter(id => eligible.value.some(clip => clip.id === id))
  preview.value = null
})
watch([name, fields, maxResults, expiresHours, selected, allowPetControl], () => { preview.value = null }, { deep: true })
watch(preview, () => { expandedPreview.value = [] }, { flush: 'sync' })
function togglePreview(id: number, event: Event) {
  expandedPreview.value = expandedPreview.value.filter(value => value !== id)
  if ((event.target as HTMLDetailsElement).open) expandedPreview.value.push(id)
}
async function run(action: () => Promise<void>) {
  if (busy.value || props.disabled) return
  busy.value = true; emit('busy', true); error.value = ''; status.value = ''
  try { await action() } catch (cause) { if (!disposed) error.value = errorText(cause) }
  finally { if (!disposed) { busy.value = false; emit('busy', false) } }
}
async function load() { await run(async () => { const value = await aiApi.state(); if (!disposed) state.value = value }) }
async function toggle() { if (!state.value) return; await run(async () => { const value = await aiApi.enable(!state.value!.enabled); if (!disposed) { state.value = value; config.value = null; preview.value = null; status.value = value.enabled ? t('本地接入已开启，请为连接选择资料或星伴权限。') : t('已关闭 AI 接入并撤销所有连接。') } }) }
async function prepare() {
  if (recordLoading.value || recordReadFailed.value) return
  if (props.loadClips && selected.value.length) await loadRecords()
  if (disposed || recordLoading.value || recordReadFailed.value || (!selected.value.length && !allowPetControl.value)) return
  preview.value = { name: name.value.trim(), clipIds: [...selected.value], fields: selected.value.length ? [...fields.value] : [], maxResults: maxResults.value, expiresHours: expiresHours.value, allowPetControl: allowPetControl.value }
}
async function create() { if (!preview.value) return; const request = preview.value; await run(async () => { const result = await aiApi.create(request); if (!disposed) { state.value = result.state; config.value = result.config; configName.value = request.name; selected.value = []; allowPetControl.value = false; preview.value = null; savedFormKey.value = formKey.value; status.value = t('连接已创建。请将下方配置添加到你信任的本机 AI 客户端。') } }) }
async function revoke(id: string) { await run(async () => { const value = await aiApi.revoke(id); if (!disposed) { state.value = value; config.value = null; status.value = t('已撤销连接，后续资料读取与星伴控制都会被拒绝。') } }) }
async function showConfig(id: string, connectionName: string) { await run(async () => { const value = await aiApi.config(id); if (!disposed) { config.value = value; configName.value = connectionName } }) }
async function clearAudit() { await run(async () => { const value = await aiApi.clearAudit(); if (!disposed) state.value = value }) }
function selectVisible() { selected.value = [...new Set([...selected.value, ...available.value.map(clip => clip.id)])].slice(0, state.value?.limits.maxRecords ?? 500) }
onMounted(() => { void load(); void loadRecords() })
onBeforeUnmount(() => { disposed = true; clearTimeout(recordTimer); config.value = null; selectedRecords.value = []; emit('busy', false) })
</script>
<template>
  <section class="ai-settings">
    <h3>{{ t('为 AI 分别授权资料与星伴') }}</h3>
    <p>{{ t('MCP、CLI 与 Skill 共用本机接口，默认关闭。剪贴历史始终只读；星伴动作与消息需单独授权，也可以只授权星伴而不开放任何记录。隐私、待扫描和图片记录不可授权。') }}</p>
    <p class="field-note">{{ t('保护范围是星笺官方接口；同账户文件读取或屏幕权限可能绕开它。已交给外部 AI 的内容无法撤回。连接仅本次应用运行有效，重启后需重新授权。') }}</p>
    <p v-if="error" class="privacy-error" role="alert">{{ error }}</p><p v-if="status" class="privacy-status" role="status">{{ status }}</p>
    <button v-if="!state" type="button" class="button subtle" :disabled="busy || disabled" @click="load">{{ busy ? t('正在读取接入状态…') : t('重新读取接入状态') }}</button>
    <template v-else>
      <div class="setting-row"><div><strong>{{ t('AI 本机接入') }}</strong><p>{{ !state.available ? t('请在桌面版管理真实连接') : state.enabled ? t('已开启 · 每个连接单独授权') : t('已关闭 · 不接受 AI 调用') }}</p></div><button type="button" :class="['switch', { on: state.enabled }]" role="switch" :aria-label="t('AI 本机接入')" :aria-checked="state.enabled" :disabled="busy || disabled || !state.available" @click="toggle"><span /></button></div>
      <div class="ai-connections"><h3>{{ t('当前连接') }}</h3><p v-if="!state.connections.length" class="field-note">{{ t('还没有连接。开启接入后，选择具体记录或星伴控制权限即可创建。') }}</p><article v-for="connection in state.connections" :key="connection.id" class="ai-connection"><div><strong>{{ connection.name }}</strong><p>{{ connection.recordCount ? t('{count} 条记录 · {fields}', { count: connection.recordCount, fields: connection.fields.map(field => fieldChoices.find(item => item.key === field)?.label ?? field).join(t('、')) }) : t('不开放剪贴历史') }}<br />{{ t('星伴动作与消息：') }}{{ connection.allowPetControl ? t('已授权') : t('未授权') }}<br />{{ t('到期 {time}', { time: formatDate(connection.expiresAt, { dateStyle: 'medium', timeStyle: 'short' }) }) }}</p></div><div class="privacy-buttons"><button type="button" class="button subtle" :disabled="busy || disabled" @click="showConfig(connection.id, connection.name)">{{ t('接入配置') }}</button><button type="button" class="button danger-quiet" :disabled="busy || disabled" @click="revoke(connection.id)">{{ t('撤销') }}</button></div></article></div>
      <fieldset class="ai-create" :disabled="busy || disabled || !state.available || !state.enabled">
        <h3>{{ t('星伴动作与消息') }}</h3>
        <label class="ai-pet-permission"><input v-model="allowPetControl" type="checkbox" :aria-label="t('允许星伴动作与消息')" />{{ t('允许这个连接控制星伴') }}</label>
        <p class="field-note">{{ t('Agent 可显示思考、工作、等待、成功或出错状态，触发已解锁的动作，显示最多 120 字的短句。请先在星伴设置中显示星伴；消息最多停留 60 秒，不会改变成长经验或你保存的心情。只用星伴时，无需选择下面的记录或扫描历史。') }}</p>
        <h3>{{ t('选择本次授权的记录（可选）') }}</h3>
        <p>{{ t('先筛选再勾选，授权只绑定这批记录的 ID，自动标签和导入不会扩大授权。') }}{{ t('隐私、图片、文件与待扫描记录不会出现在可授权列表。') }}</p>
        <div class="ai-filter"><input @keydown.enter.prevent v-model="search" :placeholder="t('筛选标题、正文或标签')" :aria-label="t('筛选可授权记录')" /><UiSelect v-model="group" :options="groupOptions" :label="t('筛选授权分组')" :disabled="busy || disabled || !state.enabled || !state.available" /></div>
        <div class="privacy-buttons"><button type="button" class="button subtle" :disabled="recordLoading || !available.length" @click="selectVisible">{{ t('选择当前结果') }}</button><button type="button" class="button subtle" :disabled="!selected.length" @click="selected = []">{{ t('清空选择') }}</button><span>{{ t('已选 {count} / {max}', { count: chosen.length, max: state.limits.maxRecords }) }}</span></div>
        <HistoryPagination v-if="history" :history="history" :disabled="recordLoading || busy || disabled" @page="changeRecordPage" /><button v-if="recordReadFailed" type="button" class="button subtle" @click="loadRecords">{{ t('重新读取') }}</button><p v-if="recordLoading" class="field-note">{{ t('正在读取…') }}</p><div class="ai-records"><label v-for="clip in available" :key="clip.id"><input v-model="selected" type="checkbox" :value="clip.id" :disabled="!selected.includes(clip.id) && selected.length >= state.limits.maxRecords" /><span><strong>{{ clip.title || clip.content.slice(0, 70) || t('未命名记录') }}</strong><small>{{ clip.tags?.join(' · ') || t('记录 #{id}', { id: clip.id }) }}</small></span></label><p v-if="!recordLoading && !available.length" class="field-note">{{ allowPetControl ? t('没有可授权记录。只授权星伴时可直接预览，无需历史扫描。') : t('没有符合条件的记录。可先到「规则与隐私」完成历史扫描。') }}</p></div>
        <label class="privacy-tags-label">{{ t('连接名称') }}<input @keydown.enter.prevent v-model="name" :aria-label="t('AI 连接名称')" maxlength="60" /></label>
        <div class="ai-fields"><label v-for="field in fieldChoices" :key="field.key"><input v-model="fields" type="checkbox" :value="field.key" :disabled="!chosen.length" />{{ t(field.label) }}</label></div>
        <div class="ai-limits"><label>{{ t('每次资料结果上限') }}<input :disabled="!chosen.length" @keydown.enter.prevent v-model.number="maxResults" type="number" min="1" :max="state.limits.maxResults" /></label><label>{{ t('有效小时数') }}<input @keydown.enter.prevent v-model.number="expiresHours" type="number" min="1" :max="state.limits.maxHours" /></label></div>
        <button type="button" class="button primary" :disabled="recordLoading || recordReadFailed || (!chosen.length && !allowPetControl) || (chosen.length > 0 && !fields.length) || !name.trim() || !Number.isInteger(maxResults) || maxResults < 1 || maxResults > state.limits.maxResults || !Number.isInteger(expiresHours) || expiresHours < 1 || expiresHours > state.limits.maxHours" @click="prepare">{{ t('预览本次授权') }}</button>
        <div v-if="preview" class="ai-preview"><strong>{{ t('确认向「{name}」', { name: preview.name }) }}{{ preview.clipIds.length ? t('开放 {count} 条记录', { count: preview.clipIds.length }) : t('仅授权星伴') }}</strong><p>{{ preview.clipIds.length ? t('仅开放 {fields}', { fields: preview.fields.map(field => fieldChoices.find(item => item.key === field)?.label).join(t('、')) }) : t('不开放剪贴历史') }}{{ t('，{hours} 小时内有效。', { hours: preview.expiresHours }) }}{{ preview.clipIds.length ? t('提交时后端会再次检查隐私状态。') : '' }}</p><p><strong>{{ t('星伴动作与消息：') }}{{ preview.allowPetControl ? t('允许') : t('不允许') }}</strong></p><details v-for="clip in previewClips" :key="clip.id" :open="expandedPreview.includes(clip.id)" @toggle="togglePreview(clip.id, $event)"><summary>{{ t('记录 #{id}', { id: clip.id }) }}{{ preview.fields.includes('title') ? ` · ${clip.title}` : '' }}</summary><pre v-if="expandedPreview.includes(clip.id) && preview.fields.includes('content')">{{ clip.content }}</pre><p v-if="expandedPreview.includes(clip.id) && preview.fields.includes('tags')">{{ t('标签：') }}{{ clip.tags?.join(t('、')) || t('无') }}</p><p v-if="expandedPreview.includes(clip.id) && preview.fields.includes('sources')">{{ t('来源：') }}{{ clip.linkHosts?.join(t('、')) || t('无') }}</p></details><div class="privacy-buttons"><button type="button" class="button primary" @click="create">{{ t('确认创建连接') }}</button><button type="button" class="button subtle" @click="preview = null">{{ t('返回修改') }}</button></div></div>
      </fieldset>
      <div v-if="config" class="ai-config"><div class="privacy-section-heading"><h3>{{ configName }} {{ t('· 接入配置') }}</h3><button type="button" class="button subtle" @click="config = null">{{ t('收起凭证') }}</button></div><p>{{ t('配置包含可撤销凭证，只放入你信任的客户端私有配置；不要分享、截图或提交到 Git。选择文字复制，不会自动修改其他应用。') }}</p><h4>{{ t('MCP 配置') }}</h4><pre tabindex="0">{{ JSON.stringify(config.mcp, null, 2) }}</pre><h4>CLI</h4><pre tabindex="0">{{ config.cli }}</pre><p class="field-note">{{ t('授权星伴后，可用 MCP 工具') }} <code>pet_signal</code> {{ t('发送') }} <code>{{ t('{"status":"working","message":"正在整理资料","durationSeconds":10}') }}</code>；<code>pet_clear</code> {{ t('收起自己发出的提示。CLI 将上方命令末尾的') }} <code>status</code> {{ t('替换成') }} <code>pet_signal</code> {{ t('并传入同样的 JSON 参数。') }}</p><h4>{{ t('Skill 使用说明') }}</h4><pre tabindex="0">{{ config.skill }}</pre></div>
      <div class="ai-audit"><div class="privacy-section-heading"><h3>{{ t('本次运行的调用记录') }}</h3><div class="privacy-buttons"><button type="button" class="button subtle" :disabled="busy || disabled" @click="load">{{ t('刷新') }}</button><button type="button" class="button subtle" :disabled="busy || disabled || !state.audit.length" @click="clearAudit">{{ t('清空记录') }}</button></div></div><p class="field-note">{{ t('仅保留最近 200 次操作，不记录搜索词、正文、标题、完整链接或凭证。') }}</p><p v-if="!state.audit.length" class="field-note">{{ t('暂无调用。') }}</p><ul><li v-for="(entry, index) in state.audit" :key="index">{{ formatDate(entry.at, { timeStyle: 'short' }) }} · {{ entry.connectionName }} · {{ entry.action }} · {{ t('{count} 条 · {result}', { count: entry.count, result: entry.result }) }}</li></ul></div>
    </template>
  </section>
</template>
