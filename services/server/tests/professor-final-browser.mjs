import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { deflateSync } from 'node:zlib';
import { chromium } from 'playwright';

const execute = promisify(execFile);
const base = process.env.PROFESSOR_FINAL_BASE_URL;
const adminEmail = process.env.PROFESSOR_FINAL_ADMIN_EMAIL;
const adminPasswordFile = process.env.PROFESSOR_FINAL_ADMIN_PASSWORD_FILE;
const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome';
if (!base || !adminEmail || !adminPasswordFile) throw new Error('PROFESSOR_FINAL_BASE_URL, PROFESSOR_FINAL_ADMIN_EMAIL, and PROFESSOR_FINAL_ADMIN_PASSWORD_FILE are required');

let assertions = 0;
let compileJobs = 0;
let successfulBuilds = 0;
let expectedFailedBuilds = 0;
let freshPdfs = 0;
const results = {};
const browserFailures = [];
const disposableEmails = [];
const suffix = randomBytes(4).toString('hex');
const accountPassword = `Final-${randomBytes(24).toString('base64url')}`;
const requestedAreas = new Set((process.env.PROFESSOR_FINAL_AREAS || '').split(',').filter(Boolean));

function equal(actual, expected, message) { assertions += 1; assert.equal(actual, expected, message); }
function ok(value, message) { assertions += 1; assert.ok(value, message); }
function match(value, pattern, message) { assertions += 1; assert.match(value, pattern, message); }
async function area(name, task) {
  if (requestedAreas.size && !requestedAreas.has(name)) { results[name] = { result: 'SKIPPED' }; return; }
  try { const detail = await task(); results[name] = { result: 'PASS', ...(detail || {}) }; }
  catch (error) { results[name] = { result: 'FAIL', error: error.message }; }
}
async function closeModal(page) {
  if (await page.locator('#productivityDialog[open]').count()) await page.keyboard.press('Escape');
  await page.locator('#productivityDialog').waitFor({ state: 'hidden' }).catch(() => null);
}
function tracked(page, label) {
  page.on('pageerror', (error) => browserFailures.push(`${label}: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error' && !message.text().startsWith('Failed to load resource:')) browserFailures.push(`${label}: ${message.text()}`);
  });
}
async function newPage(browser, label) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage(); tracked(page, label); return { context, page };
}
async function login(page, email, password, destination) {
  await page.goto(base, { waitUntil: 'domcontentloaded' });
  await page.locator('#email').fill(email); await page.locator('#password').fill(password);
  await Promise.all([page.waitForURL(/\/(?:home|admin|write|review)$/), page.getByRole('button', { name: 'Sign in' }).click()]);
  if (!new URL(page.url()).pathname.endsWith(destination)) await page.goto(`${base}${destination}`, { waitUntil: 'domcontentloaded' });
}
async function json(page, path, options = {}) {
  return page.evaluate(async ({ path, options }) => {
    const response = await fetch(path, {
      credentials: 'same-origin', method: options.method || 'GET',
      headers: options.body === undefined ? options.headers : { 'Content-Type': 'application/json', ...(options.headers || {}) },
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { path, options });
}
async function archivePost(page, archivePath, fields) {
  const archive = (await readFile(archivePath)).toString('base64');
  return page.evaluate(async ({ archive, fields }) => {
    const bytes = Uint8Array.from(atob(archive), (character) => character.charCodeAt(0));
    const form = new FormData(); Object.entries(fields).forEach(([key, value]) => form.set(key, value));
    form.set('archive', new File([bytes], 'professor-final.zip', { type: 'application/zip' }));
    const response = await fetch('/api/admin/v2/templates/import', { method: 'POST', credentials: 'same-origin', body: form });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { archive, fields });
}
function pngChunk(type, data) {
  const name = Buffer.from(type); const body = Buffer.concat([name, data]); let crc = 0xffffffff;
  for (const byte of body) { crc ^= byte; for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0); }
  const length = Buffer.alloc(4); length.writeUInt32BE(data.length); const checksum = Buffer.alloc(4); checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([length, body, checksum]);
}
function png() {
  const header = Buffer.alloc(13); header.writeUInt32BE(2, 0); header.writeUInt32BE(2, 4); header.set([8, 6, 0, 0, 0], 8);
  const pixels = Buffer.from([0, 210, 25, 25, 255, 30, 160, 40, 255, 0, 25, 25, 210, 255, 220, 140, 20, 255]);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), pngChunk('IHDR', header), pngChunk('IDAT', deflateSync(pixels)), pngChunk('IEND', Buffer.alloc(0))]);
}
async function jpeg(page) {
  const encoded = await page.evaluate(() => {
    const canvas = document.createElement('canvas'); canvas.width = 100; canvas.height = 70;
    const context = canvas.getContext('2d'); context.fillStyle = '#b23422'; context.fillRect(0, 0, 100, 70);
    context.fillStyle = '#ffffff'; context.font = '20px sans-serif'; context.fillText('JPEG', 20, 42);
    return canvas.toDataURL('image/jpeg', 0.9).split(',')[1];
  });
  return Buffer.from(encoded, 'base64');
}
async function openReport(page, name) {
  const button = page.locator('#teamPapers button').filter({ hasText: name }).first(); await button.waitFor({ state: 'visible' }); await button.click();
  await page.locator('#editorMount .cm-content').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
}
async function editorText(page) { return page.locator('#editorMount .cm-content').innerText(); }
async function waitText(page, value) { await page.waitForFunction((needle) => document.querySelector('#editorMount .cm-content')?.innerText.includes(needle), value); }
async function selectText(page, text, start = 0, length = text.length) {
  const line = page.locator('#editorMount .cm-line').filter({ hasText: text }).first(); await line.click(); await page.keyboard.press('Home');
  for (let index = 0; index < start; index += 1) await page.keyboard.press('ArrowRight');
  for (let index = 0; index < length; index += 1) await page.keyboard.press('Shift+ArrowRight');
}
async function authoritativeSource(page, paperId) {
  const files = await json(page, `/api/v2/papers/${paperId}/files`); const main = files.payload.find((file) => file.path === 'main.tex');
  return (await json(page, `/api/v2/papers/${paperId}/files/${main.file_id}`)).payload.content;
}
async function requestSave(page) {
  await page.locator('#saveFile').evaluate((button) => { if (!button.disabled) button.click(); });
}
async function setSource(page, paperId, source) {
  const content = page.locator('#editorMount .cm-content'); await content.click(); await page.keyboard.press('Control+A'); await page.keyboard.insertText(source);
  await requestSave(page);
  await page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced', null, { timeout: 30_000 });
  const until = Date.now() + 30_000;
  while (Date.now() < until) { if (await authoritativeSource(page, paperId) === source) return; await page.waitForTimeout(100); }
  throw new Error('authoritative source did not match editor source');
}
async function upload(page, paperId, path, bytes, mime) {
  const detail = await json(page, `/api/v2/papers/${paperId}`);
  return page.evaluate(async ({ paperId, path, version, encoded, mime }) => {
    const body = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
    const response = await fetch(`/api/v2/papers/${paperId}/assets?path=${encodeURIComponent(path)}&version=${version}`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': mime }, body });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { paperId, path, version: detail.payload.version, encoded: bytes.toString('base64'), mime });
}
async function build(page, paperId, expected) {
  const before = await json(page, `/api/v2/papers/${paperId}/builds`); const previousLatest = before.payload.build.latest_build_id;
  const previousCurrent = before.payload.build.current_build_id;
  await page.locator('#compilePaper').click(); compileJobs += 1;
  const until = Date.now() + 150_000;
  while (Date.now() < until) {
    const state = await json(page, `/api/v2/papers/${paperId}/builds`); const buildState = state.payload.build;
    if (!buildState.active_build_id && buildState.latest_build_id && buildState.latest_build_id !== previousLatest) {
      if (expected === 'success') {
        equal(buildState.latest_status, 'succeeded', JSON.stringify(buildState.latest_error));
        equal(buildState.current_build_id, buildState.latest_build_id, 'successful build was not promoted exactly'); successfulBuilds += 1;
      } else {
        equal(buildState.latest_status, 'failed', 'negative build unexpectedly succeeded');
        equal(buildState.current_build_id, previousCurrent, 'failed build replaced the last-good PDF'); expectedFailedBuilds += 1;
      }
      return { ...buildState, previousCurrent };
    }
    await page.waitForTimeout(300);
  }
  throw new Error(`timed out waiting for ${expected} build`);
}
async function pdfEvidence(page, paperId, buildId) {
  const evidence = await page.evaluate(async ({ paperId, buildId }) => {
    const response = await fetch(`/api/v2/papers/${paperId}/artifacts/pdf?build=${buildId}`, { credentials: 'same-origin' });
    const bytes = new Uint8Array(await response.arrayBuffer());
    const pdfjs = await import('/static/pdf.min.mjs'); pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const magic = new TextDecoder().decode(bytes.slice(0, 5));
    const document = await pdfjs.getDocument({ data: bytes }).promise; let images = 0; let text = '';
    for (let number = 1; number <= document.numPages; number += 1) {
      const pdfPage = await document.getPage(number); const operators = await pdfPage.getOperatorList();
      images += operators.fnArray.filter((operation) => [pdfjs.OPS.paintImageXObject, pdfjs.OPS.paintInlineImageXObject, pdfjs.OPS.paintImageMaskXObject].includes(operation)).length;
      const content = await pdfPage.getTextContent(); text += `${content.items.map((item) => item.str).join(' ')}\n`;
    }
    return { status: response.status, magic, pages: document.numPages, images, text };
  }, { paperId, buildId });
  equal(evidence.status, 200); equal(evidence.magic, '%PDF-'); freshPdfs += 1; return evidence;
}
async function dragSplitter(page, splitter, delta) {
  const box = await splitter.boundingBox(); ok(box, 'splitter missing');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down();
  await page.mouse.move(box.x + delta, box.y + box.height / 2, { steps: 5 }); await page.mouse.move(5, 5); await page.mouse.up();
}

const temporary = await mkdtemp(join(tmpdir(), 'latex-core-final-browser-'));
const source = String.raw`\documentclass{article}
\usepackage{graphicx}
\usepackage{amsmath}
\usepackage{wrapfig}
\begin{document}
UPLOADSLOT
InlineSlot
EmptySlot
DELETEWORD remains.
DELETELINE whole line.
COMMENTLINE stays.
\newpage
\input{chapters/second.tex}
\end{document}
`;
await writeFile(join(temporary, 'main.tex'), source);
await execute('mkdir', ['-p', join(temporary, 'chapters'), join(temporary, 'images')]);
await writeFile(join(temporary, 'chapters/second.tex'), 'Nested exact SyncTeX target.\nSecond nested line.\n');
await writeFile(join(temporary, 'images/seed.png'), png());
await execute('zip', ['-q', '-r', join(temporary, 'fixture.zip'), 'main.tex', 'chapters', 'images'], { cwd: temporary });

const browser = await chromium.launch({ headless: true, executablePath });
let admin;
const sessions = [];
try {
  admin = await newPage(browser, 'admin'); sessions.push(admin); await login(admin.page, adminEmail, (await readFile(adminPasswordFile, 'utf8')).trim(), '/admin');
  const people = {};
  for (const [key, role] of [['leader', 'writer'], ['writer', 'writer'], ['mentor', 'mentor']]) {
    const email = `prof-final-${key}-${suffix}@example.invalid`; disposableEmails.push(email);
    const response = await json(admin.page, '/api/admin/v2/users', { method: 'POST', body: { email, password: accountPassword, role, generate_temporary_password: false } });
    equal(response.status, 201, JSON.stringify(response.payload)); people[key] = { email, id: response.payload.user_id };
  }
  const imported = await archivePost(admin.page, join(temporary, 'fixture.zip'), { name: `Professor final ${suffix}`, description: 'Browser closure fixture', main: 'main.tex', arrangement: 'REPORT_CONTENT_ONLY' });
  equal(imported.status, 201, JSON.stringify(imported.payload));
  const reportName = `Professor Final Team ${suffix}`;
  const team = await json(admin.page, '/api/admin/v2/paper-teams', { method: 'POST', body: { name: reportName, template_id: imported.payload.id, leader_writer_id: people.leader.id, writer_ids: [people.leader.id, people.writer.id], mentor_ids: [people.mentor.id] } });
  equal(team.status, 201, JSON.stringify(team.payload)); const paperId = team.payload.team.id;

  const leader = await newPage(browser, 'leader'); const writer = await newPage(browser, 'writer'); const mentor = await newPage(browser, 'mentor');
  sessions.push(leader, writer, mentor); await login(leader.page, people.leader.email, accountPassword, '/write'); await login(writer.page, people.writer.email, accountPassword, '/write'); await login(mentor.page, people.mentor.email, accountPassword, '/review');
  await openReport(leader.page, reportName); await openReport(writer.page, reportName);
  await mentor.page.locator('#assignedPapers button').filter({ hasText: reportName }).click(); await mentor.page.locator('#reviewEditor .cm-content').waitFor();

  await area('R17_R19_R20', async () => {
    equal(await leader.page.locator('#newFile').getAttribute('aria-label'), 'New file'); equal(await leader.page.locator('#uploadImage').getAttribute('aria-label'), 'Upload image');
    equal((await leader.page.locator('#newFile').textContent()) === (await leader.page.locator('#uploadImage').textContent()), false, 'icons are indistinguishable');
    const labels = await leader.page.locator('#fileTree').allTextContents(); ok(labels.some((text) => text.includes('Images'))); ok(labels.every((text) => !text.includes('legacy')));
    await leader.page.locator('#insertMenu').click(); await leader.page.locator('#dialogBody .palette-category').first().waitFor();
    const categories = await leader.page.locator('#dialogBody .palette-category').allTextContents();
    for (const category of ['Structure', 'Media', 'Math', 'References']) ok(categories.includes(category), `missing ${category}: ${JSON.stringify(categories)}`);
    const dialog = await leader.page.locator('#dialogBody').innerText();
    ok(!/Comment selected|Uncomment selected/.test(dialog));
    for (const action of ['Figure', 'Wrap figure', 'Inline math', 'Display math', 'Table']) ok(dialog.includes(action), `missing ${action}`);
    await closeModal(leader.page);
  });
  await closeModal(leader.page);

  await area('R11_R13', async () => {
    await selectText(leader.page, 'DELETEWORD', 0, 'DELETEWORD'.length);
    const visible = await leader.page.locator('.cm-selectionBackground').evaluateAll((nodes) => nodes.map((node) => ({ color: getComputedStyle(node).backgroundColor, width: node.getBoundingClientRect().width })));
    ok(visible.some((item) => item.width > 0 && item.color !== 'rgba(0, 0, 0, 0)'), JSON.stringify(visible));
    await leader.page.locator('#insertMenu').focus(); ok((await leader.page.locator('.cm-selectionBackground').count()) > 0, 'unfocused selection disappeared');
    await leader.page.locator('#editorSettings summary').click(); await leader.page.locator('#editorTheme').selectOption('DARK'); await leader.page.waitForTimeout(200); await selectText(leader.page, 'DELETEWORD');
    const dark = await leader.page.locator('.cm-selectionBackground').evaluate((node) => getComputedStyle(node).backgroundColor); ok(dark !== 'rgba(0, 0, 0, 0)');
    await leader.page.keyboard.press('Delete'); await writer.page.waitForFunction(() => !document.querySelector('.cm-content')?.innerText.includes('DELETEWORD')); await leader.page.locator('#undoText').click(); await waitText(writer.page, 'DELETEWORD');
    await selectText(leader.page, 'DELETELINE whole line.'); await leader.page.keyboard.press('Backspace'); await writer.page.waitForFunction(() => !document.querySelector('.cm-content')?.innerText.includes('DELETELINE')); await leader.page.locator('#undoText').click(); await waitText(writer.page, 'DELETELINE');
    equal(await mentor.page.locator('#reviewEditor .cm-content').getAttribute('contenteditable'), 'false'); const before = await mentor.page.locator('#reviewEditor .cm-content').innerText();
    const mentorLine = mentor.page.locator('#reviewEditor .cm-line').filter({ hasText: 'UPLOADSLOT' }); const mentorBox = await mentorLine.boundingBox(); ok(mentorBox, 'Mentor source line was not rendered');
    await mentor.page.mouse.move(mentorBox.x + 2, mentorBox.y + mentorBox.height / 2); await mentor.page.mouse.down(); await mentor.page.mouse.move(mentorBox.x + mentorBox.width - 2, mentorBox.y + mentorBox.height / 2, { steps: 6 }); await mentor.page.mouse.up();
    const mentorSelection = await mentor.page.evaluate(() => ({ drawn: document.querySelectorAll('#reviewEditor .cm-selectionBackground').length, native: window.getSelection()?.toString() || '' })); ok(mentorSelection.drawn > 0 || mentorSelection.native.includes('UPLOADSLOT'), `Mentor selection was not visible: ${JSON.stringify(mentorSelection)}`);
    await mentor.page.locator('#reviewEditor .cm-content').click(); await mentor.page.keyboard.type('MUTATION'); equal(await mentor.page.locator('#reviewEditor .cm-content').innerText(), before);
  });
  await closeModal(leader.page);

  await area('R12', async () => {
    const before = await leader.page.locator('.files-pane').evaluate((node) => node.getBoundingClientRect().width); const splitter = leader.page.locator('.pane-splitter').nth(0); const box = await splitter.boundingBox(); ok(box, 'Writer splitter missing');
    await leader.page.mouse.move(box.x + 2, box.y + 10); await leader.page.mouse.down(); await leader.page.mouse.move(box.x + 82, box.y + 10, { steps: 5 }); await leader.page.mouse.up(); await leader.page.mouse.move(5, 5);
    const after = await leader.page.locator('.files-pane').evaluate((node) => node.getBoundingClientRect().width); ok(after > before, `${after} was not wider than ${before}`); ok(after >= 150); equal(await leader.page.locator('.three-pane-workspace').getAttribute('class').then((value) => value.includes('is-resizing')), false);
    const sourceBefore = await leader.page.locator('.source-pane').evaluate((node) => node.getBoundingClientRect().width); const second = leader.page.locator('.pane-splitter').nth(1); const secondBox = await second.boundingBox(); ok(secondBox, 'Writer source/PDF splitter missing');
    await leader.page.mouse.move(secondBox.x + 2, secondBox.y + 10); await leader.page.mouse.down(); await leader.page.mouse.move(secondBox.x + 62, secondBox.y + 10, { steps: 5 }); await leader.page.mouse.up(); const sourceAfter = await leader.page.locator('.source-pane').evaluate((node) => node.getBoundingClientRect().width); ok(sourceAfter > sourceBefore); ok(sourceAfter >= 280);
    await leader.page.mouse.move(secondBox.x + 2, secondBox.y + 10); await leader.page.mouse.down(); await leader.page.mouse.move(5, 5, { steps: 5 }); await leader.page.mouse.up(); equal(await leader.page.locator('.three-pane-workspace').getAttribute('class').then((value) => value.includes('is-resizing')), false);
    await leader.page.reload(); await openReport(leader.page, reportName); const restored = await leader.page.locator('.files-pane').evaluate((node) => node.getBoundingClientRect().width); ok(Math.abs(restored - after) < 4, `${restored} vs ${after}`);
    const mentorBefore = await mentor.page.locator('.review-source').evaluate((node) => node.getBoundingClientRect().width); await dragSplitter(mentor.page, mentor.page.locator('.pane-splitter').nth(1), 70);
    const mentorAfter = await mentor.page.locator('.review-source').evaluate((node) => node.getBoundingClientRect().width); ok(mentorAfter !== mentorBefore); ok(mentorAfter >= 280);
  });
  await closeModal(leader.page);

  await area('R02_R04', async () => {
    await selectText(leader.page, 'UPLOADSLOT', 0, 0); let promptSeen = false; const handler = async (dialog) => {
      if (dialog.type() === 'prompt') { promptSeen = true; await dialog.accept('images/browser upload.png'); } else await dialog.accept();
    };
    leader.page.on('dialog', handler); const chooser = leader.page.waitForEvent('filechooser'); await leader.page.locator('#uploadImage').click();
    await (await chooser).setFiles({ name: 'browser upload.png', mimeType: 'image/png', buffer: png() });
    await leader.page.waitForFunction(() => document.querySelector('#productivityDialog')?.open && document.querySelector('#dialogTitle')?.textContent === 'Figure Builder'); leader.page.off('dialog', handler); ok(promptSeen);
    await leader.page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click(); let text = await editorText(leader.page); ok(text.indexOf('includegraphics') < text.indexOf('UPLOADSLOT'));
    await leader.page.locator('#undoText').click(); await leader.page.waitForFunction(() => !document.querySelector('.cm-content')?.innerText.includes('browser upload.png'));
    const beforeCancel = await editorText(leader.page); const cancelChooser = leader.page.waitForEvent('filechooser'); await leader.page.locator('#uploadImage').click(); await (await cancelChooser).setFiles([]); await leader.page.waitForTimeout(250); equal(await editorText(leader.page), beforeCancel);
    const badHandler = async (dialog) => dialog.type() === 'prompt' ? dialog.accept('images/rejected.svg') : dialog.dismiss(); leader.page.on('dialog', badHandler);
    const rejected = leader.page.waitForEvent('filechooser'); await leader.page.locator('#uploadImage').click(); await (await rejected).setFiles({ name: 'rejected.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg/>') });
    await leader.page.locator('#writerNotice').filter({ hasText: /unsupported|accepted|PNG|JPEG/i }).waitFor({ timeout: 10_000 }); leader.page.off('dialog', badHandler); equal(await editorText(leader.page), beforeCancel);
    await selectText(leader.page, 'InlineSlot'); await leader.page.locator('#insertMenu').click(); await leader.page.getByRole('button', { name: 'Inline math', exact: true }).click(); await waitText(leader.page, '\\(InlineSlot\\)');
    await selectText(leader.page, 'EmptySlot', 0, 0); await leader.page.locator('#insertMenu').click(); await leader.page.getByRole('button', { name: 'Inline math', exact: true }).click(); await leader.page.keyboard.type('q'); await waitText(leader.page, '\\(q\\)EmptySlot');
    await leader.page.locator('#undoText').click(); await leader.page.locator('#redoText').click(); await requestSave(leader.page); await leader.page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
    const compiled = await build(leader.page, paperId, 'success'); const pdf = await pdfEvidence(leader.page, paperId, compiled.current_build_id); ok(pdf.text.includes('InlineSlot'));
  });
  await closeModal(leader.page);

  await area('R14', async () => {
    await selectText(leader.page, 'COMMENTLINE', 'COMMENTLINE'.length, 0); await leader.page.keyboard.type(' dirty');
    const inFlight = await leader.page.evaluate(() => { document.querySelector('#saveFile').click(); const button = document.querySelector('#saveFile'); return { disabled: button.disabled, text: button.textContent }; });
    ok(inFlight.disabled); match(inFlight.text, /Saving/); await leader.page.locator('#editorMount .cm-content').click(); await leader.page.keyboard.press('Control+End'); await leader.page.keyboard.type('\n% edit-during-save');
    await leader.page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced'); await waitText(writer.page, 'edit-during-save');
    await leader.context.setOffline(true); await leader.page.locator('#editorMount .cm-content').click(); await leader.page.keyboard.press('Control+End'); await leader.page.keyboard.type('\n% retained-after-failure');
    await leader.page.locator('#saveFile').click(); await leader.page.waitForFunction(() => ['offline', 'error', 'reconnecting'].includes(document.querySelector('#saveStatus')?.dataset.state), null, { timeout: 15_000 }); ok((await editorText(leader.page)).includes('retained-after-failure'));
    await leader.context.setOffline(false); await leader.page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced', null, { timeout: 30_000 }); await waitText(writer.page, 'retained-after-failure');
    await leader.page.reload({ waitUntil: 'domcontentloaded' }); await openReport(leader.page, reportName);
  });
  await closeModal(leader.page);

  const jpegBytes = await jpeg(leader.page);
  await area('JPEG', async () => {
    for (const path of ['images/photo.jpeg', 'images/photo.jpg', 'images/photo.JPG']) { const response = await upload(leader.page, paperId, path, jpegBytes, 'image/jpeg'); equal(response.status, 201, JSON.stringify(response.payload)); }
    const documents = [
      ['jpeg', 'images/photo.jpeg'], ['jpg', 'images/photo.jpg'], ['uppercase', 'images/photo.JPG'],
    ];
    for (const [label, path] of documents) {
      await setSource(leader.page, paperId, `\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}${label}\\includegraphics[width=2cm]{${path}}\\end{document}\n`);
      const compiled = await build(leader.page, paperId, 'success'); const pdf = await pdfEvidence(leader.page, paperId, compiled.current_build_id); ok(pdf.images > 0, `${label} PDF contains no image operator`);
    }
    await setSource(leader.page, paperId, '\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\nTOOLBAR\n\\end{document}\n'); await selectText(leader.page, 'TOOLBAR');
    await leader.page.locator('#insertMenu').click(); await leader.page.getByRole('button', { name: 'Figure', exact: true }).click();
    await leader.page.locator('#dialogBody label').filter({ hasText: 'Asset' }).locator('select').selectOption('images/photo.jpeg'); await leader.page.locator('#dialogBody label').filter({ hasText: 'Caption' }).locator('input').fill('Toolbar JPEG');
    await leader.page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click(); await requestSave(leader.page); await leader.page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced');
    const compiled = await build(leader.page, paperId, 'success'); const pdf = await pdfEvidence(leader.page, paperId, compiled.current_build_id); ok(pdf.images > 0); ok(pdf.text.includes('Toolbar JPEG'));
  });
  await closeModal(leader.page);

  let lastGoodSource = await authoritativeSource(leader.page, paperId); let lastGoodBuild = (await json(leader.page, `/api/v2/papers/${paperId}/builds`)).payload.build.current_build_id;
  await area('R18_NEGATIVE', async () => {
    if (!lastGoodBuild) {
      lastGoodSource = '\\documentclass{article}\n\\begin{document}Last good baseline.\\end{document}\n'; await setSource(leader.page, paperId, lastGoodSource);
      const baseline = await build(leader.page, paperId, 'success'); lastGoodBuild = baseline.current_build_id; await pdfEvidence(leader.page, paperId, lastGoodBuild);
    }
    const negatives = [
      ['missing image', '\\documentclass{article}\\usepackage{graphicx}\\begin{document}\\includegraphics{images/genuinely-absent.png}\\end{document}'],
      ['malformed equation', '\\documentclass{article}\\begin{document}$x + \\end{document}'],
      ['undefined command', '\\documentclass{article}\\begin{document}\\ProfessorUndefinedCommand\\end{document}'],
      ['invalid environment', '\\documentclass{article}\\begin{document}\\begin{notarealenvironment}x\\end{notarealenvironment}\\end{document}'],
      ['missing package', '\\documentclass{article}\\usepackage{professor-package-does-not-exist}\\begin{document}x\\end{document}'],
    ];
    for (const [label, invalid] of negatives) {
      await setSource(leader.page, paperId, `${invalid}\n`); const failed = await build(leader.page, paperId, 'failed'); equal(failed.current_build_id, lastGoodBuild, `${label} promoted a failed PDF`);
      await leader.page.locator('#problemsToggle').click(); const problems = await leader.page.locator('#buildProblemsList').innerText(); ok(problems.trim().length > 0, `${label} had no diagnostic`);
      await leader.page.locator('#buildLogTab').click(); await leader.page.waitForFunction(() => !document.querySelector('#buildLogText')?.textContent.includes('Open Build Log')); const log = await leader.page.locator('#buildLogText').innerText(); ok(log.trim().length > 20, `${label} raw log was empty`);
    }
    await setSource(leader.page, paperId, lastGoodSource);
  });
  await closeModal(leader.page);

  await area('R15', async () => {
    const checkpoint = await json(leader.page, `/api/v2/papers/${paperId}/versions`, { method: 'POST', body: { name: 'Professor final checkpoint' } }); equal(checkpoint.status, 201, JSON.stringify(checkpoint.payload));
    await leader.page.reload({ waitUntil: 'domcontentloaded' }); await openReport(leader.page, reportName); await leader.page.locator('#historyToggle').click();
    const leaderRow = leader.page.locator('#versionHistory .version-row').filter({ hasText: 'Professor final checkpoint' }); await leaderRow.waitFor(); await leaderRow.locator('input').check();
    ok((await leaderRow.getAttribute('class')).includes('selected')); match(await leader.page.locator('#versionDiff').innerText(), /selected.*new head.*preserve/i);
    await writer.page.reload({ waitUntil: 'domcontentloaded' }); await openReport(writer.page, reportName); await writer.page.locator('#historyToggle').click(); const writerRow = writer.page.locator('#versionHistory .version-row').filter({ hasText: 'Professor final checkpoint' }); await writerRow.waitFor(); ok(await writerRow.getByRole('button', { name: /Request restore/ }).isVisible());
    const versionsBefore = (await json(leader.page, `/api/v2/papers/${paperId}/versions`)).payload.length; let confirmation = '';
    const leaderActions = await leaderRow.locator('button').allTextContents(); const restoreAction = leaderActions.find((label) => /(?:Restore|Revert).*new head/i.test(label)); ok(restoreAction, `Leader history row actions: ${JSON.stringify(leaderActions)}`);
    leader.page.once('dialog', async (dialog) => { confirmation = dialog.message(); await dialog.accept(); }); await leaderRow.locator('button').filter({ hasText: /new head/i }).click(); await leader.page.locator('#writerNotice').filter({ hasText: /new current version|new current head|reverted/i }).waitFor({ timeout: 20_000 });
    match(confirmation, /new Team head.*PRE_RESTORE_SAFETY/); const versionsAfter = (await json(leader.page, `/api/v2/papers/${paperId}/versions`)).payload.length; ok(versionsAfter > versionsBefore);
  });
  await closeModal(leader.page);

  await area('R16', async () => {
    const syncSource = '\\documentclass{article}\n\\begin{document}\nMain page.\\newpage\\input{chapters/second.tex}\n\\end{document}\n'; await setSource(leader.page, paperId, syncSource);
    const compiled = await build(leader.page, paperId, 'success'); lastGoodBuild = compiled.current_build_id; await pdfEvidence(leader.page, paperId, compiled.current_build_id);
    await leader.page.waitForFunction((buildId) => document.querySelector('#pdfViewport')?.dataset.buildId === buildId && document.querySelectorAll('#pdfViewport .writer-pdf-page').length > 0, compiled.current_build_id, { timeout: 30_000 });
    await leader.page.locator('#fileTree button').filter({ hasText: 'second.tex' }).click(); const target = leader.page.locator('.cm-line').filter({ hasText: 'Nested exact SyncTeX target.' }); await target.click(); await leader.page.locator('#locateInPdf').waitFor({ state: 'visible' }); await leader.page.waitForFunction(() => !document.querySelector('#locateInPdf').disabled); await leader.page.locator('#locateInPdf').focus(); await leader.page.keyboard.press('Enter');
    await leader.page.locator('#writerNotice').filter({ hasText: /Located|Approximately located/ }).waitFor(); equal(await leader.page.locator('#pdfPage').inputValue(), '2');
    const files = await json(leader.page, `/api/v2/papers/${paperId}/files`); const nested = files.payload.find((file) => file.path === 'chapters/second.tex');
    const mapping = await json(leader.page, `/api/v2/papers/${paperId}/builds/${compiled.current_build_id}/source-position?file=${encodeURIComponent(nested.path)}&line=1&column=2`); equal(mapping.status, 200);
    for (const zoom of ['100', '125']) {
      await leader.page.locator('#pdfZoom').selectOption(zoom); await leader.page.locator('#pdfViewport [data-page="2"] canvas').waitFor();
      const point = mapping.payload; await leader.page.locator('#fileTree button').filter({ hasText: 'main.tex' }).click(); await leader.page.locator('#currentFile').filter({ hasText: 'main.tex' }).waitFor(); await waitText(leader.page, 'Main page.'); const canvas = leader.page.locator('#pdfViewport [data-page="2"] canvas');
      const scale = Number(zoom) / 100; const centerX = point.x + Number(point.width || 0) / 2; const centerY = point.y - Number(point.height || 0) / 2;
      await leader.page.evaluate(({ pageNumber, targetY }) => {
        const scroll = document.querySelector('#pdfScroll'); const shell = document.querySelector(`#pdfViewport [data-page="${pageNumber}"]`);
        scroll.scrollTop += shell.getBoundingClientRect().top - scroll.getBoundingClientRect().top + targetY - scroll.clientHeight / 2;
        scroll.dispatchEvent(new Event('scroll'));
      }, { pageNumber: 2, targetY: centerY * scale });
      await leader.page.waitForTimeout(150); const box = await canvas.boundingBox(); ok(box);
      await canvas.dispatchEvent('dblclick', { clientX: box.x + centerX * scale, clientY: box.y + centerY * scale, bubbles: true }); await leader.page.locator('#currentFile').filter({ hasText: 'chapters/second.tex' }).waitFor({ timeout: 10_000 }); await leader.page.waitForTimeout(750);
      const inverse = await leader.page.evaluate(() => ({ file: document.querySelector('#currentFile')?.textContent, relation: document.querySelector('#pdfRelation')?.textContent, source: document.querySelector('#editorMount .cm-content')?.innerText || '' }));
      ok(inverse.source.includes('Nested exact SyncTeX target.'), `inverse sync at ${zoom}%: ${JSON.stringify(inverse)}`);
    }
    await leader.page.locator('#editorMount .cm-content').click(); await leader.page.keyboard.press('Control+End'); await leader.page.keyboard.type('\nStale edit'); await leader.page.locator('#fileTree button').filter({ hasText: 'main.tex' }).click();
    const staleCanvas = leader.page.locator('#pdfViewport [data-page="2"] canvas'); const staleBox = await staleCanvas.boundingBox(); await staleCanvas.dispatchEvent('dblclick', { clientX: staleBox.x + 20, clientY: staleBox.y + 20, bubbles: true }); match(await leader.page.locator('#pdfRelation').innerText(), /stale|compile current source/i);
  });
  await closeModal(leader.page);

  await area('R07', async () => {
    await admin.page.locator('#adminNav button[data-section="Runtime Logs"]').click(); await admin.page.getByRole('heading', { name: 'Runtime Logs' }).waitFor(); const rows = admin.page.locator('.admin-log-message'); await rows.first().waitFor({ timeout: 20_000 }); ok(await rows.count() > 0, 'live Runtime Logs rendered no rows');
    const service = admin.page.getByLabel('Service'); const options = await service.locator('option').allTextContents(); ok(options.length > 1, `Runtime Logs offered only ${JSON.stringify(options)}`);
    for (let index = 1; index < options.length; index += 1) {
      await service.selectOption({ index }); await admin.page.waitForTimeout(350);
      const recordCount = await admin.page.locator('.admin-log-message').count(); const emptyCount = await admin.page.getByText(/No retained log records matched|runtime log source returned no records/i).count();
      ok(recordCount > 0 || emptyCount > 0, `selected service ${options[index]} rendered neither records nor an empty state`);
    }
    const search = admin.page.getByLabel('Search available retained container logs'); await search.fill(`no-record-${suffix}`); await search.press('Enter'); await admin.page.getByText('No retained log records matched.', { exact: true }).waitFor();
    await admin.page.route('**/api/admin/v2/runtime-logs?*', (route) => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'runtime log source unavailable' }) })); await admin.page.getByRole('button', { name: 'Refresh' }).click(); await admin.page.getByText('Runtime log source unavailable.', { exact: true }).waitFor(); await admin.page.unroute('**/api/admin/v2/runtime-logs?*');
    await search.fill(''); await service.selectOption(''); await admin.page.getByRole('button', { name: 'Refresh' }).click(); await admin.page.locator('.admin-log-message').first().waitFor(); const visible = await admin.page.locator('main').innerText(); ok(!/(bearer\s+[A-Za-z0-9._-]{12,}|password\s*[=:]\s*\S+)/i.test(visible), 'visible logs expose a credential-like value');
  });
  await closeModal(leader.page);

  await area('R03_WRAP', async () => {
    await openReport(leader.page, reportName); const wrapSource = '\\documentclass{article}\n\\usepackage{graphicx}\n\\usepackage{wrapfig}\n\\begin{document}\nINSERTWRAPMARKER\nText flows around the image. Text flows around the image. Text flows around the image.\n\\end{document}\n';
    await setSource(leader.page, paperId, wrapSource); await leader.page.reload({ waitUntil: 'domcontentloaded' }); await openReport(leader.page, reportName); await selectText(leader.page, 'INSERTWRAPMARKER'); await leader.page.locator('#insertMenu').click(); await leader.page.getByRole('button', { name: 'Wrap figure', exact: true }).click();
    await leader.page.locator('#dialogBody label').filter({ hasText: 'Asset' }).locator('select').selectOption('images/seed.png'); await leader.page.locator('#dialogBody label').filter({ hasText: 'Caption' }).locator('input').fill('Wrapped image'); await leader.page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
    await requestSave(leader.page); await leader.page.waitForFunction(() => document.querySelector('#saveStatus')?.dataset.state === 'synced'); const built = await build(leader.page, paperId, 'success'); const pdf = await pdfEvidence(leader.page, paperId, built.current_build_id); ok(pdf.images > 0); ok(pdf.text.includes('Wrapped image'));
    const manual = '\\documentclass{article}\n\\usepackage{graphicx}\n\\usepackage{wrapfig}\n\\begin{document}\n\\begin{wrapfigure}{r}{0.45\\textwidth}\\centering\\includegraphics[width=\\linewidth]{images/seed.png}\\caption{Manual wrapped image}\\end{wrapfigure}Text flows around the manual image. Text flows around the manual image.\\end{document}\n';
    await setSource(leader.page, paperId, manual); const manualBuild = await build(leader.page, paperId, 'success'); const manualPdf = await pdfEvidence(leader.page, paperId, manualBuild.current_build_id); ok(manualPdf.images > 0); ok(manualPdf.text.includes('Manual wrapped image'));
  });
  await closeModal(leader.page);

  await area('COMMENTS', async () => {
    await leader.page.locator('#commentsToggle').click(); ok(await leader.page.locator('#workspaceDrawer').isVisible()); await leader.page.locator('#drawerClose').click();
  });
} finally {
  if (admin?.page && !admin.page.isClosed()) {
    for (const email of disposableEmails) await json(admin.page, `/api/admin/users/${encodeURIComponent(email)}`, { method: 'PATCH', body: { enabled: false } }).catch(() => null);
  }
  await Promise.allSettled(sessions.map(({ context }) => context.close())); await browser.close();
}

equal(browserFailures.length, 0, browserFailures.join('\n'));
console.log(JSON.stringify({ assertions, compile_jobs: compileJobs, successful_builds: successfulBuilds, expected_failed_builds: expectedFailedBuilds, fresh_pdfs: freshPdfs, results, browser_failures: browserFailures.length, fixture: { suffix, disposable_accounts_disabled: disposableEmails.length } }, null, 2));
