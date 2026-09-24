import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');

test('Writer diagnostics footer is bottom-mounted, collapsed by default, and tabbed', () => {
  assert.match(html, /id="buildFooter" class="build-footer" data-expanded="false"/);
  assert.match(html, /id="buildProblemsTab"/);
  assert.match(html, /id="buildLogTab"/);
  assert.match(css, /\.build-footer\s*\{[^}]*flex: 0 0 38px/);
  assert.match(css, /\.build-footer\[data-expanded="true"\][^}]*35vh/);
  assert.match(css, /\.writer-shell \.build-footer\[data-expanded="true"\][^}]*35vh/);
  assert.match(writer, /expandBuildFooter\('problems'\)/);
  assert.match(writer, /api\.buildLog\(/);
  assert.match(writer, /model\.buildFooterTab === 'log'[\s\S]*await loadBuildLog\(\)/);
  assert.match(writer, /buildLogRequest/);
});

test('build outcomes update diagnostics without overriding the user-controlled footer state', () => {
  assert.match(writer, /Compilation failed/);
  assert.match(writer, /shortBuildState\(build\)/);
  assert.match(writer, /LAST_GOOD_PDF_FAILURE/);
  assert.match(writer, /buildIsStale\(build\)/);
  assert.match(css, /\.build-problem-error[^}]*var\(--danger\)/);
  assert.match(css, /\.build-problem-warning[^}]*#bd771f/);
  assert.match(css, /\.build-log-text[^}]*ui-monospace[^}]*white-space: pre/);
  assert.match(writer, /renderBuildLog\(ui\.buildLogText/);
  assert.match(css, /\.build-log-problem[^}]*var\(--danger\)/);
  const refresh = writer.slice(writer.indexOf('async function refreshBuildStatus'), writer.indexOf('async function refreshHistory'));
  assert.doesNotMatch(refresh, /expandBuildFooter/);
});

test('Writer exposes one stateful Comments toolbar action backed by the existing review drawer', () => {
  assert.equal((html.match(/id="commentsToggle"/g) || []).length, 1);
  assert.match(html, /id="commentsToggle"[^>]*aria-label="Comments"[^>]*aria-controls="workspaceDrawer"[^>]*aria-expanded="false"/);
  assert.match(html, /id="commentsToggle"[\s\S]*?<span>Comments<\/span>/);
  assert.match(writer, /ui\.commentsToggle\.setAttribute\('aria-expanded'/);
  assert.match(writer, /!ui\.workspaceDrawer\.hidden && !ui\.reviews\.hidden/);
});
