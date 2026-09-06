import assert from 'node:assert/strict';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { chromium } from 'playwright';

const baseURL = process.env.RUN2_BASE_URL;
const adminEmail = process.env.RUN2_ADMIN_EMAIL;
const adminPasswordPath = process.env.RUN2_ADMIN_PASSWORD_FILE;
const evidenceDirectory = process.env.RUN2_EVIDENCE_DIR;
const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome';
if (!baseURL || !adminEmail || !adminPasswordPath || !evidenceDirectory) throw new Error('isolated browser-test environment is required');

await mkdir(evidenceDirectory, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath });
const failures = [];

function track(page, label) {
  page.on('pageerror', (failure) => failures.push(`${label} page: ${failure.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error' && !message.text().startsWith('Failed to load resource:')) failures.push(`${label} console: ${message.text()}`);
  });
  page.on('response', (response) => {
    if (response.status() < 400) return;
    const path = new URL(response.url()).pathname;
    const expected = (path === '/branding/logo' && response.status() === 404)
      || (path.endsWith('/synctex') && [404, 409].includes(response.status()))
      || (path.includes('/files/') && [403, 404, 415].includes(response.status()))
      || (path.includes('/assets') && [404, 409, 413, 415].includes(response.status()));
    if (!expected) failures.push(`${label} HTTP ${response.request().method()} ${path} ${response.status()}`);
  });
}

async function contextPage(label) {
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 } });
  const page = await context.newPage(); track(page, label); return { context, page };
}

async function login(page, email, password, destination) {
  await page.goto(baseURL, { waitUntil: 'domcontentloaded' });
  await page.locator('#email').fill(email); await page.locator('#password').fill(password);
  await Promise.all([page.waitForURL(new RegExp(`${destination}$`)), page.getByRole('button', { name: 'Sign in' }).click()]);
}

async function json(page, path, options = {}) {
  return page.evaluate(async ({ path, options }) => {
    const response = await fetch(path, {
      credentials: 'same-origin', method: options.method || 'GET',
      headers: options.body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { path, options });
}

async function archivePost(page, path, archiveUrl, fields) {
  const archive = (await readFile(archiveUrl)).toString('base64');
  return page.evaluate(async ({ path, archive, fields }) => {
    const bytes = Uint8Array.from(atob(archive), (character) => character.charCodeAt(0));
    const form = new FormData();
    Object.entries(fields).forEach(([key, value]) => form.set(key, value));
    form.set('archive', new File([bytes], 'main-template.zip', { type: 'application/zip' }));
    const response = await fetch(path, { method: 'POST', credentials: 'same-origin', body: form });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { path, archive, fields });
}

async function makeImage(page, type, color, label) {
  const base64 = await page.evaluate(({ type, color, label }) => {
    const canvas = document.createElement('canvas'); canvas.width = 240; canvas.height = 140;
    const context = canvas.getContext('2d'); context.fillStyle = color; context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = '#ffffff'; context.font = 'bold 30px sans-serif'; context.fillText(label, 20, 78);
    return canvas.toDataURL(type, 0.92).split(',')[1];
  }, { type, color, label });
  return Buffer.from(base64, 'base64');
}

async function openReport(page, name) {
  const button = page.locator('#teamPapers button', { hasText: name }).first();
  await button.waitFor({ state: 'visible' }); await button.click();
  await page.locator('.cm-editor').waitFor({ state: 'visible' });
}

async function waitForServerSource(page, paperId, source) {
  const started = Date.now();
  while (Date.now() - started < 30_000) {
    const files = await json(page, `/api/v2/papers/${paperId}/files`);
    const main = files.payload?.find((file) => file.path === 'main.tex');
    if (main) {
      const saved = await json(page, `/api/v2/papers/${paperId}/files/${main.file_id}`);
      if (saved.payload?.content === source) return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.fail('timed out waiting for authoritative saved source');
}

async function setSource(page, paperId, source) {
  const content = page.locator('#editorMount .cm-content'); await content.click();
  await page.keyboard.press('Control+A'); await page.keyboard.insertText(source);
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await eventuallyText(page.locator('#saveStatus'), 'Saved', 30_000);
  await waitForServerSource(page, paperId, source);
}

async function eventuallyText(locator, expected, timeout = 20_000) {
  await locator.waitFor({ state: 'visible', timeout });
  await assert.doesNotReject(async () => {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      if ((await locator.textContent())?.includes(expected)) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.fail(`timed out waiting for ${expected}`);
  });
}

async function uploadImage(page, buffer, name, replacement = false) {
  const dialogs = [];
  const dialogHandler = async (dialog) => {
    dialogs.push(dialog.message());
    if (dialog.message().startsWith('Image path')) await dialog.accept(`assets/${name}`);
    else if (dialog.message().includes('Type REPLACE')) await dialog.accept(replacement ? 'REPLACE' : 'CANCEL');
    else await dialog.dismiss();
  };
  page.on('dialog', dialogHandler);
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: 'Upload image' }).click();
  await (await chooser).setFiles({ name, mimeType: name.endsWith('.jpg') ? 'image/jpeg' : 'image/png', buffer });
  await eventuallyText(page.locator('#writerNotice'), replacement ? 'Replaced' : 'Uploaded');
  page.off('dialog', dialogHandler);
  assert.ok(dialogs[0]?.includes('this report'));
  if (replacement) assert.ok(dialogs.some((message) => message.includes('REPLACE, RENAME, or CANCEL')));
}

async function compile(page, paperId) {
  const before = await json(page, `/api/v2/papers/${paperId}/builds`);
  const previousBuildId = before.payload?.build?.current_build_id || null;
  await page.getByRole('button', { name: 'Compile', exact: true }).click();
  const started = Date.now();
  while (Date.now() - started < 120_000) {
    const current = await json(page, `/api/v2/papers/${paperId}/builds`);
    const build = current.payload?.build;
    if (build?.latest_status === 'failed' && !build.active_build_id) throw new Error(`queued build failed: ${JSON.stringify(build.latest_error)}`);
    if (build?.current_build_id && build.current_build_id !== previousBuildId && !build.active_build_id
      && build.current_source_sequence === build.source_sequence) {
      await eventuallyText(page.locator('#buildStatus'), 'Current', 10_000);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  assert.fail('timed out waiting for a new current build');
}

async function insertBuilderDocument(page, paperId, preamble, builderLabel, expectedPreview) {
  await setSource(page, paperId, `${preamble}\n\\begin{document}\n`);
  await page.locator('#insertMenu').click();
  await page.getByRole('button', { name: builderLabel, exact: true }).click();
  const preview = page.locator('#dialogPreview');
  await preview.waitFor({ state: 'visible' });
  const generated = await preview.textContent();
  assert.match(generated, expectedPreview);
  await page.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
  await page.locator('#editorMount .cm-content').click();
  await page.keyboard.press('Control+End');
  await page.keyboard.insertText('\\end{document}\n');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await eventuallyText(page.locator('#saveStatus'), 'Saved', 30_000);
  await waitForServerSource(page, paperId, `${preamble}\n\\begin{document}\n${generated}\n\\end{document}\n`);
  await compile(page, paperId);
}

async function pdfEvidence(page, paperId) {
  return page.evaluate(async (paperId) => {
    const status = await fetch(`/api/v2/papers/${paperId}/builds`, { credentials: 'same-origin' }).then((response) => response.json());
    const buildId = status.build.current_build_id;
    const response = await fetch(`${status.pdf_url}?build=${buildId}`, { credentials: 'same-origin' });
    const bytes = new Uint8Array(await response.arrayBuffer());
    const magic = new TextDecoder().decode(bytes.slice(0, 5));
    if (!response.ok || magic !== '%PDF-') throw new Error(`PDF fetch failed: status=${response.status} type=${response.headers.get('content-type')} bytes=${bytes.length} magic=${JSON.stringify(magic)}`);
    const pdfjs = await import('/static/pdf.min.mjs');
    pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const pdfDocument = await pdfjs.getDocument({ data: bytes }).promise;
    const pageTexts = []; const colors = { red: 0, green: 0, blue: 0 };
    for (let number = 1; number <= pdfDocument.numPages; number += 1) {
      const pdfPage = await pdfDocument.getPage(number); const text = await pdfPage.getTextContent();
      pageTexts.push(text.items.map((item) => item.str).join(' '));
      const viewport = pdfPage.getViewport({ scale: 1 }); const canvas = document.createElement('canvas');
      canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
      const context = canvas.getContext('2d', { willReadFrequently: true }); await pdfPage.render({ canvasContext: context, viewport }).promise;
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let index = 0; index < pixels.length; index += 16) {
        const [red, green, blue] = [pixels[index], pixels[index + 1], pixels[index + 2]];
        if (red > 150 && red > green * 1.5 && red > blue * 1.5) colors.red += 1;
        if (green > 100 && green > red * 1.4 && green > blue * 1.4) colors.green += 1;
        if (blue > 150 && blue > red * 1.5 && blue > green * 1.5) colors.blue += 1;
      }
    }
    return { buildId, stateHash: status.build.desired_state_hash, pages: pdfDocument.numPages, text: pageTexts.join('\n'), pageTexts, colors };
  }, paperId);
}

function combinedSource(report, imagePath = 'assets/diagram.png') {
  const rows = Array.from({ length: 100 }, (_, index) => `Row ${index + 1} & Value ${index + 1} \\\\`).join('\n');
  return `\\documentclass{article}
\\usepackage{algorithm}
\\usepackage{algpseudocode}
\\usepackage{longtable}
\\usepackage{graphicx}
\\begin{document}
${report} DOCUMENT
\\begin{algorithm}\\caption{Application Insert Algorithm}\\begin{algorithmic}[1]\\State Compile ${report}\\end{algorithmic}\\end{algorithm}
\\begin{longtable}{ll}\\caption{Application Long Table}\\\\
\\hline Repeated heading & Value \\\\ \\hline\\endfirsthead
\\hline Repeated heading & Value \\\\ \\hline\\endhead
${rows}
Final row & Final value \\\\ \\hline\\end{longtable}
\\includegraphics[width=0.35\\linewidth]{${imagePath}}
\\includegraphics[width=0.15\\linewidth]{images/legacy.png}
\\end{document}
`;
}

try {
  const suffix = randomBytes(4).toString('hex');
  const password = `Run2-${randomBytes(18).toString('base64url')}`;
  const adminPassword = (await readFile(adminPasswordPath, 'utf8')).trim();
  const admin = await contextPage('admin'); await login(admin.page, adminEmail, adminPassword, '/admin');
  const accounts = {};
  for (const [key, role] of [['a', 'writer'], ['b', 'writer'], ['shared', 'writer'], ['mentor', 'mentor']]) {
    const email = `asset-${key}-${suffix}@example.invalid`;
    const response = await json(admin.page, '/api/admin/v2/users', { method: 'POST', body: { email, password, role, generate_temporary_password: false } });
    assert.equal(response.status, 201, JSON.stringify(response.payload)); accounts[key] = { email, id: response.payload.user_id };
  }
  const template = await archivePost(admin.page, '/api/admin/v2/templates/import', new URL('../../../examples/professor-demo/dist/main-template.zip', import.meta.url), {
    name: `Asset isolation template ${suffix}`, description: 'Isolated package and asset fixture', main: 'main.tex',
  });
  assert.equal(template.status, 201, JSON.stringify(template.payload));
  const names = { a: `Asset Report A ${suffix}`, b: `Asset Report B ${suffix}` };
  const reports = {};
  for (const key of ['a', 'b']) {
    const response = await json(admin.page, '/api/admin/v2/paper-teams', { method: 'POST', body: {
      name: names[key], template_id: template.payload.id, leader_writer_id: accounts[key].id,
      writer_ids: [accounts[key].id, accounts.shared.id], mentor_ids: [accounts.mentor.id],
    } });
    assert.equal(response.status, 201, JSON.stringify(response.payload)); reports[key] = response.payload.team.id;
  }
  const red = await makeImage(admin.page, 'image/png', '#d52222', 'REPORT A');
  const blue = await makeImage(admin.page, 'image/png', '#224ed5', 'REPORT B');
  const green = await makeImage(admin.page, 'image/png', '#169447', 'A REPLACED');
  const legacy = await makeImage(admin.page, 'image/png', '#7346a8', 'LEGACY');
  const jpeg = await makeImage(admin.page, 'image/jpeg', '#b16b16', 'JPEG');

  const shared = await contextPage('shared-writer'); await login(shared.page, accounts.shared.email, password, '/write');
  const writerA = await contextPage('writer-a'); await login(writerA.page, accounts.a.email, password, '/write');
  const uploadLegacy = async (paperId, version) => shared.page.evaluate(async ({ paperId, version, bytes }) => {
    const body = Uint8Array.from(atob(bytes), (character) => character.charCodeAt(0));
    const response = await fetch(`/api/v2/papers/${paperId}/assets?path=images%2Flegacy.png&version=${version}`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/png' }, body });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, { paperId, version, bytes: legacy.toString('base64') });

  await openReport(shared.page, names.a);
  await insertBuilderDocument(shared.page, reports.a, '\\documentclass{article}\n\\usepackage{algorithm}\n\\usepackage{algpseudocode}', 'Algorithm Builder', /\\State Describe the method/);
  const algorithmicxApplication = await pdfEvidence(shared.page, reports.a); assert.match(algorithmicxApplication.text, /Describe the method/);
  await openReport(shared.page, names.b);
  await insertBuilderDocument(shared.page, reports.b, '\\documentclass{article}\n\\usepackage{algorithm}\n\\usepackage{algorithmic}', 'Algorithmic Builder', /\\STATE Describe the method/);
  const algorithmicApplication = await pdfEvidence(shared.page, reports.b); assert.match(algorithmicApplication.text, /Describe the method/);
  await openReport(shared.page, names.a);
  await insertBuilderDocument(shared.page, reports.a, '\\documentclass{article}\n\\usepackage{longtable}', 'Long Table Builder', /\\begin\{longtable\}/);
  const longTableBuilderApplication = await pdfEvidence(shared.page, reports.a); assert.match(longTableBuilderApplication.text, /Long table/);

  await setSource(shared.page, reports.a, combinedSource('REPORT A'));
  await uploadImage(shared.page, red, 'diagram.png');
  let detail = await json(shared.page, `/api/v2/papers/${reports.a}`); let legacyResult = await uploadLegacy(reports.a, detail.payload.version); assert.equal(legacyResult.status, 201);
  await shared.page.locator('#fileTree button', { hasText: 'main.tex' }).click(); await compile(shared.page, reports.a);
  const firstA = await pdfEvidence(shared.page, reports.a); assert.ok(firstA.pages >= 2); assert.match(firstA.text, /Application Insert Algorithm/); assert.match(firstA.text, /Final row/); assert.ok(firstA.pageTexts.filter((text) => text.includes('Repeated heading')).length >= 2); assert.ok(firstA.colors.red > firstA.colors.blue);
  const checkpoint = await json(writerA.page, `/api/v2/papers/${reports.a}/versions`, { method: 'POST', body: { name: 'A before replacement' } }); assert.equal(checkpoint.status, 201);

  await openReport(shared.page, names.b); await setSource(shared.page, reports.b, combinedSource('REPORT B'));
  await uploadImage(shared.page, blue, 'diagram.png');
  detail = await json(shared.page, `/api/v2/papers/${reports.b}`); legacyResult = await uploadLegacy(reports.b, detail.payload.version); assert.equal(legacyResult.status, 201);
  detail = await json(shared.page, `/api/v2/papers/${reports.b}`);
  const jpegResult = await shared.page.evaluate(async ({ paperId, version, bytes }) => {
    const body = Uint8Array.from(atob(bytes), (character) => character.charCodeAt(0));
    const response = await fetch(`/api/v2/papers/${paperId}/assets?path=assets%2Fphoto.jpg&version=${version}`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/jpeg' }, body });
    return response.status;
  }, { paperId: reports.b, version: detail.payload.version, bytes: jpeg.toString('base64') }); assert.equal(jpegResult, 201);
  await shared.page.locator('#fileTree button', { hasText: 'main.tex' }).click(); await compile(shared.page, reports.b);
  const firstB = await pdfEvidence(shared.page, reports.b); assert.ok(firstB.colors.blue > firstB.colors.red);

  await openReport(shared.page, names.a); await uploadImage(shared.page, green, 'diagram.png', true);
  await shared.page.locator('#fileTree button', { hasText: 'main.tex' }).click(); await compile(shared.page, reports.a);
  const replacedA = await pdfEvidence(shared.page, reports.a); assert.notEqual(replacedA.stateHash, firstA.stateHash); assert.ok(replacedA.colors.green > replacedA.colors.red);
  const unchangedB = await pdfEvidence(shared.page, reports.b); assert.equal(unchangedB.stateHash, firstB.stateHash); assert.equal(unchangedB.buildId, firstB.buildId);

  await shared.page.getByRole('button', { name: 'diagram.png', exact: true }).click();
  shared.page.once('dialog', (dialog) => dialog.accept()); await shared.page.locator('#fileActionsToggle').click(); await shared.page.locator('#deleteFile').click();
  await eventuallyText(shared.page.locator('#writerNotice'), 'Deleted');
  let filesA = await json(shared.page, `/api/v2/papers/${reports.a}/files`); assert.equal(filesA.payload.some((file) => file.path === 'assets/diagram.png'), false);
  const filesB = await json(shared.page, `/api/v2/papers/${reports.b}/files`); const fileB = filesB.payload.find((file) => file.path === 'assets/diagram.png'); assert.ok(fileB);
  const bRawHash = await shared.page.evaluate(async ({ paperId, fileId }) => {
    const bytes = await fetch(`/api/v2/papers/${paperId}/files/${fileId}/raw`, { credentials: 'same-origin' }).then((response) => response.arrayBuffer());
    const digest = await crypto.subtle.digest('SHA-256', bytes); return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
  }, { paperId: reports.b, fileId: fileB.file_id });

  const restoredResult = await json(writerA.page, `/api/v2/papers/${reports.a}/versions/${checkpoint.payload.id}/revert`, { method: 'POST', body: { confirmed: true } }); assert.equal(restoredResult.status, 200, JSON.stringify(restoredResult.payload));
  filesA = await json(shared.page, `/api/v2/papers/${reports.a}/files`); const restoredA = filesA.payload.find((file) => file.path === 'assets/diagram.png'); assert.ok(restoredA);
  const restoredBytes = await shared.page.evaluate(async ({ paperId, fileId }) => Array.from(new Uint8Array(await fetch(`/api/v2/papers/${paperId}/files/${fileId}/raw`, { credentials: 'same-origin' }).then((response) => response.arrayBuffer()))), { paperId: reports.a, fileId: restoredA.file_id }); assert.deepEqual(Buffer.from(restoredBytes), red);
  const stillBHash = await shared.page.evaluate(async ({ paperId, fileId }) => { const bytes = await fetch(`/api/v2/papers/${paperId}/files/${fileId}/raw`, { credentials: 'same-origin' }).then((response) => response.arrayBuffer()); const digest = await crypto.subtle.digest('SHA-256', bytes); return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join(''); }, { paperId: reports.b, fileId: fileB.file_id }); assert.equal(stillBHash, bRawHash);
  const postRestoreDetail = await json(shared.page, `/api/v2/papers/${reports.a}`);

  const crossRead = await shared.page.evaluate(async ({ paperId, fileId }) => fetch(`/api/v2/papers/${paperId}/files/${fileId}/raw`, { credentials: 'same-origin' }).then((response) => response.status), { paperId: reports.a, fileId: fileB.file_id }); assert.equal(crossRead, 404);
  const crossMutation = await shared.page.evaluate(async ({ paperId, fileId, version, bytes }) => { const body = Uint8Array.from(atob(bytes), (character) => character.charCodeAt(0)); return fetch(`/api/v2/papers/${paperId}/assets?path=assets%2Fdiagram.png&version=${version}&replace_file_id=${fileId}&file_revision=1`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/png' }, body }).then((response) => response.status); }, { paperId: reports.a, fileId: fileB.file_id, version: postRestoreDetail.payload.version, bytes: green.toString('base64') }); assert.equal(crossMutation, 404);

  const unauthorized = await writerA.page.evaluate(async ({ paperId, fileId }) => fetch(`/api/v2/papers/${paperId}/files/${fileId}/raw`, { credentials: 'same-origin' }).then((response) => response.status), { paperId: reports.b, fileId: fileB.file_id }); assert.ok([403, 404].includes(unauthorized));

  await openReport(shared.page, names.a);
  let releaseSlow; const slowStarted = new Promise((resolve) => { releaseSlow = resolve; });
  await shared.page.route('**/api/v2/papers/*/assets?*', async (route) => { if (route.request().url().includes('slow.png')) { releaseSlow(); await new Promise((resolve) => setTimeout(resolve, 1200)); } await route.continue(); });
  const dialogHandler = (dialog) => dialog.accept('assets/slow.png'); shared.page.once('dialog', dialogHandler);
  const chooser = shared.page.waitForEvent('filechooser'); await shared.page.getByRole('button', { name: 'Upload image' }).click(); await (await chooser).setFiles({ name: 'slow.png', mimeType: 'image/png', buffer: green });
  await slowStarted; await shared.page.locator('#teamPapers button', { hasText: names.b }).first().click(); await shared.page.locator('.cm-editor').waitFor({ state: 'visible' }); await eventuallyText(shared.page.locator('#writerNotice'), 'the current report was not changed');
  assert.equal(await shared.page.getByRole('button', { name: 'slow.png', exact: true }).count(), 0);
  await openReport(shared.page, names.a); assert.equal(await shared.page.getByRole('button', { name: 'slow.png', exact: true }).count(), 1);

  const corrupt = await shared.page.evaluate(async ({ paperId, version }) => fetch(`/api/v2/papers/${paperId}/assets?path=assets%2Fcorrupt.png&version=${version}`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/png' }, body: new TextEncoder().encode('broken') }).then((response) => response.status), { paperId: reports.a, version: (await json(shared.page, `/api/v2/papers/${reports.a}`)).payload.version }); assert.equal(corrupt, 415);

  await shared.page.screenshot({ path: `${evidenceDirectory}/report-a-assets.png`, fullPage: false });
  const evidence = { schemaVersion: 1, reports, algorithmicxApplication, algorithmicApplication, longTableBuilderApplication, firstA, firstB, replacedA, restoredAssetBytes: restoredBytes.length, reportBAssetSha256: bRawHash, crossRead, crossMutation, unauthorized, slowUploadStayedWithA: true, legacyPathCompiled: true, jpegAccepted: true };
  const evidencePath = `${evidenceDirectory}/package-assets.json`; await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 }); await chmod(evidencePath, 0o600);
  assert.deepEqual(failures, []);
  await Promise.all([admin.context.close(), shared.context.close(), writerA.context.close()]);
  console.log(JSON.stringify({ status: 'PASS', reports, pages: { a: firstA.pages, b: firstB.pages }, colors: { a: firstA.colors, b: firstB.colors, replacedA: replacedA.colors }, evidencePath }));
} finally {
  await browser.close();
}
