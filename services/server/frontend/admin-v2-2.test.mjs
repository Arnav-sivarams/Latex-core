import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const html = readFileSync(new URL('../src/admin.html', import.meta.url), 'utf8');
const js = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');

test('Admin navigation exposes the scalable V2.2 control plane', () => {
  for (const section of ['INSTITUTION DATA', 'IMPORTS', 'PAPER TEAMS', 'PROGRAMME TEMPLATES']) {
    assert.match(html, new RegExp(section));
  }
  assert.doesNotMatch(html, /RESEARCH GROUPS/);
});

test('long overview values wrap inside adaptive metric cards', () => {
  assert.match(css, /\.admin-metrics\s*\{[^}]*min-width:\s*0[^}]*grid-template-columns:\s*repeat\(auto-fit/s);
  assert.match(css, /\.admin-metric\s*\{[^}]*min-width:\s*0/s);
  assert.match(css, /\.admin-metric strong\s*\{[^}]*overflow-wrap:\s*anywhere[^}]*word-break:\s*break-word/s);
});

test('import wizard maps all modes, switches CSV targets, and bounds error rendering', () => {
  for (const mode of ['VALIDATE_ONLY', 'MERGE', 'ADD_ONLY']) assert.match(js, new RegExp(mode));
  assert.match(js, /Import More — Add Only/);
  assert.match(js, /target\.hidden = xlsx/);
  assert.match(js, /Download errors\.csv/);
  assert.match(js, /errors\.slice\(0, 100\)/);
  assert.match(js, /job\.mode === 'VALIDATE_ONLY'/);
});

test('institution people and Team grid use server pagination and filters', () => {
  assert.match(js, /institution\/students|tab\.toLowerCase\(\)/);
  assert.match(js, /paper-teams\/query\?\$\{queryString\(state\)\}/);
  assert.match(js, /\[25, 50, 100\]/);
  assert.match(js, /programme_code/);
  assert.match(js, /mentor_user_id/);
  assert.match(js, /leader_user_id/);
  assert.match(js, /review_state/);
  assert.match(js, /unresolved/);
  assert.doesNotMatch(js, /api\('\/api\/admin\/v2\/paper-teams'\)/);
  assert.doesNotMatch(js, /team-card/);
});

test('manual Team and safe template override workflows remain explicit', () => {
  assert.match(js, /Create Team Manually/);
  assert.match(js, /ordered_writer_user_ids/);
  assert.match(js, /Leader must be one of the selected Writers/);
  assert.match(js, /MANUAL_OVERRIDE/);
  assert.match(js, /Preview Template Change/);
  assert.match(js, /preview_token/);
  assert.match(js, /PRE_TEMPLATE_CHANGE/);
});
