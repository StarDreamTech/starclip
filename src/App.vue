<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api, isDesktop } from './api'
import type { AppState, Clip, HistoryQuery, Settings } from './types'
import Icon from './components/Icon.vue'
import ClipImage from './components/ClipImage.vue'
import HistoryPagination from './components/HistoryPagination.vue'
import StartupScreen from './components/StartupScreen.vue'
import DataSettings from './components/DataSettings.vue'
import UpdateSettings from './components/UpdateSettings.vue'
import StartupSettings from './components/StartupSettings.vue'
import { quitStarclip } from './startup-api'
import UpdateNotice from './components/UpdateNotice.vue'
import ProductShare from './components/ProductShare.vue'
import { appVersion } from './update-api'
import CleanupPanel from './components/CleanupPanel.vue'
import StickerScene from './components/StickerScene.vue'
import OnboardingGuide from './components/OnboardingGuide.vue'
import UiSelect from './components/UiSelect.vue'
import StarPetSettings from './components/StarPetSettings.vue'
import AppearanceSettings from './components/AppearanceSettings.vue'
import PrivacyRules from './components/PrivacyRules.vue'
import TelemetrySettings from './components/TelemetrySettings.vue'
import ClipStatsPanel from './components/ClipStatsPanel.vue'
import { recordStudioOpened } from './telemetry-api'
import ClipPrivacy from './components/ClipPrivacy.vue'
import AiSettings from './components/AiSettings.vue'
import { privacyReasonExplanation, protectedClip, sourceNames } from './privacy'
import { applyAppearanceToRoot, defaultAppearance } from './appearance'
import type { Appearance } from './appearance'
import { getAppearance } from './appearance-api'
import { cancelThemeTransition, createThemeWriter, revealTheme } from './theme'
import type { ThemeOrigin } from './theme'
import { applyLanguage, errorText, formatDate, formatNumber, locale, relativeTime as formatRelativeTime, t } from './i18n'

const CodeStudio = defineAsyncComponent(() => import('./components/CodeStudio.vue'))

const state = ref<AppState>({ onboardingCompleted: true, clips: [], groups: [], settings: { paused: true, pauseRevision: 0, uiLanguage: 'system', theme: 'dark', layout: 'grid', maxHistory: 0, retentionDays: 0, alwaysOnTop: false, showStartup: true, stickerPack: 'auto', shortcut: 'CommandOrControl+Shift+V' }, clipboardError: null, shortcutError: null, windowError: null })
const loading = ref(true)
const windowReady = ref(false)
const busy = ref(false)
const dataOperationBusy = ref(false)
const updateBusy = ref(false)
const startupBusy = ref(false)
const updateNoticeBusy = ref(false)
const shareBusy = ref(false)
const appearance = ref(defaultAppearance())
const appearanceBusy = ref(false)
const privacyBusy = ref(false)
const telemetryBusy = ref(false)
const statsBusy = ref(false)
const statsRevision = ref(0)
const aiBusy = ref(false)
const clipPrivacyBusy = ref(false)
const clipDetailsLoading = ref(false), clipDetailsError = ref('')
let clipDetailVersion = 0
const clipDetailsBlocked = computed(() => clipDetailsLoading.value || !!clipDetailsError.value)
const rulesRevision = ref(0)
const privateRevealed = ref(false)
const appearanceLoading = ref(true)
const appearanceError = ref('')
const dataBusy = computed(() => startupBusy.value || dataOperationBusy.value || updateBusy.value || shareBusy.value || appearanceBusy.value || privacyBusy.value || telemetryBusy.value || statsBusy.value || aiBusy.value || clipPrivacyBusy.value || clipDetailsLoading.value)
const customSticker = computed(() => appearance.value.stickerEnabled ? appearance.value.stickerDataUrl : null)
const startupVisible = ref(false)
const firstGuidePending = ref(false)
const guideVisible = ref(false)
const guideBusy = ref(false)
const guideError = ref('')
const tutorialButton = ref<HTMLButtonElement>()
const filter = ref('all')
const query = ref('')
const sourceFilters = ref<string[]>([])
const hostFilter = ref('')
const tagFilter = ref('')
const privacyFilter = ref('all')
const sourceExpanded = ref(false)
const historyPage = ref(1), historyLoading = ref(false)
const historyScroll = ref<HTMLElement>()
let historySearchTimer: ReturnType<typeof setTimeout> | undefined
const historyRequest = (): HistoryQuery => ({ page: historyPage.value, filter: filter.value, query: query.value, sources: metadataSources.value, tag: tagFilter.value, privacy: privacyFilter.value })
const metadataSources = computed(() => [...sourceFilters.value, ...(hostFilter.value ? [`host:${hostFilter.value}`] : [])])
const sourceChoices = computed(() => Object.entries(sourceNames).filter(([key]) => state.value.history?.sources.includes(key) ?? state.value.clips.some(clip => clip.linkSources?.includes(key))).map(([key, label]) => [key, t(label)] as [string, string]))
const hostChoices = computed(() => [{ value: '', label: t('全部域名') }, ...(state.value.history?.hosts ?? [...new Set(state.value.clips.flatMap(clip => clip.linkHosts ?? []))].sort()).map(host => ({ value: host, label: host }))])
const tagChoices = computed(() => [{ value: '', label: t('全部普通标签') }, ...(state.value.history?.tags ?? [...new Set(state.value.clips.flatMap(clip => clip.tags ?? []))].sort()).map(tag => ({ value: tag, label: tag }))])
const privacyChoices = computed(() => [{ value: 'all', label: t('全部隐私状态') }, { value: 'private', label: t('隐私记录') }, { value: 'pending', label: t('待扫描 / 复核') }, { value: 'ordinary', label: t('已扫描普通记录') }])
const hasMetadataFilters = computed(() => !!(metadataSources.value.length || tagFilter.value || privacyFilter.value !== 'all'))
function clearMetadataFilters() { sourceFilters.value = []; hostFilter.value = ''; tagFilter.value = ''; privacyFilter.value = 'all' }
const selected = ref<number | null>(null)
const searchInput = ref<HTMLInputElement>()
const fileInput = ref<HTMLInputElement>()
const modal = ref<'' | 'clip' | 'group' | 'settings' | 'delete-group'>('')
const dialog = ref<HTMLDialogElement>()
const modalToastHost = ref<HTMLElement>()
const cleanupOpen = ref(false)
const cleanupBusy = ref(false)
const cleanupDialog = ref<HTMLDialogElement>()
const cleanupToastHost = ref<HTMLElement>()
let cleanupTrigger: HTMLElement | null = null
let cleanupScrollTop = 0
const editing = ref<Clip | null>(null)
const studioOpen = ref(false)
const studioClip = ref<Clip | null>(null)
const clipTitle = ref('')
const clipContent = ref('')
const clipGroup = ref<number | null>(null)
const clipPinned = ref(false)
const clipKept = ref(false)
const groupName = ref('')
const groupColor = ref('#8b93d9')
const settingsDraft = ref<Settings>({ ...state.value.settings })
const settingsSaving = ref(false)
const settingsTabKeys = ['appearance', 'starpet', 'habits', 'privacy', 'ai', 'data', 'stats', 'share', 'help'] as const
type SettingsTab = typeof settingsTabKeys[number]
const settingsTabs = computed<Array<{ key: SettingsTab; label: string; icon: string }>>(() => [
  { key: 'appearance', label: t('外观'), icon: 'moon' },
  { key: 'starpet', label: t('星伴'), icon: 'star' },
  { key: 'habits', label: t('使用习惯'), icon: 'sliders' },
  { key: 'privacy', label: t('规则与隐私'), icon: 'shield' },
  { key: 'ai', label: t('AI 与自动化'), icon: 'code' },
  { key: 'data', label: t('数据管理'), icon: 'folder' },
  { key: 'stats', label: t('星迹统计'), icon: 'chart' },
  { key: 'share', label: t('分享'), icon: 'spark' },
  { key: 'help', label: t('帮助'), icon: 'spark' },
])
const settingsTab = ref<SettingsTab>('appearance')
const settingsScroll = ref<HTMLElement>()
const dataSettings = ref<InstanceType<typeof DataSettings>>()
const appearanceSettings = ref<InstanceType<typeof AppearanceSettings>>()
const starPetSettings = ref<InstanceType<typeof StarPetSettings>>()
const privacySettings = ref<InstanceType<typeof PrivacyRules>>()
const telemetrySettings = ref<InstanceType<typeof TelemetrySettings>>()
const aiSettings = ref<InstanceType<typeof AiSettings>>()
const productShare = ref<InstanceType<typeof ProductShare>>()
const clipStatsPanel = ref<InstanceType<typeof ClipStatsPanel>>()
const updateSettings = ref<InstanceType<typeof UpdateSettings>>()
const startupSettings = ref<InstanceType<typeof StartupSettings>>()
const dataTabVisited = ref(false)
const privacyTabVisited = ref(false)
const aiTabVisited = ref(false)
const settingsDirty = computed(() => (Object.keys(settingsDraft.value) as Array<keyof Settings>).some(key => key !== 'theme' && key !== 'pauseRevision' && settingsDraft.value[key] !== state.value.settings[key]))
const settingsCanRelease = computed(() => !settingsDirty.value && !appearanceLoading.value
  && appearanceSettings.value?.canRelease === true
  && (!privacyTabVisited.value || (privacySettings.value?.canRelease === true && telemetrySettings.value?.canRelease === true))
  && (!aiTabVisited.value || aiSettings.value?.canRelease === true)
  && (!dataTabVisited.value || dataSettings.value?.canRelease === true)
  && (settingsTab.value !== 'habits' || startupSettings.value?.canRelease === true)
  && (settingsTab.value !== 'starpet' || starPetSettings.value?.canRelease === true)
  && (settingsTab.value !== 'stats' || clipStatsPanel.value?.canRelease === true)
  && (settingsTab.value !== 'share' || productShare.value?.canRelease === true)
  && (settingsTab.value !== 'help' || updateSettings.value?.canRelease === true))
const updateNoticeBlocked = computed(() => !windowReady.value || loading.value || busy.value || dataBusy.value || themeSaving.value
  || !!modal.value || studioOpen.value || cleanupOpen.value || guideVisible.value || startupVisible.value)
