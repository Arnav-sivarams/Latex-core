import assert from 'node:assert/strict';
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
  await leader.getByText('Complete document details', { exact: true }).waitFor();
  const detailsPath = `/api/v2/papers/${config.paper_id}/document-details`;
  const detail = await api(leader, detailsPath);
  for (const key of ['team_name', 'team_size', 'student_a_name', 'student_a_reg_no', 'guide_name', 'guide_designation', 'team_semester', 'hod_name', 'dean_name']) {
    assert.equal(await leader.locator(`[name="field:${key}"]`).count(), 0, `${key} must not be requested`);
  }
  for (const [key, value] of Object.entries(config.manual)) await leader.locator(`[name="field:${key}"]`).fill(value);
  const guide = leader.locator('[name="field:guide_identity"]');
  await guide.selectOption({ label: 'Dr. Grace Guide' });
  await leader.getByRole('button', { name: 'Save document details', exact: true }).click();
  await leader.getByText('Document details saved. Front Matter was rebuilt.', { exact: true }).waitFor();
  await waitCompile(leader);
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
  assert.equal(await leader.locator('[name="field:course_code"]').inputValue(), 'CSE4999');
  const writer = await login(config.writer, 'write');
  await writer.locator('#documentDetails').click();
  await writer.getByText('Front Matter ready', { exact: true }).waitFor();
  assert.equal(await writer.getByRole('button', { name: 'Save document details' }).count(), 0);
  assert.equal(await writer.locator('[name^="field:"]').count(), 0);
  assert.ok((await pdfText(writer)).includes('Alice Alpha'));
  const mentor = await login(config.mentor, 'review');
  assert.equal(await mentor.getByRole('button', { name: 'Document details' }).count(), 0);
  // Mentor PDF is loaded by the existing read-only review viewer.
  await mentor.waitForFunction(() => document.querySelector('#pdfCanvas')?.width > 0 && document.querySelector('#pdfCanvas')?.height > 0);
  assert.ok((await pdfText(mentor)).includes('Dr. Grace Guide'));
  await mentor.context().close();
  // One optional field is deliberately omitted, while all other choices persist.
  const saved = await api(leader, detailsPath);
  const values = Object.fromEntries(saved.values.filter((value) => value.value_source === 'TEAM_OVERRIDE').map((value) => [value.field_key, value.value]));
  delete values.specialization;
  await api(leader, detailsPath, { values, sections: {} });
  await waitCompile(leader);
  const optionalText = await pdfText(leader);
  assert.ok(!optionalText.includes('Intelligent Systems'));
  assert.ok(optionalText.includes('Synthetic Solar Project'));
  await leader.locator('#documentDetails').click();
  await leader.getByText('Specialization is not available from institution data or Document details.', { exact: true }).waitFor();
  assert.deepEqual(failures, []);
  console.log(JSON.stringify({ leader: 'passed', writer: 'passed', mentor: 'passed', m7: config.environment, populated_pdf_assertions: 27, optional_blank_compile: 'passed', browser_errors: failures.length }));
} finally {
  await browser.close();
}
