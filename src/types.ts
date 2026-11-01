export interface Clip {
  id: number;
  content: string;
  title: string;
  kind: 'text' | 'link' | 'code' | 'color' | 'image' | 'file';
  image?: { width: number; height: number; bytes: number } | null;
  language: string | null;
  pinned: boolean;
  keepForever: boolean;
  groupId: number | null;
  createdAt: number;
  updatedAt: number;
  copyCount: number;
  privacyManual?: boolean;
  isPrivate?: boolean;
  privacyReasons?: string[];
  scanVersion?: number;
  tags?: string[];
  manualTags?: string[];
  linkSources?: string[];
  linkHosts?: string[];
  /** 复制时记录的来源应用，也会出现在 tags 中。 */
  appTags?: string[];
  /** 浏览器提供的来源页面，已去掉查询与片段。 */
  originUrl?: string;
  /** 复制时附带了 HTML 格式。 */
  rich?: boolean;
}
export interface Group { id: number; name: string; color: string }
export type UILanguage = 'system' | 'zh-CN' | 'en-US';
export interface Settings {
  paused: boolean;
  pauseRevision: number;
  uiLanguage: UILanguage;
  theme: 'dark' | 'light' | 'white' | 'forest' | 'rose' | 'system';
  showStartup: boolean;
  stickerPack: 'auto' | 'none' | 'cosmos' | 'cats' | 'puppies' | 'garden';
  layout: 'grid' | 'list' | 'horizontal';
  maxHistory: number;
  retentionDays: number;
  alwaysOnTop: boolean;
  /** 复制后收起主窗口；旧偏好缺省视为开启。 */
  hideAfterCopy?: boolean;
  shortcut: string;
}
export interface HistoryQuery {
  page?: number;
  filter?: string;
  query?: string;
  sources?: string[];
  tag?: string;
  privacy?: string;
  aiEligible?: boolean;
}
export interface HistorySummary {
  page: number;
  pageSize: number;
  total: number;
  counts: Record<string, number>;
  groupCounts: Record<string, number>;
  sources: string[];
  hosts: string[];
  tags: string[];
}
export interface AppState {
  onboardingCompleted: boolean;
  clips: Clip[];
  history?: HistorySummary;
  groups: Group[];
  settings: Settings;
  clipboardError: string | null;
  shortcutError: string | null;
  windowError: string | null;
}
export interface HandwritingFont {
  id: string;
  name: string;
  author: string;
  tags: string[];
  path: string;
  downloaded: boolean;
  bytes: number;
  previewUrl: string;
  license: string;
  licenseUrl?: string;
  sourceUrl: string;
}
export interface HandwritingPaper {
  id: string;
  name: string;
  path: string;
  width: number;
  height: number;
  color: string | null;
  previewUrl: string | null;
}
export interface HandwritingAssets {
  available: boolean;
  path: string | null;
  fonts: HandwritingFont[];
  backgrounds: HandwritingPaper[];
  error: string | null;
}
export interface HandwritingOptions {
  text: string;
  font: string;
  background: string;
  fontSize: number;
  ink: string;
  padding: number;
}

export type DataDirectoryMode = 'move' | 'use';
export interface DataLocation {
  currentPath: string;
  defaultPath: string;
  configPath: string;
  pendingPath: string | null;
  totalBytes: number;
  databaseBytes: number;
  warnings: string[];
}
export interface DataDirectoryPlan {
  mode: DataDirectoryMode;
  currentPath: string;
  targetPath: string;
  totalBytes: number;
  warnings: string[];
}

export interface CleanupFilter {
  minBytes: number;
  olderThanDays: number;
  kind: 'all' | Clip['kind'];
  includeGrouped: boolean;
}
export interface CleanupPreview {
  token: string;
  totalCount: number;
  protectedCount: number;
  matchedCount: number;
  contentBytes: number;
  databaseBytes: number;
  reusableBytes: number;
  items: Array<{ id: number; title: string; kind: Clip['kind']; bytes: number; updatedAt: number }>;
  truncated: boolean;
}
export interface CleanupResult {
  deletedCount: number;
  contentBytes: number;
  databaseBytesBefore: number;
  databaseBytesAfter: number;
  releasedBytes: number;
  warning: string | null;
}