const toast = ref('')
const toastError = ref(false)
const toastTarget = computed(() => (cleanupOpen.value ? cleanupToastHost.value : modal.value ? modalToastHost.value : null) || 'body')
const backupPhase = ref<'' | 'choosing' | 'writing'>('')
const backupStatus = ref('')
const backupStatusKind = ref<'working' | 'success' | 'cancelled' | 'error'>('working')
const copied = ref<number | null>(null)
let toastTimer: ReturnType<typeof setTimeout>
let copyTimer: ReturnType<typeof setTimeout>
let unsubscribe: (() => void) | undefined
let unsubscribeGrowthError: (() => void) | undefined
let unsubscribeCloseRequest: (() => void) | undefined
const colors = ['#8b93d9', '#71baad', '#d7b678', '#d98eaa', '#89afd5', '#b3a192']
const kindNames = computed<Record<Clip['kind'], string>>(() => ({ text: t('文本'), link: t('链接'), code: t('代码'), color: t('色彩'), image: t('图片'), file: t('文件') }))
const navItems = computed(() => [{ key: 'all', name: t('全部记录'), icon: 'grid' }, { key: 'pinned', name: t('星标收藏'), icon: 'star' }, { key: 'kept', name: t('永久保留'), icon: 'shield' }, { key: 'link', name: t('链接'), icon: 'link' }, { key: 'image', name: t('图片'), icon: 'image' }, { key: 'file', name: t('文件'), icon: 'file' }, { key: 'code', name: t('星码集'), icon: 'code' }])
const visible = computed(() => historyLoading.value ? [] : state.value.clips)
const sectionName = computed(() => navItems.value.find(n => n.key === filter.value)?.name || state.value.groups.find(g => `group-${g.id}` === filter.value)?.name || t('全部记录'))
const currentGroup = computed(() => state.value.groups.find(g => `group-${g.id}` === filter.value))
const selectedIndex = computed(() => visible.value.findIndex(c => c.id === selected.value))
const dateLabel = computed(() => formatDate(new Date(), { month: 'long', day: 'numeric', weekday: 'long' }))
const isMac = navigator.platform.toLowerCase().includes('mac')
const modKey = isMac ? '⌘' : 'Ctrl'
// 按住 ⌘ / Ctrl 时在前 9 条显示快选序号。
const quickKeysVisible = ref(false)
function trackQuickKeys(event: KeyboardEvent) { quickKeysVisible.value = event.type === 'keydown' ? event.key === 'Meta' || event.key === 'Control' || (quickKeysVisible.value && (event.metaKey || event.ctrlKey)) : event.metaKey || event.ctrlKey }
function hideQuickKeys() { quickKeysVisible.value = false }
const shortcutLabel = computed(() => state.value.settings.shortcut.replace(/CommandOrControl|CmdOrCtrl/g, isMac ? '⌘' : 'Ctrl').replace(/Shift/g, '⇧').replaceAll('+', ' '))
const themeChoices = computed<Array<{ value: Settings['theme']; label: string; description: string }>>(() => [
  { value: 'dark', label: t('星夜'), description: t('深色 · 深邃夜空') },
  { value: 'light', label: t('月白'), description: t('浅色 · 清透微光') },
  { value: 'white', label: t('素白'), description: t('纯净白色') },
  { value: 'forest', label: t('青苔'), description: t('轻松绿意') },
  { value: 'rose', label: t('暮樱'), description: t('柔和暖粉') },
  { value: 'system', label: t('跟随系统'), description: t('随系统切换') },
])
const groupChoices = computed(() => [{ value: null as number | null, label: t('未分组') }, ...state.value.groups.map(group => ({ value: group.id, label: group.name }))])
const uiLanguageChoices = computed<Array<{ value: Settings['uiLanguage']; label: string }>>(() => [{ value: 'system', label: t('跟随系统') }, { value: 'zh-CN', label: '简体中文' }, { value: 'en-US', label: 'English' }])
const layoutChoices = computed<Array<{ value: Settings['layout']; label: string }>>(() => [{ value: 'grid', label: t('卡片网格') }, { value: 'list', label: t('紧凑列表') }, { value: 'horizontal', label: t('流星栏 · 横向快取') }])
const historyChoices = computed(() => [0, 100, 500, 1000, 5000].map(value => ({ value, label: value ? t('{count} 条', { count: formatNumber(value) }) : t('不限条数（默认）') })))
const retentionChoices = computed(() => [0, 1, 7, 30, 90, 180, 365].map(value => ({ value, label: value ? t('{count} 天', { count: value }) : t('不限天数（默认）') })))
const stickerOptions = computed<Array<{ value: Settings['stickerPack']; name: string; description: string }>>(() => [
  { value: 'auto', name: t('随主题'), description: t('随底色搭配，素白保持留白') },
  { value: 'cosmos', name: t('星月漫游'), description: t('月亮、星星与小行星') },
  { value: 'cats', name: t('奶油猫咪'), description: t('蝴蝶结与软乎乎的小猫') },
  { value: 'puppies', name: t('云朵小狗'), description: t('线条小狗与小爪印') },
  { value: 'garden', name: t('花园来信'), description: t('花朵、嫩叶与小盆栽') },
  { value: 'none', name: t('清爽留白'), description: t('关闭装饰，专注内容') },
])
const systemTheme = window.matchMedia('(prefers-color-scheme: dark)')
watch(locale, () => { if (typeof document !== 'undefined') document.title = t('星笺 Starclip — 让灵感有迹可循') }, { immediate: true })
const themeChoice = ref<Settings['theme']>('dark')
const themeSaving = ref(false)
const themeWriter = createThemeWriter(theme => api.setTheme(theme))
let themeVersion = 0, refreshVersion = 0, disposed = false
const resolvedTheme = ref<Exclude<Settings['theme'], 'system'>>('dark')
function applyTheme() {
  resolvedTheme.value = themeChoice.value === 'system' ? (systemTheme.matches ? 'dark' : 'light') : themeChoice.value
  document.documentElement.dataset.theme = resolvedTheme.value
  applyAppearanceToRoot(appearance.value, resolvedTheme.value)
}
function acceptAppearance(value: Appearance) {
  if (disposed) return
  appearance.value = value
  applyAppearanceToRoot(value, resolvedTheme.value)
}
async function loadAppearance() {
  if (appearanceBusy.value) return
  appearanceLoading.value = true; appearanceError.value = ''
  try { acceptAppearance(await getAppearance()) }
  catch (error) { if (!disposed) appearanceError.value = t('外观设置未能读取：{error}', { error: errorText(error) }) }
  finally { if (!disposed) appearanceLoading.value = false }
}
watch(filter, () => { selected.value = null; query.value = ''; privateRevealed.value = false })
watch([filter, query, metadataSources, tagFilter, privacyFilter], () => {
  historyPage.value = 1; selected.value = null; historyLoading.value = true
  ++refreshVersion // Invalidate requests immediately, including during the typing debounce.
  clearTimeout(historySearchTimer)
  historySearchTimer = setTimeout(() => { void refresh(); historyScroll.value?.scrollTo({ top: 0 }) }, 150)
}, { flush: 'sync' })
watch(modal, value => { privateRevealed.value = false; if (value !== 'clip') { ++clipDetailVersion; clipDetailsLoading.value = false; clipDetailsError.value = '' } }, { flush: 'sync' })
watch(visible, clips => { if (!clips.some(c => c.id === selected.value)) selected.value = clips[0]?.id ?? null })
watch(modal, async value => { await nextTick(); if (value && !dialog.value?.open) dialog.value?.showModal(); if (!value && dialog.value?.open) dialog.value?.close() })
function notify(message: string, error = false) { clearTimeout(toastTimer); toast.value = message; toastError.value = error; toastTimer = setTimeout(() => { toast.value = '' }, 3600) }
async function openProjectLink(event: MouseEvent, target: 'repository' | 'issues') {
  if (!isDesktop) return;
  event.preventDefault();
  try { await api.openProjectLink(target) }
  catch (error) { notify(errorText(error), true) }
}
async function refresh() {
  const request = ++refreshVersion, themeRevision = themeVersion, pendingTheme = themeSaving.value
  try {
    const [next, rules] = await Promise.all([api.getState(historyRequest()), api.privacyRules()])
    if (request !== refreshVersion || disposed) return
    const acceptTheme = !pendingTheme && !themeSaving.value && themeRevision === themeVersion
    if (!acceptTheme) next.settings.theme = state.value.settings.theme
    rulesRevision.value = rules.revision
    applyLanguage(next.settings.uiLanguage)
    if (isDesktop) void api.setNativeLanguage(locale.value).catch(() => {})
    // A newer saved capture choice (for example from the tray) supersedes an
    // older capture draft. Other unsaved preferences remain untouched.
    if (next.settings.pauseRevision !== settingsDraft.value.pauseRevision || next.settings.paused !== state.value.settings.paused) settingsDraft.value.paused = next.settings.paused
    settingsDraft.value.pauseRevision = next.settings.pauseRevision
    state.value = next
    historyPage.value = next.history?.page ?? 1
    if (acceptTheme) {
      const changedTheme = themeChoice.value !== next.settings.theme
      themeChoice.value = next.settings.theme; settingsDraft.value.theme = next.settings.theme
      if (changedTheme || loading.value) applyTheme()
    }
  } catch (error) {
    if (!disposed && request === refreshVersion) {
      if (historyLoading.value) state.value.clips = []
      notify(errorText(error), true)
    }
  } finally { if (!disposed && request === refreshVersion) { loading.value = false; historyLoading.value = false } }
}
function changeHistoryPage(page: number) {
  if (historyLoading.value || busy.value || dataBusy.value) return
  clearTimeout(historySearchTimer)
  historyPage.value = page; historyLoading.value = true; selected.value = null
  historyScroll.value?.scrollTo({ top: 0 })
  void refresh()
}
async function run(action: () => Promise<unknown>, success?: string): Promise<boolean> { if (busy.value || dataBusy.value || cleanupOpen.value) return false; busy.value = true; try { await themeWriter.idle(); await action(); await refresh(); if (success) notify(success); return true } catch (error) { notify(errorText(error), true); return false } finally { busy.value = false } }
function count(key: string) { return state.value.history?.counts[key] ?? state.value.clips.filter(c => key === 'all' || (key === 'pinned' && c.pinned) || (key === 'kept' && c.keepForever) || (key === 'link' ? c.kind === 'link' || !!c.linkHosts?.length : c.kind === key)).length }
function groupFor(clip: Clip) { return state.value.groups.find(g => g.id === clip.groupId) }
function relativeTime(time: number) { return formatRelativeTime(time) }
function fileNames(clip: Clip) { return clip.content.split('\n').filter(Boolean).map(path => path.split(/[\\/]/).filter(Boolean).pop() || path) }
function titleFor(clip: Clip) {
  if (protectedClip(clip)) return t('隐私记录')
  if (!clip.title && clip.kind === 'file') { const names = fileNames(clip); return names.length > 1 ? t('{name} 等 {count} 个文件', { name: names[0]!, count: names.length }) : names[0] || t('未命名片段') }
  return clip.title || (clip.kind === 'image' ? t('未命名图片') : clip.content.trim().split('\n')[0]?.slice(0, 44)) || t('未命名片段')
}
function imageLabel(clip: Clip) { const image = clip.image; if (!image) return t('PNG 图片'); const size = image.bytes < 1024 ? `${image.bytes} B` : image.bytes < 1048576 ? `${(image.bytes / 1024).toFixed(1)} KiB` : `${(image.bytes / 1048576).toFixed(1)} MiB`; return `${image.width} × ${image.height} · ${size}` }
function linkHost(content: string) { try { return new URL(content.trim()).hostname } catch { return t('网页链接') } }
function colorValue(content: string) { return /^#[\da-f]{3,8}$/i.test(content.trim()) ? content.trim() : '#8b93d9' }
async function copy(clip: Clip, quick = false, plain = false) {
  try {
    await api.copyClip(clip.id, plain); copied.value = clip.id; clearTimeout(copyTimer); copyTimer = setTimeout(() => { copied.value = null }, 1800)
    // 键盘快选 / 回车 / 双击后收起窗口，回到原应用直接粘贴；隐私内容同样收起，但提示仍保留在下次打开时可见。
    const hide = quick && isDesktop && state.value.settings.hideAfterCopy !== false
    notify(protectedClip(clip) ? t('隐私内容已复制，请留意目标应用。') : plain && (clip.rich || clip.kind === 'file') ? t('已复制为纯文本') : hide ? t('已复制，回到原应用即可粘贴') : t('已复制，切换到目标应用即可粘贴'))
    if (hide) { query.value = ''; void hideMainWindow(undefined, true) }
    await refresh()
  } catch (error) { notify(errorText(error), true) }
}
async function togglePin(clip: Clip) { await run(() => api.updateClip(clip.id, clip.title, clip.groupId, !clip.pinned), clip.pinned ? t('已取消星标') : t('已收入星标收藏')) }
async function toggleKept(clip: Clip) { await run(() => api.setKeepForever(clip.id, !clip.keepForever), clip.keepForever ? t('已取消永久保留') : t('已永久保留，不参与清理')) }
async function openCleanup(event: MouseEvent) {
  if (busy.value || dataBusy.value || cleanupOpen.value) return
  cleanupTrigger = event.currentTarget instanceof HTMLElement ? event.currentTarget : document.activeElement as HTMLElement
  cleanupScrollTop = settingsScroll.value?.scrollTop || 0
  cleanupOpen.value = true
  await nextTick()
  cleanupDialog.value?.showModal()
}
function closeCleanup() {
  if (cleanupBusy.value || busy.value || dataBusy.value) return
  cleanupDialog.value?.close()
}
function cleanupClosed() {
  cleanupOpen.value = false
  void nextTick(() => {
    if (settingsScroll.value) settingsScroll.value.scrollTop = cleanupScrollTop
    if (cleanupTrigger?.isConnected) cleanupTrigger.focus({ preventScroll: true })
    cleanupTrigger = null
  })
}
async function cleanupChanged() { await Promise.all([refresh(), dataSettings.value?.refresh()]) }
function openClip(clip?: Clip) { ++clipDetailVersion; clipDetailsLoading.value = false; clipDetailsError.value = ''; privateRevealed.value = false; editing.value = clip ?? null; clipTitle.value = clip?.title || ''; clipContent.value = clip?.kind === 'image' ? '' : clip?.content || ''; clipGroup.value = clip?.groupId ?? currentGroup.value?.id ?? null; clipPinned.value = clip?.pinned ?? false; clipKept.value = clip?.keepForever ?? false; modal.value = 'clip' }
function openStudio(clip: Clip | null = null) { if (clip?.kind === 'file') return notify(t('文件记录可直接复制到目标应用，或复制为纯文本路径。'), true); if (clip?.kind === 'image') return notify(t('星绘坊用于文字和代码，图片可直接复制到目标应用。'), true); modal.value = ''; studioClip.value = clip; studioOpen.value = true; recordStudioOpened() }
async function saveClip() {
  if (clipDetailsBlocked.value) return
  if (editing.value?.kind !== 'image' && !clipContent.value.trim()) return notify(t('先写下一点内容吧'), true)
  await run(async () => {
    if (editing.value) {
      // Protect first: automatic cleanup may run between these two native commands.
      if (clipKept.value && !editing.value.keepForever) await api.setKeepForever(editing.value.id, true)
      await api.updateClip(editing.value.id, clipTitle.value.trim(), clipGroup.value, clipPinned.value)
      if (!clipKept.value && editing.value.keepForever) await api.setKeepForever(editing.value.id, false)
    }
    else { const clip = await api.addClip(clipContent.value, clipTitle.value.trim(), clipGroup.value, clipKept.value); if (clipPinned.value) await api.updateClip(clip.id, clip.title, clipGroup.value, true) }
    modal.value = ''
  }, editing.value ? t('片段已更新') : t('新片段已收好'))
}
async function privacyChanged() {
  const id = modal.value === 'clip' ? editing.value?.id : undefined
  if (!id) { await refresh(); return }
  const request = ++clipDetailVersion
  privateRevealed.value = false; clipDetailsLoading.value = true; clipDetailsError.value = ''
  try {
    const [, clips] = await Promise.all([refresh(), api.getClips([id])])
    if (disposed || request !== clipDetailVersion || modal.value !== 'clip' || editing.value?.id !== id) return
    const current = clips.find(clip => clip.id === id)
    if (!current) { modal.value = ''; editing.value = null; clipContent.value = ''; return }
    editing.value = current
    clipContent.value = current.kind === 'image' ? '' : current.content
  } catch (error) {
    if (!disposed && request === clipDetailVersion) clipDetailsError.value = errorText(error)
  } finally { if (!disposed && request === clipDetailVersion) clipDetailsLoading.value = false }
}
function openGroup() { groupName.value = ''; groupColor.value = colors[state.value.groups.length % colors.length]!; modal.value = 'group' }
async function saveGroup() { if (!groupName.value.trim()) return notify(t('请为星座起一个名字'), true); await run(async () => { const group = await api.createGroup(groupName.value.trim(), groupColor.value); filter.value = `group-${group.id}`; modal.value = '' }, t('分组已创建')) }
async function quitFromSettings() {
  if (busy.value || dataBusy.value || themeSaving.value) return
  if (!settingsCanRelease.value) {
    notify(t('请先保存或取消尚未保存的修改，再退出星笺。'), true)
    return
  }
  busy.value = true
  try { await quitStarclip() }
  catch (error) { notify(errorText(error), true) }
  finally { busy.value = false }
}
function closeModal() { if (!dataBusy.value && !busy.value && !cleanupOpen.value) modal.value = '' }
// A close remains pending while drafts or work prevent releasing the WebView.
// Native request IDs ensure a delayed reply can never hide a reopened window.
const canReleaseWindow = computed(() => windowReady.value && !loading.value && !busy.value && !dataBusy.value && !themeSaving.value
  && !updateNoticeBusy.value
  && (!modal.value || (modal.value === 'settings' && settingsCanRelease.value))
  && !studioOpen.value && !cleanupOpen.value && !guideVisible.value && !guideBusy.value)
let pendingWindowClose: { requestId?: number; returnFocus?: boolean } | null = null
let closingWindow = false
let windowReleaseTimer: ReturnType<typeof setTimeout> | undefined
async function releasePendingWindow() {
  const request = pendingWindowClose
  if (!request || closingWindow || disposed) return
  clearTimeout(windowReleaseTimer)
  closingWindow = true
  try {
    const result = await api.hideWindow(canReleaseWindow.value, request.requestId, !!request.returnFocus)
    if (disposed || pendingWindowClose !== request) return
    if (!result.pending) pendingWindowClose = null
    else {
      request.requestId = result.requestId
      // Only native update/storage guards still need polling; frontend guards
      // retry when their reactive state settles, without a background timer.
      if (canReleaseWindow.value) windowReleaseTimer = setTimeout(() => { void releasePendingWindow() }, 1000)
    }
  } catch (error) { if (!disposed) notify(errorText(error), true) }
  finally {
    closingWindow = false
    if (!disposed && pendingWindowClose && pendingWindowClose !== request) void releasePendingWindow()
  }
}
function hideMainWindow(requestId?: number, returnFocus = false) {
  pendingWindowClose = returnFocus ? { requestId, returnFocus } : { requestId }
  return releasePendingWindow()
}
watch(canReleaseWindow, release => {
  clearTimeout(windowReleaseTimer)
  if (release) void releasePendingWindow()
})
async function listenForWindowClose() {
  if (!isDesktop) return
  try {
    unsubscribeCloseRequest = await api.onWindowCloseRequested(requestId => { if (!disposed) void hideMainWindow(requestId) })
    if (disposed) { unsubscribeCloseRequest(); return }
    const previous = pendingWindowClose
    const requestId = await api.getWindowCloseRequest()
    // Query after subscribing: even a close before JavaScript loaded is kept.
    // A newer live event wins over a delayed initial query.
    if (!disposed && pendingWindowClose === previous && requestId !== null) void hideMainWindow(requestId)
  } catch (error) { if (!disposed) notify(t('窗口关闭连接失败：{error}', { error: errorText(error) }), true) }
}
function replayStartup() { if (!dataBusy.value) startupVisible.value = true }
function openSettings() { settingsDraft.value = { ...state.value.settings, theme: themeChoice.value }; dataTabVisited.value = settingsTab.value === 'data'; privacyTabVisited.value = settingsTab.value === 'privacy'; aiTabVisited.value = settingsTab.value === 'ai'; modal.value = 'settings' }
function openUpdateSettings() {
  if (updateNoticeBlocked.value || updateNoticeBusy.value) return
  openSettings()
  changeSettingsTab('help')
}
function changeSettingsTab(tab: SettingsTab) {
  if (busy.value || dataBusy.value || cleanupOpen.value) return
  settingsTab.value = tab
  if (tab === 'data') dataTabVisited.value = true
  if (tab === 'privacy') privacyTabVisited.value = true
  if (tab === 'ai') aiTabVisited.value = true
  void nextTick(() => settingsScroll.value?.scrollTo({ top: 0 }))
}
function settingsTabKeyboard(event: KeyboardEvent) {
  if (event.altKey || event.ctrlKey || event.metaKey || event.isComposing || !['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  if (busy.value || dataBusy.value || cleanupOpen.value) return
  const index = settingsTabs.value.findIndex(tab => tab.key === settingsTab.value)
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? settingsTabs.value.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + settingsTabs.value.length) % settingsTabs.value.length
  const tab = settingsTabs.value[next]!.key
  changeSettingsTab(tab)
  void nextTick(() => {
    if (modal.value === 'settings' && settingsTab.value === tab && !cleanupOpen.value && !busy.value && !dataBusy.value) dialog.value?.querySelector<HTMLButtonElement>(`#settings-tab-${tab}`)?.focus()
  })
}
function finishStartup() {
  startupVisible.value = false
  if (firstGuidePending.value) { firstGuidePending.value = false; guideVisible.value = true }
}
function openGuide() {
  if (busy.value || dataBusy.value) return
  guideError.value = ''; guideVisible.value = true
}
function dismissGuide() {
  guideVisible.value = false; guideError.value = ''
  void nextTick(() => tutorialButton.value?.focus())
}
async function finishGuide() {
  if (guideBusy.value) return
  guideBusy.value = true; guideError.value = ''
  try {
    if (!state.value.onboardingCompleted) await api.completeOnboarding()
    state.value.onboardingCompleted = true
    dismissGuide()
  } catch (error) { guideError.value = errorText(error) }
  finally { guideBusy.value = false }
}
async function saveSettings() {
  if (busy.value || dataBusy.value || cleanupOpen.value || !settingsDirty.value) return
  settingsSaving.value = true
  try { if (await run(() => api.saveSettings({ ...settingsDraft.value, theme: themeChoice.value }, settingsDraft.value.pauseRevision))) notify(t('偏好设置已保存')) }
  finally { settingsSaving.value = false; applyLanguage(state.value.settings.uiLanguage) }
}
async function toggleCapture() {
  const { paused, pauseRevision } = state.value.settings
  if (await run(() => api.saveSettings({ ...state.value.settings, paused: !paused }, pauseRevision))) notify(state.value.settings.paused ? t('已暂停记录') : t('已开启记录，将先拾取当前剪贴内容'))
}
async function changeLayout(layout: Settings['layout']) { await run(() => api.saveSettings({ ...state.value.settings, layout }, state.value.settings.pauseRevision)) }
async function changeTheme(theme: Settings['theme'], origin?: ThemeOrigin) {
  if (busy.value || dataBusy.value || theme === themeChoice.value) return
  const request = ++themeVersion
  themeSaving.value = true
  themeChoice.value = theme; settingsDraft.value.theme = theme
  revealTheme(() => { applyTheme(); return nextTick() }, origin)
  try {
    const saved = await themeWriter.save(theme)
    if (disposed) return
    state.value.settings.theme = saved.theme
    if (request === themeVersion) { themeSaving.value = false; settingsDraft.value.theme = saved.theme }
  } catch (error) {
    if (request !== themeVersion || disposed) return
    themeSaving.value = false
    themeChoice.value = state.value.settings.theme; settingsDraft.value.theme = themeChoice.value
    revealTheme(() => { applyTheme(); return nextTick() }, origin)
    notify(t('主题未能保存，已恢复上次主题：{error}', { error: errorText(error) }), true)
  }
}
async function deleteEditing() { const id = editing.value?.id; if (id === undefined) return; await run(async () => { await api.deleteClip(id); modal.value = '' }, t('片段已删除')) }
async function exportBackup() {
  if (busy.value || dataBusy.value || cleanupOpen.value) return
  busy.value = true; backupStatusKind.value = 'working'
  backupPhase.value = isDesktop ? 'choosing' : 'writing'
  backupStatus.value = isDesktop ? t('请在保存窗口中选择备份位置…') : t('正在准备下载备份…')
  try {
    await themeWriter.idle()
    const saved = await api.saveBackup(() => {
      backupPhase.value = 'writing'
      backupStatus.value = isDesktop ? t('正在写入备份，请稍候…') : t('正在请求浏览器下载…')
    })
    backupStatusKind.value = saved ? 'success' : 'cancelled'
    backupStatus.value = saved
      ? isDesktop ? t('备份已保存到：\n{path}', { path: saved }) : t('已请求下载：{path}\n请查看浏览器下载记录；保存位置由浏览器决定。', { path: saved })
      : t('已取消导出，未写入备份。')
  } catch (error) {
    backupStatusKind.value = 'error'
    backupStatus.value = t('导出未完成：{error}', { error: errorText(error) })
  } finally { backupPhase.value = ''; busy.value = false }
}
async function importBackup(event: Event) { const input = event.target as HTMLInputElement; const file = input.files?.[0]; if (!file) return; if (file.size > 20 * 1024 * 1024) { notify(t('备份文件过大，最大支持 20 MB'), true); input.value = ''; return }; await run(async () => { const count = await api.importBackup(await file.text()); notify(t('已导入 {count} 条记录', { count: formatNumber(count) })) }); input.value = '' }
async function deleteCurrentGroup() { const id = currentGroup.value?.id; if (id === undefined) return; await run(async () => { await api.deleteGroup(id); filter.value = 'all'; modal.value = '' }, t('分组已删除，片段仍保留在全部记录中')) }
function moveSelection(delta: number) {
  if (!visible.value.length) return
  const index = Math.min(visible.value.length - 1, Math.max(0, Math.max(0, selectedIndex.value) + delta))
  selected.value = visible.value[index]!.id
  const card = document.querySelector<HTMLElement>(`[data-clip-id="${selected.value}"]`)
  card?.focus({ preventScroll: true })
  card?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}
function keyboard(event: KeyboardEvent) {
  const target = event.target as HTMLElement
  const typing = target.matches('input, textarea, [role=combobox]') || target.isContentEditable
  const modifier = event.metaKey || event.ctrlKey
  if (guideVisible.value) { if (modifier && event.key.toLowerCase() === 's') event.preventDefault(); return }
  if (cleanupOpen.value) { if (modifier && event.key.toLowerCase() === 's') event.preventDefault(); return }
  if (startupVisible.value || studioOpen.value || event.isComposing) return
  if (modal.value === 'settings' && modifier && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 's') {
    event.preventDefault()
    if (!busy.value && !dataBusy.value && settingsDirty.value) dialog.value?.querySelector<HTMLFormElement>('.settings-form')?.requestSubmit()
    return
  }
  if (modal.value) return
  if (modifier && event.key.toLowerCase() === 'k') { event.preventDefault(); searchInput.value?.focus(); return }
  // 搜索框内也能 ⌘/Ctrl + 1–9 快选、↓ 进入结果、回车复制第一条，呼出后输入即可选取。
  const inSearch = target === searchInput.value
  // ⇧ 组合：⇧⌘1–9 / ⇧回车复制为纯文本（去掉格式，文件只复制路径）。Shift+数字在部分键盘上报符号，用 code 判断。
  const digit = /^Digit[1-9]$/.test(event.code ?? '') ? event.code.slice(5) : /^[1-9]$/.test(event.key) ? event.key : ''
  if (inSearch && modifier && !event.altKey && digit) {
    const clip = visible.value[Number(digit) - 1]
    if (clip) { event.preventDefault(); void copy(clip, true, event.shiftKey) }
    return
  }
  if (inSearch && !modifier && !event.altKey && visible.value.length) {
    if (event.key === 'ArrowDown' && !event.shiftKey) { event.preventDefault(); selected.value = null; moveSelection(0); return }
    if (event.key === 'Enter') { event.preventDefault(); void copy(visible.value.find(c => c.id === selected.value) ?? visible.value[0]!, true, event.shiftKey); return }
  }
  if (typing) return
  if (modifier && event.shiftKey && !event.altKey && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
    event.preventDefault()
    const sections = [...navItems.value.map(item => item.key), ...state.value.groups.map(group => `group-${group.id}`)]
    const direction = event.key === 'ArrowRight' ? 1 : -1
    filter.value = sections[(Math.max(0, sections.indexOf(filter.value)) + direction + sections.length) % sections.length]!
    void nextTick(() => moveSelection(0))
    return
  }
  if (target.closest('button, a')) return
  if (modifier && !event.altKey && digit) {
    const clip = visible.value[Number(digit) - 1]
    if (clip) { event.preventDefault(); void copy(clip, true, event.shiftKey) }
    return
  }
  if (!modifier && !event.altKey && event.shiftKey && event.key === 'Enter' && selected.value !== null) {
    const clip = visible.value.find(c => c.id === selected.value)
    if (clip) { event.preventDefault(); void copy(clip, true, true) }
    return
  }
  if (modifier || event.altKey || event.shiftKey) return
  if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key) && visible.value.length) {
    event.preventDefault()
    const columns = state.value.settings.layout === 'grid' ? (window.innerWidth > 1150 ? 3 : 2) : 1
    moveSelection(event.key === 'ArrowDown' ? columns : event.key === 'ArrowUp' ? -columns : event.key === 'ArrowRight' ? 1 : -1)
  } else if (event.key === 'Enter' && selected.value !== null) {
    const clip = visible.value.find(c => c.id === selected.value)
    if (clip) { event.preventDefault(); void copy(clip, true) }
  } else if (event.key === 'Escape') { if (query.value) query.value = ''; else if (isDesktop) void hideMainWindow(undefined, true) }
}
onMounted(async () => {
  await Promise.all([listenForWindowClose(), refresh(), loadAppearance()]);
  if (disposed) return;
  if (isDesktop) void api.setNativeLanguage(locale.value).catch(() => {})
  firstGuidePending.value = !state.value.onboardingCompleted;
  startupVisible.value = state.value.settings.showStartup && !(window as Window & { __STARCLIP_REOPENED__?: boolean }).__STARCLIP_REOPENED__;
  if (!startupVisible.value) finishStartup();
  unsubscribe = await api.onChange(() => { statsRevision.value++; void refresh() });
  if (disposed) { unsubscribe(); return; }
  if (isDesktop) {
    try {
      unsubscribeGrowthError = await (await import('@tauri-apps/api/event')).listen<string>('starpet-growth-error', event => { if (!disposed) notify(event.payload, true) });
    } catch (error) { if (!disposed) notify(t('成长提示连接失败：{error}', { error: errorText(error) }), true) }
    if (disposed) { unsubscribeGrowthError?.(); return; }
  }
  window.addEventListener('keydown', keyboard); systemTheme.addEventListener('change', applyTheme)
  window.addEventListener('keydown', trackQuickKeys); window.addEventListener('keyup', trackQuickKeys); window.addEventListener('blur', hideQuickKeys)
  windowReady.value = true
})
onUnmounted(() => { disposed = true; pendingWindowClose = null; clearTimeout(windowReleaseTimer); clearTimeout(historySearchTimer); cancelThemeTransition(); unsubscribe?.(); unsubscribeGrowthError?.(); unsubscribeCloseRequest?.(); clearTimeout(toastTimer); clearTimeout(copyTimer); window.removeEventListener('keydown', keyboard); window.removeEventListener('keydown', trackQuickKeys); window.removeEventListener('keyup', trackQuickKeys); window.removeEventListener('blur', hideQuickKeys); systemTheme.removeEventListener('change', applyTheme) })
</script>

