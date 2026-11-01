import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';

test('all nine original papers have pinned sources and lightweight bundled previews', () => {
  const root = new URL('../public/handwriting-papers/', import.meta.url);
  const catalog = JSON.parse(readFileSync(new URL('catalog.json', root), 'utf8'));
  assert.equal(catalog.length, 9);
  assert.equal(new Set(catalog.map(item => item.id)).size, 9);
  assert.deepEqual(new Set(catalog.map(item => item.file)), new Set([
    'grid.jpeg', 'letter.png', 'my.jpg', '村庄信笺纸.jpg', '树信笺纸.jpg',
    '#000000-1000x1000.png', '#000000-200x200.png', '#000000-500x500.png', '#ff55ff-500x500.png',
  ]));
  let bytes = 0;
  for (const paper of catalog) {
    const source = new URL(paper.url);
    assert.equal(source.origin, 'https://raw.githubusercontent.com');
    assert.equal(source.hash, '');
    assert.ok(source.pathname.startsWith('/cpython666/HandwritingGenerator/d7036998501e9de3dbe57af4aaa1b7d289547ec8/bgs/'));
    assert.equal(decodeURIComponent(source.pathname.split('/').at(-1)!), paper.file);
    assert.match(paper.sha256, /^[a-f0-9]{64}$/);
    assert.ok(paper.width * paper.height <= 8_000_000);
    assert.equal(paper.previewUrl, `/handwriting-papers/${paper.id}.png`);
    const preview = readFileSync(new URL(`${paper.id}.png`, root));
    assert.equal(preview.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
    assert.ok(preview.readUInt32BE(16) <= 240 && preview.readUInt32BE(20) <= 300);
    bytes += preview.length;
    const [left, top, right, bottom] = paper.insets;
    assert.ok(left + right + paper.padding * 2 < paper.width);
    assert.ok(top + bottom + paper.padding * 2 < paper.height);
  }
  assert.equal(catalog.filter(item => item.solid).length, 4);
  assert.ok(bytes < 128 * 1024);
  assert.equal(readdirSync(root).some(file => file.endsWith('.jpg') || file.endsWith('.jpeg') || file.endsWith('.img')), false, 'original paper bitmaps are not bundled');
});
