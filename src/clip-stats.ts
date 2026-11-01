// 星迹统计：本机剪贴活动计数。只有次数、大小与本地时间分布，没有正文或记录 ID。
export const clipStatsDesktopAvailable = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
export const statKinds = ['text', 'link', 'code', 'color', 'image', 'file'] as const;
export type StatKind = typeof statKinds[number];
export const statRanges = [7, 30, 90, 365] as const;
export type KindActivity = { new: number; repeat: number; reuse: number; size: number };
export type DailyActivity = { day: number; captures: Partial<Record<StatKind, number>>; new: number; repeat: number; reuse: number };
export type LibraryStats = {
  total: number; kinds: Partial<Record<StatKind, number>>; pinned: number; kept: number; grouped: number; private: number;
  reusedClips: number; copyTotal: number; imageBytes: number; oldestCreatedAt: number | null;
  topHosts: Array<{ host: string; count: number }>;
};
export type ClipStats = {
  days: number; utcOffsetMinutes: number; firstDay: number; lastDay: number; recordingSince: number | null;
  daily: DailyActivity[]; hours: number[]; weekdays: number[];
  kinds: Partial<Record<StatKind, KindActivity>>; library: LibraryStats;
};
export type StatBar = { key: number; label: string; firstDay: number; lastDay: number; captures: number; reuse: number; byKind: Record<StatKind, number> };

const DAY_MS = 86_400_000;
const emptyKinds = (): Record<StatKind, number> => ({ text: 0, link: 0, code: 0, color: 0, image: 0, file: 0 });
const captures = (day: Pick<DailyActivity, 'new' | 'repeat'>) => day.new + day.repeat;

/** Offset the native side uses to group buckets into this computer's local days. */
export function localUtcOffsetMinutes(now = new Date()): number {
  const offset = -now.getTimezoneOffset();
  return Math.round(offset / 15) * 15 || 0; // avoid -0 at UTC
}

/** Local day number → a Date whose UTC fields carry that local calendar day. */
export const dayDate = (day: number) => new Date(day * DAY_MS);

export function totals(stats: ClipStats) {
  const value = stats.daily.reduce((sum, day) => ({ new: sum.new + day.new, repeat: sum.repeat + day.repeat, reuse: sum.reuse + day.reuse }), { new: 0, repeat: 0, reuse: 0 });
  const captured = value.new + value.repeat;
  const activeDays = stats.daily.filter(day => captures(day) > 0).length;
  const busiest = stats.daily.reduce<DailyActivity | null>((best, day) => (!best || captures(day) > captures(best) ? day : best), null);
  return { ...value, captures: captured, activeDays, perDay: captured / Math.max(1, stats.days), busiest: busiest && captures(busiest) > 0 ? busiest : null };
}

/** One bar per day up to 90 days, then one per week so a year stays legible. */
export function activityBars(stats: ClipStats, formatDay: (day: number) => string): StatBar[] {
  const width = stats.days > 90 ? 7 : 1;
  const byDay = new Map(stats.daily.map(day => [day.day, day]));
  const bars: StatBar[] = [];
  // Align the newest bar to today so a partial bucket can only be the oldest one.
  for (let end = stats.lastDay; end >= stats.firstDay; end -= width) {
    const start = Math.max(stats.firstDay, end - width + 1);
    const bar: StatBar = { key: start, label: width === 1 ? formatDay(start) : `${formatDay(start)}–${formatDay(end)}`, firstDay: start, lastDay: end, captures: 0, reuse: 0, byKind: emptyKinds() };
    for (let day = start; day <= end; day++) {
      const item = byDay.get(day);
      if (!item) continue;
      bar.captures += captures(item);
      bar.reuse += item.reuse;
      for (const kind of statKinds) bar.byKind[kind] += item.captures[kind] ?? 0;
    }
    bars.unshift(bar);
  }
  return bars;
}

