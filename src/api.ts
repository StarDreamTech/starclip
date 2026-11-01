import type { AppState, Clip, CleanupFilter, CleanupPreview, CleanupResult, DataDirectoryMode, DataDirectoryPlan, DataLocation, Group, HistoryQuery, HandwritingAssets, HandwritingOptions, Settings } from './types';
import { download, languages } from './snippets';
import { t } from './i18n/index.ts';
import { analyzeDemoClip, matchesMetadata, pendingClip, protectedClip } from './privacy';
import type { PrivacyRules, PrivacyScan } from './privacy';

export const isDesktop = '__TAURI_INTERNALS__' in window;
// Original 320×180 moon-and-hills fixture, drawn programmatically; no clipboard or external assets.
const demoImage = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAUAAAAC0CAIAAABqhmJGAAAEQklEQVR42u3cX3LTZhTG4W8ZJQVaoBRKueoC+JdASiDACnrb6Z66ieyml1kDkwVg4hJCAjiKHUnvd56Z59pCR+eHbNmT9sNPj4BQzQhAwICAAQGDgAEBAwIGvhrwYyCUgEHAwDQB//wYCCVgEDAgYEDAUCjgJ0AoAYOAAQEDAgYBAwEB33oChBIwZAf8FAjVrt16CoQSMAgYEDCXc3R4YAgCJrXeJaMQMKn11mz4/X//LgiYHuot1fAy3eL1LgN+Rq5z9ZY465N6jwMuvQDt2u1nRPtcb43z/aLe8ldfwJ00XLBeAc8o4DoryKbqFfBcAq72JpCN1CvgWQR89jGMZUXAKQF//YsQ+8oF6hWwOzBuv/EBb0/uVL3bcMa3br/HDVcfTpvJv0O9CDg4YBhar4AFjIDTA76zDbO1IuDy8xEwAhYwTBGwhgWMgLMD3oHZWhnwccN15yNgBJwc8NadHZitiwS8UHY+AkbAAoZJAy7bsIDpJOCaDQuYfgIu2HDb+mUH5mxww5WGswj4OczZ0IA/ZVxiOAKm24Yvl3FW/AKm54AHZZx49xYwJRo+n3Qf770FTMWAu/nwLGA0HPzoS8BoOPjBtYDRcPDXTouAX0CWK6g3dRQCpnrD0XNoW3dfQKh1082fgICpmHE35y5gamXc2VkLeLWjwwNDQMCp9S4ZBQJOrVfDzDbgXb7jVL2mwewI+EINGwICBjYd8I93d4FQAobogH/dBUIJGAQMTBTwn0AoAYOAAQEDAgYBAwKmY0eHB4YgYFLrXTKKSQO+9xKGOvOnDgxkKgJm3YaNQsCkNmwIAgYEDAIGBAwIGFgV8B4QSsAgYGCagO/vARvx8Zct4x6xXb+/B6zv5LelYx5UwLDJekduWMCw4XrHbFjAkH0HfgWs71S94x1UwLDJhkc+ooAhmIBBwICAgYEB//YKCCVgyA74NRBKwCDgufr4xbrLjIBD6/3/p22uNAIOrVfDCNgdGATsMzAIGDgJeB8I1a4/2AdCCRgEDAgYGBbwjQf7QCgBg4CBiQJ+A1fnj/2/RlZqvAImslJtC5jOc62QtIAplGt/Sbcbv7+B06oVu6LneV8sASPa4JgFrFiCexawaAmOeRHwWzomrSliHu/6Cli0BMcsYNESHLOARUtwzG2q9+6IlvW7a3P4II5ouVx3bW5P1RAtF++uDX7Rh2+5IlaWod21tV/3HeuwmqwTXbui10W0jKCN8/+EYiEs4Mo9Wyx6C7jjnq0R5QLOrdrSIOBhbj58Nz7LgYABAfPJ83/+HoE5C5iAUFUtYPrMVdICprdi9SxgxVbhogtYtHpGwKIVs4BRrJhnF7BpilbMuRvYTFO0es5dv2aaihVz7gY20xStnnPXr1Wepgz0nL6Brc5Abbmk+9vA1uVYrS/TVj3aGTUXFXIJGAQMCBgQMAgYEDAgYEDAIGBAwICAQcBAog95oc6u5iG0XAAAAABJRU5ErkJggg==';
const now = Date.now();
const samples: Array<Pick<Clip, 'title' | 'content' | 'kind' | 'language' | 'groupId' | 'pinned' | 'image'>> = [
  { title: t('让灵感有迹可循'), content: t('有些灵感像流星，只出现一瞬。\n\n把值得留下的文字、链接与代码，收进自己的小小星系。'), kind: 'text', language: null, groupId: 1, pinned: true },
  { title: t('一个温柔的问候'), content: 'fn main() {\n    let universe = "你好，星笺";\n    println!("{universe} ✦");\n}', kind: 'code', language: 'rust', groupId: 2, pinned: true },
  { title: t('Tauri · 小巧的桌面应用'), content: 'https://v2.tauri.app/', kind: 'link', language: null, groupId: 2, pinned: false },
  { title: t('下一次灵感，留一点余地'), content: t('设计不是把每个角落填满，\n而是让重要的东西拥有呼吸的空间。'), kind: 'text', language: null, groupId: 1, pinned: false },
  { title: t('星尘金'), content: '#E9C98B', kind: 'color', language: null, groupId: 3, pinned: false },
  { title: t('一个轻巧的 debounce'), content: 'function debounce(fn, delay = 300) {\n  let timer;\n  return (...args) => {\n    clearTimeout(timer);\n    timer = setTimeout(() => fn(...args), delay);\n  };\n}', kind: 'code', language: 'javascript', groupId: 2, pinned: true },
  { title: t('今晚的待办'), content: t('☑ 留下一个好点子\n☑ 把常用代码收进收藏\n☐ 早点休息，明天再看星星'), kind: 'text', language: null, groupId: null, pinned: false },
  { title: t('MDN Web 文档'), content: 'https://developer.mozilla.org/zh-CN/', kind: 'link', language: null, groupId: 2, pinned: false },
  { title: t('月亮落在山丘上'), content: 'starclip-image:original-demo-moon', kind: 'image', language: null, groupId: null, pinned: false, image: { width: 320, height: 180, bytes: 1147 } },
];
let demo: AppState = {
  onboardingCompleted: false,
  clips: samples.map((clip, i) => ({ ...clip, keepForever: false, id: i + 1, createdAt: now - i * 1800000, updatedAt: now - i * 1800000, copyCount: i === 1 ? 12 : 0 })),
  groups: [{ id: 1, name: t('灵感碎片'), color: '#d9b780' }, { id: 2, name: t('开发工具箱'), color: '#87bab6' }, { id: 3, name: t('设计收藏'), color: '#b69bd7' }],
  settings: { paused: true, pauseRevision: 0, uiLanguage: 'system', showStartup: true, stickerPack: 'auto', theme: 'dark', layout: 'grid', maxHistory: 0, retentionDays: 0, alwaysOnTop: false, shortcut: 'CommandOrControl+Shift+V' },
  clipboardError: null, shortcutError: null, windowError: null,
};
// ponytail: browser preview stays in memory; only the native app persists real history.
const listeners = new Set<() => void>();
let demoRules: PrivacyRules = { revision: 1, rules: [] };
demo.clips = demo.clips.map(clip => analyzeDemoClip(clip, demoRules));
let demoScan: { result: PrivacyScan; fingerprint: string; clips: Clip[] } | null = null;
function pruneDemo() {
  const { retentionDays, maxHistory } = demo.settings;
  const cutoff = retentionDays === 0 ? -Infinity : Date.now() - retentionDays * 86400000;
  let historyCount = 0;
  demo.clips = demo.clips.sort((a, b) => b.updatedAt - a.updatedAt || b.id - a.id).filter(clip =>
    clip.pinned || clip.keepForever || clip.groupId !== null || (clip.updatedAt >= cutoff && (maxHistory === 0 || ++historyCount <= maxHistory)));
}
let cleanupSequence = 0;
let pendingCleanup: { token: string; clips: Clip[] } | null = null;
const contentBytes = (clip: Clip) => (clip.kind === 'image' ? clip.image?.bytes ?? 0 : new TextEncoder().encode(clip.content).length) + new TextEncoder().encode(clip.title).length;
const cleanupFingerprint = (clip: Clip) => JSON.stringify([clip.id, clip.content, clip.title, clip.kind, clip.image, clip.language, clip.pinned, clip.keepForever, clip.groupId, clip.createdAt, clip.updatedAt]);
function changed() { listeners.forEach((listener) => listener()); }
function id() { return Math.max(0, ...demo.clips.map((clip) => clip.id)) + 1; }
function requiredClip(clipId: number) { const clip = demo.clips.find((item) => item.id === clipId); if (!clip) throw new Error('记录不存在'); return clip; }
function validateContent(content: string) { if (!content.trim() || new TextEncoder().encode(content).length > 256 * 1024) throw new Error('内容不能为空，且不能超过 256 KB'); }
function classify(content: string): Clip['kind'] {
  if (/^#[\da-f]{3}(?:[\da-f]{3})?(?:[\da-f]{2})?$/i.test(content.trim())) return 'color';
  if (/^https?:\/\/\S+$/i.test(content.trim())) return 'link';
  if (/\b(function|const|let|fn |def |import |SELECT |class )/.test(content)) return 'code';
  return 'text';
}
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> { return (await import('@tauri-apps/api/core')).invoke<T>(command, args); }
// Keep font discovery, downloads and rendering serial across panels, including
// a quick close and reopen.
let handwritingQueue: Promise<unknown> = Promise.resolve();
function handwritingInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const request = handwritingQueue.then(() => invoke<T>(command, args));
  handwritingQueue = request.catch(() => {});
  return request;
}

