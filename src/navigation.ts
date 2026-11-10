export const settingsTabKeys = ['appearance', 'starpet', 'habits', 'privacy', 'ai', 'data', 'stats', 'share', 'help'] as const
export type SettingsTab = typeof settingsTabKeys[number]

type Navigation = {
  filter: string
  page: number
  settingsTab: SettingsTab
  settingsOpen: boolean
}

const cacheKey = 'starclip.navigation.v1'
const defaultNavigation = (): Navigation => ({ filter: 'all', page: 1, settingsTab: 'appearance', settingsOpen: false })

export function loadNavigation(): Navigation {
  try {
    const value = JSON.parse(localStorage.getItem(cacheKey) ?? 'null')
    if (!value || typeof value !== 'object' || Array.isArray(value)) return defaultNavigation()
    const groupId = typeof value.filter === 'string' && /^group-[1-9]\d*$/.test(value.filter) ? Number(value.filter.slice(6)) : 0
    const validFilter = ['all', 'pinned', 'kept', 'link', 'image', 'file', 'code'].includes(value.filter)
      || (groupId > 0 && Number.isSafeInteger(groupId))
    return {
      filter: validFilter ? value.filter : 'all',
      page: validFilter && Number.isInteger(value.page) && value.page > 0 && value.page <= 0xffffffff ? value.page : 1,
      settingsTab: settingsTabKeys.includes(value.settingsTab) ? value.settingsTab : 'appearance',
      settingsOpen: value.settingsOpen === true,
    }
  } catch { return defaultNavigation() }
}

export function saveNavigation(value: Navigation): void {
  try {
    // Persist navigation IDs only. Search text and editing drafts stay in memory.
    const { filter, page, settingsTab, settingsOpen } = value
    localStorage.setItem(cacheKey, JSON.stringify({ filter, page, settingsTab, settingsOpen }))
  } catch { /* An unavailable UI cache must not prevent opening or closing the window. */ }
}
