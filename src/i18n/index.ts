import { ref } from 'vue'
import { en } from './en.ts'
import { enMore } from './en-more.ts'
import { enMore2 } from './en-more2.ts'
import { enMore3 } from './en-more3.ts'
import { enData } from './en-data.ts'
import { enUpdates } from './en-updates.ts'
import { enPrivacy } from './en-privacy.ts'
import { enHandwriting } from './en-handwriting.ts'
import { enTelemetry } from './en-telemetry.ts'
import { enMarkdown } from './en-markdown.ts'
import { enStats } from './en-stats.ts'
import { backendEn, backendPatterns } from './backend.ts'

export type Locale = 'zh-CN' | 'en-US'
export type UILanguage = 'system' | Locale

const CACHE_KEY = 'starclip.ui-locale'
export const locale = ref<Locale>('zh-CN')

function browserLanguages(): string[] {
  if (typeof navigator === 'undefined') return []
  const values = [navigator.language, ...(Array.isArray(navigator.languages) ? navigator.languages : [])]
  return values.filter((value): value is string => typeof value === 'string' && value.length > 0)
}

export function systemLocale(): Locale {
  // Node-based component tests have no browser UI; keep their fixture text Chinese.
  if (typeof window === 'undefined' && typeof document === 'undefined') return 'zh-CN'
  if (typeof navigator === 'undefined' || /Node\.js/i.test(navigator.userAgent ?? '')) return 'zh-CN'
  return browserLanguages().some(value => /^zh\b/i.test(value)) ? 'zh-CN' : 'en-US'
}

export function resolveLocale(preference: UILanguage | string | undefined | null): Locale {
  return preference === 'zh-CN' || preference === 'en-US' ? preference : systemLocale()
}

export function cachedLocale(): Locale | null {
  try {
    const value = typeof localStorage === 'undefined' ? null : localStorage.getItem(CACHE_KEY)
    return value === 'zh-CN' || value === 'en-US' ? value : null
  } catch { return null }
}

export function setLocale(next: Locale, cache = true): void {
  locale.value = next
  if (cache) {
    try { if (typeof localStorage !== 'undefined') localStorage.setItem(CACHE_KEY, next) } catch { /* storage can be unavailable */ }
  }
  try {
    if (typeof document !== 'undefined') {
      document.documentElement.lang = next
      document.documentElement.dataset.locale = next === 'en-US' ? 'en' : 'zh'
    }
  } catch { /* non-browser fixtures */ }
}

export function applyLanguage(preference: UILanguage | string | undefined | null): Locale {
  const next = resolveLocale(preference)
  setLocale(next)
  return next
}

export function formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(locale.value, options).format(value)
}

export function formatDate(value: number | Date, options: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat(locale.value, options).format(value)
}

export function relativeTime(time: number, now = Date.now()): string {
  const minutes = Math.max(0, Math.floor((now - time) / 60000))
  if (minutes < 1) return t('刚刚')
  const formatter = new Intl.RelativeTimeFormat(locale.value, { numeric: 'auto' })
  if (minutes < 60) return formatter.format(-minutes, 'minute')
  if (minutes < 1440) return formatter.format(-Math.floor(minutes / 60), 'hour')
  if (minutes < 10080) return formatter.format(-Math.floor(minutes / 1440), 'day')
  return formatDate(time, { month: 'short', day: 'numeric' })
}

const catalog: Record<string, string> = { ...en, ...enMore, ...enMore2, ...enMore3, ...enData, ...enUpdates, ...enPrivacy, ...enHandwriting, ...enTelemetry, ...enStats, ...enMarkdown, ...backendEn }
function lookup(source: string): string {
  return catalog[source] ?? source
}

export function t(source: string, params?: Record<string, string | number>): string {
  let text = locale.value === 'zh-CN' ? source : lookup(source)
  if (params?.count !== undefined && text.includes('|')) {
    const [one, other] = text.split('|')
    text = (Number(params.count) === 1 ? one! : other!)
  }
  if (!params) return text
  return text.replace(/\{(\w+)\}/g, (match, key: string) => params[key] === undefined ? match : String(params[key]))
}

export function hasTranslation(source: string): boolean { return Object.prototype.hasOwnProperty.call(catalog, source) }

export function errorText(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error)
  if (locale.value === 'zh-CN') return raw
  const exact = catalog[raw]
  if (exact) return exact
  for (const [pattern, replacement] of backendPatterns) {
    if (pattern.test(raw)) return raw.replace(pattern, replacement)
  }
  if (/[\u4e00-\u9fff]/.test(raw)) {
    // Native diagnostics are translated where they are common. For the long
    // tail, keep the English UI readable and leave the original for support.
    console.warn('[i18n] untranslated native message:', raw)
    return 'The operation could not be completed. Please try again.'
  }
  return raw
}

// Initial paint already uses the last explicit choice when storage is available.
setLocale(cachedLocale() ?? systemLocale(), false)
