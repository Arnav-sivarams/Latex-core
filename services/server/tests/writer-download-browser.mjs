// Invoked by the disposable database regression after a real successful build.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chromium } from 'playwright';
import * as pdfjs from '../../../node_modules/pdfjs-dist/legacy/build/pdf.mjs';
import { createHash } from 'node:crypto';

const config = JSON.parse(process.env.LATEX_CORE_WRITER_DOWNLOAD_CONFIG);
const temporary = await mkdtemp(join(tmpdir(), 'latex-core-writer-download-'));
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || chromium.executablePath(), args: ['--no-sandbox'] });
try {
  const context = await browser.newContext({ acceptDownloads: true });
  context.setDefaultTimeout(30000);
  const [name, ...parts] = config.cookie.split('=');
  await context.addCookies([{ name, value: parts.join('='), url: config.base }]);
  if (config.nonleaderCookie) {
    const other = await browser.newContext();
    other.setDefaultTimeout(30000);
    const [otherName, ...otherValue] = config.nonleaderCookie.split('=');
    await other.addCookies([{ name: otherName, value: otherValue.join('='), url: config.base }]);
    const view = await other.newPage();
    await view.goto(`${config.base}/write?paper=${config.paperId}`);
    await view.locator('.cm-editor').waitFor();
    if (!await view.locator('#documentDetailsPanel').isVisible()) await view.locator('#documentDetails').click();
    const panel = view.locator('#documentDetailsBody');
    await panel.getByText(`Course code: ${config.registration.course}`, { exact: true }).waitFor();
    for (const key of ['course_code', 'team_academic_year', 'team_semester']) assert.equal(await panel.locator(`[name="field:${key}"]`).count(), 0);
    assert.ok((await panel.innerText()).includes(config.registration.year));
    assert.ok((await panel.innerText()).includes(config.registration.semester));
    await other.close();
  }
  const detailsResponse = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/document-details`);
  assert.equal(detailsResponse.status(),200);
  const details = await detailsResponse.json();
  console.log('WRITER_BROWSER_STAGE: registration resolved');
  assert.equal(details.single_source,true, 'Verification must use an actual Complete Report');
  for (const key of ['course_code','team_academic_year','team_semester']) {
    const value = details.values.find(item => item.field_key === key)?.value;
    assert.ok(typeof value === 'string' && value.length, `Assigned Leader registration is unresolved: ${key}`);
  }
  const page = await context.newPage();
  await page.goto(`${config.base}/write?paper=${config.paperId}`);
  await page.locator('.cm-editor').waitFor();
  console.log('WRITER_BROWSER_STAGE: served editor loaded');
  if (!await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#documentDetails').click();
  await page.locator('#documentDetailsBody').waitFor();
  for (const key of ['course_code','team_academic_year','team_semester']) assert.equal(await page.locator(`#documentDetailsBody [name="field:${key}"]`).count(), 0);
  if (await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#drawerClose').click();
  await page.locator('#downloadPdf').waitFor({ state: 'attached' });
  if (!config.buildId) {
    const response = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/builds`);
    assert.equal(response.status(),200);
    config.buildId = (await response.json()).build.current_build_id;
    assert.ok(config.buildId, 'The normal-app report must have a successful PDF for download verification');
  }
  await page.waitForFunction(() => !document.querySelector('#downloadPdf')?.disabled);
  console.log('WRITER_BROWSER_STAGE: PDF download ready');
  assert.equal(await page.locator('#downloadMenu').count(), 0);
  assert.equal(await page.locator('#downloadPdf').innerText(), 'Download PDF');
  assert.equal(await page.locator('#downloadSource').count(), 0);
  assert.equal(await page.getByText(/Download Source|Source\s*(?:\(\.zip\)|ZIP|Install)/i).count(), 0);
  assert.equal(await page.locator('#setMain').count(), 0);
  assert.equal(await page.locator('#mainBadge').isVisible(), true);
  const paperResponse = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}`);
  assert.equal(paperResponse.status(), 200);
  const mainPath = (await paperResponse.json()).main_file;
  const mainActions = page.getByRole('button', { name: `File actions for ${mainPath}`, exact: true });
  for (const open of [
    () => page.locator('#fileActionsToggle').click(),
    () => mainActions.click(),
    () => mainActions.locator('..').locator('button').first().click({ button: 'right' }),
  ]) {
    await open();
    await page.locator('#fileActionsMenu').waitFor({ state: 'visible' });
    assert.equal(await page.locator('#fileActionsMenu').getByRole('menuitem', { name: /set main/i }).count(), 0);
    await page.keyboard.press('Escape');
  }
  for (const id of ['commandPalette', 'moreActions']) {
    await page.locator(`#${id}`).click();
    await page.locator('#dialogBody').waitFor({ state: 'visible' });
    assert.equal(await page.locator('#dialogBody').getByRole('button', { name: /set main/i }).count(), 0);
    await page.keyboard.press('Escape');
  }
  const denied = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/source.zip`);
  assert.equal(denied.status(), 403);
  assert.match((await denied.json()).error, /disabled/i);
  const downloaded = page.waitForEvent('download');
  await page.locator('#downloadPdf').click();
  const download = await downloaded;
  const target = join(temporary, 'report.pdf');
  await download.saveAs(target);
  assert.equal((await readFile(target)).subarray(0, 5).toString(), '%PDF-');
  const pdf = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/artifacts/pdf?build=${config.buildId}&download=true`);
  assert.equal(pdf.status(), 200);
  assert.deepEqual(await pdf.body(), await readFile(target));
  console.log('WRITER_BROWSER_STAGE: downloaded PDF verified');
  if (config.editableSave) {
    await page.locator('#documentDetails').click();
    const panel = page.locator('#documentDetailsBody');
    await panel.locator('[name="field:course_name"]').waitFor();
    assert.deepEqual((await panel.locator('input,select,textarea').evaluateAll(elements => elements.map(element => element.name))).sort(),
      ['field:course_name', 'field:project_title', 'field:submission_date']);
    for (const key of ['course_code', 'team_academic_year', 'team_semester']) assert.equal(await panel.locator(`[name="field:${key}"]`).count(), 0);
    await panel.locator('[name="field:course_name"]').fill('Browser intentional course name');
    await panel.locator('[name="field:project_title"]').fill('Browser saved report title');
    await panel.locator('[name="field:submission_date"]').fill('2026-07-11');
    const saved = page.waitForResponse(response => response.request().method() === 'PUT' && response.url().endsWith('/document-details'));
    await panel.getByRole('button', { name: 'Save document details', exact: true }).click();
    const response = await saved;
    assert.equal(response.status(), 200);
    console.log('WRITER_BROWSER_STAGE: authorized Save completed');
    assert.deepEqual(Object.keys(response.request().postDataJSON().values).sort(), ['course_name','project_title','submission_date']);
    await page.waitForFunction(() => document.querySelector('.cm-content')?.cmTile?.root?.view?.state.doc.toString().includes('\\newcommand{\\coursename}{Browser intentional course name}'));
    if (await page.locator('#documentDetailsPanel').isVisible()) await page.locator('#drawerClose').click();
    await page.locator('#documentDetails').click();
    await panel.locator('[name="field:course_name"]').waitFor();
    assert.equal(await panel.locator('[name="field:course_name"]').inputValue(), 'Browser intentional course name');
    await page.locator('#drawerClose').click();
    const submitted = page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/builds'));
    await page.locator('#compilePaper').click();
    const build = await (await submitted).json();
    assert.notEqual(build.build_id, config.buildId);
    let current;
    for (let attempt = 0; attempt < 240; attempt++) {
      const result = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/builds`);
      assert.equal(result.status(), 200);
      current = (await result.json()).build;
      if (current.current_build_id === build.build_id) break;
      assert.ok(!/failed|timed_out|cancelled/i.test(current.latest_status || ''), JSON.stringify(current.latest_error));
      await page.waitForTimeout(1000);
    }
    assert.equal(current.current_build_id, build.build_id);
    assert.equal(current.current_source_sequence, current.source_sequence);
    await page.waitForFunction(() => /PDF matches source version/i.test(document.querySelector('#pdfRelation')?.innerText || ''), null, { timeout: 30000 });
    const updated = await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/artifacts/pdf?build=${build.build_id}&download=true`);
    assert.equal(updated.status(), 200);
    assert.equal((await updated.body()).subarray(0,5).toString(), '%PDF-');
    const bytes = await updated.body();
    const task = pdfjs.getDocument({ data: new Uint8Array(bytes), disableWorker: true });
    const document = await task.promise;
    let text = '';
    for (let number = 1; number <= document.numPages; number++) text += (await (await document.getPage(number)).getTextContent()).items.map(item => item.str).join(' ') + '\n';
    await task.destroy();
    assert.ok(text.includes('Browser intentional course name'), text);
    assert.ok(text.includes('Browser saved report title'), text);
    console.log(`BROWSER_PDF: sha256=${createHash('sha256').update(bytes).digest('hex')}`);
    console.log(`BROWSER_SAVE_COMPILE: PASS; submitted=${build.build_id}; current=${current.current_build_id}; source=${current.source_sequence}; artifact_source=${current.current_source_sequence}`);
  }
  console.log('WRITER_DOWNLOAD_BROWSER: PASS — PDF only; Source API denied; actual compiled PDF downloaded.');
} finally {
  await browser.close();
  await rm(temporary, { recursive: true, force: true });
}
