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
  assert.match(writer, /expandBuildFooter\('problems'\)/);
  assert.match(writer, /api\.buildLog\(/);
});

test('failed builds expand Problems while successful builds use short states', () => {
  assert.match(writer, /Compilation failed/);
  assert.match(writer, /expandBuildFooter\('problems'\)/);
  assert.match(writer, /shortBuildState\(build\)/);
  assert.match(writer, /LAST_GOOD_PDF_FAILURE/);
  assert.match(writer, /buildIsStale\(build\)/);
  assert.match(css, /\.build-problem-error[^}]*var\(--danger\)/);
  assert.match(css, /\.build-problem-warning[^}]*#bd771f/);
  assert.match(css, /\.build-log-text[^}]*ui-monospace[^}]*white-space: pre/);
  assert.match(writer, /renderBuildLog\(ui\.buildLogText/);
  assert.match(css, /\.build-log-problem[^}]*var\(--danger\)/);
});
