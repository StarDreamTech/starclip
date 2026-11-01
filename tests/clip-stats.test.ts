import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { activityBars, barRatio, demoClipStats, formatBytes, kindRows, localUtcOffsetMinutes, totals } from '../src/clip-stats.ts';
import type { ClipStats } from '../src/clip-stats.ts';

const fixture = (days: number): ClipStats => ({
  days, utcOffsetMinutes: 480, firstDay: 100 - days + 1, lastDay: 100, recordingSince: 0,
  daily: [
    { day: 100, captures: { text: 3, link: 1 }, new: 3, repeat: 1, reuse: 2 },
    { day: 98, captures: { image: 1 }, new: 1, repeat: 0, reuse: 0 },
    { day: 90, captures: { code: 5 }, new: 4, repeat: 1, reuse: 1 },
  ],
  hours: Array(24).fill(0), weekdays: Array(7).fill(0),
  kinds: { text: { new: 2, repeat: 1, reuse: 2, size: 30 }, link: { new: 1, repeat: 0, reuse: 0, size: 20 }, code: { new: 4, repeat: 1, reuse: 1, size: 200 }, image: { new: 1, repeat: 0, reuse: 0, size: 4096 } },
  library: { total: 0, kinds: {}, pinned: 0, kept: 0, grouped: 0, private: 0, reusedClips: 0, copyTotal: 0, imageBytes: 0, oldestCreatedAt: null, topHosts: [] },
});

test('daily bars fill empty days and end on today', () => {
  const bars = activityBars(fixture(7), day => String(day));
  assert.deepEqual(bars.map(bar => bar.key), [94, 95, 96, 97, 98, 99, 100]);
  assert.deepEqual(bars.map(bar => bar.captures), [0, 0, 0, 0, 1, 0, 4]);
  assert.equal(bars.at(-1)!.byKind.text, 3);
  assert.equal(bars.at(-1)!.reuse, 2);
});

test('long ranges group into weeks aligned to today', () => {
  const bars = activityBars(fixture(365), day => String(day));
  assert.equal(bars.length, 53);
  assert.deepEqual([bars.at(-1)!.firstDay, bars.at(-1)!.lastDay], [94, 100]);
  assert.equal(bars.at(-1)!.captures, 5);
  assert.equal(bars.at(-2)!.captures, 5, 'day 90 falls into the previous week');
  assert.equal(bars[0]!.firstDay, fixture(365).firstDay, 'the oldest bar may be partial');
  assert.equal(bars.at(-1)!.label, '94–100');
});

test('totals, kind shares and helpers', () => {
  const value = totals(fixture(30));
  assert.deepEqual([value.captures, value.new, value.repeat, value.reuse, value.activeDays], [10, 8, 2, 3, 3]);
  assert.equal(value.busiest?.day, 90);
  const rows = kindRows(fixture(30));
  assert.deepEqual(rows.map(row => row.kind), ['text', 'link', 'code', 'color', 'image', 'file']);
  assert.equal(rows.find(row => row.kind === 'code')!.captures, 5);
  assert.equal(rows.find(row => row.kind === 'color')!.share, 0);
  assert.ok(Math.abs(rows.reduce((sum, row) => sum + row.share, 0) - 1) < 1e-9);
  assert.equal(barRatio(0, 10), 0);
  assert.equal(barRatio(1, 1000), 0.04);
  assert.equal(barRatio(5, 10), 0.5);
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(2048), '2.0 KiB');
  assert.equal(localUtcOffsetMinutes(new Date(0)) % 15, 0);
});

test('browser demo has the native shape and never reads clipboard data', () => {
  const now = Date.UTC(2026, 9, 31, 4);
  const demo = demoClipStats(90, 480, now);
  assert.equal(demo.lastDay - demo.firstDay + 1, 90);
  assert.equal(demo.hours.length, 24);
  assert.equal(demo.weekdays.length, 7);
  assert.ok(demo.daily.every(day => day.day >= demo.firstDay && day.day <= demo.lastDay));
  assert.equal(demo.hours.reduce((a, b) => a + b, 0), totals(demo).captures);
  const source = fs.readFileSync('src/clip-stats.ts', 'utf8');
  assert.doesNotMatch(source, /readText|clipboard\.read|navigator\.clipboard/);
});

test('stats commands stay main-window only', () => {
  const permissions = JSON.parse(fs.readFileSync('src-tauri/permissions/windows.json', 'utf8'));
  const [main, pet] = ['main-commands', 'starpet-commands'].map(id => permissions.permission.find((item: { identifier: string }) => item.identifier === id).commands.allow as string[]);
  for (const command of ['get_clip_stats', 'clear_clip_stats']) {
    assert.ok(main.includes(command));
    assert.ok(!pet.includes(command));
  }
});
