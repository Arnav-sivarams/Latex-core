// Run against the installed app with ACCEPTANCE_ADMIN_EMAIL and
// ACCEPTANCE_ADMIN_PASSWORD_FILE. Synthetic records and temporary evidence only.
import assert from 'node:assert/strict';
import { randomUUID, randomBytes, createHash } from 'node:crypto';
import { readFile, writeFile, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { chromium } from 'playwright';
import * as pdfjs from '../../../node_modules/pdfjs-dist/legacy/build/pdf.mjs';

const execute = promisify(execFile);
const base = process.env.ACCEPTANCE_BASE_URL || 'http://localhost:8080';
const passwordFile = process.env.ACCEPTANCE_ADMIN_PASSWORD_FILE;
assert.ok(passwordFile, 'ACCEPTANCE_ADMIN_PASSWORD_FILE is required');
const baseline = process.env.ACCEPTANCE_BASELINE === '1';
const temp = await mkdtemp(join(tmpdir(), 'latex-core-professor-acceptance-'));
const suffix = randomBytes(5).toString('hex');
const password = randomBytes(24).toString('base64url');
const evidence = { baseline, suffix, builds: [], checks: [] };
const contexts = [];
const browserErrors = [];
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || chromium.executablePath() });
function check(value, label) { assert.ok(value, label); evidence.checks.push(label); }
async function request(cookie, path, { method = 'GET', json, body, expected = 200 } = {}) {
  const response = await fetch(`${base}${path}`, { method, headers: { Cookie: cookie, ...(json === undefined ? {} : { 'Content-Type': 'application/json' }) }, body: json === undefined ? body : JSON.stringify(json) });
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.equal(response.status, expected, `${path}: ${bytes.toString().slice(0, 1500)}`);
  return response.headers.get('content-type')?.includes('json') ? JSON.parse(bytes) : bytes;
}
async function login(email, secret) {
  const response = await fetch(`${base}/api/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ email, password: secret }) });
  assert.ok([200, 201].includes(response.status), 'login');
  return response.headers.getSetCookie().find((cookie) => cookie.startsWith('latex_core_session_v2=')).split(';')[0];
}
async function pageFor(cookie, route) {
  const context = await browser.newContext(); contexts.push(context);
  const [name, value] = cookie.split('='); await context.addCookies([{ name, value, url: base }]);
  const page = await context.newPage(); page.on('pageerror', (error) => browserErrors.push(error.message));
  await page.goto(`${base}${route}`); return page;
}
async function importFile(cookie, name, bytes, mode = 'MERGE', apply = true) {
  const form = new FormData(); form.set('mode', mode); form.set('file', new Blob([bytes]), name);
  const job = await request(cookie, '/api/admin/v2/institution/imports/validate', { method: 'POST', body: form, expected: 201 });
  assert.equal(job.error_rows, 0, `validate ${name}`);
  if (apply) await request(cookie, `/api/admin/v2/institution/imports/${job.id}/apply`, { method: 'POST', json: {} });
  return job;
}
async function workbook(rows) {
  const path = join(temp, 'departments.xlsx');
  await execute('python3', ['-c', `import sys,json,zipfile,html
rows=json.loads(sys.argv[2]); ns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'
with zipfile.ZipFile(sys.argv[1],'w') as z:
 z.writestr('[Content_Types].xml','<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
 z.writestr('_rels/.rels','<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
 z.writestr('xl/workbook.xml','<workbook xmlns="'+ns+'" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="departments" sheetId="1" r:id="rId1"/></sheets></workbook>')
 z.writestr('xl/_rels/workbook.xml.rels','<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
 data=''.join('<row r="'+str(i+1)+'">'+''.join('<c r="'+chr(65+j)+str(i+1)+'" t="inlineStr"><is><t>'+html.escape(v)+'</t></is></c>' for j,v in enumerate(row))+'</row>' for i,row in enumerate(rows))
 z.writestr('xl/worksheets/sheet1.xml','<worksheet xmlns="'+ns+'"><sheetData>'+data+'</sheetData></worksheet>')`, path, JSON.stringify(rows)]);
  return readFile(path);
}
async function editor(page) { return page.evaluate(() => document.querySelector('.cm-content')?.cmTile?.root?.view?.state.doc.toString()); }
async function select(page, from, to) {
  await page.evaluate(({ from, to }) => { const view = document.querySelector('.cm-content').cmTile.root.view; view.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true }); view.focus(); }, { from, to });
}
async function waitSource(page, text) { await page.waitForFunction((text) => document.querySelector('.cm-content')?.cmTile?.root?.view?.state.doc.toString().includes(text), text); }
async function menu(page, label) { await page.locator('#insertMenu').click(); await page.getByRole('button', { name: label, exact: true }).click(); }
try {
  const adminCookie = await login(process.env.ACCEPTANCE_ADMIN_EMAIL || 'final-acceptance-20261008@example.test', (await readFile(passwordFile, 'utf8')).trim());
  const department = randomUUID(); const school = `SCHOOL-${suffix}`; const programme = `BTECH-${suffix}`;
  const faculty = `MENTOR-${suffix}`; const dean = `DEAN-${suffix}`;
  const people = [];
  for (let index = 0; index < 4; index++) {
    const email = `final-writer-${index}-${suffix}@example.test`;
    const user = await request(adminCookie, '/api/admin/v2/users', { method: 'POST', json: { email, role: 'writer', password, generate_temporary_password: false }, expected: 201 });
    people.push({ email, id: user.user_id, reg: `FINAL-${index}-${suffix}` });
  }
  const mentorEmail = `final-mentor-${suffix}@example.test`;
  const mentor = await request(adminCookie, '/api/admin/v2/users', { method: 'POST', json: { email: mentorEmail, role: 'mentor', password, generate_temporary_password: false }, expected: 201 });
  await importFile(adminCookie, 'departments.csv', `department_id\n${department}\n`);
  const xlsx = await workbook([['department_id', 'department_name'], [department, 'Computer Science and Engineering']]);
  await importFile(adminCookie, 'departments.xlsx', xlsx, 'VALIDATE_ONLY', false);
  let grid = await request(adminCookie, `/api/admin/v2/institution/data/departments?search=${department}`);
  assert.equal(grid.items[0].department_name, null, 'VALIDATE_ONLY must not mutate');
  await importFile(adminCookie, 'departments.xlsx', xlsx);
  await importFile(adminCookie, 'departments.csv', `department_id\n${department}\n`);
  await importFile(adminCookie, 'departments.csv', `department_id,department_name\n${department},Must not overwrite\n`, 'ADD_ONLY');
  grid = await request(adminCookie, `/api/admin/v2/institution/data/departments?search=${department}`);
  assert.equal(grid.items[0].department_name, 'Computer Science and Engineering');
  check(true, 'XLSX validate/apply, CSV legacy preservation, ADD_ONLY preservation');
  await importFile(adminCookie, 'schools.csv', `school_id,school_name\n${school},School of Computing\n`);
  await importFile(adminCookie, 'faculty.csv', `faculty_id,name,email,dept_id,honorific,designation,status\n${faculty},Synthetic Mentor,${mentorEmail},${department},Dr.,Professor,active\n${dean},Synthetic Dean,,${department},Dr.,Dean,active\n`);
  await importFile(adminCookie, 'programmes.csv', `programme_code,programme_name,degree_name,hod_id\n${programme},Computer Science and Engineering,Bachelor of Technology,${faculty}\n`);
  await importFile(adminCookie, 'faculty_roles.csv', `role_id,faculty_id,role_type,school_id,department_id,programme_code,status\n${randomUUID()},${dean},dean,${school},${department},${programme},active\n${randomUUID()},${faculty},hod,${school},${department},${programme},active\n`);
  await importFile(adminCookie, 'students.csv', `reg_no,name,email,programme_code\n${people.map((person, index) => `${person.reg},Synthetic Student ${index + 1},${person.email},${programme}`).join('\n')}\n`);
  const form = new FormData(); form.set('name', `Final real Complete Report ${suffix}`); form.set('arrangement', 'SINGLE_SOURCE'); form.set('main', 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
  form.set('archive', new Blob([await readFile(new URL('../../../artifacts/Full_Report_template_v1.1.zip', import.meta.url))]), 'complete-report.zip');
  const template = await request(adminCookie, '/api/admin/v2/templates/import', { method: 'POST', body: form, expected: 201 });
  await request(adminCookie, `/api/admin/v2/institution/template-defaults/programmes/${programme}`, { method: 'PUT', json: { template_id: template.id }, expected: 204 });
  // Import the Team so Writer order and department resolution use the actual institutional path.
  const external = `FINAL-TEAM-${suffix}`;
  const teamForm = new FormData(); teamForm.set('operation', 'ADD');
  for (const [name, bytes] of Object.entries({
    'paper_teams.csv': `external_team_key,team_name,academic_year,semester,status\n${external},Final Imported Report ${suffix},2026-2027,Winter Semester,active\n`,
    'paper_team_writers.csv': `external_team_key,student_reg_no,writer_order,is_leader\n${external},${people[0].reg},1,true\n`,
    'paper_team_mentors.csv': `external_team_key,faculty_id\n${external},${faculty}\n`,
  })) teamForm.append('files[]', new Blob([bytes]), name);
  const teamBatch = await request(adminCookie, '/api/admin/v2/institution/import-batches/validate', { method: 'POST', body: teamForm, expected: 201 });
  assert.equal(teamBatch.batch.error_rows, 0, JSON.stringify(teamBatch.issues));
  await request(adminCookie, `/api/admin/v2/institution/import-batches/${teamBatch.batch.id}/apply`, { method: 'POST', json: {} });
  const teams = await request(adminCookie, `/api/admin/v2/paper-teams?search=${encodeURIComponent(`Final Imported Report ${suffix}`)}`);
  const team = (teams.items || teams).find((item) => item.name === `Final Imported Report ${suffix}`); assert.ok(team?.id, JSON.stringify(teams));
  const writerCookie = await login(people[0].email, password); const root = `/api/v2/papers/${team.id}`;
  const files = await request(writerCookie, `${root}/files`); const main = files.find((file) => file.path.endsWith('/Full_Report_v1.0.tex'));
  const source = () => request(writerCookie, `${root}/files/${main.file_id}`);
  evidence.paper_id = team.id; evidence.department_id = department;
  if (!baseline) {
    const initial = await source();
    const duplicated = String.raw`% Known institutional single-source bindings.
\providecommand{\coursecode}{}
\renewcommand{\coursecode}{BA099}
\providecommand{\coursename}{}
\renewcommand{\coursename}{Captsone project - II}
\providecommand{\programdegree}{}
\renewcommand{\programdegree}{Bachelor of Technology}
% acceptance user comment survives normalization
\newcommand{\acceptancecustom}{User-authored macro}
% LATEX_CORE_SINGLE_SOURCE_BINDINGS
`;
    await request(writerCookie, `${root}/files/${main.file_id}`, { method: 'PUT', json: { content: initial.content.replace('\\begin{document}', `${duplicated}\\begin{document}`), version: initial.version } });
  }
  const adminPage = await pageFor(adminCookie, '/admin');
  if (!baseline) {
    await adminPage.getByRole('button', { name: 'Imports', exact: true }).click();
    await adminPage.locator('.batch-import-form input[type="file"]').setInputFiles({ name: 'departments.xlsx', mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', buffer: xlsx });
    await adminPage.locator('[name="operation"][value="EDIT"]').check();
    const validated = adminPage.waitForResponse((response) => response.url().endsWith('/import-batches/validate') && response.request().method() === 'POST');
    await adminPage.getByRole('button', { name: 'Review changes', exact: true }).click();
    assert.equal((await (await validated).json()).batch.error_rows, 0);
    const applied = adminPage.waitForResponse((response) => /import-batches\/[^/]+\/apply$/.test(response.url()));
    adminPage.once('dialog', (dialog) => dialog.accept());
    await adminPage.getByRole('button', { name: 'Edit records', exact: true }).click(); assert.equal((await applied).status(), 200);
    check(true, 'Admin browser uploads, validates and applies actual XLSX worksheet');
    await adminPage.getByRole('button', { name: 'Institution Data', exact: true }).click();
    await adminPage.getByRole('button', { name: 'Departments', exact: true }).click();
    await adminPage.getByLabel('Search Departments', { exact: true }).fill(department);
    await adminPage.getByRole('button', { name: 'Apply filters', exact: true }).click();
    const servedWriter = await fetch(`${base}/static/writer.js`);
    assert.equal(createHash('sha256').update(Buffer.from(await servedWriter.arrayBuffer())).digest('hex'), createHash('sha256').update(await readFile(new URL('../static/writer.js', import.meta.url))).digest('hex'), 'live Writer bundle differs from this checkout');
    const servedAdmin = await fetch(`${base}/static/admin.js`);
    assert.equal(createHash('sha256').update(Buffer.from(await servedAdmin.arrayBuffer())).digest('hex'), createHash('sha256').update(await readFile(new URL('../static/admin.js', import.meta.url))).digest('hex'), 'live Admin bundle differs from this checkout');
    const row = adminPage.locator('tr').filter({ hasText: department }); await row.getByText('Computer Science and Engineering', { exact: true }).waitFor();
    assert.deepEqual(await row.locator('xpath=ancestor::table').locator('thead th').allTextContents(), ['Department ID', 'Department Name', 'Actions']);
    await row.getByRole('button', { name: 'Edit', exact: true }).click();
    assert.equal(await adminPage.locator('[name="department_name"]').inputValue(), 'Computer Science and Engineering');
    const edited = adminPage.waitForResponse((response) => /import-batches\/[^/]+\/apply$/.test(response.url()));
    adminPage.once('dialog', (dialog) => dialog.accept());
    await adminPage.getByRole('button', { name: 'Review edit', exact: true }).click(); assert.equal((await edited).status(), 200);
    await adminPage.getByRole('button', { name: '+ Add Department', exact: true }).click();
    await adminPage.locator('[name="department_id"]').fill(randomUUID());
    await adminPage.locator('[name="department_name"]').fill(`Synthetic Added Department ${suffix}`);
    const added = adminPage.waitForResponse((response) => /import-batches\/[^/]+\/apply$/.test(response.url()));
    adminPage.once('dialog', (dialog) => dialog.accept());
    await adminPage.getByRole('button', { name: 'Review add', exact: true }).click(); assert.equal((await added).status(), 200);
    await adminPage.getByRole('heading', { name: 'Account setup', exact: true }).first().waitFor();
    await adminPage.getByRole('button', { name: 'Close dialog', exact: true }).click();
    for (const [tab, identity, fields] of [
      ['Schools', school, { school_name: 'School of Computing' }],
      ['Programmes', programme, { programme_name: 'Computer Science and Engineering', degree_name: 'Bachelor of Technology' }],
    ]) {
      await adminPage.getByRole('button', { name: tab, exact: true }).click();
      await adminPage.getByLabel(`Search ${tab}`, { exact: true }).fill(identity);
      await adminPage.getByRole('button', { name: 'Apply filters', exact: true }).click();
      const record = adminPage.locator('tr').filter({ hasText: identity });
      for (const value of Object.values(fields)) await record.getByText(value, { exact: true }).waitFor();
      await record.getByRole('button', { name: 'Edit', exact: true }).click();
      for (const [field, value] of Object.entries(fields)) assert.equal(await adminPage.locator(`[name="${field}"]`).inputValue(), value);
      await adminPage.getByRole('button', { name: 'Close dialog', exact: true }).click();
    }
    await adminPage.getByRole('button', { name: 'Imports', exact: true }).click();
    await adminPage.getByRole('button', { name: 'What data do I need?', exact: true }).click();
    const guide = await adminPage.locator('.import-guide-dialog').innerText();
    for (const field of ['department_name', 'school_name', 'programme_name', 'degree_name']) assert.ok(guide.includes(field));
    assert.match(guide, /department_id, department_name/);
    check(true, 'Department, School and Programme grid/edit/import guide agree with persisted names');
  }
  const page = await pageFor(writerCookie, `/write?paper=${team.id}`);
  await page.locator('.cm-editor').waitFor(); await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
  await page.locator('#documentDetailsPanel').waitFor({ state: 'visible' });
  if (!baseline) {
    const details = await request(writerCookie, `${root}/document-details`);
    assert.equal(details.team_size, 1);
    assert.equal(details.values.find((field) => field.field_key === 'department_name').value, 'Computer Science and Engineering');
    const panel = await page.locator('#documentDetailsBody').innerText();
    assert.ok(!/Student [BCD] (?:name|registration)/.test(panel), panel);
    for (const heading of ['Document details', 'Institutional details', 'Team members', 'Project details']) assert.ok(panel.includes(heading));
    for (const field of ['department_name', 'programme_name', 'degree_name', 'guide_name', 'dean_name', 'hod_name']) assert.equal(await page.locator(`[name="field:${field}"]`).count(), 0, `${field} is read-only`);
    assert.equal(await page.locator('[name="field:team_academic_year"]').getAttribute('required'), '');
    assert.equal(await page.locator('[name="field:team_semester"]').evaluate((input) => input.tagName), 'SELECT');
    check(true, 'Document Details resolves department and omits inactive student slots');
  }
  async function saveDetails(code, direct = false) {
    const buildBeforeSave = (await request(writerCookie, `${root}/builds`)).build.latest_build_id;
    if (!await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#documentDetails').click();
    await page.locator('[name="field:course_code"]').fill(code);
    await page.locator('[name="field:submission_date"]').fill('2026-10-08');
    await page.locator('[name="project_type"]').selectOption('capstone');
    await page.locator('[name="field:team_academic_year"]').fill('2026-2027');
    await page.locator('[name="field:team_semester"]').selectOption('Winter Semester');
    const invalid = await page.locator('#documentDetailsBody input, #documentDetailsBody select, #documentDetailsBody textarea').evaluateAll((controls) => controls.filter((control) => !control.checkValidity()).map((control) => ({ name: control.name, value: control.value, message: control.validationMessage })));
    assert.deepEqual(invalid, [], 'Document Details invalid controls');
    const response = page.waitForResponse((response) => response.url().endsWith(direct ? '/document-details' : '/project-metadata') && response.request().method() === 'PUT');
    await page.getByRole('button', { name: direct ? 'Save document details' : 'Save project metadata', exact: true }).click();
    const result = await response.catch(async (error) => { throw new Error(`${error.message}; notice: ${await page.locator('#writerNotice').innerText()}`); }); assert.equal(result.status(), 200, await result.text());
    await waitSource(page, `\\newcommand{\\coursecode}{${code}}`);
    await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
    assert.equal((await request(writerCookie, `${root}/builds`)).build.latest_build_id, buildBeforeSave, 'metadata save must not compile');
    const durable = await source(); assert.ok(durable.content.includes(`\\newcommand{\\coursecode}{${code}}`));
    if (!baseline) {
      for (const macro of ['coursecode', 'coursename', 'programdegree', 'academicyear', 'latexcoresemester', 'latexcoreacademicyear']) {
        assert.equal((durable.content.match(new RegExp(`\\\\(?:newcommand|providecommand|renewcommand)\\{\\\\${macro}\\}`, 'g')) || []).length, 1, macro);
      }
      assert.ok(durable.content.includes('\\newcommand{\\hoddept}{Computer Science and Engineering}'));
      assert.ok(durable.content.includes('\\newcommand{\\teamsize}{1}'));
      assert.ok(durable.content.includes('\\newcommand{\\studentBname}{}'));
      assert.ok(durable.content.includes('% acceptance user comment survives normalization'));
      assert.ok(durable.content.includes('\\newcommand{\\acceptancecustom}{User-authored macro}'));
    }
    if (await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#drawerClose').click();
    return durable;
  }
  async function compile(label, expected = 'succeeded', context = null) {
    const compilePage = context?.page || page;
    const compileRoot = context?.root || root;
    const compileSource = context?.source || source;
    const before = await request(writerCookie, `${compileRoot}/builds`);
    const pdfRequests = []; const trackPdf = (request) => { if (request.url().includes('/artifacts/pdf')) pdfRequests.push(request.url()); };
    compilePage.on('request', trackPdf);
    const response = compilePage.waitForResponse((response) => response.url().endsWith(`${compileRoot}/builds`) && response.request().method() === 'POST');
    await compilePage.locator('#compilePaper').click(); const submitted = await (await response).json(); assert.ok(submitted.build_id);
    if (!submitted.reused) {
      await compilePage.waitForFunction(() => /Queued|Compiling/.test(document.querySelector('#buildStatus')?.textContent || ''));
      if (before.build.current_build_id) assert.match(await compilePage.locator('#pdfRelation').innerText(), /out of date|current source|stale/i);
    }
    const captured = await compileSource();
    const record = { label, ...submitted, source_sequence: captured.version, source_sha256: createHash('sha256').update(captured.content).digest('hex') }; evidence.builds.push(record); console.log(JSON.stringify({ submitted: record }));
    const deadline = Date.now() + 180000; let status;
    do {
      status = (await request(writerCookie, `${compileRoot}/builds`)).build;
      if (submitted.reused && submitted.status === 'succeeded') {
        assert.equal(status.current_source_sequence, record.source_sequence, 'cached build must match the current durable source sequence');
        assert.equal(status.latest_build_id, submitted.build_id, 'explicit compile must become the latest attempt');
        break;
      }
      if (status.latest_build_id === submitted.build_id && !status.active_build_id && ['succeeded', 'failed'].includes(status.latest_status)) break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    } while (Date.now() < deadline);
    record.terminal = status.latest_status; record.current_build_id = status.current_build_id; record.current_source_sequence = status.current_source_sequence; record.current_state_hash = status.current_state_hash;
    record.pdf_requests = pdfRequests;
    console.log(JSON.stringify({ terminal: record }));
    if (status.latest_status !== expected) {
      const log = await request(writerCookie, `${compileRoot}/artifacts/log?build=${submitted.build_id}`);
      await writeFile(join(temp, `${label}.log`), log); throw new Error(`${label}: ${JSON.stringify(status.latest_error)}; log ${join(temp, `${label}.log`)}`);
    }
    if (expected === 'failed') {
      assert.equal(status.current_build_id, before.build.current_build_id);
      await compilePage.waitForFunction(() => document.querySelector('#buildStatus')?.textContent.includes('failed'));
      assert.match(await compilePage.locator('#pdfRelation').innerText(), /out of date|current source|stale/i);
      assert.match(await compilePage.locator('#pdfRelation').innerText(), /last successful PDF/i);
      assert.equal(await compilePage.locator('#pdfViewport').getAttribute('data-build-id'), before.build.current_build_id);
      await compilePage.waitForFunction(() => document.querySelector('#problemsToggle')?.dataset.severity === 'error');
      compilePage.off('request', trackPdf);
      return;
    }
    assert.equal(status.current_build_id, submitted.build_id); assert.equal(status.source_sequence, record.source_sequence); assert.equal(status.current_source_sequence, record.source_sequence); assert.equal(status.desired_state_hash, status.current_state_hash);
    await compilePage.waitForFunction((id) => document.querySelector('#pdfViewport')?.dataset.buildId === id && document.querySelector('#pdfRelation')?.textContent.includes('matches source'), submitted.build_id, { timeout: 60000 });
    compilePage.off('request', trackPdf);
    if (!submitted.reused) assert.ok(pdfRequests.some((url) => url.includes(`build=${submitted.build_id}`)), 'viewer fetches the submitted build artifact');
    const artifact = await request(writerCookie, `${compileRoot}/artifacts/pdf?build=${submitted.build_id}`);
    record.pdf_sha256 = createHash('sha256').update(artifact).digest('hex');
    const task = pdfjs.getDocument({ data: new Uint8Array(artifact), disableWorker: true }); const pdf = await task.promise; let text = '';
    for (let n = 1; n <= pdf.numPages; n++) text += (await (await pdf.getPage(n)).getTextContent()).items.map((item) => item.str).join(' ') + '\n';
    await task.destroy(); return text;
  }
  const first = await saveDetails('BA101');
  const firstPdf = await compile('course-BA101'); check(firstPdf.includes('BA101'), 'real Complete Report PDF renders BA101');
  assert.ok(firstPdf.toLowerCase().includes('synthetic student 1') && firstPdf.includes(people[0].reg) && !firstPdf.includes('Not assigned'));
  const second = await saveDetails('BA102', true); assert.ok(second.version > first.version);
  await page.waitForFunction(() => /out of date|current source|stale/i.test(document.querySelector('#pdfRelation')?.textContent || ''));
  const secondPdf = await compile('course-BA102'); check(secondPdf.includes('BA102') && !secondPdf.includes('BA101'), 'real Complete Report PDF advances to BA102');
  if (baseline) { console.log(JSON.stringify(evidence, null, 2)); process.exitCode = 0; }
  else {
    const third = await saveDetails('BA102'); assert.equal(third.content, second.content);
    const zipPath = join(temp, 'source.zip');
    const download = page.waitForEvent('download');
    await page.locator('#downloadMenu summary').click(); await page.locator('#downloadSource').click();
    await (await download).saveAs(zipPath);
    const extracted = await execute('python3', ['-c', 'import sys,zipfile;sys.stdout.buffer.write(zipfile.ZipFile(sys.argv[1]).read(sys.argv[2]))', zipPath, main.path], { maxBuffer: 1024 * 1024 });
    assert.equal(extracted.stdout, third.content); check(true, 'three saves idempotent and Source ZIP matches durable canonical source');
    const history = await request(writerCookie, `${root}/versions`); check(history.length > 0 || history.versions?.length > 0, 'History retained');
    const peer = await pageFor(writerCookie, `/write?paper=${team.id}`); await peer.locator('.cm-editor').waitFor();
    if (await peer.locator('#documentDetailsPanel').isVisible()) await peer.locator('#drawerClose').click();
    const text = await editor(page); const start = text.indexOf('\\newcommand{\\coursecode}'); const end = text.indexOf('\n', text.indexOf('\\newcommand{\\programdegree}'));
    await select(page, start + 3, end - 2); await menu(page, 'Comment selected lines');
    const commented = await editor(page); assert.ok(commented.includes('% \\newcommand{\\coursecode}{BA102}'));
    await waitSource(peer, '% \\newcommand{\\coursecode}{BA102}');
    const selection = await page.evaluate(() => { const view = document.querySelector('.cm-content').cmTile.root.view; return { from: view.state.selection.main.from, to: view.state.selection.main.to }; });
    assert.ok(selection.to > selection.from); check(true, 'three partial-selected lines commented and synchronized with visible selection');
    await page.locator('#undoText').click(); assert.equal(await editor(page), text);
    await page.locator('#redoText').click(); assert.equal(await editor(page), commented);
    await select(page, start, start + commented.length - text.length + end - start);
    await menu(page, 'Uncomment selected lines'); assert.equal(await editor(page), text); await waitSource(peer, '\\newcommand{\\coursecode}{BA102}');
    check(true, 'comment undo/redo is one operation and uncomment restores exact source');
    await select(page, start, end); await page.keyboard.press('Control+/'); assert.equal(await editor(page), commented); await page.keyboard.press('Control+/'); assert.equal(await editor(page), text);
    check(true, 'keyboard comment toggle uses same engine');
    const mentorCookie = await login(mentorEmail, password); const mentorPage = await pageFor(mentorCookie, '/review');
    await mentorPage.locator('#assignedPapers button').filter({ hasText: team.name }).click(); await mentorPage.locator('#reviewEditor .cm-content').waitFor();
    assert.equal(await mentorPage.locator('#reviewEditor .cm-content').getAttribute('contenteditable'), 'false');
    await request(mentorCookie, `${root}/files/${main.file_id}`, { method: 'PUT', json: { content: 'forbidden', version: third.version }, expected: 403 }); check(true, 'Mentor source read-only enforced');
    const normal = await editor(page); const bodyEnd = normal.indexOf('\\end{document}');
    await select(page, bodyEnd, bodyEnd); await page.keyboard.insertText('\\UndefinedAcceptanceCommand\n');
    await compile('deliberate-failure', 'failed');
    await page.locator('#undoText').click(); assert.equal(await editor(page), normal);
    const analysis = await request(writerCookie, `${root}/intelligence`); assert.ok(!analysis.diagnostics.some((item) => item.code === 'MissingProjectDependency'), JSON.stringify(analysis.diagnostics));
    await select(page, bodyEnd, bodyEnd); await page.keyboard.insertText('\\includegraphics{images/acceptance-genuinely-missing.png}\n');
    await page.keyboard.press('Control+s'); await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
    const missing = await request(writerCookie, `${root}/intelligence`); assert.ok(missing.diagnostics.some((item) => item.code === 'MissingProjectDependency'));
    await page.locator('#undoText').click(); assert.equal(await editor(page), normal); check(true, 'real images resolve and genuine missing image still warns');
    await compile('restored-source');
    await select(page, bodyEnd, bodyEnd); await menu(page, 'Figure');
    const assets = await page.locator('#dialogBody').getByLabel('Asset').evaluate((select) => Array.from(select.options, (option) => option.value));
    assert.ok(assets.length > 0 && !assets.includes(main.path.replace(/\.tex$/i, '.pdf')), `Figure Builder asset choices: ${JSON.stringify({ main: main.path, assets, paper: await request(writerCookie, root) })}`);
    await page.locator('#dialogBody').getByLabel('Caption', { exact: true }).fill('Acceptance real image');
    await page.locator('#dialogBody').getByLabel('Label', { exact: true }).fill(`fig:acceptance-${suffix}`);
    await page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
    await compile('figure-builder'); check(true, 'Figure Builder output compiles in real Complete Report');
    await page.locator('#undoText').click(); assert.equal(await editor(page), normal);
    await compile('final-clean');
    for (const size of [2, 4]) {
      const created = await request(adminCookie, '/api/admin/v2/paper-teams', { method: 'POST', json: { name: `Acceptance ${size} writers ${suffix}`, template_id: template.id, writer_ids: people.slice(0, size).map((person) => person.id), leader_writer_id: people[0].id, mentor_ids: [mentor.user_id] }, expected: 201 });
      const detail = await request(writerCookie, `/api/v2/papers/${created.team.id}/document-details`);
      assert.equal(detail.team_size, size); assert.equal(detail.manifest.fields.filter((field) => /^student_[a-d]_name$/.test(field.key)).length, size);
      assert.ok(!detail.missing_required_fields.some((key) => /^student_/.test(key)));
      const teamPage = await pageFor(writerCookie, `/write?paper=${created.team.id}`);
      await teamPage.locator('#documentDetailsPanel').waitFor({ state: 'visible' });
      const visible = await teamPage.locator('#documentDetailsBody').innerText();
      for (const [index, slot] of ['A', 'B', 'C', 'D'].entries()) assert.equal(visible.includes(`Student ${slot} name:`), index < size);
      if (size === 2) {
        await teamPage.locator('#drawerClose').click();
        const secondWriterCookie = await login(people[1].email, password);
        const secondWriterPage = await pageFor(secondWriterCookie, `/write?paper=${created.team.id}`);
        await secondWriterPage.locator('.cm-editor').waitFor();
        if (await secondWriterPage.locator('#documentDetailsPanel').isVisible()) await secondWriterPage.locator('#drawerClose').click();
        const original = await editor(teamPage);
        const from = original.indexOf('\\newcommand{\\coursecode}');
        const to = original.indexOf('\n', original.indexOf('\\newcommand{\\programdegree}'));
        await select(teamPage, from + 2, to - 2); await menu(teamPage, 'Comment selected lines');
        const shared = await editor(teamPage);
        await secondWriterPage.waitForFunction((source) => document.querySelector('.cm-content')?.cmTile?.root?.view?.state.doc.toString() === source, shared);
        await teamPage.locator('#undoText').click(); assert.equal(await editor(teamPage), original);
        await secondWriterPage.waitForFunction((source) => document.querySelector('.cm-content')?.cmTile?.root?.view?.state.doc.toString() === source, original);
        check(true, 'multiline edit and one-step undo synchronize between two assigned Writers');
      }
      if (await teamPage.locator('#documentDetailsPanel').isVisible()) await teamPage.locator('#drawerClose').click();
      await teamPage.locator('#documentDetails').click();
      for (const [field, value] of Object.entries({ course_code: `TEAM${size}`, submission_date: '2026-10-08', team_academic_year: '2026-2027' })) await teamPage.locator(`[name="field:${field}"]`).fill(value);
      await teamPage.locator('[name="field:team_semester"]').selectOption('Winter Semester');
      const saved = teamPage.waitForResponse((response) => response.url().endsWith('/document-details') && response.request().method() === 'PUT');
      await teamPage.getByRole('button', { name: 'Save document details', exact: true }).click(); assert.equal((await saved).status(), 200);
      await waitSource(teamPage, `\\newcommand{\\coursecode}{TEAM${size}}`);
      await teamPage.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
      if (await teamPage.locator('#documentDetailsPanel').isVisible()) await teamPage.locator('#drawerClose').click();
      const teamRoot = `/api/v2/papers/${created.team.id}`;
      const teamFiles = await request(writerCookie, `${teamRoot}/files`); const teamMain = teamFiles.find((file) => file.path === main.path);
      const rendered = await compile(`team-${size}-certificate`, 'succeeded', { page: teamPage, root: teamRoot, source: () => request(writerCookie, `${teamRoot}/files/${teamMain.file_id}`) });
      for (let index = 0; index < size; index++) assert.ok(rendered.toLowerCase().includes(`synthetic student ${index + 1}`) && rendered.includes(people[index].reg));
      assert.ok(!rendered.includes('Not assigned'));
    }
    check(true, '1/2/4 assigned Writers expose exactly their slots and certificate/acknowledgement compile');
    const unmapped = await request(adminCookie, '/api/admin/v2/users', { method: 'POST', json: { email: `unmapped-${suffix}@example.test`, role: 'writer', password, generate_temporary_password: false }, expected: 201 });
    const missingTeam = await request(adminCookie, '/api/admin/v2/paper-teams', { method: 'POST', json: { name: `Acceptance missing mapping ${suffix}`, template_id: template.id, writer_ids: [people[0].id, unmapped.user_id], leader_writer_id: people[0].id, mentor_ids: [mentor.user_id] }, expected: 201 });
    const unmappedDetails = await request(writerCookie, `/api/v2/papers/${missingTeam.team.id}/document-details`);
    assert.ok(unmappedDetails.missing_required_fields.includes('student_b_name'));
    assert.ok(unmappedDetails.missing_required_fields.includes('student_b_reg_no'));
    const missingPage = await pageFor(writerCookie, `/write?paper=${missingTeam.team.id}`);
    await missingPage.locator('#documentDetailsPanel').waitFor({ state: 'visible' });
    assert.match(await missingPage.locator('#documentDetailsBody').innerText(), /Student B name: Not assigned — contact your administrator/);
    check(true, 'assigned Writer without institutional mapping remains an actionable error');
    assert.deepEqual(browserErrors, []); evidence.real_browser_smoke = 'PASS';
    await writeFile(join(temp, 'evidence.json'), JSON.stringify(evidence, null, 2));
    console.log(JSON.stringify({ ...evidence, evidence_path: join(temp, 'evidence.json') }, null, 2));
  }
} finally { await Promise.allSettled(contexts.map((context) => context.close())); await browser.close(); }
