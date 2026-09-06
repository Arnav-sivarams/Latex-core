import assert from 'node:assert/strict';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { chromium } from 'playwright';

const executeFile = promisify(execFile);

const baseURL = process.env.RUN2_BASE_URL;
const phase = process.env.RUN2_PHASE;
const statePath = process.env.RUN2_STATE_FILE;
const adminPasswordPath = process.env.RUN2_ADMIN_PASSWORD_FILE;
const adminEmail = process.env.RUN2_ADMIN_EMAIL || 'run2-admin@example.invalid';
const evidenceDirectory = process.env.RUN2_EVIDENCE_DIR || '/tmp/latex-core-run2-browser-evidence';
const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome';
if (!baseURL || !['setup', 'after-restart', 'backup-ready'].includes(phase) || !statePath || !adminPasswordPath) {
  throw new Error('RUN2_BASE_URL, RUN2_PHASE, RUN2_STATE_FILE, and RUN2_ADMIN_PASSWORD_FILE are required.');
}


await mkdir(evidenceDirectory, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath });
const browserFailures = [];
const expectedDroppedRequests = new Set();

function track(page, label) {
  page.on('console', (message) => {
    if (message.type() === 'error'
      && !message.text().startsWith('Failed to load resource:')
      && !message.text().includes('net::ERR_INTERNET_DISCONNECTED')) {
      browserFailures.push(`${label} console: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => browserFailures.push(`${label} page: ${error.message}`));
  page.on('response', (response) => {
    if (response.status() < 400) return;
    const path = new URL(response.url()).pathname;
    const expected = (path === '/branding/logo' && response.status() === 404)
      || (path === '/api/admin/v2/branding' && response.status() === 415)
      || (path.endsWith('/synctex') && [404, 409].includes(response.status()));
    if (!expected) browserFailures.push(`${label} HTTP: ${response.request().method()} ${path} ${response.status()}`);
  });
  page.on('requestfailed', (request) => {
    const path = new URL(request.url()).pathname;
    if (!request.url().startsWith(baseURL) || request.failure()?.errorText === 'net::ERR_INTERNET_DISCONNECTED'
      || expectedDroppedRequests.delete(path)) return;
    browserFailures.push(`${label} network: ${request.method()} ${request.url()} ${request.failure()?.errorText}`);
  });
}

async function newPage(label, viewport = { width: 1366, height: 768 }) {
  const context = await browser.newContext({ viewport });
  await context.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    window.__run2Sockets = [];
    window.WebSocket = class Run2TrackedWebSocket extends NativeWebSocket {
      constructor(...arguments_) {
        super(...arguments_);
        window.__run2Sockets.push(this);
      }
    };
  });
  const page = await context.newPage();
  track(page, label);
  return { context, page };
}

async function login(page, email, password, expectedPath) {
  await page.goto(baseURL, { waitUntil: 'domcontentloaded' });
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await Promise.all([
    page.waitForURL(new RegExp(`${expectedPath.replace('/', '\\/')}$`)),
    page.getByRole('button', { name: 'Sign in' }).click(),
  ]);
}

async function json(page, path, options = {}) {
  return page.evaluate(async ({ path, options }) => {
    const response = await fetch(path, {
      credentials: 'same-origin',
      method: options.method || 'GET',
      headers: options.body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    });
    const payload = await response.json().catch(() => null);
    return { status: response.status, payload };
  }, { path, options });
}

async function archivePost(page, path, archivePath, fields) {
  const archive = (await readFile(archivePath)).toString('base64');
  return page.evaluate(async ({ path, archive, fields, filename }) => {
    const bytes = Uint8Array.from(atob(archive), (character) => character.charCodeAt(0));
    const form = new FormData();
    Object.entries(fields).forEach(([key, value]) => form.set(key, value));
    form.set('archive', new File([bytes], filename, { type: 'application/zip' }));
    const response = await fetch(path, { method: 'POST', credentials: 'same-origin', body: form });
    const payload = await response.json().catch(() => null);
    return { status: response.status, payload };
  }, { path, archive, fields, filename: archivePath.split('/').at(-1) });
}

async function openNamedReport(page, name, listId) {
  const report = page.locator(`${listId} button`, { hasText: name }).first();
  await report.waitFor({ state: 'visible' });
  await report.click();
  await page.locator('.cm-editor').waitFor({ state: 'visible' });
}

async function addSelectedComment(page, filePath, lineText, message) {
  if (filePath) {
    await page.locator('#reviewFiles button', { hasText: filePath }).click();
    await page.locator('.cm-editor').waitFor({ state: 'visible' });
  }
  const line = page.locator('.cm-line', { hasText: lineText }).first();
  await line.waitFor({ state: 'visible' });
  const box = await line.boundingBox();
  assert.ok(box, `missing editor coordinates for ${filePath || 'main file'}`);
  await page.mouse.move(box.x + 3, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(Math.min(box.x + box.width - 3, box.x + 190), box.y + box.height / 2, { steps: 8 });
  await page.mouse.up();
  await page.locator('#reviewSelection').waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('#reviewSelection').disabled);
  await page.locator('#reviewSelection').click();
  await page.locator('#threadMessage').fill(message);
  await page.locator('#createComment').click();
  await assertEventuallyText(page.locator('#draftStatus'), 'Draft saved — not yet visible to writers');
}

async function assertEventuallyText(locator, expected, timeout = 15_000) {
  await locator.waitFor({ state: 'visible', timeout });
  await assert.doesNotReject(async () => {
    const started = Date.now();
    while (Date.now() - started < timeout) {
      if ((await locator.textContent())?.includes(expected)) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.fail(`Timed out waiting for text: ${expected}`);
  });
}

async function compileCurrentReport(page, paperId) {
  const before = await json(page, `/api/v2/papers/${paperId}/builds`);
  const previousBuildId = before.payload?.build?.current_build_id || null;
  await page.locator('#compilePaper').click();
  const started = Date.now();
  while (Date.now() - started < 120_000) {
    const current = await json(page, `/api/v2/papers/${paperId}/builds`);
    const build = current.payload?.build;
    if (build?.latest_status === 'failed' && !build.active_build_id) throw new Error(`queued build failed: ${JSON.stringify(build.latest_error)}`);
    if (build?.current_build_id && build.current_build_id !== previousBuildId && !build.active_build_id
      && build.current_source_sequence === build.source_sequence) {
      await assertEventuallyText(page.locator('#buildStatus'), 'Current', 10_000);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  assert.fail('timed out waiting for a new current build');
}

async function setup() {
  const suffix = randomBytes(4).toString('hex');
  const reportName = `Run 2 Recovery Report ${suffix}`;
  const frontMatterArchive = `/tmp/latex-core-run2-front-matter-${suffix}.zip`;
  await executeFile('zip', ['-q', '-j', frontMatterArchive, 'frontmatter.json', 'frontmatter.tex', 'cover.tex'], {
    cwd: 'services/server/tests/fixtures/recovery-front-matter',
  });
  const adminPassword = (await readFile(adminPasswordPath, 'utf8')).trim();
  const admin = await newPage('admin');
  await login(admin.page, adminEmail, adminPassword, '/admin');
  await assertEventuallyText(admin.page.locator('#adminAccountName'), adminEmail);
  assert.equal((await admin.page.locator('.role-badge').textContent()).trim(), 'Admin');
  await admin.page.locator('#adminNav button[data-section="System"]').click();
  await assertEventuallyText(admin.page.locator('#adminContent'), 'Backup and recovery');
  await assertEventuallyText(admin.page.locator('#adminContent'), 'No successful backup recorded');

  const badLogo = await admin.page.evaluate(async () => {
    const response = await fetch('/api/admin/v2/branding', { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/png' }, body: new TextEncoder().encode('not an image') });
    return response.status;
  });
  assert.ok(badLogo >= 400 && badLogo < 500, `bad logo returned ${badLogo}`);
  const logoBase64 = await admin.page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 8; canvas.height = 4;
    const context = canvas.getContext('2d'); context.fillStyle = '#234f84'; context.fillRect(0, 0, 8, 4);
    return canvas.toDataURL('image/png').split(',')[1];
  });
  await admin.page.locator('.branding-form input[type="file"]').setInputFiles({ name: 'institution.png', mimeType: 'image/png', buffer: Buffer.from(logoBase64, 'base64') });
  await admin.page.locator('.branding-form button[type="submit"]').click();
  await admin.page.locator('.branding-preview img').waitFor({ state: 'visible' });

  const password = `Run2-${randomBytes(18).toString('base64url')}`;
  const accounts = {};
  for (const [key, role] of [['writer', 'writer'], ['writer2', 'writer'], ['mentor', 'mentor']]) {
    const email = `run2-${key}-${suffix}@example.invalid`;
    const response = await json(admin.page, '/api/admin/v2/users', { method: 'POST', body: { email, password, role, generate_temporary_password: false } });
    assert.equal(response.status, 201, JSON.stringify(response.payload));
    accounts[key] = { email, userId: response.payload.user_id };
  }

  const template = await archivePost(admin.page, '/api/admin/v2/templates/import', 'examples/professor-demo/dist/main-template.zip', { name: `Run 2 recovery template ${suffix}`, description: 'Isolated drill fixture', main: 'main.tex' });
  assert.equal(template.status, 201, JSON.stringify(template.payload));
  const frontMatter = await archivePost(admin.page, '/api/admin/v2/front-matter-packs/import', frontMatterArchive, { name: `Run 2 recovery front matter ${suffix}`, description: 'Isolated drill fixture' });
  assert.equal(frontMatter.status, 201, JSON.stringify(frontMatter.payload));
  const team = await json(admin.page, '/api/admin/v2/paper-teams', { method: 'POST', body: {
    name: reportName, template_id: template.payload.id, front_matter_pack_id: frontMatter.payload.id,
    use_front_matter_default: false, leader_writer_id: accounts.writer.userId,
    writer_ids: [accounts.writer.userId, accounts.writer2.userId], mentor_ids: [accounts.mentor.userId],
  } });
  assert.equal(team.status, 201, JSON.stringify(team.payload));
  const paperId = team.payload.team.id;

  const writer = await newPage('writer');
  await login(writer.page, accounts.writer.email, password, '/write');
  await openNamedReport(writer.page, reportName, '#teamPapers');
  await assertEventuallyText(writer.page.locator('#roleIdentity'), 'Writer · Team leader');
  await writer.page.locator('.header-logo').waitFor({ state: 'visible' });
  for (const label of ['Save', 'Compile', 'Reviews', 'Send for review']) await writer.page.getByRole('button', { name: label, exact: true }).waitFor({ state: 'visible' });
  assert.equal(await writer.page.getByText('Outline', { exact: true }).count(), 0);
  assert.equal(await writer.page.getByText(/Show (source )?in PDF/i).count(), 0);

  const editor = writer.page.locator('#editorMount .cm-content');
  await editor.click(); await writer.page.keyboard.press('Control+End');
  await writer.page.keyboard.type('\n% server-acknowledged-before-restart');
  await assertEventuallyText(writer.page.locator('#saveStatus'), 'Saved');

  await writer.context.setOffline(true);
  await writer.page.evaluate(() => window.__run2Sockets.forEach((socket) => socket.close()));
  await writer.page.waitForFunction(() => ['reconnecting', 'offline'].includes(document.querySelector('#saveStatus').dataset.state));
  await writer.page.keyboard.type('\n% offline-recovery-marker');
  await assertEventuallyText(writer.page.locator('#saveStatus'), 'Offline');
  assert.equal(await writer.page.evaluate(() => indexedDB.databases().then((items) => items.some((item) => item.name?.startsWith('latex-core:')))), true);
  await writer.context.setOffline(false);
  await assertEventuallyText(writer.page.locator('#saveStatus'), 'Saved', 30_000);

  let detail = await json(writer.page, `/api/v2/papers/${paperId}`);
  assert.equal(detail.status, 200);
  const second = await json(writer.page, `/api/v2/papers/${paperId}/files`, { method: 'POST', body: { path: 'chapters/results.tex', content: 'Second-file anchor text for review.\n', version: detail.payload.version } });
  assert.equal(second.status, 201, JSON.stringify(second.payload));
  detail = await json(writer.page, `/api/v2/papers/${paperId}`);
  const assetStatus = await writer.page.evaluate(async ({ paperId, version, logoBase64 }) => {
    const bytes = Uint8Array.from(atob(logoBase64), (character) => character.charCodeAt(0));
    const response = await fetch(`/api/v2/papers/${paperId}/assets?path=${encodeURIComponent('images/fixture.png')}&version=${version}`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'image/png' }, body: bytes });
    return response.status;
  }, { paperId, version: detail.payload.version, logoBase64 });
  assert.equal(assetStatus, 201);
  await compileCurrentReport(writer.page, paperId);
  await writer.page.locator('#sendReview').click();
  await assertEventuallyText(writer.page.locator('#writerNotice'), 'sent for Mentor review');

  const writer2 = await newPage('writer-2');
  await login(writer2.page, accounts.writer2.email, password, '/write');
  await openNamedReport(writer2.page, reportName, '#teamPapers');
  await assertEventuallyText(writer2.page.locator('#editorMount'), 'offline-recovery-marker');
  await writer2.context.close();

  const mentor = await newPage('mentor');
  await login(mentor.page, accounts.mentor.email, password, '/review');
  await openNamedReport(mentor.page, reportName, '#assignedPapers');
  await mentor.page.locator('.header-logo').waitFor({ state: 'visible' });
  await mentor.page.getByRole('button', { name: 'Push review', exact: true }).waitFor({ state: 'visible' });
  await addSelectedComment(mentor.page, null, 'synthetic template demonstrates', 'Main-file private draft');
  await addSelectedComment(mentor.page, 'chapters/results.tex', 'Second-file anchor text', 'Second-file private draft');
  await mentor.page.reload({ waitUntil: 'domcontentloaded' });
  await openNamedReport(mentor.page, reportName, '#assignedPapers');
  await mentor.page.locator('#mentorCommentsToggle').click();
  await assertEventuallyText(mentor.page.locator('#threadList'), 'Main-file private draft');
  await assertEventuallyText(mentor.page.locator('#threadList'), 'Second-file private draft');
  assert.equal(await mentor.page.locator('#threadList .draft-thread').count(), 2);

  const writerThreads = await json(writer.page, `/api/v2/reviews/papers/${paperId}/threads`);
  assert.equal(writerThreads.status, 200);
  assert.equal(writerThreads.payload.threads.length, 0, 'Writer API exposed unpublished drafts');
  assert.equal((await writer.page.locator('#toolbarReviewCount').textContent()).trim(), '0');
  await mentor.page.screenshot({ path: `${evidenceDirectory}/setup-private-drafts.png`, fullPage: false });

  const state = { schemaVersion: 1, paperId, reportName, password, accounts,
    fileIds: { main: team.payload.files.find((file) => file.path === 'main.tex').file_id, second: second.payload.file.file_id },
    sourceVersionAfterReconnect: detail.payload.version };
  await writeFile(statePath, `${JSON.stringify(state)}\n`, { mode: 0o600 }); await chmod(statePath, 0o600);
  await admin.context.close(); await writer.context.close(); await mentor.context.close();
  return { paperId, draftPrivacy: 'writer API 0, mentor UI 2', offlineReconnect: 'converged in second Writer context' };
}

async function afterRestart() {
  const state = JSON.parse(await readFile(statePath, 'utf8'));
  const writer = await newPage('writer-after-restart');
  await login(writer.page, state.accounts.writer.email, state.password, '/write');
  await openNamedReport(writer.page, state.reportName, '#teamPapers');
  const resumedHistory = await json(writer.page, `/api/v2/reviews/papers/${state.paperId}/threads`);
  const resolvedSecond = resumedHistory.payload.threads.find((thread) => thread.state === 'RESOLVED'
    && thread.messages.some((message) => message.body === 'Second-file private draft'));
  if (resolvedSecond) {
    await assertEventuallyText(writer.page.locator('#toolbarReviewCount'), '1');
    await writer.page.locator('#commentsToggle').click();
    await writer.page.locator('#writerReviewFilters button[data-filter="RESOLVED"]').click();
    const secondCard = writer.page.locator('#writerReviewList .thread-card', { hasText: 'Second-file private draft' });
    await secondCard.locator('.thread-title').click();
    await assertEventuallyText(writer.page.locator('#currentFile'), 'chapters/results.tex');
    const appearance = await writer.page.evaluate(() => ({
      fontSize: getComputedStyle(document.querySelector('#editorMount .cm-content')).fontSize,
      background: getComputedStyle(document.querySelector('#editorMount .cm-editor')).backgroundColor,
    }));
    assert.deepEqual(appearance, { fontSize: '20px', background: 'rgb(31, 35, 41)' });
    assert.equal(resumedHistory.payload.threads.length, 2, 'new round removed historical feedback');
    await writer.page.screenshot({ path: `${evidenceDirectory}/resolved-history-new-round-resumed.png`, fullPage: false });
    await writer.context.close();
    return { paperId: state.paperId, resumedCompletedState: true, secondFileNavigation: true, resolvedHistory: 2 };
  }
  const mentor = await newPage('mentor-after-restart');
  await login(mentor.page, state.accounts.mentor.email, state.password, '/review');
  await openNamedReport(mentor.page, state.reportName, '#assignedPapers');
  const beforePublication = await json(writer.page, `/api/v2/reviews/papers/${state.paperId}/threads`);
  if (beforePublication.payload.threads.length === 0) {
    await mentor.page.locator('#mentorCommentsToggle').click();
    await assertEventuallyText(mentor.page.locator('#threadList'), 'Main-file private draft');
    assert.equal(await mentor.page.locator('#threadList .draft-thread').count(), 2);
    mentor.page.once('dialog', (dialog) => { assert.match(dialog.message(), /2 draft comments\/suggestions/); dialog.accept(); });
    await mentor.page.locator('#pushReview').click();
    await assertEventuallyText(mentor.page.locator('#reviewNotice'), 'Writers can now see your feedback');
  } else {
    assert.equal(beforePublication.payload.threads.length, 2, 'resume requires the already-published two-thread fixture');
  }
  await assertEventuallyText(writer.page.locator('#toolbarReviewCount'), '2', 20_000);
  await writer.page.locator('#commentsToggle').click();
  const secondCard = writer.page.locator('#writerReviewList .thread-card', { hasText: 'Second-file private draft' });
  await secondCard.locator('.thread-title').click();
  await assertEventuallyText(writer.page.locator('#currentFile'), 'chapters/results.tex');
  await writer.page.locator('#editorMount .review-source-highlight').waitFor({ state: 'visible' });
  await writer.page.locator('#writerReviewPopover').waitFor({ state: 'visible' });

  await writer.page.locator('#editorSettings summary').click();
  await writer.page.locator('#editorFontSize').fill('20'); await writer.page.locator('#editorFontSize').dispatchEvent('change');
  await writer.page.locator('#editorTheme').selectOption('DARK');
  await new Promise((resolve) => setTimeout(resolve, 500));
  const appearance = await writer.page.evaluate(() => ({
    fontSize: getComputedStyle(document.querySelector('#editorMount .cm-content')).fontSize,
    background: getComputedStyle(document.querySelector('#editorMount .cm-editor')).backgroundColor,
  }));
  assert.equal(appearance.fontSize, '20px', JSON.stringify(appearance));
  assert.equal(appearance.background, 'rgb(31, 35, 41)', JSON.stringify(appearance));
  await writer.page.locator('#editorMount .review-source-highlight').waitFor({ state: 'visible' });
  const cdp = await writer.context.newCDPSession(writer.page);
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1.5 });
  const toolbarBox = await writer.page.locator('#commentsToggle').boundingBox();
  assert.ok(toolbarBox && toolbarBox.x + toolbarBox.width <= 1366 && toolbarBox.y + toolbarBox.height <= 768, 'primary toolbar action clipped at enlarged zoom');
  await writer.page.screenshot({ path: `${evidenceDirectory}/published-second-file-zoom.png`, fullPage: false });
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1 });

  await secondCard.getByRole('button', { name: 'Done', exact: true }).click();
  await assertEventuallyText(writer.page.locator('#toolbarReviewCount'), '1');
  assert.equal(await writer.page.locator('#editorMount .review-source-highlight').count(), 0);
  await writer.page.locator('#writerReviewFilters button[data-filter="RESOLVED"]').click();
  await assertEventuallyText(writer.page.locator('#writerReviewList'), 'Second-file private draft');
  await writer.page.locator('#sendReview').click();
  await assertEventuallyText(writer.page.locator('#writerNotice'), 'sent for Mentor review');
  const history = await json(writer.page, `/api/v2/reviews/papers/${state.paperId}/threads`);
  assert.equal(history.payload.threads.length, 2, 'new round removed historical feedback');
  await writer.page.screenshot({ path: `${evidenceDirectory}/resolved-history-new-round.png`, fullPage: false });
  await writer.context.close(); await mentor.context.close();
  return { paperId: state.paperId, liveDelivery: 2, secondFileNavigation: true, resolvedHistory: history.payload.threads.length };
}

async function backupReady() {
  const state = JSON.parse(await readFile(statePath, 'utf8'));
  const mentor = await newPage('mentor-lost-response');
  const writer = await newPage('writer-backup-ready');
  await login(mentor.page, state.accounts.mentor.email, state.password, '/review');
  await login(writer.page, state.accounts.writer.email, state.password, '/write');
  await openNamedReport(mentor.page, state.reportName, '#assignedPapers');
  await openNamedReport(writer.page, state.reportName, '#teamPapers');

  const rounds = await json(mentor.page, `/api/v2/reviews/papers/${state.paperId}/rounds`);
  const participation = rounds.payload?.current_review_round?.mentor_participation;
  assert.equal(participation?.status, 'PENDING');
  const roundId = rounds.payload.current_review_round.id;
  const submissionId = crypto.randomUUID();
  const requestBody = { expected_revision: participation.draft_revision, submission_id: submissionId };
  const submitPath = `/api/v2/reviews/papers/${state.paperId}/rounds/${roundId}/submit`;
  let upstreamStatus = null;
  expectedDroppedRequests.add(submitPath);
  await mentor.page.route(`**${submitPath}`, async (route) => {
    const response = await route.fetch();
    upstreamStatus = response.status();
    await route.abort('connectionreset');
  });
  const lostResponse = await mentor.page.evaluate(async ({ submitPath, requestBody }) => {
    try {
      await fetch(submitPath, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(requestBody) });
      return false;
    } catch {
      return true;
    }
  }, { submitPath, requestBody });
  await mentor.page.unroute(`**${submitPath}`);
  assert.equal(lostResponse, true, 'publication response unexpectedly reached the browser');
  assert.equal(upstreamStatus, 200, `publication upstream returned ${upstreamStatus}`);

  let committed = false;
  const started = Date.now();
  while (Date.now() - started < 10_000) {
    const current = await json(mentor.page, `/api/v2/reviews/papers/${state.paperId}/rounds`);
    if (current.payload?.current_review_round?.mentor_participation?.status === 'SUBMITTED'
      || current.payload?.review_open === false) {
      committed = true;
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.equal(committed, true, 'server did not commit the deliberately unacknowledged publication request');
  const retry = await json(mentor.page, submitPath, { method: 'POST', body: requestBody });
  assert.equal(retry.status, 200, JSON.stringify(retry.payload));
  assert.equal(retry.payload.submission_id, submissionId);
  assert.equal(retry.payload.published_count, 2);

  const published = await json(writer.page, `/api/v2/reviews/papers/${state.paperId}/threads`);
  assert.equal(published.status, 200);
  assert.equal(published.payload.threads.length, 2);
  await writer.page.locator('#sendReview').click();
  await assertEventuallyText(writer.page.locator('#writerNotice'), 'sent for Mentor review');
  await mentor.page.reload({ waitUntil: 'domcontentloaded' });
  await openNamedReport(mentor.page, state.reportName, '#assignedPapers');
  await addSelectedComment(mentor.page, null, 'synthetic template demonstrates', 'Private draft preserved in backup');
  const privateWriterView = await json(writer.page, `/api/v2/reviews/papers/${state.paperId}/threads`);
  assert.equal(privateWriterView.payload.threads.length, 2, 'new unpublished draft leaked into Writer API');
  await mentor.page.screenshot({ path: `${evidenceDirectory}/backup-ready-private-draft.png`, fullPage: false });
  await mentor.context.close(); await writer.context.close();
  return { paperId: state.paperId, clientReceivedResponse: false, retryPublishedCount: retry.payload.published_count,
    writerPublishedCount: privateWriterView.payload.threads.length, privateDraftCount: 1 };
}


try {
  const result = phase === 'setup' ? await setup()
    : phase === 'after-restart' ? await afterRestart()
      : await backupReady();
  assert.deepEqual(browserFailures, []);
  console.log(JSON.stringify({ phase, status: 'PASS', ...result }));
} finally {
  await browser.close();
}
