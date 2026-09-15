import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { chromium } from 'playwright';

const execute = promisify(execFile);
const config = JSON.parse(process.env.PROFESSOR_REMAINING_CONFIG);
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome' });
const failures = [];

function track(page, label) {
  page.on('pageerror', (error) => failures.push(`${label}: ${error.message}`));
  page.on('console', (message) => { if (message.type() === 'error' && !message.text().startsWith('Failed to load resource:')) failures.push(`${label}: ${message.text()}`); });
  page.on('response', (response) => { if (response.status() >= 500) failures.push(`${label}: ${response.status()} ${new URL(response.url()).pathname}`); });
}
async function login(email, route, label) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage(); track(page, label);
  await page.goto(config.base);
  await page.locator('#email').fill(email); await page.locator('#password').fill(config.password);
  await Promise.all([page.waitForURL(`**/${route}`), page.getByRole('button', { name: 'Sign in' }).click()]);
  return { context, page };
}
async function json(page, path, options = {}) {
  return page.evaluate(async ({ path, options }) => {
    const response = await fetch(path, { credentials: 'same-origin', method: options.method || 'GET', headers: options.body === undefined ? options.headers : { 'Content-Type': 'application/json', ...(options.headers || {}) }, body: options.body === undefined ? undefined : JSON.stringify(options.body) });
    const payload = await response.json().catch(() => null);
    return { status: response.status, payload };
  }, { path, options });
}
async function openReport(page, name) {
  await page.getByRole('button', { name, exact: true }).click();
  await page.locator('.cm-editor').waitFor({ state: 'visible' });
}
async function compile(page, paperId) {
  const before = await json(page, `/api/v2/papers/${paperId}/builds`); const previous = before.payload.build.current_build_id;
  await page.locator('#compilePaper').click();
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    const result = await json(page, `/api/v2/papers/${paperId}/builds`); const build = result.payload.build;
    if (!build.active_build_id && build.latest_status === 'failed') throw new Error(JSON.stringify(build.latest_error));
    if (!build.active_build_id && build.current_build_id && build.current_build_id !== previous && build.current_source_sequence === build.source_sequence) break;
    await page.waitForTimeout(250);
  }
  const completed = await json(page, `/api/v2/papers/${paperId}/builds`);
  assert.ok(!completed.payload.build.active_build_id && completed.payload.build.current_build_id !== previous, 'manual build did not complete');
  try {
    await page.waitForFunction((buildId) => document.querySelector('#pdfViewport')?.dataset.buildId === buildId && document.querySelectorAll('#pdfViewport .writer-pdf-page').length > 0, completed.payload.build.current_build_id, { timeout: 30_000 });
  } catch (error) {
    throw new Error(JSON.stringify(await page.evaluate(() => ({ notice: document.querySelector('#writerNotice')?.textContent, build: document.querySelector('#buildStatus')?.textContent, relation: document.querySelector('#pdfRelation')?.textContent, pdf: document.querySelector('#pdfScroll')?.hidden, pages: document.querySelector('#pdfViewport')?.children.length }))), { cause: error });
  }
}
async function pdfText(page, paperId) {
  return page.evaluate(async (paperId) => {
    const pdfjs = await import('/static/pdf.min.mjs'); pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const response = await fetch(`/api/v2/papers/${paperId}/artifacts/pdf`); const loading = pdfjs.getDocument({ data: new Uint8Array(await response.arrayBuffer()) }); const pdf = await loading.promise;
    let text = ''; for (let number = 1; number <= pdf.numPages; number += 1) { const content = await (await pdf.getPage(number)).getTextContent(); text += content.items.map((item) => item.str).join(' '); }
    await loading.destroy(); return text.replace(/\s+/g, ' ');
  }, paperId);
}
async function pdfPageCount(page, paperId) {
  return page.evaluate(async (paperId) => {
    const pdfjs = await import('/static/pdf.min.mjs'); pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const response = await fetch(`/api/v2/papers/${paperId}/artifacts/pdf`); const loading = pdfjs.getDocument({ data: new Uint8Array(await response.arrayBuffer()) }); const pdf = await loading.promise;
    const count = pdf.numPages; await loading.destroy(); return count;
  }, paperId);
}
async function adminAddPerson(form, role, email) {
  const section = form.locator('section.admin-section').filter({ hasText: role === 'writer' ? 'Ordered Writers' : 'Mentors' }).first();
  await section.getByRole('searchbox').fill(email);
  const results = section.locator('select');
  await results.page().waitForFunction(({ label, email }) => [...document.querySelector(`[aria-label="${label}"]`)?.options || []].some((option) => option.textContent === email), { label: `${role} search results`, email }, { timeout: 10_000 });
  await results.selectOption({ label: email }); await section.getByRole('button', { name: 'Add', exact: true }).click();
}
async function createTeam(admin, name) {
  await admin.locator('#adminNav button[data-section="Paper Teams"]').click(); await admin.getByRole('heading', { name: 'Paper Teams', exact: true }).waitFor();
  const details = admin.locator('details.admin-section').filter({ hasText: '+ Create Team' }).last(); await details.locator('summary').click();
  await adminAddPerson(details, 'writer', config.writer); await adminAddPerson(details, 'mentor', config.mentor);
  const form = details.locator('form.manual-team-form'); await form.locator('[name="name"]').fill(name);
  await form.locator('label').filter({ hasText: 'Leader' }).locator('select').selectOption({ label: config.writer });
  await form.locator('label').filter({ hasText: 'Template' }).locator('select').selectOption({ label: `Override: ${config.template_name}` });
  await form.locator('label').filter({ hasText: 'Front Matter' }).locator('select').selectOption('none');
  const created = admin.waitForResponse((response) => response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/admin/v2/paper-teams');
  await form.getByRole('button', { name: 'Create Paper Team' }).click(); assert.equal((await created).status(), 201);
  await admin.locator('#adminStatus').filter({ hasText: 'Paper Team created' }).waitFor();
  await admin.getByRole('heading', { name: 'Paper Teams', exact: true }).waitFor();
}
async function saveMetadata(page, reportName, departmentName, schoolName) {
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced', null, { timeout: 20_000 });
  if (!(await page.locator('#documentDetailsPanel').isVisible())) await page.locator('#documentDetails').click();
  await page.locator('[name="project_type"]').selectOption('capstone'); await page.locator('[name="executive_summary"]').fill('Single source metadata browser evidence.');
  const departments = page.locator('fieldset').filter({ hasText: 'Department display names' }).locator('input');
  const schools = page.locator('fieldset').filter({ hasText: 'School display names' }).locator('input');
  assert.equal(await departments.count(), 1); assert.equal(await schools.count(), 1);
  await departments.fill(departmentName); await schools.fill(schoolName);
  await page.locator('[name="project_type"]').selectOption({ label: 'Capstone' });
  assert.equal(await page.locator('[name="project_type"]').inputValue(), 'capstone');
  const invalid = await page.locator('.project-metadata-editor form').evaluate((form) => [...form.elements].filter((control) => !control.checkValidity()).map((control) => control.name || control.dataset.identity || control.type));
  assert.deepEqual(invalid, []);
  const saved = page.waitForResponse((response) => response.request().method() === 'PUT' && /\/api\/v2\/papers\/[^/]+\/project-metadata$/.test(new URL(response.url()).pathname));
  await page.getByRole('button', { name: 'Save project metadata' }).click(); const response = await saved;
  assert.equal(response.status(), 200, await response.text());
  await page.reload({ waitUntil: 'domcontentloaded' }); await openReport(page, reportName);
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced', null, { timeout: 20_000 });
  await page.waitForFunction(() => /^(?:Compile to generate|PDF (?:is out of date|matches))/.test(document.querySelector('#pdfRelation')?.textContent || ''), null, { timeout: 20_000 });
}
async function editorText(page) { return page.locator('.cm-content').innerText(); }
async function selectAcrossLines(page, firstText) {
  const lines = page.locator('.cm-line'); const contents = await lines.allInnerTexts(); const index = contents.findIndex((line) => line.includes(firstText));
  assert.ok(index >= 0 && index + 1 < contents.length, `multiline selection start not found: ${firstText}`);
  const first = await lines.nth(index).boundingBox(); const second = await lines.nth(index + 1).boundingBox();
  assert.ok(first && second, 'multiline source lines must be visible');
  await page.mouse.move(first.x + 2, first.y + first.height / 2); await page.mouse.down();
  await page.mouse.move(second.x + second.width - 2, second.y + second.height / 2, { steps: 8 }); await page.mouse.up();
}

const temp = await mkdtemp(join(tmpdir(), 'latex-core-professor-remaining-'));
const main = String.raw`\documentclass{article}
\usepackage{array}
\newcommand{\thesistitle}{Placeholder Title}
\newcommand{\studentAname}{Placeholder Student}
\newcommand{\studentAregno}{Placeholder Registration}
\newcommand{\projguidename}{Placeholder Guide}
\newcommand{\hoddept}{Placeholder Department}
\newcommand{\schoolname}{Placeholder School}
% LATEX_CORE_SINGLE_SOURCE_BINDINGS
\begin{document}
Title: \thesistitle\par
Student: \studentAname\ (\studentAregno)\par
Guide: \projguidename\par Department: \hoddept\par School: \schoolname\par
Table target.
Repeated navigation phrase in main.
\newpage
Page two.
\newpage
Locate target phrase.
\newpage
Page four.\input{second.tex}
\end{document}
`;
const second = 'Repeated navigation phrase in second.\nMentor multiline first.\nMentor multiline second.\nRepeated navigation phrase in second.\n';
await writeFile(join(temp, 'main.tex'), main); await writeFile(join(temp, 'second.tex'), second);
await execute('zip', ['-q', join(temp, 'single-source.zip'), 'main.tex', 'second.tex'], { cwd: temp });

try {
  const adminSession = await login(config.admin, 'admin', 'admin'); const admin = adminSession.page;
  await admin.locator('#adminNav button[data-section="Templates"]').click();
  const templateForm = admin.locator('form.admin-template-form').first(); await templateForm.waitFor();
  await templateForm.locator('[name="name"]').fill(config.template_name); await templateForm.locator('[name="arrangement"]').selectOption('SINGLE_SOURCE');
  await templateForm.locator('[name="archive"]').setInputFiles(join(temp, 'single-source.zip')); await templateForm.locator('[data-action="validate-template"]').click();
  await templateForm.locator('[name="main"]').waitFor({ state: 'visible' }); await templateForm.locator('[name="main"]').selectOption('main.tex');
  await templateForm.getByRole('button', { name: '+ Import Main Template' }).click(); await admin.locator('#adminStatus').filter({ hasText: `Imported ${config.template_name}` }).waitFor();
  assert.ok(!(await admin.locator('#adminStatus').textContent()).includes('unsupported'));
  await createTeam(admin, config.report_one); await createTeam(admin, config.report_two);
  const teams = await json(admin, '/api/admin/v2/paper-teams/query?page=1&limit=100');
  const paperOne = teams.payload.items.find((item) => item.name === config.report_one).id; const paperTwo = teams.payload.items.find((item) => item.name === config.report_two).id;

  const writerSession = await login(config.writer, 'write', 'writer'); const writer = writerSession.page;
  await openReport(writer, config.report_one); await saveMetadata(writer, config.report_one, 'Department Alpha', 'School Alpha');
  let build = await json(writer, `/api/v2/papers/${paperOne}/builds`); assert.equal(build.payload.build.current_build_id, null); assert.equal(build.payload.build.active_build_id, null);
  await compile(writer, paperOne); let text = await pdfText(writer, paperOne);
  for (const value of [config.report_one, 'Alice Single', 'Grace Guide', 'Department Alpha', 'School Alpha']) assert.ok(text.includes(value), `first report PDF missing ${value}: ${text}`);
  assert.equal((await editorText(writer)).match(/LATEX_CORE_SINGLE_SOURCE_BINDINGS/g)?.length, 1);
  assert.ok((await editorText(writer)).includes('\\input{.latex-core/frontmatter/Front-Matter.tex}'));

  assert.ok(await pdfPageCount(writer, paperOne) >= 3, 'the initial manual PDF must have a later reading page');
  await writer.locator('#pdfViewport [data-page="3"]').waitFor({ state: 'attached' });
  const beforeRebuild = await writer.evaluate(() => { const scroll = document.querySelector('#pdfScroll'); const page = document.querySelector('#pdfViewport [data-page="3"]'); scroll.scrollTop = page.offsetTop + page.offsetHeight * 0.42; scroll.dispatchEvent(new Event('scroll')); return { page: page.dataset.page, top: scroll.scrollTop }; });
  await writer.waitForTimeout(150); assert.equal(await writer.locator('#pdfPage').inputValue(), '3');
  await saveMetadata(writer, config.report_one, 'Department Alpha Revised', 'School Alpha');
  assert.equal(await writer.locator('#pdfPage').inputValue(), '3', 'saved per-report reading position must survive metadata refresh');
  assert.equal(await writer.locator('#locateInPdf').isDisabled(), true, 'stale PDF must not offer source navigation');
  await compile(writer, paperOne); text = await pdfText(writer, paperOne); assert.match(text, /Department Alpha\s*Revised/, text);
  const afterRebuild = await writer.evaluate(() => { const scroll = document.querySelector('#pdfScroll'); const page = document.querySelector('#pdfViewport [data-page="3"]'); return { current: document.querySelector('#pdfPage').value, fraction: (scroll.scrollTop - page.offsetTop) / page.offsetHeight }; });
  assert.equal(afterRebuild.current, '3'); assert.ok(afterRebuild.fraction > 0.30 && afterRebuild.fraction < 0.55, JSON.stringify({ beforeRebuild, afterRebuild }));
  const locateLine = writer.locator('.cm-line').filter({ hasText: 'Locate target phrase.' }); await locateLine.click(); await writer.locator('#locateInPdf').click();
  await writer.locator('#writerNotice').filter({ hasText: /Located|Approximately located/ }).waitFor(); assert.equal(await writer.locator('#pdfPage').inputValue(), '3');

  await writer.locator('#documentDetails').click(); await writer.locator('#drawerClose').click();
  const target = writer.locator('.cm-line').filter({ hasText: 'Table target.' }); await target.click(); await writer.keyboard.press('Home'); for (let i = 0; i < 13; i += 1) await writer.keyboard.press('Shift+ArrowRight');
  await writer.locator('#insertMenu').click(); await writer.getByRole('button', { name: 'Table', exact: true }).click();
  const body = writer.locator('#dialogBody'); const field = (text) => body.locator('label').filter({ hasText: text }).first();
  await field('Rows').locator('input').fill('2'); await field('Columns').locator('input').fill('2');
  await field('Column widths').locator('input').fill(',5cm'); await field('Selected cell (row,column').locator('input').fill('2,1');
  await field('Selected cell content').locator('textarea').fill('First line\nSecond line');
  await field('shared column width').locator('input').fill('3cm'); await field('shared row minimum height').locator('input').fill('8mm'); await field('Caption').locator('input').fill('Selected cell dimensions');
  await writer.locator('#dialogActions').getByRole('button', { name: 'Insert' }).click();
  const tableSource = await editorText(writer); assert.ok(tableSource.indexOf('\\caption{Selected cell dimensions}') < tableSource.indexOf('\\begin{tabular}{p{3cm}p{5cm}}')); assert.ok(tableSource.includes('\\rule{0pt}{8mm}\\shortstack{First line\\\\Second line}'));
  await compile(writer, paperOne); assert.ok((await pdfText(writer, paperOne)).includes('Selected cell dimensions'));

  await openReport(writer, config.report_two); await saveMetadata(writer, config.report_two, 'Department Beta', 'School Beta'); await compile(writer, paperTwo);
  const secondText = await pdfText(writer, paperTwo); assert.ok(secondText.includes('Department Beta')); assert.ok(!secondText.includes('Department Alpha Revised'));
  assert.equal(await writer.locator('#pdfPage').inputValue(), '1', 'another report must not inherit report-one PDF position');

  await openReport(writer, config.report_one);
  await writer.waitForFunction(() => !document.querySelector('#compilePaper').disabled && !document.querySelector('#sendReview').disabled);
  const sentForReview = writer.waitForResponse((response) => response.request().method() === 'POST' && new URL(response.url()).pathname === `/api/v2/reviews/papers/${paperOne}/rounds`);
  await writer.getByRole('button', { name: 'Send for review' }).click(); assert.equal((await sentForReview).status(), 201);
  await writer.locator('#reviewStateBadge').filter({ hasText: 'In Review' }).waitFor(); assert.equal(await writer.locator('#compilePaper').isDisabled(), true);
  const mentorSession = await login(config.mentor, 'review', 'mentor'); const mentor = mentorSession.page;
  await mentor.locator('#assignedPapers button').filter({ hasText: config.report_one }).click();
  await mentor.locator('#reviewGateBadge').filter({ hasText: 'Review open' }).waitFor();
  await mentor.locator('#reviewFiles button').filter({ hasText: 'second.tex' }).click();
  await mentor.locator('.cm-line').filter({ hasText: 'Mentor multiline first.' }).waitFor(); await selectAcrossLines(mentor, 'Mentor multiline first.');
  await mentor.locator('#reviewSelection').waitFor(); await mentor.waitForFunction(() => !document.querySelector('#reviewSelection').disabled); await mentor.locator('#reviewSelection').click();
  await mentor.locator('#threadMessage').fill('First paragraph.\nSecond paragraph retained.'); await mentor.locator('#createComment').click(); await mentor.locator('#draftStatus').filter({ hasText: 'Draft saved' }).waitFor();
  mentor.once('dialog', (dialog) => dialog.accept()); await mentor.locator('#pushReview').click(); await mentor.locator('#draftStatus').filter({ hasText: 'Review submitted' }).waitFor();
  await writer.waitForFunction(() => !document.querySelector('#compilePaper').disabled); await writer.locator('#commentsToggle').click();
  const card = writer.locator('#writerReviewList .thread-card').filter({ hasText: 'First paragraph.' }); await card.waitFor();
  const cardText = await card.textContent(); assert.ok(cardText.includes('Second paragraph retained.'));
  assert.ok(cardText.includes('Mentor multiline first.') && cardText.includes('Mentor multiline second.'), 'published anchor must retain both selected lines');
  assert.match(await card.locator('.reviewed-location').textContent(), /second\.tex · reviewed lines 2–3/);
  await card.locator('.thread-title').click(); await writer.locator('#currentFile').filter({ hasText: 'second.tex' }).waitFor();
  const firstRange = writer.locator('.cm-line').filter({ hasText: 'Mentor multiline first.' }).first();
  const secondRange = writer.locator('.cm-line').filter({ hasText: 'Mentor multiline second.' }).first();
  await writer.waitForFunction(() => document.querySelectorAll('.cm-selectionBackground').length > 0);
  assert.ok((await firstRange.boundingBox()) && (await secondRange.boundingBox()), 'the exact second-file multiline source range must remain present');
  assert.ok((await editorText(writer)).includes('Mentor multiline first.'));
  const changedAnchor = writer.locator('.cm-line').filter({ hasText: 'Mentor multiline first.' }); await changedAnchor.click(); await writer.keyboard.press('Home');
  for (let index = 0; index < 6; index += 1) await writer.keyboard.press('Shift+ArrowRight'); await writer.keyboard.type('Changed'); await writer.locator('#saveFile').click();
  await card.locator('.thread-title').click(); await writer.locator('#writerNotice').filter({ hasText: 'Original reviewed excerpt' }).waitFor();
  assert.equal(await card.locator('.thread-title').isDisabled(), true); assert.ok((await card.locator('.reviewed-location').textContent()).includes('second.tex'));

  const metadata = await json(writer, `/api/v2/papers/${paperOne}/project-metadata`); assert.equal(metadata.payload.values.department_display_names[0].display_name, 'Department Alpha Revised');
  const client = await json(admin, '/api/admin/integration/v1/clients', { method: 'POST', body: { name: 'Professor remaining browser', scopes: ['reports.read'], report_ids: [paperOne], institution_wide: false, expires_at: null } });
  assert.equal(client.status, 201); const jobsBeforeRead = await json(writer, `/api/v2/papers/${paperOne}/builds`);
  const external = await json(admin, `/api/integration/v1/reports/${paperOne}/front-matter`, { headers: { Authorization: `Bearer ${client.payload.secret}` } }); assert.equal(external.status, 200);
  assert.equal(external.payload.project_metadata.values.department_display_names[0].display_name, 'Department Alpha Revised');
  const jobsAfterRead = await json(writer, `/api/v2/papers/${paperOne}/builds`);
  assert.equal(jobsAfterRead.payload.build.current_build_id, jobsBeforeRead.payload.build.current_build_id);
  assert.equal(jobsAfterRead.payload.build.source_sequence, jobsBeforeRead.payload.build.source_sequence);

  await writer.locator('#fileTree button').filter({ hasText: 'main.tex' }).click(); await writer.locator('.cm-content').waitFor();
  await writer.locator('#pdfPage').fill('4'); await writer.locator('#pdfPage').dispatchEvent('change'); assert.equal(await writer.locator('#pdfPage').inputValue(), '4');
  await writer.locator('.cm-content').click(); await writer.keyboard.press('Control+A');
  await writer.keyboard.insertText(String.raw`\documentclass{article}
\newcommand{\thesistitle}{Placeholder Title}
\newcommand{\studentAname}{Placeholder Student}
\newcommand{\studentAregno}{Placeholder Registration}
\newcommand{\projguidename}{Placeholder Guide}
\newcommand{\hoddept}{Placeholder Department}
\newcommand{\schoolname}{Placeholder School}
\input{.latex-core/frontmatter/Front-Matter.tex} % LATEX_CORE_SINGLE_SOURCE_BINDINGS
\begin{document}\thesistitle\end{document}
`);
  await writer.locator('#saveFile').click(); await writer.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
  const beforeShortBuild = await json(writer, `/api/v2/papers/${paperOne}/builds`); assert.equal(beforeShortBuild.payload.build.current_build_id, jobsAfterRead.payload.build.current_build_id, 'editing must not auto-compile');
  await compile(writer, paperOne); assert.equal(await pdfPageCount(writer, paperOne), 1); assert.equal(await writer.locator('#pdfPage').inputValue(), '1', 'shorter replacement PDF must clamp the saved page');

  assert.deepEqual(failures, []); console.log(JSON.stringify({ single_source_gui_reports: 2, immutable_template: 'verified by Rust fixture', metadata_names: 'save/reload/API', table_selected_cell: 'compiled', pdf_scroll: afterRebuild, shorter_pdf_clamped_to: 1, locate_source: 3, annotation: 'multiline second-file exact plus fallback', browser_errors: failures.length }));
  await Promise.all([adminSession.context.close(), writerSession.context.close(), mentorSession.context.close()]);
} finally { await browser.close(); }
