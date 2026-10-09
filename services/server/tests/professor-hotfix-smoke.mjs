import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright';

// Called by the disposable PostgreSQL integration fixture against a live Axum listener.
const { base, cookie, paperId, adminCookie, writerId, mentorId } = JSON.parse(process.env.LATEX_CORE_HOTFIX_SMOKE_CONFIG);
const root = `/api/v2/papers/${paperId}`;
async function request(path, method = 'GET', body) {
  const response = await fetch(`${base}${path}`, {
    method, headers: { Cookie: cookie, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json();
  assert.equal(response.status, path.endsWith("/builds") && method === "POST" ? 202 : 200, JSON.stringify(result));
  return result;
}
const files = await request(`${root}/files`);
const main = files.find((file) => file.path === 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
const nested = files.find((file) => file.path === 'Full_Report_template_v1.0/chapters/body.tex');
assert.ok(files.some((file) => file.path === 'Full_Report_template_v1.0/images/foo.png'));
const before = await request(`${root}/files/${main.file_id}`);
assert.ok(before.content.includes('Professor Durable Title One'));
const detail = await request(`${root}/document-details`);
assert.ok(detail.values.some((item) => item.field_key === 'student_a_name' && item.value === 'Institutional Alice'));
const saved = await request(`${root}/document-details`, 'PUT', {
  values: { project_title: 'Professor Durable Title Two' }, sections: {},
});
const after = await request(`${root}/files/${main.file_id}`);
assert.ok(after.content.includes('Professor Durable Title Two'));
assert.ok(!after.content.includes('Professor Durable Title One'));
assert.ok(after.content.includes('% unrelated manual preamble'));
assert.ok(after.version > before.version);
assert.ok(after.file.revision > before.file.revision);
assert.equal(saved.workspace_version, after.version);
const reopened = await request(`${root}/document-details`);
assert.ok(reopened.values.some((item) => item.field_key === 'project_title' && item.value === 'Professor Durable Title Two'));
const analysis = await request(`${root}/intelligence`);
assert.ok(!analysis.diagnostics.some((item) => item.code === 'MissingProjectDependency'), JSON.stringify(analysis.diagnostics));
const body = await request(`${root}/files/${nested.file_id}`);
await request(`${root}/files/${nested.file_id}`, 'PUT', {
  content: `${body.content}\n\\includegraphics{images/actually-missing.png}\n`, version: body.version,
});
const missing = await request(`${root}/intelligence`);
assert.ok(missing.diagnostics.some((item) => item.code === 'MissingProjectDependency'));
const current = await request(`${root}/files/${nested.file_id}`);
await request(`${root}/files/${nested.file_id}`, 'PUT', { content: body.content, version: current.version });
assert.ok(!(await request(`${root}/intelligence`)).diagnostics.some((item) => item.code === 'MissingProjectDependency'));
console.log('HTTP smoke: durable TeX, revision, reopen, institutional authority, existing/nested/graphicspath image, real missing image PASS');

// Exercise the actual Complete Report archive through the real Writer save button.
const archive = await readFile(new URL('../../../artifacts/Full_Report_template_v1.1.zip', import.meta.url));
const form = new FormData();
form.set('name', `Complete Report hotfix ${Date.now()}`);
form.set('main', 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
form.set('arrangement', 'SINGLE_SOURCE');
form.set('archive', new Blob([archive], { type: 'application/zip' }), 'complete-report.zip');
const importedResponse = await fetch(`${base}/api/admin/v2/templates/import`, { method: 'POST', headers: { Cookie: adminCookie }, body: form });
const imported = await importedResponse.json();
assert.equal(importedResponse.status, 201, JSON.stringify(imported));
const teamResponse = await fetch(`${base}/api/admin/v2/paper-teams`, { method: 'POST', headers: { Cookie: adminCookie, 'Content-Type': 'application/json' }, body: JSON.stringify({ name: `Real Complete Report ${Date.now()}`, writer_ids: [writerId], leader_writer_id: writerId, mentor_ids: [mentorId], template_id: imported.id }) });
const team = await teamResponse.json();
assert.equal(teamResponse.status, 201, JSON.stringify(team));
const realRoot = `/api/v2/papers/${team.team.id}`;
const realFiles = await request(`${realRoot}/files`);
const realMain = realFiles.find((file) => file.path === 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
assert.ok(realMain);
const realBefore = await request(`${realRoot}/files/${realMain.file_id}`);
assert.ok(realBefore.content.includes(String.raw`\newcommand{\coursecode}{BCSXXXX}`));
const browser = await chromium.launch({ headless: true, executablePath: '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome' });
try {
  const context = await browser.newContext();
  const bundle = await readFile(process.env.PROFESSOR_WRITER_BUNDLE || new URL('../static/writer.js', import.meta.url));
  await context.route('**/static/writer.js*', (route) => route.fulfill({ status: 200, contentType: 'text/javascript', body: bundle }));
  const [name, ...parts] = cookie.split('=');
  await context.addCookies([{ name, value: parts.join('=').split(';')[0], url: base }]);
  const page = await context.newPage();
  await page.goto(`${base}/write?paper=${team.team.id}`);
  await page.locator('.cm-editor').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
  await page.locator('#documentDetailsPanel').waitFor({ state: 'visible' });
  for (const key of ['course_code','team_academic_year','team_semester']) assert.equal(await page.locator(`[name="field:${key}"]`).count(), 0);
  await page.locator('[name="field:course_name"]').fill('Hotfix intentional course name');
  assert.equal(await page.locator('[name="field:guide_identity"], [name="field:dean_identity"]').count(), 0);
  await page.getByRole('button', { name: 'Save document details', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.cm-content')?.textContent.includes('Hotfix intentional course name'));
  const realAfter = await request(`${realRoot}/files/${realMain.file_id}`);
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\coursecode}{BCSXXXX}`));
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\coursename}{Hotfix intentional course name}`));
  assert.ok(realAfter.content.includes(String.raw`\include{coverpage.tex}`));
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\semester}{Winter Semester 2029-2030}`));
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\academicyear}{2029-2030}`));
  assert.ok(realAfter.file.revision > realBefore.file.revision);
  assert.ok(realAfter.version > realBefore.version);
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\programdegree}{Bachelor of Technology}`));
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\projguidename}{Dr. Rao}`));
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\deanname}{Dr. Krishnan}`));
  const exported = await fetch(`${base}${realRoot}/source.zip`, { headers: { Cookie: cookie } });
  assert.equal(exported.status, 403);
  // ZIP contents are checked against exact durable bytes in the Rust integration regression.
  assert.equal(await page.locator('#writerHelp').isEnabled(), true);
  assert.equal(await page.locator('#writerHelp').getAttribute('title'), 'Contact support: help@institution.example');
  console.log('Real Complete Report: durable coursecode BA101 and open Writer BA101 PASS');
  if (await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#drawerClose').click();
  const policy = await request(realRoot);
  const baselineDiagnostics = (await request(`${realRoot}/intelligence`)).diagnostics;
  assert.equal(policy.main_file_fixed, true);
  assert.equal(policy.canonical_main_file, realMain.path);
  await page.locator('#fileActionsToggle').click();
  assert.equal(await page.locator('#setMain').isVisible(), false);
  await page.locator('#fileActionsToggle').click();
  await page.locator('#insertMenu').click();
  for (const category of ['Structure', 'Math', 'Media', 'Data / visualization', 'Code / formal content', 'References']) {
    assert.ok(await page.locator('.palette-category').filter({ hasText: category }).isVisible());
  }
  await page.keyboard.press('Escape');
  // Upload the tiny PNG generated by the Rust fixture, using the product API.
  const pngResponse = await fetch(`${base}${root}/files/${files.find((file) => file.path.endsWith('images/foo.png')).file_id}/raw`, { headers: { Cookie: cookie } });
  assert.equal(pngResponse.status, 200);
  const png = await pngResponse.arrayBuffer();
  const upload = await fetch(`${base}${realRoot}/assets?path=${encodeURIComponent('Full_Report_template_v1.0/images/demo-figure.png')}&version=${policy.version}`, { method: 'POST', headers: { Cookie: cookie, 'Content-Type': 'image/png' }, body: png });
  assert.equal(upload.status, 201, await upload.text());
  // File-change collaboration semantics refresh the asset choices without reload.
  await page.waitForTimeout(500);
  await page.locator('.cm-content').click();
  await page.locator('.cm-content').press('Control+End');
  await page.locator('.cm-content').press('ArrowUp');
  await page.locator('.cm-content').press('Home');
  await page.locator('#insertMenu').click();
  await page.getByRole('button', { name: 'Figure', exact: true }).click();
  const control = (label) => page.locator('#dialogBody label').filter({ hasText: new RegExp(`^${label}`) }).locator('input,select,textarea');
  await control('Asset').selectOption('Full_Report_template_v1.0/images/demo-figure.png');
  await control('Width').selectOption('custom');
  await control('Custom width').fill(String.raw`0.55\linewidth`);
  await control('Caption').fill('Campus monitoring dashboard');
  await control('Label').fill('fig:campus-dashboard');
  assert.ok(await page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).isEnabled(), await page.locator('#dialogBody').innerText());
  await page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
  await page.keyboard.press('Control+s');
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
  const figureSource = await request(`${realRoot}/files/${realMain.file_id}`);
  assert.equal((figureSource.content.match(/\\label\{fig:campus-dashboard\}/g) || []).length, 1);
  assert.ok(figureSource.content.includes(String.raw`\includegraphics[width=0.55\linewidth]{images/demo-figure.png}`));
  const intelligence = await request(`${realRoot}/intelligence`);
  const syntax = (items) => items.filter((item) => ['SyntaxError', 'MissingSyntax'].includes(item.code)).length;
  assert.equal(syntax(intelligence.diagnostics), syntax(baselineDiagnostics), JSON.stringify(intelligence.diagnostics));
  console.log('Professor 07.10: fixed Main, categorized Insert, real uploaded Figure inserted once and durably saved PASS');
  const build = await request(`${realRoot}/builds`, 'POST', { trigger_type: 'manual' });
  console.log(JSON.stringify({ real_build_id: build.build_id }));
} finally { await browser.close(); }