export const api = {
  async privacyRules(): Promise<PrivacyRules> { return isDesktop ? invoke('get_privacy_rules') : structuredClone(demoRules); },
  async savePrivacyRules(rules: PrivacyRules): Promise<PrivacyRules> {
    if (isDesktop) return invoke('save_privacy_rules', { rules });
    if (rules.revision !== demoRules.revision) throw new Error('规则已变更，请重新读取后保存。');
    if (rules.rules.length > 100 || rules.rules.some(rule => !rule.name.trim() || !rule.value.trim() || (!rule.tag?.trim() && !rule.markPrivate))) throw new Error('每条规则需要名称、条件，以及至少一种动作。最多 100 条规则。');
    demoRules = structuredClone({ ...rules, revision: rules.revision + 1 }); demoScan = null; changed(); return structuredClone(demoRules);
  },
  async setClipPrivacy(id: number, isPrivate: boolean, reviewed = false): Promise<void> {
    if (isDesktop) return invoke('set_clip_privacy', { id, isPrivate, reviewed });
    const clip = requiredClip(id);
    if (!isPrivate && !reviewed) throw new Error('请先复核隐私原因。');
    Object.assign(clip, analyzeDemoClip({ ...clip, privacyManual: isPrivate, privacyReasons: isPrivate ? clip.privacyReasons : [] }, demoRules)); changed();
  },
  async setClipTags(id: number, tags: string[]): Promise<void> {
    if (isDesktop) return invoke('set_clip_tags', { id, tags });
    if (tags.length > 32 || tags.some(tag => !tag.trim() || tag.length > 40)) throw new Error('最多 32 个标签，每个不超过 40 个字。');
    const clip = requiredClip(id); Object.assign(clip, analyzeDemoClip({ ...clip, manualTags: [...new Set(tags)] }, demoRules)); changed();
  },
  async previewPrivacyRescan(): Promise<PrivacyScan> {
    if (isDesktop) return invoke('preview_privacy_rescan');
    const clips = demo.clips.map(clip => analyzeDemoClip(clip, demoRules));
    const result = { token: `demo-scan-${Date.now()}`, total: clips.length, privateCount: clips.filter(protectedClip).length, newlyPrivateCount: clips.filter((clip, index) => protectedClip(clip) && !protectedClip(demo.clips[index]!)).length, changedCount: clips.filter((clip, index) => JSON.stringify(clip) !== JSON.stringify(demo.clips[index])).length };
    demoScan = { result, fingerprint: JSON.stringify([demoRules, demo.clips]), clips }; return structuredClone(result);
  },
  async applyPrivacyRescan(token: string): Promise<PrivacyScan> {
    if (isDesktop) return invoke('apply_privacy_rescan', { token });
    if (!demoScan || demoScan.result.token !== token || demoScan.fingerprint !== JSON.stringify([demoRules, demo.clips])) throw new Error('记录或规则已变更，请重新预览。');
    const scan = demoScan; demoScan = null; demo.clips = scan.clips; changed(); return structuredClone(scan.result);
  },
  async completeOnboarding(): Promise<void> {
    if (isDesktop) return invoke('complete_onboarding');
    demo.onboardingCompleted = true; changed();
  },
  async setKeepForever(id: number, keepForever: boolean): Promise<void> {
    if (isDesktop) return invoke('set_keep_forever', { id, keepForever });
    requiredClip(id).keepForever = keepForever; changed();
  },
  async previewCleanup(filter: CleanupFilter): Promise<CleanupPreview> {
    if (isDesktop) return invoke('preview_cleanup', { filter });
    if (!Number.isSafeInteger(filter.minBytes) || filter.minBytes < 0 || !Number.isSafeInteger(filter.olderThanDays) || filter.olderThanDays < 0 || filter.olderThanDays > 36500 || !['all', 'text', 'link', 'code', 'color', 'image'].includes(filter.kind) || typeof filter.includeGrouped !== 'boolean') throw new Error('清理条件无效，请检查大小和天数。');
    const cutoff = Date.now() - filter.olderThanDays * 86400000;
    const clips = demo.clips.filter(c => !c.pinned && !c.keepForever && (filter.includeGrouped || c.groupId === null)
      && (filter.minBytes === 0 || contentBytes(c) > filter.minBytes)
      && (filter.olderThanDays === 0 || c.updatedAt < cutoff) && (filter.kind === 'all' || c.kind === filter.kind));
    const token = `demo-${++cleanupSequence}`;
    pendingCleanup = { token, clips: structuredClone(clips) };
    return { token, totalCount: demo.clips.length, protectedCount: demo.clips.filter(c => c.pinned || c.keepForever).length,
      matchedCount: clips.length, contentBytes: clips.reduce((sum, c) => sum + contentBytes(c), 0), databaseBytes: 0, reusableBytes: 0,
      items: clips.map(c => ({ id: c.id, title: c.title || (c.kind === 'image' ? '未命名图片' : c.content.slice(0, 44)), kind: c.kind, bytes: contentBytes(c), updatedAt: c.updatedAt })).sort((a, b) => b.bytes - a.bytes || a.id - b.id).slice(0, 100), truncated: clips.length > 100 };
  },
  async cancelCleanup(token: string): Promise<void> {
    if (isDesktop) return invoke('cancel_cleanup', { token });
    if (pendingCleanup?.token === token) pendingCleanup = null;
  },
  async executeCleanup(token: string, compact: boolean): Promise<CleanupResult> {
    if (isDesktop) return invoke('execute_cleanup', { token, compact });
    const pending = pendingCleanup; pendingCleanup = null;
    if (!pending || pending.token !== token) throw new Error('预览已失效，请重新预览。');
    const current = new Map(demo.clips.map(c => [c.id, c]));
    if (pending.clips.some(c => !current.has(c.id) || cleanupFingerprint(current.get(c.id)!) !== cleanupFingerprint(c))) throw new Error('待清理记录已变化，请重新预览。');
    const targets = new Set(pending.clips.map(c => c.id));
    demo.clips = demo.clips.filter(c => !targets.has(c.id)); changed();
    return { deletedCount: pending.clips.length, contentBytes: pending.clips.reduce((sum, c) => sum + contentBytes(c), 0), databaseBytesBefore: 0, databaseBytesAfter: 0, releasedBytes: 0, warning: '浏览器演示仅整理内存样例，不会释放本机磁盘空间。' };
  },
  async getDataLocation(): Promise<DataLocation> {
    if (!isDesktop) throw new Error('请在桌面版查看和管理数据目录。');
    return invoke('get_data_location');
  },
  async chooseDataDirectory(mode: DataDirectoryMode): Promise<DataDirectoryPlan | null> {
    if (!isDesktop) throw new Error('请在桌面版选择数据目录。');
    return invoke('choose_data_directory', { mode });
  },
  async applyDataDirectory(mode: DataDirectoryMode, targetPath: string): Promise<DataLocation> {
    if (!isDesktop) throw new Error('请在桌面版迁移数据。');
    return invoke('apply_data_directory', { mode, targetPath });
  },
  async openDataDirectory(): Promise<void> {
    if (!isDesktop) throw new Error('请在桌面版打开数据目录。');
    return invoke('open_data_directory');
  },
  async openProjectLink(target: 'repository' | 'issues'): Promise<void> {
    return invoke('open_project_link', { target });
  },
  async restartApp(): Promise<void> {
    if (!isDesktop) throw new Error('请在桌面版重启应用。');
    return invoke('restart_app');
  },
  async handwritingAssets(): Promise<HandwritingAssets> {
    if (isDesktop) return handwritingInvoke('handwriting_assets');
    // Only the bundled catalog and thumbnail images are read in the browser.
    // Font files are fetched exclusively by the native, verified installer.
    const [response, papers] = await Promise.all([fetch('/handwriting-fonts/catalog.json'), fetch('/handwriting-papers/catalog.json')]);
    if (!response.ok || !papers.ok) throw new Error('暂时无法读取字体列表，请重试。');
    const catalog = await response.json() as Array<Omit<HandwritingAssets['fonts'][number], 'path' | 'downloaded'>>;
    return { available: true, path: 'builtin', fonts: catalog.map(item => ({ ...item, path: item.id, downloaded: false })), backgrounds: await papers.json() as HandwritingAssets['backgrounds'], error: null };
  },
  async downloadHandwritingFont(id: string): Promise<HandwritingAssets> {
    if (!isDesktop) throw new Error('请在桌面版下载字体并生成月光笺。');
    return handwritingInvoke('download_handwriting_font', { id });
  },
  async renderHandwriting(options: HandwritingOptions): Promise<string> {
    if (!isDesktop) throw new Error('请在桌面版中生成月光笺。');
    return handwritingInvoke('render_handwriting', { options });
  },
  async getState(query: HistoryQuery = {}): Promise<AppState> {
    if (isDesktop) return invoke('get_state', { query });
    const counts: Record<string, number> = {}, groupCounts: Record<string, number> = {};
    const inFilter = (clip: Clip, filter = 'all') => filter === 'all' || (filter === 'pinned' && clip.pinned) || (filter === 'kept' && clip.keepForever) || (filter === 'link' ? !!clip.linkHosts?.length || clip.kind === 'link' : filter === clip.kind) || filter === `group-${clip.groupId}`;
    for (const key of ['all', 'pinned', 'kept', 'text', 'link', 'code', 'color', 'image']) counts[key] = demo.clips.filter(clip => inFilter(clip, key)).length;
    for (const clip of demo.clips) if (clip.groupId !== null) groupCounts[clip.groupId] = (groupCounts[clip.groupId] ?? 0) + 1;
    const term = (query.query ?? '').trim().toLocaleLowerCase();
    const matches = demo.clips.filter(clip => inFilter(clip, query.filter)
      && matchesMetadata(clip, query.sources ?? [], query.tag ?? '', query.privacy ?? 'all', demoRules.revision)
      && (!query.aiEligible || (!protectedClip(clip) && !pendingClip(clip, demoRules.revision) && clip.kind !== 'image'))
      && (!term || `${clip.title}\n${clip.kind === 'image' ? '' : clip.content}${query.aiEligible ? `\n${clip.tags?.join(' ') ?? ''}` : ''}`.toLocaleLowerCase().includes(term)))
      .sort((a, b) => b.updatedAt - a.updatedAt || b.id - a.id);
    const pageSize = 50, total = matches.length, page = Math.min(Math.max(1, Math.floor(query.page || 1)), Math.max(1, Math.ceil(total / pageSize)));
    const facet = (key: 'linkSources' | 'linkHosts' | 'tags') => [...new Set(demo.clips.flatMap(clip => clip[key] ?? []))].sort();
    return structuredClone({ ...demo, clips: matches.slice((page - 1) * pageSize, page * pageSize), history: { page, pageSize, total, counts, groupCounts, sources: facet('linkSources'), hosts: facet('linkHosts'), tags: facet('tags') } });
  },
  async getClips(ids: number[]): Promise<Clip[]> {
    if (ids.length > 500 || ids.some(id => !Number.isSafeInteger(id) || id <= 0)) throw new Error('每次最多读取 500 条有效记录。');
    if (isDesktop) return invoke('get_clips', { ids });
    return structuredClone(demo.clips.filter(clip => ids.includes(clip.id)));
  },
  async setNativeLanguage(language: 'zh-CN' | 'en-US'): Promise<void> { if (isDesktop) return invoke('set_native_language', { language }); },
  async imageData(id: number, thumbnail: boolean): Promise<string> {
    if (isDesktop) return invoke('get_clip_image', { id, thumbnail });
    if (requiredClip(id).kind !== 'image') throw new Error('这条记录不是图片');
    return demoImage;
  },
  async addClip(content: string, title: string, groupId: number | null, keepForever = false): Promise<Clip> {
    if (isDesktop) return invoke('add_clip', { content, title, groupId, keepForever });
    validateContent(content);
    const existing = demo.clips.find((clip) => clip.kind !== 'image' && clip.content === content);
    if (existing) {
      existing.updatedAt = Date.now();
      existing.keepForever ||= keepForever;
      if (title.trim()) existing.title = title.trim();
      if (groupId !== null) existing.groupId = groupId;
      Object.assign(existing, analyzeDemoClip(existing, demoRules)); pruneDemo(); changed(); return structuredClone(existing);
    }
    const clip: Clip = { id: id(), content, title, groupId, kind: classify(content), language: null, pinned: false, keepForever, createdAt: Date.now(), updatedAt: Date.now(), copyCount: 0 };
    const scanned = analyzeDemoClip(clip, demoRules); demo.clips.unshift(scanned); pruneDemo(); changed(); return structuredClone(scanned);
  },
  async updateClip(id: number, title: string, groupId: number | null, pinned: boolean): Promise<void> {
    if (isDesktop) return invoke('update_clip', { id, title, groupId, pinned });
    const clip = requiredClip(id); Object.assign(clip, analyzeDemoClip({ ...clip, title: title.trim(), groupId, pinned }, demoRules)); changed();
  },
  async deleteClip(id: number): Promise<void> { if (isDesktop) return invoke('delete_clip', { id }); if (requiredClip(id).keepForever) throw new Error('请先取消这条记录的永久保留，再删除。'); demo.clips = demo.clips.filter((clip) => clip.id !== id); pendingCleanup = null; changed(); },
  async copyClip(id: number, plain = false): Promise<void> {
    if (isDesktop) return invoke('copy_clip', plain ? { id, plain } : { id });
    const clip = requiredClip(id);
    if (clip.kind === 'image') {
      if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') throw new Error('当前浏览器不支持复制图片，请使用桌面版。');
      const blob = new Blob([Uint8Array.from(atob(demoImage.split(',')[1]!), char => char.charCodeAt(0))], { type: 'image/png' });
      try { await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]); }
      catch { throw new Error('图片未能复制，请允许剪贴板权限，或使用桌面版。'); }
    } else await navigator.clipboard.writeText(clip.content);
    clip.copyCount++; changed();
  },
  async createGroup(name: string, color: string): Promise<Group> {
    if (isDesktop) return invoke('create_group', { name, color });
    if (!name.trim() || name.length > 40 || !/^#[\da-f]{6}$/i.test(color)) throw new Error('分组名称或颜色无效');
    if (demo.groups.some((group) => group.name === name.trim())) throw new Error('分组名称已存在');
    const group = { id: Math.max(0, ...demo.groups.map((g) => g.id)) + 1, name: name.trim(), color: color.trim() };
    demo.groups.push(group); changed(); return structuredClone(group);
  },
  async deleteGroup(id: number): Promise<void> { if (isDesktop) return invoke('delete_group', { id }); demo.groups = demo.groups.filter((g) => g.id !== id); demo.clips.forEach((c) => { if (c.groupId === id) c.groupId = null; }); changed(); },
  async setTheme(theme: Settings['theme']): Promise<Settings> {
    if (isDesktop) return invoke('set_theme', { theme });
    if (!['dark', 'light', 'white', 'forest', 'rose', 'system'].includes(theme)) throw new Error('主题无效，请重新选择。');
    demo.settings.theme = theme; changed(); return { ...demo.settings };
  },
  async saveSettings(settings: Settings, expectedPauseRevision = settings.pauseRevision): Promise<void> {
    if (isDesktop) return invoke('save_settings', { settings, expectedPauseRevision });
    if (typeof settings.paused !== 'boolean' || typeof settings.alwaysOnTop !== 'boolean' || typeof settings.showStartup !== 'boolean'
      || !['dark', 'light', 'white', 'forest', 'rose', 'system'].includes(settings.theme) || !['grid', 'list', 'horizontal'].includes(settings.layout)
      || !['auto', 'none', 'cosmos', 'cats', 'puppies', 'garden'].includes(settings.stickerPack)
      || !Number.isSafeInteger(settings.maxHistory) || (settings.maxHistory !== 0 && (settings.maxHistory < 50 || settings.maxHistory > 10000))
      || ![0, 1, 7, 30, 90, 180, 365].includes(settings.retentionDays)
      || typeof settings.shortcut !== 'string' || !settings.shortcut.trim() || settings.shortcut.length > 80) {
      throw new Error('设置无效：请检查布局、历史上限、保留时长与快捷键。');
    }
    const paused = expectedPauseRevision !== demo.settings.pauseRevision ? demo.settings.paused : settings.paused;
    const pauseRevision = demo.settings.pauseRevision + Number(paused !== demo.settings.paused);
    demo.settings = { ...settings, paused, pauseRevision }; pruneDemo(); changed();
  },
  async clearHistory(): Promise<void> { if (isDesktop) return invoke('clear_history'); demo.clips = demo.clips.filter((clip) => clip.pinned || clip.keepForever); pendingCleanup = null; changed(); },
  async exportBackup(): Promise<string> {
    if (isDesktop) return invoke('export_backup');
    if (demo.clips.some(clip => clip.kind === 'image')) throw new Error('浏览器演示暂不支持包含图片的备份，请在桌面版导出完整备份。没有导出任何记录。');
    const data = JSON.stringify({ format: 'starclip', version: 3, exportedAt: Date.now(), clips: demo.clips, groups: demo.groups, privacyRules: demoRules }, null, 2);
    if (new TextEncoder().encode(data).length > 20 * 1024 * 1024 || demo.clips.length > 10000 || demo.groups.length > 1000) throw new Error('数据超过单个备份支持的范围（20 MB / 10000 条 / 1000 组）');
    return data;
  },
  async importBackup(data: string): Promise<number> {
    if (isDesktop) return invoke('import_backup', { data });
    if (new TextEncoder().encode(data).length > 20 * 1024 * 1024) throw new Error('备份不能超过 20 MB');
    const parsed: unknown = JSON.parse(data);
    if (!parsed || typeof parsed !== 'object') throw new Error('备份格式无效');
    const backup = parsed as { format?: unknown; version?: unknown; clips?: unknown; groups?: unknown; privacyRules?: PrivacyRules };
    if (backup.version === 2 || (Array.isArray(backup.clips) && backup.clips.some(clip => clip && typeof clip === 'object' && (clip.kind === 'image' || clip.image != null))) || ('images' in backup && backup.images != null)) throw new Error('浏览器演示暂不支持图片备份，请在桌面版导入。现有记录未改动。');
    if (backup.format !== 'starclip' || ![1, 3].includes(backup.version as number) || !Array.isArray(backup.clips) || !Array.isArray(backup.groups) || backup.clips.length > 10000 || backup.groups.length > 1000) throw new Error('不支持的备份格式或条目过多');
    const importedRules = backup.version === 3 ? backup.privacyRules?.rules ?? [] : [];
    if (!Array.isArray(importedRules) || demoRules.rules.length + importedRules.length > 100 || importedRules.some(rule => !rule || typeof rule.name !== 'string' || !rule.name.trim() || rule.name.length > 60 || typeof rule.value !== 'string' || !rule.value.trim() || rule.value.length > 200 || !['keyword', 'domain'].includes(rule.condition) || typeof rule.markPrivate !== 'boolean' || (rule.tag != null && (typeof rule.tag !== 'string' || rule.tag.length > 40)))) throw new Error('备份规则无效或超过 100 条，现有记录未改动。');
    const next = structuredClone(demo), groupMap = new Map<number, number>();
    for (const group of backup.groups as Group[]) {
      if (!group || !Number.isSafeInteger(group.id) || typeof group.name !== 'string' || !group.name.trim() || group.name.length > 40 || !/^#[\da-f]{6}$/i.test(group.color)) throw new Error('备份分组无效');
      let match = next.groups.find((g) => g.name === group.name);
      if (!match) { match = { ...group, id: Math.max(0, ...next.groups.map((g) => g.id)) + 1 }; next.groups.push(match); }
      groupMap.set(group.id, match.id);
    }
    let added = 0;
    for (const clip of backup.clips as Clip[]) {
      if (!clip || typeof clip.content !== 'string' || typeof clip.title !== 'string' || clip.title.length > 200 || typeof clip.pinned !== 'boolean' || !Number.isFinite(clip.createdAt) || !Number.isFinite(clip.updatedAt) || !Number.isSafeInteger(clip.copyCount) || clip.copyCount < 0) throw new Error('备份条目无效');
      validateContent(clip.content);
      for (const tags of [clip.tags, clip.manualTags, clip.privacyReasons]) if (tags !== undefined && (!Array.isArray(tags) || tags.length > 256 || tags.some(tag => typeof tag !== 'string' || tag.length > 200))) throw new Error('备份标签或隐私原因无效');
      for (const flag of [clip.privacyManual, clip.isPrivate]) if (flag !== undefined && typeof flag !== 'boolean') throw new Error('备份隐私状态无效');
      const importedTags = backup.version === 3 ? clip.manualTags ?? [] : clip.tags ?? [];
      const privacy = { privacyManual: !!clip.privacyManual || (!!clip.isPrivate && !clip.privacyReasons?.length), privacyReasons: clip.privacyReasons ?? [], manualTags: importedTags };
      if (clip.keepForever !== undefined && typeof clip.keepForever !== 'boolean') throw new Error('备份保留标记无效');
      if (clip.language != null && !languages.some(([key]) => key === clip.language)) throw new Error('未知的代码语言');
      if (clip.groupId != null && !groupMap.has(clip.groupId)) throw new Error('分组引用无效');
      const existing = next.clips.find(c => c.kind !== 'image' && c.content === clip.content);
      if (existing) {
        existing.privacyManual ||= privacy.privacyManual;
        existing.privacyReasons = [...new Set([...(existing.privacyReasons ?? []), ...privacy.privacyReasons])];
        existing.manualTags = [...new Set([...(existing.manualTags ?? existing.tags ?? []), ...importedTags])];
        existing.keepForever ||= clip.keepForever ?? false;
        existing.pinned ||= clip.pinned;
        existing.groupId ??= clip.groupId == null ? null : groupMap.get(clip.groupId)!;
        if (!existing.title) existing.title = clip.title;
        existing.language ??= clip.language ?? null;
        if (existing.language) existing.kind = 'code';
        continue;
      }
      next.clips.push({ ...clip, ...privacy, keepForever: clip.keepForever ?? false, id: Math.max(0, ...next.clips.map((c) => c.id)) + 1, groupId: clip.groupId == null ? null : groupMap.get(clip.groupId)!, language: clip.language ?? null, kind: clip.language ? 'code' : classify(clip.content) }); added++;
    }
    // Import preserves the complete backup; only explicit capture/settings changes apply retention.
    next.clips = next.clips.map(clip => analyzeDemoClip(clip, demoRules));
    if (importedRules.length) demoRules = { revision: demoRules.revision + 1, rules: [...demoRules.rules, ...importedRules.map((rule, index) => ({ ...rule, id: `import-${Date.now()}-${index}`, enabled: false, tag: rule.tag?.trim() || null }))] };
    demo = next; pendingCleanup = null; demoScan = null; changed(); return added;
  },
  async saveBackup(onWriting?: () => void): Promise<string | null> {
    if (isDesktop) {
      const stop = onWriting ? await (await import('@tauri-apps/api/event')).listen('backup-writing', onWriting) : undefined;
      try { return await invoke<string | null>('save_backup'); }
      finally { stop?.(); }
    }
    const contents = await api.exportBackup();
    const filename = `starclip-${new Date().toISOString().slice(0, 10)}.json`;
    onWriting?.();
    download(new Blob([contents], { type: 'application/json' }), filename);
    return filename;
  },
  async saveCode(id: number | null, title: string, content: string, language: string, groupId: number | null): Promise<Clip> {
    if (isDesktop) return invoke('save_code', { id, title, content, language, groupId });
    if (id != null && requiredClip(id).kind === 'image') throw new Error('图片不能带入星绘坊，请选择文字或代码。');
    validateContent(content);
    if (!languages.some(([key]) => key === language)) throw new Error('未知的代码语言');
    const conflict = demo.clips.find((clip) => clip.content === content && clip.id !== id);
    if (conflict) throw new Error('相同内容已存在，请编辑已有记录');
    const actual: Clip = id == null
      ? { id: Math.max(0, ...demo.clips.map(clip => clip.id)) + 1, content, title: title.trim(), groupId, kind: 'code', language, pinned: true, keepForever: false, createdAt: Date.now(), updatedAt: Date.now(), copyCount: 0 }
      : requiredClip(id);
    Object.assign(actual, { title: title.trim(), content, language, kind: 'code', groupId, pinned: true, updatedAt: Date.now() });
    Object.assign(actual, analyzeDemoClip(actual, demoRules));
    if (id == null) demo.clips.unshift(actual);
    pruneDemo(); changed(); return structuredClone(actual);
  },
  async saveExport(data: string, format: 'md' | 'png', filename: string): Promise<boolean> {
    if (isDesktop) return invoke('save_export', { data, format, filename });
    const blob = format === 'md' ? new Blob([data], { type: 'text/markdown;charset=utf-8' }) : new Blob([Uint8Array.from(atob(data), (c) => c.charCodeAt(0))], { type: 'image/png' });
    download(blob, filename); return true;
  },
  async hideWindow(release = false, requestId?: number, returnFocus = false): Promise<{ requestId: number; pending: boolean }> {
    if (isDesktop) return invoke('hide_window', returnFocus ? { release, requestId, returnFocus } : { release, requestId });
    return { requestId: requestId ?? 0, pending: false };
  },
  async getWindowCloseRequest(): Promise<number | null> {
    if (isDesktop) return invoke('get_main_window_close_request');
    return null;
  },
  async onWindowCloseRequested(callback: (requestId: number) => void): Promise<() => void> {
    if (!isDesktop) return () => {};
    return (await import('@tauri-apps/api/event')).listen<{ requestId: number }>('main-window-close-requested', event => callback(event.payload.requestId));
  },
  async onChange(callback: () => void): Promise<() => void> {
    if (isDesktop) return (await import('@tauri-apps/api/event')).listen('clips-changed', callback);
    listeners.add(callback); return () => { listeners.delete(callback); };
  },
};
