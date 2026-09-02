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

test('data import supports multi-file drag/drop and Add Edit Delete batches', () => {
  for (const operation of ['ADD', 'EDIT', 'DELETE']) assert.match(js, new RegExp(`value="${operation}"`));
  for (const event of ['dragenter', 'dragover', 'dragleave', 'drop']) assert.match(js, new RegExp(`'${event}'`));
  assert.match(js, /fileInput\.multiple = true/);
  assert.match(js, /files\[\]/);
  assert.match(js, /Review changes/);
  assert.match(js, /What data is this\?/);
  assert.match(js, /Technical details/);
  assert.doesNotMatch(js, /Legacy single-file imports/);
  assert.doesNotMatch(js, /Internal mode/);
  assert.match(js, /import-batches\/validate/);
  assert.doesNotMatch(js, /Select the CSV target table; unknown filenames are never guessed/);
  assert.match(css, /\.import-dropzone/);
});

test('data import guide documents Team creation without exposing legacy history', () => {
  assert.match(js, /aria-label', 'What data do I need\?'/);
  assert.match(js, /title', 'What data do I need\?'/);
  assert.match(js, /showModal\(\)/);
  assert.match(js, /event\.target === guide/);

  for (const dataset of [
    'departments',
    'faculty',
    'programmes',
    'students',
    'paper_teams',
    'paper_team_writers',
    'paper_team_mentors',
  ]) assert.match(js, new RegExp(`\\['${dataset}',`));

  for (const field of [
    'department_id',
    'faculty_id', 'name', 'email', 'dept_id', 'honorific', 'designation', 'status',
    'programme_code', 'hod_id',
    'reg_no',
    'external_team_key', 'team_name', 'academic_year', 'semester',
    'student_reg_no', 'writer_order', 'is_leader',
  ]) assert.match(js, new RegExp(`['"]${field}['"]`));

  assert.match(js, /Student\.email must resolve to an existing V2 WRITER account/);
  assert.match(js, /does NOT create a privileged Writer account/);
  assert.match(js, /Faculty\.email must resolve to an existing V2 MENTOR account/);
  assert.match(js, /Exactly one paper_team_writers row per Team must be marked is_leader = true/);
  assert.match(js, /programme mapping is available, the configured global fallback template is used/);
  assert.match(js, /Existing Team template pins do not silently change later/);
  assert.match(js, /Additional institutional data \(optional\)/);
  assert.match(js, /optional for basic Paper Team creation/);
  assert.match(css, /\.import-guide-dialog/);
});

test('institution data manager uses server pagination and authoritative manual operations', () => {
  for (const dataset of ['students', 'faculty', 'programmes', 'departments', 'schools', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles', 'paper_teams']) assert.match(js, new RegExp(dataset));
  assert.match(js, /institution\/data\/\$\{config\.dataset\}/);
  assert.match(js, /manualInstitutionRecord/);
  assert.match(js, /Check dependencies/);
  assert.match(js, /Add Writer/);
  assert.match(js, /Add Mentor/);
  assert.match(js, /Materialized →/);
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
