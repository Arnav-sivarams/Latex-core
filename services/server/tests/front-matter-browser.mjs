import assert from 'node:assert/strict';
import { deflateSync } from 'node:zlib';
import { chromium } from 'playwright';

const config = JSON.parse(process.env.FRONTMATTER_TEST_CONFIG);
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome' });
const failures = [];
async function login(email, route) {
  const context = await browser.newContext();
  const page = await context.newPage();
  page.on('pageerror', (error) => failures.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error' && !message.text().includes('Failed to load resource:')) failures.push(message.text()); });
  page.on('response', (response) => { if (response.status() >= 500) failures.push(`${response.status()} ${response.url()}`); });
  await page.goto(config.base);
  assert.equal(await page.locator('.login-brand').evaluate((node) => getComputedStyle(node).justifyContent), 'center');
  assert.equal(await page.locator('.login-brand img').evaluate((node) => getComputedStyle(node).objectFit), 'contain');
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(config.password);
  await Promise.all([page.waitForURL(`**/${route}`), page.getByRole('button', { name: 'Sign in', exact: true }).click()]);
  await page.getByRole('button', { name: route === 'review' ? /^Synthetic Solar Project/ : 'Synthetic Solar Project', exact: route !== 'review' }).click();
  return page;
}
async function api(page, path, body) {
  return page.evaluate(async ({ path, body }) => {
    const response = await fetch(path, body ? { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : {});
    if (!response.ok) throw new Error(`${response.status}: ${await response.text()}`);
    return response.json();
  }, { path, body });
}
async function pdfText(page) {
  return page.evaluate(async (paper) => {
    const pdfjs = await import('/static/pdf.min.mjs');
    pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const response = await fetch(`/api/v2/papers/${paper}/artifacts/pdf`);
    if (!response.ok) throw new Error(`PDF: ${response.status}`);
    const loading = pdfjs.getDocument({ data: new Uint8Array(await response.arrayBuffer()) });
    const pdf = await loading.promise;
    let text = '';
    for (let index = 1; index <= pdf.numPages; index++) {
      const page = await pdf.getPage(index); const content = await page.getTextContent();
      text += content.items.map((item) => item.str).join(' ') + '\n';
    }
    await loading.destroy();
    return text.replace(/\s+/g, ' ');
  }, config.paper_id);
}
async function pdfHasImage(page) {
  return page.evaluate(async (paper) => {
    const pdfjs = await import('/static/pdf.min.mjs');
    pdfjs.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
    const response = await fetch(`/api/v2/papers/${paper}/artifacts/pdf`);
    const loading = pdfjs.getDocument({ data: new Uint8Array(await response.arrayBuffer()) });
    const pdf = await loading.promise;
    let found = false;
    for (let index = 1; index <= pdf.numPages && !found; index++) {
      const operators = await (await pdf.getPage(index)).getOperatorList();
      found = operators.fnArray.some((operation) => [pdfjs.OPS.paintImageXObject, pdfjs.OPS.paintInlineImageXObject, pdfjs.OPS.paintImageMaskXObject].includes(operation));
    }
    await loading.destroy();
    return found;
  }, config.paper_id);
}
async function editorText(page) {
  return page.locator('.cm-content').innerText();
}
async function editorLines(page) {
  return page.locator('.cm-line').allTextContents();
}
async function selectLineRange(page, lineText, start, length) {
  const line = page.locator('.cm-line').filter({ hasText: lineText }).first();
  await line.click();
  await page.keyboard.press('Home');
  for (let index = 0; index < start; index++) await page.keyboard.press('ArrowRight');
  for (let index = 0; index < length; index++) await page.keyboard.press('Shift+ArrowRight');
}
async function waitEditorContains(page, expected) {
  await page.waitForFunction((text) => document.querySelector('.cm-content')?.innerText.includes(text), expected);
}
async function insertAction(page, label) {
  await page.locator('#insertMenu').click();
  await page.getByRole('button', { name: label, exact: true }).click();
}
async function fillBuilder(page, label, value) {
  const control = page.locator('#dialogBody label').filter({ hasText: label }).locator('input,textarea,select').first();
  if (await control.getAttribute('type') === 'checkbox') {
    if (value) await control.check(); else await control.uncheck();
  } else await control.fill(String(value));
}
function pngChunk(type, data) {
  const name = Buffer.from(type);
  const body = Buffer.concat([name, data]);
  let crc = 0xffffffff;
  for (const byte of body) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  const length = Buffer.alloc(4); length.writeUInt32BE(data.length);
  const checksum = Buffer.alloc(4); checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([length, body, checksum]);
}
function testPng() {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(1, 0); header.writeUInt32BE(1, 4);
  header.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(Buffer.from([0, 40, 120, 220, 255]))),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}
async function waitCompile(page) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    const result = await api(page, `/api/v2/papers/${config.paper_id}/builds`);
    if (!result.build.active_build_id && result.build.current_build_id && result.build.current_source_sequence === result.build.source_sequence) return result.build;
    if (!result.build.active_build_id && result.build.latest_status === 'failed') throw new Error(JSON.stringify(result.build.latest_error));
    await page.waitForTimeout(500);
  }
  throw new Error('Timed out waiting for M7 compilation');
}
try {
  const leader = await login(config.leader, 'write');
  assert.equal(await leader.locator('header #projectSearch').count(), 1, 'Search paper belongs in the header');
  assert.equal(await leader.locator('.files-pane #projectSearch').count(), 0, 'file explorer must not duplicate Search paper');
  await leader.getByText('Complete document details', { exact: true }).waitFor();
  const detailsPath = `/api/v2/papers/${config.paper_id}/document-details`;
  const detail = await api(leader, detailsPath);
  for (const key of ['team_name', 'team_size', 'student_a_name', 'student_a_reg_no', 'guide_name', 'guide_designation', 'team_semester', 'hod_name', 'dean_name']) {
    assert.equal(await leader.locator(`[name="field:${key}"]`).count(), 0, `${key} must not be requested`);
  }
  await leader.locator('[name="project_type"]').selectOption('capstone');
  await leader.locator('[name="executive_summary"]').fill('Browser-qualified executive summary.');
  const departmentInputs = leader.locator('fieldset').filter({ hasText: 'Department display names' }).locator('input');
  const schoolInputs = leader.locator('fieldset').filter({ hasText: 'School display names' }).locator('input');
  assert.equal(await departmentInputs.count(), 2);
  assert.equal(await schoolInputs.count(), 2);
  await departmentInputs.nth(0).fill('Computer Science'); await departmentInputs.nth(1).fill('Data Science');
  await schoolInputs.nth(0).fill('School of Computing'); await schoolInputs.nth(1).fill('School of Data');
  await leader.getByRole('button', { name: 'Save project metadata', exact: true }).click();
  await leader.getByText('Project metadata saved. Compile to refresh the PDF.', { exact: true }).waitFor();
  for (const [key, value] of Object.entries(config.manual)) {
    const control = leader.locator(`[name="field:${key}"]`);
    if (await control.count()) await control.fill(value);
  }
  const guide = leader.locator('[name="field:guide_identity"]');
  await guide.selectOption({ label: 'Dr. Grace Guide' });
  await leader.getByRole('button', { name: 'Save document details', exact: true }).click();
  await leader.getByText('Document details saved. Front Matter was rebuilt. Compile to refresh the PDF.', { exact: true }).waitFor();
  const beforeManual = await api(leader, `/api/v2/papers/${config.paper_id}/builds`);
  assert.equal(beforeManual.build.current_build_id, null, 'metadata and Front Matter saves must not compile');
  assert.equal(beforeManual.build.active_build_id, null, 'metadata and Front Matter saves must not queue a build');
  await leader.getByRole('button', { name: 'Compile', exact: true }).click();
  await waitCompile(leader);
  const text = await pdfText(leader);
  for (const expected of ['Synthetic Solar Project', 'Alice Alpha', 'Ben Beta', 'Cara Gamma', 'Dev Delta', 'Dr. Grace Guide', 'Associate Professor', 'Bachelor of Technology', 'Computer Science and Engineering', 'VIII', 'Dr. Helen Head', 'Prof. Dana Dean', 'School of Computing', 'September', '2026', 'Cover', 'Certificate', 'Declaration', 'Acknowledgement']) assert.ok(text.includes(expected), `PDF missing: ${expected}\n${text}`);
  for (const value of detail.values.filter((value) => /student_[abcd]_reg_no/.test(value.field_key))) assert.ok(text.includes(value.value), `PDF missing registration ${value.value}`);
  for (const placeholder of ['Student A name', 'Dr. Project guide name', 'departmentname']) assert.ok(!text.includes(placeholder), `PDF contains placeholder ${placeholder}`);
  assert.ok(text.indexOf('Cover') < text.indexOf('Certificate') && text.indexOf('Certificate') < text.indexOf('Declaration') && text.indexOf('Declaration') < text.indexOf('Acknowledgement'));
  await leader.reload();
  await leader.getByRole('button', { name: 'Synthetic Solar Project', exact: true }).click();
  await leader.waitForTimeout(500);
  assert.equal(await leader.getByText('Complete document details', { exact: true }).isVisible(), false);
  await leader.locator('#documentDetails').click();
  const reloadedProject = await api(leader, `/api/v2/papers/${config.paper_id}/project-metadata`);
  assert.deepEqual(reloadedProject.values.department_display_names.map((item) => item.display_name).sort(), ['Computer Science', 'Data Science']);
  assert.deepEqual(reloadedProject.values.school_display_names.map((item) => item.display_name).sort(), ['School of Computing', 'School of Data']);
  assert.equal(await leader.locator('[name="field:course_code"]').inputValue(), 'CSE4999');
  const writer = await login(config.writer, 'write');
  await writer.locator('#documentDetails').click();
  await writer.getByText('Front Matter ready', { exact: true }).waitFor();
  assert.equal(await writer.getByRole('button', { name: 'Save document details' }).count(), 0);
  assert.equal(await writer.locator('[name^="field:"]').count(), 0);
  assert.ok((await pdfText(writer)).includes('Alice Alpha'));
  // Real CodeMirror/Yjs selection, replacement and undo across two already-open Writers.
  await selectLineRange(leader, 'Main content target phrase.', 5, 7);
  await leader.keyboard.type('section');
  await waitEditorContains(writer, 'Main section target phrase.');
  await leader.locator('#undoText').click();
  await waitEditorContains(leader, 'Main content target phrase.');
  await waitEditorContains(writer, 'Main content target phrase.');

  // Dialog insertion preserves a Yjs-relative range while another Writer inserts before it.
  await selectLineRange(leader, 'CodeSlot', 0, 'CodeSlot'.length);
  await insertAction(leader, 'Code block');
  await fillBuilder(leader, 'Inline code', 'fn demo() {\n    println!("# % & _");\n}');
  await writer.locator('.cm-content').click();
  await writer.keyboard.press('Control+Home');
  await writer.keyboard.type('% second Writer shift\n');
  await waitEditorContains(leader, '% second Writer shift');
  await leader.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
  await waitEditorContains(writer, 'fn demo()');
  assert.ok((await editorText(leader)).includes('Main content target phrase.'), 'remote shift must not replace unrelated text');

  await selectLineRange(leader, 'InlineSlot', 0, 'InlineSlot'.length);
  await insertAction(leader, 'Inline math');
  await waitEditorContains(writer, '\\(InlineSlot\\)');
  await selectLineRange(leader, 'SymbolSlot', 0, 'SymbolSlot'.length);
  await insertAction(leader, 'Symbols');
  const symbol = leader.locator('.symbol-grid button').first();
  const symbolCommand = (await symbol.getAttribute('title')).split('—').at(-1).trim();
  await symbol.click();
  await waitEditorContains(writer, symbolCommand);
  assert.ok(!(await editorText(leader)).includes('SymbolSlot'));

  // Create and edit the chapter at the destination offered by the real wrapped template.
  let fileDialogIndex = 0;
  let offeredChapterPath = null;
  const fileDialog = async (dialog) => {
    fileDialogIndex += 1;
    if (dialog.type() === 'prompt') {
      offeredChapterPath = dialog.defaultValue();
      await dialog.accept(offeredChapterPath);
    } else await dialog.accept();
  };
  leader.on('dialog', fileDialog);
  await leader.locator('#newFile').click();
  await leader.waitForTimeout(1000);
  assert.ok((await leader.locator('#currentFile').textContent()).includes('chapter9.tex'), JSON.stringify({ dialogs: fileDialogIndex, offeredChapterPath, notice: await leader.locator('#writerNotice').textContent(), current: await leader.locator('#currentFile').textContent() }));
  leader.off('dialog', fileDialog);
  assert.equal(fileDialogIndex, 2);
  assert.equal(offeredChapterPath, 'Thesis_content_v1.0/chapters/chapter9.tex');
  await leader.locator('.cm-content').click();
  await leader.keyboard.type('First line\n\nThird line');
  await leader.keyboard.press('Control+a');
  await insertAction(leader, 'Comment selected lines');
  await waitEditorContains(leader, '% First line');
  assert.deepEqual(await editorLines(leader), ['% First line', '% ', '% Third line']);
  await leader.keyboard.press('Control+a');
  await insertAction(leader, 'Uncomment selected lines');
  assert.deepEqual(await editorLines(leader), ['First line', '', 'Third line']);
  await leader.locator('#undoText').click();
  await waitEditorContains(leader, '% Third line');
  await leader.locator('#redoText').click();
  assert.deepEqual(await editorLines(leader), ['First line', '', 'Third line']);
  await leader.locator('#fileTree button').filter({ hasText: 'main.tex' }).click();
  await waitEditorContains(leader, 'TableSlot');

  await selectLineRange(leader, 'TableSlot', 0, 'TableSlot'.length);
  await insertAction(leader, 'Table');
  await fillBuilder(leader, 'Rows', 2);
  await fillBuilder(leader, 'Columns', 2);
  await fillBuilder(leader, 'Column alignments', 'left,left');
  await fillBuilder(leader, 'Column widths', ',6cm');
  await fillBuilder(leader, 'Selected cell (row,column', '2,1');
  await fillBuilder(leader, 'Selected cell content', 'Multiline first\nMultiline second');
  await fillBuilder(leader, 'Selected cell’s shared column width', '3cm');
  await fillBuilder(leader, 'Selected cell’s shared row minimum height', '8mm');
  await fillBuilder(leader, 'Caption', 'Browser sized table');
  await leader.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
  assert.ok((await editorText(leader)).indexOf('\\caption{Browser sized table}') < (await editorText(leader)).indexOf('\\begin{tabular}{p{3cm}p{6cm}}'));
  assert.ok((await editorText(leader)).includes('\\rule{0pt}{8mm}\\shortstack{Multiline first\\\\Multiline second}'));

  await selectLineRange(leader, 'FigureSlot', 0, 'FigureSlot'.length);
  const png = testPng();
  let offeredImagePath = null;
  const uploadDialogs = async (dialog) => {
    if (dialog.type() === 'prompt') {
      offeredImagePath = dialog.defaultValue();
      await dialog.accept(offeredImagePath);
    } else await dialog.accept();
  };
  leader.on('dialog', uploadDialogs);
  const chooser = leader.waitForEvent('filechooser');
  await leader.locator('#uploadImage').click();
  await (await chooser).setFiles({ name: 'browser.png', mimeType: 'image/png', buffer: png });
  try {
    await leader.waitForFunction(() => document.querySelector('#productivityDialog')?.open && document.querySelector('#dialogTitle')?.textContent === 'Figure Builder', null, { timeout: 10000 });
  } catch (error) {
    throw new Error(JSON.stringify({ offeredImagePath, notice: await leader.locator('#writerNotice').textContent(), inputFiles: await leader.locator('#assetInput').evaluate((input) => input.files?.length), dialogOpen: await leader.locator('#productivityDialog').evaluate((dialog) => dialog.open) }), { cause: error });
  }
  leader.off('dialog', uploadDialogs);
  assert.equal(offeredImagePath, 'Thesis_content_v1.0/images/browser.png');
  await fillBuilder(leader, 'Caption', 'Browser uploaded image');
  await leader.locator('#dialogActions').getByRole('button', { name: 'Insert', exact: true }).click();
  const figureSource = await editorText(leader);
  assert.ok(figureSource.includes('\\includegraphics[width=\\linewidth]{images/browser.png}'));
  assert.ok(figureSource.indexOf('\\includegraphics') < figureSource.indexOf('\\caption{Browser uploaded image}'));

  await selectLineRange(leader, 'PublicationSlot', '\\bibitem{placeholder} '.length, 'PublicationSlot'.length);
  await insertAction(leader, 'Publications');
  await leader.getByRole('button', { name: 'Insert categorized bibitems', exact: true }).click();
  await leader.locator('#drawerClose').click();
  for (const key of ['solar-communicated', 'solar-accepted', 'solar-published']) await waitEditorContains(leader, `\\bibitem{${key}}`);
  await selectLineRange(leader, '\\bibitem{solar-communicated}', 0, 0);
  await insertAction(leader, 'Publications');
  await leader.getByRole('button', { name: 'Insert categorized bibitems', exact: true }).click();
  await leader.getByText(/must be present and unique across this report/).waitFor();
  await leader.locator('#drawerClose').click();

  const jobsBeforeInsertionCompile = await api(leader, `/api/v2/papers/${config.paper_id}/builds`);
  assert.equal(jobsBeforeInsertionCompile.build.active_build_id, null, 'editor and insertion actions must not compile');
  await leader.getByRole('button', { name: 'Compile', exact: true }).click();
  await waitCompile(leader);
  const insertionPdf = await pdfText(leader);
  for (const expected of ['Browser sized table', 'Browser uploaded image', 'Communicated', 'Accepted', 'Published', 'Draft Solar Work', 'demo']) assert.ok(insertionPdf.includes(expected), `insertion PDF missing ${expected}`);
  assert.equal(await pdfHasImage(leader), true, 'uploaded PNG must be painted in the compiled PDF');
  const mentor = await login(config.mentor, 'review');
  assert.equal(await mentor.getByRole('button', { name: 'Document details' }).count(), 0);
  // Mentor PDF is loaded by the existing read-only review viewer.
  await mentor.waitForFunction(() => document.querySelector('#pdfCanvas')?.width > 0 && document.querySelector('#pdfCanvas')?.height > 0);
  const mentorColumns = await mentor.evaluate(() => ({ source: document.querySelector('.review-source').getBoundingClientRect().width, pdf: document.querySelector('.review-pdf').getBoundingClientRect().width }));
  assert.ok(mentorColumns.pdf > mentorColumns.source, `Mentor PDF must be wider than source: ${JSON.stringify(mentorColumns)}`);
  assert.ok((await pdfText(mentor)).includes('Dr. Grace Guide'));
  await mentor.context().close();
  // One optional field is deliberately omitted, while all other choices persist.
  const saved = await api(leader, detailsPath);
  const values = Object.fromEntries(saved.values.filter((value) => value.value_source === 'TEAM_OVERRIDE').map((value) => [value.field_key, value.value]));
  delete values.specialization;
  await api(leader, detailsPath, { values, sections: {} });
  await leader.getByRole('button', { name: 'Compile', exact: true }).click();
  await waitCompile(leader);
  const optionalText = await pdfText(leader);
  assert.ok(!optionalText.includes('Intelligent Systems'));
  assert.ok(optionalText.includes('Synthetic Solar Project'));
  await leader.locator('#documentDetails').click();
  await leader.getByText('Specialization is not available from institution data or Document details.', { exact: true }).waitFor();
  await leader.locator('#drawerClose').click();
  await leader.locator('#insertMenu').click();
  const insertEntries = await leader.locator('#dialogBody button').allTextContents();
  for (const entry of ['Table', 'Figure', 'Code block', 'Inline math', 'Display math', 'Symbols', 'Publications', 'BibTeX entry']) assert.equal(insertEntries.filter((value) => value === entry).length, 1, `${entry} must appear once in Insert`);
  await leader.getByRole('button', { name: 'Symbols', exact: true }).click();
  assert.ok(await leader.locator('.symbol-grid button').count() > 10, 'symbol catalogue must render as a grid');
  const firstSymbol = leader.locator('.symbol-grid button').first();
  assert.ok(await firstSymbol.getAttribute('aria-label'));
  assert.ok(await firstSymbol.getAttribute('title'));
  await leader.locator('#dialogSearch').press('ArrowDown');
  await leader.locator('#dialogSearch').press('ArrowUp');
  await leader.getByRole('button', { name: 'Close', exact: true }).click();
  await leader.locator('#moreActions').click();
  const moreEntries = await leader.locator('#dialogBody button').allTextContents();
  assert.equal(moreEntries.filter((value) => ['Table', 'Figure', 'Code block', 'Inline math', 'Display math', 'Symbols'].includes(value)).length, 0, 'More actions must not duplicate Insert commands');
  await leader.getByRole('button', { name: 'Close', exact: true }).click();
  await leader.getByRole('button', { name: 'Send for review', exact: true }).click();
  await leader.waitForFunction(() => document.querySelector('#compilePaper')?.disabled === true && document.querySelector('#endReview')?.disabled === false);
  await writer.waitForFunction(() => document.querySelector('#compilePaper')?.disabled === true && document.querySelector('.cm-content')?.getAttribute('contenteditable') === 'false');
  assert.equal(await writer.locator('#endReview').isVisible(), false, 'regular Writer must not receive End review');
  const reviewingMentor = await login(config.mentor, 'review');
  await reviewingMentor.waitForFunction(() => document.querySelector('#pushReview')?.disabled === false && document.querySelector('#createComment')?.disabled === false);
  reviewingMentor.once('dialog', (dialog) => dialog.accept());
  await reviewingMentor.getByRole('button', { name: 'Push review', exact: true }).click();
  await reviewingMentor.waitForFunction(() => ['Review submitted', 'No active review'].includes(document.querySelector('#draftStatus')?.textContent));
  await writer.bringToFront();
  assert.equal(await writer.locator('#compilePaper').isDisabled(), true, 'one Mentor submission must not unlock a multi-Mentor round');
  const secondMentor = await login(config.mentor_two, 'review');
  await secondMentor.waitForFunction(() => document.querySelector('#pushReview')?.disabled === false);
  secondMentor.once('dialog', (dialog) => dialog.accept());
  await secondMentor.getByRole('button', { name: 'Push review', exact: true }).click();
  await secondMentor.waitForFunction(() => ['Review submitted', 'No active review'].includes(document.querySelector('#draftStatus')?.textContent));
  await writer.bringToFront();
  await writer.waitForFunction(() => document.querySelector('#compilePaper')?.disabled === false);
  assert.notEqual(await writer.locator('.cm-content').getAttribute('contenteditable'), 'false', 'Writer editor must leave read-only mode after the round');
  await reviewingMentor.context().close();
  await secondMentor.context().close();
  assert.deepEqual(failures, []);
  console.log(JSON.stringify({ leader: 'passed', writer: 'passed', editor_insertions: 'passed', wrapped_paths: 'passed', uploaded_pdf_image: 'passed', publications: 'passed', mentor: 'passed', review_turn_taking: 'passed', m7: config.environment, populated_pdf_assertions: 27, optional_blank_compile: 'passed', browser_errors: failures.length }));
} finally {
  await browser.close();
}
