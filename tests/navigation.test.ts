import test from 'node:test';
import assert from 'node:assert/strict';
import { loadNavigation, saveNavigation } from '../src/navigation.ts';

test('navigation cache validates old or damaged values and tolerates unavailable storage', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const defaults = { filter: 'all', page: 1, settingsTab: 'appearance', settingsOpen: false };
  let stored: string | null = null;
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: () => stored,
    setItem: (_key: string, value: string) => { stored = value; },
  } });
  try {
    for (const value of [null, '{broken', 'null', '[]', '42', '{}']) {
      stored = value;
      assert.deepEqual(loadNavigation(), defaults);
    }
    for (const filter of ['all', 'pinned', 'kept', 'link', 'image', 'file', 'code', 'group-42']) {
      const location = { ...defaults, filter, page: 3, settingsTab: 'stats' as const, settingsOpen: true };
      saveNavigation({ ...location, query: '原创临时搜索', content: '原创草稿' } as typeof location);
      assert.deepEqual(JSON.parse(stored!), location, 'only navigation fields are persisted');
      assert.deepEqual(loadNavigation(), location);
    }
    for (const filter of ['removed', 'group-0', 'group--1', 'group-1e2', 'group-9007199254740992', {}, null]) {
      stored = JSON.stringify({ filter, page: 3, settingsTab: 'removed', settingsOpen: 'true' });
      assert.deepEqual(loadNavigation(), defaults);
    }
    for (const page of [-1, 0, 1.5, '2', null, 0x100000000]) {
      stored = JSON.stringify({ ...defaults, filter: 'image', page });
      assert.deepEqual(loadNavigation(), { ...defaults, filter: 'image' });
    }
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('fixed storage failure'); } });
    assert.deepEqual(loadNavigation(), defaults);
    assert.doesNotThrow(() => saveNavigation(defaults));
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  }
});
