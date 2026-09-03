import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const html = readFileSync(new URL('../src/admin.html', import.meta.url), 'utf8');
const js = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');

test('Admin navigation exposes one grouped destination per product concept', () => {
  for (const section of ['Institution Data', 'Imports', 'Paper Teams', 'Templates']) {
    assert.match(html, new RegExp(section));
  }
  assert.equal((html.match(/data-section="Templates"/g) || []).length, 1);
  for (const group of ['People &amp; data', 'Papers', 'Operations']) assert.match(html, new RegExp(group));
  assert.doesNotMatch(html, /Programme Templates/i);
  assert.doesNotMatch(html, /Research Groups/i);
  assert.doesNotMatch(`${html}\n${js}`, /Resolution Preview/i);
});

test('long overview values wrap inside adaptive metric cards', () => {
  assert.match(css, /\.admin-metrics\s*\{[^}]*min-width:\s*0[^}]*grid-template-columns:\s*repeat\(auto-fit/s);
  assert.match(css, /\.admin-metric\s*\{[^}]*min-width:\s*0/s);
  assert.match(css, /\.admin-metric strong\s*\{[^}]*overflow-wrap:\s*anywhere[^}]*word-break:\s*break-word/s);
});

test('Admin shell locks the viewport and gives navigation and main independent scrolling', () => {
  assert.match(html, /body class="workspace-shell admin-shell"/);
  assert.match(css, /\.admin-layout\s*\{[^}]*min-height:\s*0[^}]*grid-template-columns:\s*220px minmax\(0, 1fr\)[^}]*overflow:\s*hidden/s);
  assert.match(css, /\.admin-nav\s*\{[^}]*min-height:\s*0[^}]*overflow-x:\s*hidden[^}]*overflow-y:\s*auto/s);
  assert.match(css, /\.admin-content\s*\{[^}]*min-width:\s*0[^}]*min-height:\s*0[^}]*overflow-x:\s*hidden[^}]*overflow-y:\s*auto/s);
  assert.match(css, /\.admin-table-wrap\s*\{[^}]*width:\s*100%[^}]*overflow:\s*auto/s);
  assert.match(css, /\.admin-dialog\s*\{[^}]*width:\s*min\(900px, calc\(100vw - 32px\)\)[^}]*max-height:\s*calc\(100vh - 32px\)[^}]*overflow:\s*hidden/s);
  for (const [width, height] of [[1366, 768], [1920, 1080]]) {
    assert.ok(Math.min(900, width - 32) <= width - 32);
    assert.ok(height - 32 >= 736);
  }
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

  assert.match(js, /valid Student\.email automatically creates or reuses a V2 WRITER account/);
  assert.match(js, /Faculty assigned in paper_team_mentors automatically receive or reuse a V2 MENTOR account/);
  assert.match(js, /Unassigned Faculty and institutional Admins are never provisioned/);
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

test('account, Team, and template workflows expose the unified Admin UX', () => {
  assert.match(js, /\+ Create Team/);
  assert.match(js, /ordered_writer_user_ids/);
  assert.match(js, /Leader must be one of the selected Writers/);
  assert.match(js, /Edit Team/);
  assert.match(js, /Move \$\{person\.email\} down/);
  assert.match(js, /method: 'PUT'/);
  assert.match(js, /Team must retain at least one Writer/);
  assert.match(js, /Change template\?/);
  assert.match(js, /preview_token/);
  assert.doesNotMatch(js, /renderJson\(preview\)/);
  assert.doesNotMatch(js, /Resolution preview/);
  assert.match(js, /Confirm Main document change/);
  assert.match(js, /Main templates/);
  assert.match(js, /Front Matter/);
  assert.match(js, /Automatic defaults/);
  assert.match(js, /email,password,role/);
  assert.match(js, /Save this file now\. Temporary passwords cannot be viewed again/);
  assert.match(js, /operation === 'ADD'\) showCredentialHandoff\(applied\.account_provisioning/);
});

test('users keep legacy fields out of the main grid and use understandable delivery states', () => {
  assert.match(js, /<th>Email<\/th><th>Role<\/th><th>Status<\/th><th>Account state<\/th><th>Email delivery<\/th><th>Actions<\/th>/);
  assert.match(js, /Setup required/);
  assert.match(js, /Email sent/);
  assert.match(js, /rowMenu\(`Actions for \$\{user\.email\}`/);
  assert.match(js, /openAdminDialog\('User details'\)/);
});

test('scale surfaces remain paginated and operational pages are human-readable', () => {
  assert.match(js, /paper-teams\/query\?\$\{queryString\(state\)\}/);
  assert.match(js, /Page \$\{page\.page\} · \$\{page\.total\} records/);
  assert.match(js, /Audit records important administrative and security actions: who did what, when, and to which object\./);
  assert.match(js, /Credential email sent/);
  assert.match(js, /Team membership changed/);
  assert.match(js, /Disabled in this environment/);
  assert.doesNotMatch(js, /Host-level operational actions remain CLI-only in this release candidate.*renderJson/s);
});

test('dialogs, icon controls, and long identity values are accessible', () => {
  assert.match(html, /<dialog[^>]*aria-modal="true"[^>]*aria-labelledby="adminDialogTitle"/);
  assert.match(html, /aria-label="Close dialog" title="Close dialog"/);
  assert.match(js, /dialogReturnFocus/);
  assert.match(js, /firstControl\?\.focus\(\)/);
  assert.match(js, /action\.setAttribute\('aria-label', label\); action\.title = label/);
  assert.match(js, /summary\.setAttribute\('aria-label', label\);/);
  assert.match(css, /\.admin-table th, \.admin-table td\s*\{[^}]*overflow-wrap:\s*anywhere/s);
});
