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
assert.ok(!missing.diagnostics.some((item) => item.code === 'MissingProjectDependency')); // Writer suppresses this diagnostic; parser analysis remains intact.
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
  const [name, ...parts] = cookie.split('=');
  await context.addCookies([{ name, value: parts.join('=').split(';')[0], url: base }]);
  const page = await context.newPage();
  await page.goto(`${base}/write?paper=${team.team.id}`);
  await page.locator('.cm-editor').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
  await page.locator('#documentDetails').click();
  await page.locator('[name="field:course_code"]').fill('BA101');
  await page.locator('[name="project_type"]').selectOption('capstone');
  const saved = page.waitForResponse((response) => response.request().method() === 'PUT' && response.url().endsWith('/project-metadata'));
  await page.getByRole('button', { name: 'Save project metadata', exact: true }).click();
  assert.equal((await saved).status(), 200);
  await page.waitForFunction(() => document.querySelector('.cm-content')?.textContent.includes(String.raw`\newcommand{\coursecode}{BA101}`), null, { timeout: 20000 });
  const realAfter = await request(`${realRoot}/files/${realMain.file_id}`);
  assert.ok(realAfter.content.includes(String.raw`\newcommand{\coursecode}{BA101}`));
  assert.ok(!realAfter.content.includes('BCSXXXX'));
  assert.ok(realAfter.content.includes(String.raw`\include{coverpage.tex}`));
  console.log('Real Complete Report: durable coursecode BA101 and open Writer BA101 without reload PASS');
  const build = await request(`${realRoot}/builds`, 'POST', { trigger_type: 'manual' });
  console.log(JSON.stringify({ real_build_id: build.build_id }));
} finally { await browser.close(); }
