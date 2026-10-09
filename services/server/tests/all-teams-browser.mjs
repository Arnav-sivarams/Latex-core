import assert from 'node:assert/strict';
import { chromium } from 'playwright';
const config = JSON.parse(process.env.LATEX_CORE_ALL_TEAMS_CONFIG);
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || chromium.executablePath(), args: ['--no-sandbox'] });
try {
  const context = await browser.newContext();
  context.setDefaultTimeout(30000);
  const [name, ...value] = config.cookie.split('=');
  await context.addCookies([{ name, value: value.join('='), url: config.base }]);
  const page = await context.newPage();
  await page.goto(`${config.base}/admin`);
  await page.getByRole('button', { name: 'File Policies', exact: true }).click();
  const teams = page.getByRole('combobox', { name: 'Paper Team', exact: true });
  await teams.waitFor();
  assert.equal(await teams.locator('option').first().innerText(), 'All Teams');
  await page.getByText(`${config.count} Teams affected.`, { exact: false }).waitFor();
  assert.ok(await page.getByText('Mixed / Varies', { exact: true }).count());
  const policy = () => page.getByRole('combobox', { name: 'Change policy for shared-policy.tex across All Teams', exact: true });
  await policy().selectOption('STRUCTURE_LOCKED');
  let writes = 0;
  page.on('request', request => { if (request.method() === 'PATCH' && request.url().endsWith('/api/admin/v2/file-policies')) writes++; });
  page.once('dialog', async dialog => { assert.match(dialog.message(), new RegExp(`all ${config.count} Teams`)); await dialog.dismiss(); });
  await page.getByRole('button', { name: 'Apply policies to All Teams', exact: true }).click();
  assert.equal(writes, 0, 'cancel must make no bulk request');
  page.once('dialog', dialog => dialog.accept());
  const applied = page.waitForResponse(response => response.request().method() === 'PATCH' && response.url().endsWith('/api/admin/v2/file-policies'));
  await page.getByRole('button', { name: 'Apply policies to All Teams', exact: true }).click();
  const response = await applied;
  assert.equal(response.status(), 200);
  assert.deepEqual(response.request().postDataJSON().changes, [{ path: 'shared-policy.tex', policy: 'STRUCTURE_LOCKED' }]);
  await page.waitForFunction(() => !document.querySelector('[aria-label="Paper Team"]')?.disabled);
  await teams.selectOption(config.teams[0]);
  await page.getByRole('combobox', { name: 'Change policy for shared-policy.tex', exact: true }).waitFor();
  assert.equal(await page.getByRole('combobox', { name: 'Change policy for shared-policy.tex', exact: true }).inputValue(), 'STRUCTURE_LOCKED');
  for (const team of config.teams) {
    const result = await context.request.get(`${config.base}/api/admin/v2/paper-teams/${team}/file-policies`);
    assert.equal(result.status(), 200);
    const files = await result.json();
    assert.equal(files.find(file => file.path === 'shared-policy.tex').policy, 'STRUCTURE_LOCKED');
    assert.equal(files.find(file => file.path === 'untouched-policy.tex').policy, 'EDITABLE');
  }
  await page.reload();
  await page.getByRole('button', { name: 'File Policies', exact: true }).click();
  await teams.waitFor();
  assert.equal(await teams.locator('option').first().innerText(), 'All Teams');
  console.log('ALL_TEAMS_BROWSER: PASS — first option, mixed/count, confirmation/cancel, explicit changes and persistence.');
} finally { await browser.close(); }
