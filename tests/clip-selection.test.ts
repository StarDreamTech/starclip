import test from 'node:test';
import assert from 'node:assert/strict';
import { selectClip } from '../src/clip-selection.ts';

test('image selection toggles, ranges on the loaded page, retains other pages and rejects overflow atomically', () => {
  const page = [8, 7, 6, 5];
  assert.deepEqual(selectClip([2], 7, page, null), [2, 7]);
  assert.deepEqual(selectClip([2, 7], 7, page, 7), [2]);
  assert.deepEqual(selectClip([2, 7], 5, page, 7, true), [2, 7, 6, 5]);
  assert.deepEqual(selectClip([2, 5], 7, page, 5, true), [2, 5, 7, 6]);
  assert.deepEqual(selectClip([2], 7, page, 2, true), [2, 7]);
  const full = Array.from({ length: 50 }, (_, i) => i + 10);
  assert.throws(() => selectClip(full, 1, page, null), /50/);
  assert.equal(full.length, 50);
  assert.equal(selectClip(full, 10, page, null).length, 49);
});
