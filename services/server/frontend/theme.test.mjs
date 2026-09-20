import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const theme = readFileSync(new URL('../static/theme.js', import.meta.url), 'utf8');
const shells = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
const writerHtml = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');

test('application theme uses one persisted root and synchronizes the editor preference', () => {
  assert.match(theme, /localStorage\.getItem\(storageKey\)/);
  assert.match(theme, /document\.documentElement\.dataset\.theme/);
  assert.match(theme, /fetch\('\/api\/v2\/preferences\/editor'/);
  assert.match(shells, /html\[data-theme="dark"\]/);
  assert.match(writerHtml, /\/static\/theme\.js/);
});

test('control primitives keep text natural and icon controls square', () => {
  assert.match(shells, /\.ui-button[^}]*width: auto[^}]*min-width: max-content[^}]*min-height: 38px[^}]*height: 38px/);
  assert.match(shells, /\.ui-icon-button, \.icon-button, \.icon-link, \.mini-icon[^}]*width: 38px[^}]*min-width: 38px[^}]*height: 38px/);
  assert.match(shells, /white-space: nowrap/);
  assert.doesNotMatch(shells, /button,\s*a\s*\{/);
});

test('status labels are not assigned control primitives', () => {
  assert.match(writerHtml, /id="buildStatus" class="toolbar-status"/);
  assert.doesNotMatch(writerHtml, /id="buildStatus" class="[^\"]*ui-button/);
});
