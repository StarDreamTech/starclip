import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown, checkMarkdownImageSize } from '../src/markdown-render.ts';
test('renders article structures while retaining literal code', () => {
 const html = renderMarkdown('# 星空\n\n> 星光\n\n- 第一颗星\n\n| 名称 | 数量 |\n| --- | --- |\n| 星 | 2 |\n\n```js\nconst x = "<tag>";\n```');
 for (const tag of ['h1','blockquote','ul','table','pre']) assert.ok(html.includes(`<${tag}`));
 assert.ok(html.includes('&lt;tag&gt;'));
});
test('does not emit active HTML, links or images', () => {
 const html = renderMarkdown('<script>alert(1)</script>\n\n![x](https://example.com/pixel.png)\n\n[click](https://example.com)');
 assert.ok(!/<(?:script|img|a)\b/.test(html));
 assert.ok(!html.includes('src='));
 assert.ok(html.includes('&lt;script&gt;'));
});
test('rejects oversized documents and raster dimensions', () => {
 assert.throws(() => renderMarkdown('x'.repeat(30001)));
 assert.throws(() => checkMarkdownImageSize(960, 9000));
 assert.throws(() => checkMarkdownImageSize(960, NaN));
 assert.doesNotThrow(() => checkMarkdownImageSize(720, 2000));
});