<template>
  <StartupScreen v-if="startupVisible" @done="finishStartup" />
  <OnboardingGuide v-if="guideVisible" :busy="guideBusy" :error="guideError" :theme="resolvedTheme" @finish="finishGuide" @dismiss="dismissGuide" />
  <div class="app-shell" :class="{ 'custom-background': appearance.background !== 'theme' }" :inert="startupVisible || guideVisible">
    <aside class="sidebar">
      <a class="brand" href="#" @click.prevent="filter = 'all'" :aria-label="t('星笺首页')"><span class="brand-mark"><Icon name="spark" :size="27" /><i /></span><span><strong>{{ t('星笺') }}<span class="brand-dot">.</span></strong><small>STARCLIP</small></span></a>
      <div class="workspace-label"><span class="workspace-orbit" />{{ t('我的星空') }} <span class="local-badge">LOCAL</span></div>
      <nav class="main-nav" :aria-label="t('记录分类')"><button v-for="item in navItems" :key="item.key" :class="['nav-item', { active: filter === item.key }]" @click="filter = item.key" :aria-current="filter === item.key ? 'page' : undefined" :title="item.key === 'code' ? t('星码集 · 收藏与管理代码片段') : item.name"><Icon :name="item.icon" /><span>{{ item.name }}</span><span class="nav-count">{{ count(item.key) }}</span></button></nav>
      <div class="group-heading"><span>{{ t('我的星座') }}</span><button class="icon-button" :title="t('新建分组')" :aria-label="t('新建分组')" @click="openGroup"><Icon name="plus" :size="16" /></button></div>
      <nav class="group-nav" :aria-label="t('分组')"><button v-for="group in state.groups" :key="group.id" :class="['nav-item', { active: filter === `group-${group.id}` }]" @click="filter = `group-${group.id}`"><span class="group-dot" :style="{ background: group.color }" /><span class="truncate">{{ group.name }}</span><span class="nav-count">{{ (state.history?.groupCounts[group.id] ?? state.clips.filter(c => c.groupId === group.id).length) }}</span></button><button v-if="!state.groups.length" class="empty-group" @click="openGroup"><Icon name="plus" :size="14" />{{ t('给灵感一个小分组') }}</button></nav>
      <div class="sidebar-bottom"><div class="night-note"><StickerScene :pack="state.settings.stickerPack" :theme="resolvedTheme" :custom-image="customSticker" placement="sidebar" /><p>{{ t('让散落的灵感，') }}<br>{{ t('在这里连成星座。') }}</p><span>YOUR LITTLE UNIVERSE</span></div><button class="capture-toggle" :title="state.settings.paused ? t('当前已暂停，开启时会先读取当前剪贴内容') : t('点击暂停剪贴记录')" :class="{ paused: state.settings.paused }" @click="toggleCapture" :disabled="busy || dataBusy"><span class="status-dot" /><span>{{ state.settings.paused ? t('开启剪贴记录') : t('正在拾取灵感') }}</span><Icon :name="state.settings.paused ? 'play' : 'pause'" :size="13" /></button><button class="nav-item settings-nav" @click="openSettings"><Icon name="sliders" /><span>{{ t('偏好设置') }}</span><span class="version">v{{ appVersion }}</span></button></div>
    </aside>
    <main class="main-panel">
      <header class="topbar"><div class="breadcrumb"><Icon name="spark" :size="15" /><span>{{ t('我的星空') }}</span><Icon name="chevron" :size="12" /><strong>{{ sectionName }}</strong></div><div class="topbar-right"><StickerScene class="header-stickers" :pack="state.settings.stickerPack" :theme="resolvedTheme" :custom-image="customSticker" placement="header" /><span v-if="!isDesktop" class="demo-badge">{{ t('交互演示 · 示例数据') }}</span><span class="date-label">{{ dateLabel }}</span><div class="theme-picker"><UiSelect :model-value="themeChoice" :options="themeChoices" :label="t('切换主题')" :disabled="busy || dataBusy" @update:model-value="changeTheme" /></div></div></header>
      <UpdateNotice v-if="isDesktop" :blocked="updateNoticeBlocked" @busy="updateNoticeBusy = $event" @open-settings="openUpdateSettings" />
      <div ref="historyScroll" :class="['main-scroll', { 'horizontal-layout': state.settings.layout === 'horizontal' }]">
        <div v-if="state.clipboardError || state.shortcutError || state.windowError" class="error-banner" role="alert"><Icon name="alert" :size="16" /><span>{{ errorText(state.clipboardError || state.shortcutError || state.windowError) }}</span><button @click="openSettings">{{ t('查看设置') }}</button></div>
        <section class="collection" :aria-label="t('剪贴板记录')">
          <div class="collection-heading"><div><h2>{{ sectionName }} <span>{{ state.history?.total ?? visible.length }}</span></h2><p>{{ query ? t('正在查找「{query}」', { query }) : filter === 'pinned' ? t('那些想要再次遇见的片段') : filter === 'kept' ? t('认真留下的，时间也不会带走') : filter === 'image' ? t('截图与画面 · 把看见的美好，留在自己的星空') : filter === 'code' ? t('代码收藏 · 把常用的几行，收进自己的星空') : currentGroup ? t('把相近的灵感，放在同一片星空') : t('好想法，总会再次闪光') }}</p></div><div class="collection-actions"><button class="button subtle cleanup-entry" :title="t('星尘整理 · 按大小、时间与类型清理历史')" :disabled="busy || dataBusy" @click="openCleanup"><Icon name="spark" :size="16" />{{ t('星尘整理') }}</button><button v-if="currentGroup" class="icon-button" :title="t('删除当前分组')" :aria-label="t('删除当前分组')" @click="modal = 'delete-group'"><Icon name="trash" :size="16" /></button><button class="button subtle code-create" :title="t('星绘坊 · 把文字与代码做成卡片')" @click="openStudio()"><Icon name="code" :size="16" />{{ t('星绘坊') }}</button><button class="button primary" @click="openClip()"><Icon name="plus" :size="16" />{{ t('新建片段') }}</button></div></div>
          <div class="toolbar"><label class="search-box"><Icon name="search" :size="18" /><input ref="searchInput" v-model="query" :placeholder="t('搜索你收藏的每一点灵感…')" :aria-label="t('搜索记录')" /><button v-if="query" class="icon-button" @click="query = ''" :title="t('清空搜索')" :aria-label="t('清空搜索')"><Icon name="close" :size="14" /></button><kbd v-else>{{ modKey }} K</kbd></label><div class="layout-switch" role="group" :aria-label="t('显示方式')"><button :class="{ selected: state.settings.layout === 'grid' }" :title="t('网格视图')" :aria-label="t('网格视图')" :aria-pressed="state.settings.layout === 'grid'" @click="changeLayout('grid')"><Icon name="grid" :size="17" /></button><button :class="{ selected: state.settings.layout === 'list' }" :title="t('列表视图')" :aria-label="t('列表视图')" :aria-pressed="state.settings.layout === 'list'" @click="changeLayout('list')"><Icon name="list" :size="17" /></button><button :class="{ selected: state.settings.layout === 'horizontal' }" :title="t('流星栏 · 横向快取')" :aria-label="t('流星栏 · 横向快取')" :aria-pressed="state.settings.layout === 'horizontal'" @click="changeLayout('horizontal')"><Icon name="list" :size="17" class="horizontal-view-icon" /></button></div></div>
          <div v-if="state.settings.layout === 'horizontal'" class="meteor-bar-heading"><div><Icon name="spark" :size="15" /><strong>{{ t('流星栏') }}</strong><span>{{ t('横向浏览，快速取用 · ← / ↑ 上一条，→ / ↓ 下一条') }}</span></div><div class="meteor-controls"><span>{{ visible.length ? selectedIndex + 1 : 0 }} / {{ visible.length }}</span><button class="icon-button" :aria-label="t('上一条片段')" :title="t('上一条片段')" :disabled="selectedIndex <= 0" @click="moveSelection(-1)"><Icon name="chevron" :size="15" class="previous-icon" /></button><button class="icon-button" :aria-label="t('下一条片段')" :title="t('下一条片段')" :disabled="selectedIndex < 0 || selectedIndex >= visible.length - 1" @click="moveSelection(1)"><Icon name="chevron" :size="15" /></button></div></div>
          <div class="results-label"><span>{{ query ? t('搜索结果') : t('最近收录') }}</span><span>{{ t('按最近更新排序') }} <Icon name="clock" :size="12" /></span></div>
          <div class="metadata-filters" :aria-label="t('来源、标签和隐私筛选')">
            <div class="metadata-filter-controls"><button type="button" class="button subtle" :aria-expanded="sourceExpanded" @click="sourceExpanded = !sourceExpanded">{{ t('链接来源') }}{{ metadataSources.length ? ` · ${metadataSources.length}` : '' }}</button><UiSelect v-model="tagFilter" :options="tagChoices" :label="t('按普通标签筛选')" /><UiSelect v-model="privacyFilter" :options="privacyChoices" :label="t('按隐私状态筛选')" /><button v-if="hasMetadataFilters" type="button" class="button subtle" @click="clearMetadataFilters">{{ t('清空筛选') }}</button></div>
            <div v-if="sourceExpanded" class="source-filters"><label v-for="[key, label] in sourceChoices" :key="key"><input v-model="sourceFilters" type="checkbox" :value="key" />{{ label }}</label><UiSelect v-model="hostFilter" :options="hostChoices" :label="t('按精确域名筛选')" /><p>{{ t('来源之间满足任一；与搜索、类型、标签和隐私状态同时匹配。短链不自动展开。') }}</p></div>
            <div v-if="hasMetadataFilters" class="metadata-chips"><button v-for="source in sourceFilters" :key="source" type="button" @click="sourceFilters = sourceFilters.filter(value => value !== source)">{{ t(sourceNames[source] ?? source) }} ×</button><button v-if="hostFilter" type="button" @click="hostFilter = ''">{{ hostFilter }} ×</button><button v-if="tagFilter" type="button" @click="tagFilter = ''">#{{ tagFilter }} ×</button><button v-if="privacyFilter !== 'all'" type="button" @click="privacyFilter = 'all'">{{ privacyChoices.find(choice => choice.value === privacyFilter)?.label }} ×</button></div>
          </div>
          <HistoryPagination v-if="state.history" :history="state.history" :disabled="historyLoading || busy || dataBusy" @page="changeHistoryPage" />
          <div v-if="loading || historyLoading" class="empty-state"><span class="loading-orbit" /><h3>{{ t('正在点亮你的星空…') }}</h3></div>
          <div v-else-if="!visible.length" class="empty-state"><div class="empty-illustration"><Icon :name="query ? 'search' : filter === 'pinned' ? 'star' : 'spark'" :size="38" /><i>✧</i><b>·</b></div><h3>{{ hasMetadataFilters ? t('没有匹配这些条件的记录') : query ? t('还没找到这颗星') : filter === 'pinned' ? t('为喜欢的灵感点一颗星') : filter === 'kept' ? t('给重要的灵感一个长久的家') : filter === 'image' ? t('把看见的美好，留在这里') : t('你的第一颗灵感，还在路上') }}</h3><p>{{ hasMetadataFilters ? t('来源、标签、隐私和搜索会同时生效，可以移除部分条件。') : query ? t('换个关键词试试，或查看全部记录。') : filter === 'pinned' ? t('点击片段上的星星，它就会出现在这里。') : filter === 'kept' ? t('点击片段上的盾牌，标记为永久保留。') : filter === 'image' ? t('开启记录后，复制图片或截图即可收录；图片文件路径不等于图片。') : t('复制文字或图片，或亲手写下一个新片段。') }}</p><button v-if="hasMetadataFilters" type="button" class="button subtle" @click="clearMetadataFilters">{{ t('清空筛选') }}</button><button v-else-if="query || filter !== 'image'" class="button subtle" @click="query ? query = '' : openClip()">{{ query ? t('清空搜索') : t('写下第一条片段') }}<Icon name="arrow" :size="15" /></button></div>
          <div v-else :class="['clip-collection', state.settings.layout]" :aria-label="state.settings.layout === 'horizontal' ? t('流星栏：横向片段列表') : undefined">
            <article v-for="(clip, index) in visible" :key="clip.id" :data-clip-id="clip.id" :class="['clip-card', `kind-${clip.kind}`, { 'is-selected': selected === clip.id, 'is-copied': copied === clip.id }]" tabindex="0" :aria-label="t('{title}，{kind}，双击复制', { title: titleFor(clip), kind: kindNames[clip.kind] })" @click="selected = clip.id" @focus="selected = clip.id" @dblclick="copy(clip, true)" @keydown.enter="event => { if (event.target === event.currentTarget) { event.stopPropagation(); event.preventDefault(); copy(clip, true) } }">
              <div class="card-top"><span :class="['kind-icon', clip.kind]"><Icon :name="clip.kind" :size="16" /></span><span class="kind-label">{{ clip.language || kindNames[clip.kind] }}</span><kbd v-if="quickKeysVisible && index < 9" class="quick-key" aria-hidden="true">{{ modKey }}{{ index + 1 }}</kbd><div class="card-top-actions" @dblclick.stop><button :class="['icon-button', 'keep-button', { kept: clip.keepForever }]" :title="clip.keepForever ? t('永久保留中 · 点击取消') : t('永久保留，不参与清理')" :aria-label="`${clip.keepForever ? t('取消永久保留') : t('永久保留')} ${titleFor(clip)}`" :aria-pressed="clip.keepForever" @click.stop="toggleKept(clip)"><Icon name="shield" :size="15" /></button><button :class="['icon-button', 'pin-button', { pinned: clip.pinned }]" :title="clip.pinned ? t('取消星标') : t('收藏到星标')" :aria-label="clip.pinned ? t('取消星标') : t('收藏到星标')" :aria-pressed="clip.pinned" @click.stop="togglePin(clip)"><Icon name="star" :size="15" /></button><button class="icon-button edit-button" :title="t('查看和编辑')" :aria-label="t('查看和编辑')" @click.stop="openClip(clip)"><Icon name="more" :size="17" /></button></div></div>
              <div class="card-main" @click="selected = clip.id"><template v-if="protectedClip(clip)"><h3>{{ t('隐私记录') }}</h3><p class="private-card-note"><Icon name="shield" :size="18" />{{ t('隐私 · 不向 AI 提供') }}</p><button type="button" class="private-card-open" @click.stop="openClip(clip)" @dblclick.stop>{{ t('在本机查看与复核') }}</button></template><template v-else-if="clip.kind === 'image'"><h3>{{ titleFor(clip) }}</h3><button class="image-open" type="button" :aria-label="t('预览图片：{title}', { title: titleFor(clip) })" @click.stop="openClip(clip)" @dblclick.stop><ClipImage :id="clip.id" :alt="titleFor(clip)" /><span class="image-card-meta">{{ imageLabel(clip) }}</span></button></template><template v-else-if="clip.kind === 'color'"><div class="color-preview" :style="{ background: colorValue(clip.content) }"><span>{{ clip.content.trim() }}</span></div><h3>{{ titleFor(clip) }}</h3></template><template v-else><h3>{{ titleFor(clip) }}</h3><div v-if="clip.kind === 'link'" class="link-preview"><span class="link-favicon">{{ linkHost(clip.content)[0]?.toUpperCase() }}</span><div><strong>{{ linkHost(clip.content) }}</strong><span>{{ clip.content }}</span></div><Icon name="arrow" :size="15" /></div><pre v-else-if="clip.kind === 'code'" class="code-preview">{{ clip.content }}</pre><ul v-else-if="clip.kind === 'file'" class="file-preview"><li v-for="name in fileNames(clip).slice(0, 4)" :key="name"><Icon name="file" :size="13" />{{ name }}</li><li v-if="fileNames(clip).length > 4" class="file-more">{{ t('还有 {count} 个', { count: fileNames(clip).length - 4 }) }}</li></ul><p v-else class="text-preview">{{ clip.content }}</p></template></div>
              <div v-if="!protectedClip(clip) && (clip.tags?.length || clip.linkSources?.length)" class="card-metadata"><span v-for="source in (clip.linkSources ?? []).slice(0, 1)" :key="source">{{ t(sourceNames[source] ?? source) }}</span><span v-for="tag in (clip.tags ?? []).slice(0, 2)" :key="tag">#{{ tag }}</span><small v-if="(clip.tags?.length ?? 0) > 2">+{{ clip.tags!.length - 2 }}</small></div>
              <div class="card-bottom"><span v-if="groupFor(clip)" class="group-tag" :style="{ '--group-color': groupFor(clip)?.color }"><span />{{ groupFor(clip)?.name }}</span><span v-else class="time-label">{{ relativeTime(clip.updatedAt) }}</span><div class="card-copy-area" @dblclick.stop><kbd v-if="index < 9">{{ modKey }} {{ index + 1 }}</kbd><button :class="['copy-button', { copied: copied === clip.id }]" @click.stop="copy(clip)" :title="t('复制{title}', { title: titleFor(clip) })" :aria-label="t('复制{title}', { title: titleFor(clip) })"><Icon :name="copied === clip.id ? 'check' : 'copy'" :size="14" /><span>{{ copied === clip.id ? t('已复制') : t('复制') }}</span></button></div></div>
            </article>
          </div>
        </section>
        <HistoryPagination v-if="state.history && state.history.total > state.history.pageSize" :history="state.history" :disabled="historyLoading || busy || dataBusy" @page="changeHistoryPage" />
        <div class="collection-end" v-if="visible.length"><span /> <Icon name="spark" :size="11" /> {{ t('每一点灵感，都有它的位置') }} <span /></div>
      </div>
      <footer class="statusbar"><span><span :class="['status-dot', { idle: state.settings.paused }]" />{{ state.settings.paused ? t('记录已暂停') : t('本地记录中') }}<span class="footer-divider">/</span>{{ t('{count} 条记录', { count: formatNumber(count('all')) }) }}</span><div><span><kbd>{{ state.settings.layout === 'horizontal' ? '← →' : '↑ ↓' }}</kbd> {{ t('选择') }}</span><span><kbd>↵</kbd> {{ t('复制') }}</span><span class="group-shortcut-footer"><kbd>{{ modKey }} ⇧ ← / →</kbd> {{ t('切换分类') }}</span><span class="shortcut-footer"><kbd>{{ shortcutLabel }}</kbd> {{ t('呼出星笺') }}</span></div></footer>
    </main>
    <dialog ref="dialog" :class="['modal', { 'settings-modal': modal === 'settings' }]" @cancel.prevent="closeModal" @close.self="modal = ''" @click="event => { if (event.target === dialog) closeModal() }">
      <div class="modal-content">
        <div class="modal-heading"><div><div class="eyebrow">{{ modal === 'settings' ? 'MAKE IT YOURS' : modal === 'group' ? 'A NEW CONSTELLATION' : 'A LITTLE THOUGHT' }}</div><h2>{{ modal === 'clip' ? editing ? t('这颗灵感的细节') : t('留住一点灵感') : modal === 'group' ? t('点亮一个新星座') : modal === 'settings' ? t('偏好设置') : t('删除这个分组？') }}</h2></div><button class="icon-button" :aria-label="t('关闭弹窗')" :disabled="busy || dataBusy" @click="closeModal"><Icon name="close" :size="20" /></button></div>
        <div ref="modalToastHost" class="dialog-toast-host" />
        <form v-if="modal === 'clip'" @submit.prevent="saveClip" class="clip-form">
          <div v-if="editing && protectedClip(editing)" class="private-detail-note"><strong>{{ t('隐私记录 · 原文默认折叠') }}</strong><p>{{ t('在本机展开不会授予 AI 访问权限。') }}</p><p v-if="editing.privacyManual">{{ t('保护依据：手动标记为隐私') }}</p><div v-if="editing.privacyReasons?.length" class="privacy-match-reasons"><strong>{{ t('保护依据与命中规则') }}</strong><ul><li v-for="reason in editing.privacyReasons" :key="reason"><strong>{{ reason.startsWith('自定义规则：') ? t('自定义规则：{name}', { name: reason.slice(6) }) : t(reason) }}</strong><p>{{ t(privacyReasonExplanation(reason)) }}</p></li></ul><small>{{ t('包含此前保留的命中记录；规则停用或内容修改后，可在下方复核。不会展示敏感值。') }}</small></div><button type="button" class="button subtle" :disabled="busy || dataBusy || clipDetailsBlocked" @click="privateRevealed = !privateRevealed">{{ privateRevealed ? t('收起私密内容') : t('仅在本机展开内容') }}</button></div>
          <p v-if="clipDetailsLoading" class="field-note">{{ t('正在读取…') }}</p><p v-if="clipDetailsError" class="privacy-error" role="alert">{{ clipDetailsError }} <button type="button" class="button subtle" @click="privacyChanged">{{ t('重新读取') }}</button></p>
          <template v-if="!clipDetailsBlocked && (!editing || !protectedClip(editing) || privateRevealed)"><label>{{ t('片段名称') }} <span class="optional">{{ t('选填') }}</span><input v-model="clipTitle" maxlength="200" :placeholder="t('为这颗灵感起个名字')" autofocus /></label>
          <div v-if="editing?.kind === 'image'" class="image-detail"><ClipImage :id="editing.id" :alt="titleFor(editing)" :thumbnail="false" /><p class="field-note">{{ imageLabel(editing) }} · {{ t('收录于 {time}', { time: formatDate(editing.createdAt, { dateStyle: 'medium', timeStyle: 'short' }) }) }}</p><p class="field-note">{{ t('图片保持原样；可修改标题、分组和保留方式，复制后粘贴到支持图片的应用。') }}</p></div>
          <label v-else>{{ t('内容') }} <span v-if="editing" class="optional">{{ t('原始记录 · 只读') }}</span><textarea v-model="clipContent" :readonly="!!editing" :placeholder="t('写下一段文字、一个链接，或者几行代码…')" rows="7" required /><span class="field-note">{{ t('{count} 个字符', { count: formatNumber([...clipContent].length) }) }}{{ editing ? ` · ${t('收录于 {time}', { time: formatDate(editing.createdAt, { dateStyle: 'medium', timeStyle: 'short' }) })}` : '' }}</span></label>
          </template>
          <ClipPrivacy v-if="editing" :clip="editing" :revision="rulesRevision" :disabled="busy || clipDetailsBlocked || dataOperationBusy || updateBusy || shareBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy" @busy="clipPrivacyBusy = $event" @changed="privacyChanged" />
          <div class="form-row"><div class="group-field"><span>{{ t('所属星座') }}</span><UiSelect v-model="clipGroup" :options="groupChoices" :label="t('所属星座')" :disabled="busy || dataBusy" /></div><label class="checkbox-label"><input type="checkbox" v-model="clipPinned" /><Icon name="star" :size="17" />{{ t('加入星标收藏') }}</label></div>
          <label class="checkbox-label keep-setting"><input type="checkbox" v-model="clipKept" /><Icon name="shield" :size="17" />{{ t('永久保留，不参与清理') }}</label><p v-if="editing?.keepForever" class="field-note">{{ t('这条记录已受保护；如需删除，请先取消永久保留并保存。') }}</p>
          <div class="modal-actions"><button v-if="editing" type="button" class="button danger-quiet" @click="deleteEditing" :disabled="busy || dataBusy || editing.keepForever"><Icon name="trash" :size="16" />{{ t('删除') }}</button><span class="spacer" /><button v-if="editing && (editing.rich || editing.kind === 'file')" type="button" class="button subtle" :disabled="busy || dataBusy || clipDetailsBlocked || (protectedClip(editing) && !privateRevealed)" :title="editing.kind === 'file' ? t('只复制文件路径') : t('去掉原有格式，只复制文字')" @click="copy(editing, false, true)"><Icon name="text" :size="15" />{{ t('复制为纯文本') }}</button><button v-if="editing?.kind === 'image'" type="button" class="button subtle" :disabled="busy || dataBusy || clipDetailsBlocked" @click="copy(editing)"><Icon name="copy" :size="15" />{{ t('复制图片') }}</button><button v-else-if="editing && editing.kind !== 'file'" type="button" class="button subtle" :disabled="busy || dataBusy || clipDetailsBlocked || (protectedClip(editing) && !privateRevealed)" @click="openStudio(editing)"><Icon name="code" :size="15" />{{ t('带入星绘坊') }}</button><button type="submit" class="button primary" :disabled="busy || dataBusy || clipDetailsBlocked || (editing?.kind !== 'image' && !clipContent.trim())">{{ editing ? t('保存修改') : t('收下这颗灵感') }}<Icon name="check" :size="16" /></button></div>
        </form>
        <form v-else-if="modal === 'group'" @submit.prevent="saveGroup"><p class="modal-intro">{{ t('把常用片段放在一起，下次相遇更容易。') }}</p><label>{{ t('星座名称') }}<input v-model="groupName" maxlength="40" :placeholder="t('例如：工作碎片、值得一读')" required autofocus /></label><label class="color-label">{{ t('标记颜色') }}<div class="color-choices"><button v-for="color in colors" :key="color" type="button" :style="{ background: color }" :class="{ chosen: groupColor === color }" :aria-label="t('选择颜色 {color}', { color })" :aria-pressed="groupColor === color" @click="groupColor = color"><Icon v-if="groupColor === color" name="check" :size="16" /></button></div></label><div class="modal-actions"><button type="button" class="button subtle" :disabled="dataBusy" @click="closeModal">{{ t('取消') }}</button><button class="button primary" :disabled="busy || dataBusy || !groupName.trim()">{{ t('创建分组') }}<Icon name="plus" :size="16" /></button></div></form>
        <form v-else-if="modal === 'settings'" @submit.prevent="saveSettings" class="settings-form">
          <nav class="settings-tabs" role="tablist" :aria-label="t('偏好分类')" aria-orientation="vertical" @keydown="settingsTabKeyboard">
            <button v-for="tab in settingsTabs" :key="tab.key" :id="`settings-tab-${tab.key}`" type="button" role="tab" :aria-selected="settingsTab === tab.key" :aria-controls="`settings-panel-${tab.key}`" :tabindex="settingsTab === tab.key ? 0 : -1" :disabled="busy || dataBusy" @click="changeSettingsTab(tab.key)"><Icon :name="tab.icon" :size="15" /><span>{{ tab.label }}</span></button>
          </nav>
          <div ref="settingsScroll" class="settings-scroll">
            <section v-show="settingsTab === 'appearance'" id="settings-panel-appearance" role="tabpanel" aria-labelledby="settings-tab-appearance" class="settings-panel">
              <p v-if="appearanceLoading" class="appearance-load-note" role="status">{{ t('正在读取自定义外观…') }}</p>
              <div v-else-if="appearanceError" class="appearance-load-error" role="alert">{{ appearanceError }}<button type="button" class="button subtle" :disabled="busy || dataBusy" @click="loadAppearance">{{ t('重新读取') }}</button></div>
              <AppearanceSettings ref="appearanceSettings" v-else :value="appearance" :disabled="busy || dataOperationBusy || updateBusy || shareBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy || themeSaving" @change="acceptAppearance" @busy="appearanceBusy = $event" />
              <p v-if="customSticker" class="appearance-load-note">{{ t('当前优先显示导入的贴纸。停用自定义贴纸后，将显示下方选择的内置贴纸。') }}</p>
              <fieldset class="settings-section preferences-boundary" :disabled="busy || dataBusy" :aria-label="t('外观')"><div class="setting-row"><div><strong>{{ t('基础主题') }}</strong><p role="status">{{ themeSaving ? t('主题已生效，正在保存…') : t('选择后立即生效并保存，无需点底部保存') }}</p></div><UiSelect :model-value="themeChoice" :options="themeChoices" :label="t('主题')" :disabled="busy || dataBusy" @update:model-value="changeTheme" /></div><div class="sticker-settings"><div class="sticker-settings-heading"><strong>{{ t('贴纸小伙伴') }}</strong><span>{{ t('原创小图案，陪你收好灵感') }}</span></div><div class="sticker-options" role="group" :aria-label="t('贴纸风格')"><button v-for="option in stickerOptions" :key="option.value" type="button" :class="['sticker-option', { chosen: settingsDraft.stickerPack === option.value }]" :aria-pressed="settingsDraft.stickerPack === option.value" :aria-label="option.name" :title="option.description" @click="settingsDraft.stickerPack = option.value"><StickerScene v-if="option.value !== 'none' && !(option.value === 'auto' && resolvedTheme === 'white')" :pack="option.value" :theme="settingsDraft.theme === 'system' ? resolvedTheme : settingsDraft.theme" placement="preview" /><span v-else class="sticker-none" aria-hidden="true">—</span><strong>{{ option.name }}</strong><span>{{ option.description }}</span></button></div></div></fieldset>
            </section>
            <section v-if="settingsTab === 'starpet'" id="settings-panel-starpet" role="tabpanel" aria-labelledby="settings-tab-starpet" class="settings-panel">
              <StarPetSettings ref="starPetSettings" :disabled="busy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || shareBusy || updateBusy" @busy="dataOperationBusy = $event" />
            </section>
            <section v-show="settingsTab === 'habits'" id="settings-panel-habits" role="tabpanel" aria-labelledby="settings-tab-habits" class="settings-panel">
              <StartupSettings v-if="settingsTab === 'habits'" ref="startupSettings" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || shareBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy" @busy="startupBusy = $event" @quit="quitFromSettings" />
              <fieldset class="settings-section preferences-boundary" :disabled="busy || dataBusy" :aria-label="t('使用习惯')"><h3><Icon name="sliders" :size="16" />{{ t('使用习惯') }}</h3><div class="setting-row"><div><strong>{{ t('界面语言') }}</strong><p>{{ t('保存后立即切换界面语言；跟随系统时，中文系统显示简体中文，其他系统显示英文。') }}</p></div><UiSelect v-model="settingsDraft.uiLanguage" :options="uiLanguageChoices" :label="t('界面语言')" :disabled="busy || dataBusy" /></div><div class="setting-row"><div><strong>{{ t('流星启动画面') }}</strong><p>{{ t('仅首次打开短暂显示，可随时跳过') }}</p></div><div class="startup-preference"><button type="button" class="startup-replay" :disabled="dataBusy" @click="replayStartup">{{ t('重看一次') }}</button><button type="button" :class="['switch', { on: settingsDraft.showStartup }]" role="switch" :aria-checked="settingsDraft.showStartup" :aria-label="t('流星启动画面')" :disabled="dataBusy" @click="settingsDraft.showStartup = !settingsDraft.showStartup"><span /></button></div></div><div class="setting-row"><div><strong>{{ t('默认布局') }}</strong><p>{{ t('网格、列表，或横向快取的流星栏') }}</p></div><UiSelect v-model="settingsDraft.layout" :options="layoutChoices" :label="t('默认布局')" :disabled="busy || dataBusy" /></div><div class="setting-row"><div><strong>{{ t('窗口置顶') }}</strong><p>{{ isDesktop ? t('保持在其他应用窗口上方') : t('桌面版生效；此处仅预览设置') }}</p></div><button type="button" :class="['switch', { on: settingsDraft.alwaysOnTop }]" role="switch" :aria-checked="settingsDraft.alwaysOnTop" :aria-label="t('窗口置顶')" @click="settingsDraft.alwaysOnTop = !settingsDraft.alwaysOnTop"><span /></button></div><div class="setting-row"><div><strong>{{ t('复制后收起窗口') }}</strong><p>{{ t('回车、双击或 {key} 1–9 复制后收起星笺，回到原应用直接粘贴', { key: modKey }) }}</p></div><button type="button" :class="['switch', { on: settingsDraft.hideAfterCopy !== false }]" role="switch" :aria-checked="settingsDraft.hideAfterCopy !== false" :aria-label="t('复制后收起窗口')" @click="settingsDraft.hideAfterCopy = settingsDraft.hideAfterCopy === false"><span /></button></div><div class="setting-row"><div><strong>{{ t('快选复制') }}</strong><p>{{ t('按住 {key} 查看序号；搜索框内也可用，回车复制第一条', { key: modKey }) }}</p></div><kbd class="setting-shortcut">{{ modKey }} 1–9</kbd></div><div class="setting-row"><div><strong>{{ t('呼出快捷键') }}</strong><p>{{ t('从任何应用打开星笺') }}</p></div><input class="shortcut-input" v-model="settingsDraft.shortcut" :aria-label="t('呼出快捷键')" placeholder="CommandOrControl+Shift+V" /></div><div class="setting-row"><div><strong>{{ t('分类 / 分组切换') }}</strong><p>{{ t('窗口内循环切换；输入文字和弹窗内不触发') }}</p></div><kbd class="setting-shortcut">{{ modKey }} ⇧ ← / →</kbd></div><p class="retention-note">{{ t('关闭主窗口后，星笺继续在后台运行，已开启的记录也会继续。可用呼出快捷键、再次打开应用或菜单栏 / 托盘菜单重新打开；要完全退出，可使用上方的「退出星笺」或托盘菜单。Windows 托盘图标可能收在通知区域的折叠菜单中。') }}</p></fieldset>
            </section>
            <section v-show="settingsTab === 'privacy'" id="settings-panel-privacy" role="tabpanel" aria-labelledby="settings-tab-privacy" class="settings-panel">
              <fieldset class="settings-section preferences-boundary" :disabled="busy || dataBusy" :aria-label="t('记录与隐私')"><h3><Icon name="shield" :size="16" />{{ t('采集与保留') }}</h3><div class="setting-row"><div><strong>{{ t('自动记录新复制的文本与图片') }}</strong><p>{{ t('开启或恢复时先记录当前剪贴内容，再监听新的复制；内容保存在本机') }}</p></div><button type="button" :class="['switch', { on: !settingsDraft.paused }]" role="switch" :aria-checked="!settingsDraft.paused" :aria-label="t('自动记录新复制的文本与图片')" @click="settingsDraft.paused = !settingsDraft.paused"><span /></button></div><div class="setting-row"><div><strong>{{ t('历史记录上限') }}</strong><p>{{ t('星标、永久保留与分组记录不计入上限') }}</p></div><UiSelect v-model="settingsDraft.maxHistory" :options="historyChoices" :label="t('历史记录上限')" :disabled="busy || dataBusy" /></div><div class="setting-row"><div><strong>{{ t('历史保留时长') }}</strong><p>{{ t('按最近更新时间计算；星标、永久保留与分组受保护') }}</p></div><UiSelect v-model="settingsDraft.retentionDays" :options="retentionChoices" :label="t('历史保留时长')" :disabled="busy || dataBusy" /></div><p class="field-note">{{ t('默认长期保留：不限条数、不限天数。历史不会因数量或时间自动移除；你仍可主动删除或整理，也可自行设置清理规则。') }}</p><p class="retention-note">{{ t('保存偏好后立即应用清理规则。桌面版还会在启动、新记录和约每分钟检查；暂停记录时也会按已选时长清理。浏览器演示只在保存偏好或新增内容时检查。永久保留、星标和已分组内容不会自动清理；重新捕获相同内容会更新保留起点。清理无法撤回，可先导出备份。同时选择「不限条数」和「不限天数」可关闭自动淘汰；重要内容也可单独标记永久保留。') }}</p><div class="privacy-note"><Icon name="shield" :size="16" /><p>{{ t('内容仅保存在本机，不上传云端。剪贴板可能含有私密内容，复制密码等敏感信息前请先暂停记录。') }}</p></div></fieldset>
              <PrivacyRules ref="privacySettings" v-if="privacyTabVisited" :revision="rulesRevision" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || shareBusy || appearanceBusy || telemetryBusy || aiBusy" @busy="privacyBusy = $event" @changed="privacyChanged" />
              <TelemetrySettings ref="telemetrySettings" v-if="privacyTabVisited" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || shareBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy" @busy="telemetryBusy = $event" />
            </section>
            <section v-show="settingsTab === 'ai'" id="settings-panel-ai" role="tabpanel" aria-labelledby="settings-tab-ai" class="settings-panel">
              <AiSettings ref="aiSettings" v-if="aiTabVisited" :clips="state.clips" :groups="state.groups" :revision="rulesRevision" :load-history="api.getState" :load-clips="api.getClips" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || shareBusy || appearanceBusy || privacyBusy" @busy="aiBusy = $event" />
            </section>
            <section v-show="settingsTab === 'data'" id="settings-panel-data" role="tabpanel" aria-labelledby="settings-tab-data" class="settings-panel">
              <div class="settings-section backup-section"><h3><Icon name="folder" :size="16" />{{ t('你的数据，由你保管') }}</h3><fieldset class="data-settings-boundary" :disabled="busy || themeSaving || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy"><DataSettings v-if="dataTabVisited" ref="dataSettings" @busy="dataOperationBusy = $event" /></fieldset><div class="backup-buttons"><button type="button" class="button subtle" @click="exportBackup" :disabled="busy || dataBusy" :aria-busy="!!backupPhase"><Icon name="down" :size="16" />{{ backupPhase === 'choosing' ? t('选择保存位置…') : backupPhase ? t('正在导出…') : t('导出备份') }}</button><button type="button" class="button subtle" @click="fileInput?.click()" :disabled="busy || dataBusy"><Icon name="upload" :size="16" />{{ t('导入备份') }}</button><button type="button" class="button subtle" :disabled="busy || dataBusy" @click="openCleanup">{{ t('星尘整理') }}</button></div><p v-if="backupStatus" :class="['backup-status', backupStatusKind]" :role="backupStatusKind === 'error' ? 'alert' : 'status'">{{ backupStatus }}</p><p class="field-note">{{ t('导入、清理与目录操作会单独执行，不会替你保存其他 Tab 的修改。') }}{{ isDesktop ? t('完整备份是本机明文 JSON，包含全部记录（含隐私）、标签、规则、分组及图片；不含 AI 凭证。导入会合并记录并重新检测；导入规则仅保存为停用草稿，需手动启用。') : t('浏览器演示暂不支持含图片的备份，请在桌面版导入或导出完整备份。') }}</p></div>
            </section>
            <section v-show="settingsTab === 'stats'" id="settings-panel-stats" role="tabpanel" aria-labelledby="settings-tab-stats" class="settings-panel">
              <ClipStatsPanel ref="clipStatsPanel" v-if="settingsTab === 'stats'" :revision="statsRevision" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || startupBusy || shareBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy" @busy="statsBusy = $event" />
            </section>
            <section v-show="settingsTab === 'share'" id="settings-panel-share" role="tabpanel" aria-labelledby="settings-tab-share" class="settings-panel">
              <ProductShare ref="productShare" v-if="settingsTab === 'share'" :disabled="busy || themeSaving || dataOperationBusy || updateBusy || startupBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy" @busy="shareBusy = $event" />
            </section>
            <section v-show="settingsTab === 'help'" id="settings-panel-help" role="tabpanel" aria-labelledby="settings-tab-help" class="settings-panel">
              <UpdateSettings ref="updateSettings" v-if="settingsTab === 'help'" :disabled="busy || themeSaving || dataOperationBusy || shareBusy || appearanceBusy || privacyBusy || telemetryBusy || aiBusy || clipPrivacyBusy" :unsaved-changes="settingsDirty" @busy="updateBusy = $event" />
              <div class="settings-project">
                <h3>{{ t('一起让星笺更好') }}</h3>
                <p>{{ t('查看项目进展，或把遇到的问题告诉我们。') }}</p>
                <div class="settings-project-links">
                  <a href="https://github.com/cpython666/starclip" target="_blank" rel="noopener noreferrer" @click="openProjectLink($event, 'repository')"><Icon name="code" :size="20" /><span><strong>{{ t('GitHub 项目') }}</strong><small>cpython666 / starclip</small></span><Icon name="arrow" :size="16" /></a>
                  <a href="https://github.com/cpython666/starclip/issues/new" target="_blank" rel="noopener noreferrer" @click="openProjectLink($event, 'issues')"><Icon name="edit" :size="20" /><span><strong>{{ t('反馈问题（Issue）') }}</strong><small>{{ t('描述问题与复现步骤') }}</small></span><Icon name="arrow" :size="16" /></a>
                </div>
                <p class="field-note">{{ t('将在浏览器打开 GitHub。Issue 内容公开，上传截图前请遮住私密剪贴内容。') }}</p>
              </div>
              <div class="settings-help-intro"><span class="settings-help-icon"><Icon name="spark" :size="24" /></span><div><h3>{{ t('初见星笺') }}</h3><p>{{ t('四步认识记录、收藏、卡片和数据管理。随时重看，按自己的节奏熟悉。') }}</p></div></div>
              <button ref="tutorialButton" type="button" class="button subtle tutorial-replay" :disabled="busy || dataBusy" @click="openGuide"><Icon name="play" :size="15" />{{ t('重看新手教程') }}</button>
              <p class="field-note">{{ t('重看教程不会开启记录或改动设置，也会保留你在各个 Tab 中尚未保存的修改。') }}</p>
              <div class="settings-help-shortcuts"><h3>{{ t('窗口关了，星笺还在') }}</h3><p class="field-note">{{ t('关闭主窗口只是收起界面，应用会继续在菜单栏 / 托盘运行，并按当前设置记录剪贴内容。macOS 可点击 Dock 中的星笺图标重新打开主窗口；也可使用呼出快捷键，或从托盘菜单选择打开星笺。完全退出请使用托盘菜单的「退出星笺」。Windows 通知区域可能折叠了图标，可展开查看。') }}</p><h3 class="help-shortcuts-heading">{{ t('常用快捷键') }}</h3><dl>
                <div><dt>{{ t('从其他应用呼出星笺') }}</dt><dd><kbd>{{ shortcutLabel }}</kbd></dd></div>
                <div><dt>{{ t('搜索记录') }}</dt><dd><kbd>{{ modKey }} K</kbd></dd></div>
                <div><dt>{{ t('切换分类与分组') }}</dt><dd><kbd>{{ modKey }} ⇧ ← / →</kbd></dd></div>
                <div><dt>{{ t('选择 / 复制记录') }}</dt><dd><kbd>{{ t('方向键 / Enter') }}</kbd></dd></div>
                <div><dt>{{ t('复制前九条记录') }}</dt><dd><kbd>{{ modKey }} 1–9</kbd></dd></div>
                <div><dt>{{ t('保存全部偏好') }}</dt><dd><kbd>{{ modKey }} S</kbd></dd></div>
              </dl><p class="field-note">{{ t('除全局呼出键外，请在星笺窗口中使用。输入文字、教程和弹窗中不会触发记录复制或分类切换。') }}</p></div>
            </section>
          </div>
          <div class="modal-actions settings-actions">
            <div class="settings-save-state"><span role="status" aria-live="polite" :class="{ dirty: settingsDirty }">{{ settingsSaving ? t('正在保存…') : settingsDirty ? t('有未保存的修改') : t('设置已同步') }}</span><small><kbd>{{ modKey }} S</kbd> {{ t('保存其他偏好与内置贴纸 · 自定义外观即时保存') }}</small></div>
            <button type="button" class="button subtle" :disabled="busy || dataBusy" @click="closeModal">{{ settingsDirty ? t('取消修改') : t('完成') }}</button>
            <button type="submit" class="button primary" :disabled="busy || dataBusy || !settingsDirty">{{ settingsSaving ? t('保存中…') : t('保存全部偏好') }}<Icon name="check" :size="16" /></button>
          </div>
        </form>
        <template v-else-if="modal === 'delete-group'"><p class="modal-intro">{{ t('「{name}」中的片段仍保留在全部记录中，仅移除分组。', { name: currentGroup?.name ?? '' }) }}</p><div class="modal-actions"><button class="button subtle" @click="closeModal">{{ t('取消') }}</button><button class="button danger" @click="deleteCurrentGroup" :disabled="busy || dataBusy">{{ t('删除分组') }}</button></div></template>
      </div>
    </dialog>
    <dialog ref="cleanupDialog" class="modal cleanup-modal" aria-labelledby="cleanup-heading" @cancel.prevent="closeCleanup" @close.self="cleanupClosed" @click="event => { if (event.target === cleanupDialog) closeCleanup() }">
      <div class="modal-content">
        <div class="modal-heading"><div><div class="eyebrow">A LITTLE MORE SPACE</div><h2 id="cleanup-heading">{{ t('星尘整理') }}</h2></div><button class="icon-button" :aria-label="t('关闭星尘整理')" :disabled="cleanupBusy || busy || dataBusy" @click="closeCleanup"><Icon name="close" :size="20" /></button></div>
        <div ref="cleanupToastHost" class="dialog-toast-host" />
        <CleanupPanel v-if="cleanupOpen" @busy="cleanupBusy = $event" @changed="cleanupChanged" />
      </div>
    </dialog>
    <CodeStudio v-if="studioOpen" :clip="studioClip" :groups="state.groups" :initial-mode="studioClip && studioClip.kind !== 'code' ? 'handwriting' : 'code'" @close="studioOpen = false" @saved="refresh" />
    <input ref="fileInput" type="file" accept="application/json,.json" hidden @change="importBackup" />
    <Teleport :to="toastTarget"><Transition name="toast"><div v-if="toast" :class="['toast-message', { error: toastError }]" role="status"><Icon :name="toastError ? 'alert' : 'check'" :size="17" /><span>{{ toast }}</span><button class="icon-button" @click="toast = ''" :aria-label="t('关闭提示')"><Icon name="close" :size="14" /></button></div></Transition></Teleport>
  </div>
</template>
