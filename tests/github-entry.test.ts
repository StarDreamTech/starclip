import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import { parse } from '@vue/compiler-sfc'
import { en } from '../src/i18n/en.ts'

test('main window GitHub entry reuses the safe project-link handler with an accessible translated label', () => {
  const source = fs.readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
  const template = parse(source).descriptor.template!.content
  const entry = template.match(/<a class="icon-button github-link"[^>]*>[\s\S]*?<\/a>/)?.[0]
  assert.ok(entry)
  assert.match(entry, /href="https:\/\/github.com\/cpython666\/starclip"/)
  assert.match(entry, /rel="noopener noreferrer"/)
  assert.match(entry, /:title="t\('查看开源项目（GitHub）'\)"/)
  assert.match(entry, /:aria-label="t\('查看开源项目（GitHub）'\)"/)
  assert.match(entry, /openProjectLink\(\$event, 'repository'\)/)
  assert.match(entry, /<Icon name="github"/)
  assert.equal(en['查看开源项目（GitHub）'], 'View source on GitHub')
})