export function kindRows(stats: ClipStats) {
  const rows = statKinds.map(kind => {
    const value = stats.kinds[kind] ?? { new: 0, repeat: 0, reuse: 0, size: 0 };
    return { kind, ...value, captures: value.new + value.repeat };
  });
  const sum = rows.reduce((total, row) => total + row.captures, 0);
  return rows.map(row => ({ ...row, share: sum ? row.captures / sum : 0 }));
}

/** Bar height as a share of the tallest value; zero stays zero, anything else is visible. */
export function barRatio(value: number, max: number): number {
  if (value <= 0 || max <= 0) return 0;
  return Math.max(0.04, value / max);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return (await import('@tauri-apps/api/core')).invoke<T>(command, args);
}

export async function getClipStats(days: number, utcOffsetMinutes = localUtcOffsetMinutes()): Promise<ClipStats> {
  if (!clipStatsDesktopAvailable) return demoClipStats(days, utcOffsetMinutes);
  return invoke<ClipStats>('get_clip_stats', { days, utcOffsetMinutes });
}

export async function clearClipStats(): Promise<void> {
  if (!clipStatsDesktopAvailable) { demoCleared = true; return; }
  await invoke('clear_clip_stats');
}

let demoCleared = false;
/** Browser preview: a fixed, original pattern derived only from the day number. */
export function demoClipStats(days: number, utcOffsetMinutes = 0, now = Date.now()): ClipStats {
  const lastDay = Math.floor((now + utcOffsetMinutes * 60_000) / DAY_MS);
  const firstDay = lastDay - days + 1;
  const daily: DailyActivity[] = [];
  const hours = Array<number>(24).fill(0), weekdays = Array<number>(7).fill(0);
  const kinds: Record<StatKind, KindActivity> = { text: { new: 0, repeat: 0, reuse: 0, size: 0 }, link: { new: 0, repeat: 0, reuse: 0, size: 0 }, code: { new: 0, repeat: 0, reuse: 0, size: 0 }, color: { new: 0, repeat: 0, reuse: 0, size: 0 }, image: { new: 0, repeat: 0, reuse: 0, size: 0 }, file: { new: 0, repeat: 0, reuse: 0, size: 0 } };
  if (!demoCleared) for (let day = Math.max(firstDay, lastDay - 59); day <= lastDay; day++) {
    const weekday = (day + 4) % 7;
    const weekend = weekday === 0 || weekday === 6;
    const base = weekend ? 4 : 14 + (day % 5) * 3;
    const split: Record<StatKind, number> = { text: Math.round(base * 0.45), link: Math.round(base * 0.25), code: Math.round(base * 0.18), color: day % 3 === 0 ? 1 : 0, image: Math.round(base * 0.1), file: day % 4 === 0 ? 1 : 0 };
    const entry: DailyActivity = { day, captures: {}, new: 0, repeat: 0, reuse: Math.round(base / 3) };
    for (const kind of statKinds) {
      const count = split[kind];
      if (!count) continue;
      const repeat = Math.floor(count / 4);
      entry.captures[kind] = count; entry.new += count - repeat; entry.repeat += repeat;
      kinds[kind].new += count - repeat; kinds[kind].repeat += repeat;
      kinds[kind].size += count * (kind === 'image' ? 48_000 : kind === 'code' ? 320 : 60);
      weekdays[weekday] += count;
      for (let i = 0; i < count; i++) hours[[9, 10, 11, 14, 15, 16, 17, 20, 22][(day + i) % 9]] += 1;
    }
    kinds.text.reuse += entry.reuse;
    daily.push(entry);
  }
  return {
    days, utcOffsetMinutes, firstDay, lastDay, recordingSince: demoCleared ? null : (lastDay - 59) * DAY_MS - utcOffsetMinutes * 60_000,
    daily, hours, weekdays, kinds,
    library: { total: 9, kinds: { text: 3, link: 2, code: 2, color: 1, image: 1 }, pinned: 3, kept: 0, grouped: 7, private: 0, reusedClips: 1, copyTotal: 12, imageBytes: 1147, oldestCreatedAt: now - 8 * 1_800_000, topHosts: [{ host: 'developer.mozilla.org', count: 1 }, { host: 'v2.tauri.app', count: 1 }] },
  };
}
