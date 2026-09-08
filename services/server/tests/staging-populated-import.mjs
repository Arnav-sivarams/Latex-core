import assert from 'node:assert/strict';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { randomBytes, randomUUID } from 'node:crypto';
import { chromium } from 'playwright';

const baseURL = process.env.STAGING_HANDOFF_BASE_URL;
const phase = process.env.STAGING_HANDOFF_PHASE;
const statePath = process.env.STAGING_HANDOFF_STATE_FILE;
const passwordPath = process.env.STAGING_HANDOFF_ADMIN_PASSWORD_FILE;
const evidenceDirectory = process.env.STAGING_HANDOFF_EVIDENCE_DIR;
const adminEmail = process.env.STAGING_HANDOFF_ADMIN_EMAIL || 'handoff-admin@example.invalid';
const browserPath = process.env.PLAYWRIGHT_CHROMIUM_PATH || '/home/arnav/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome';
if (!baseURL || !['populate', 'verify'].includes(phase) || !statePath || !passwordPath || !evidenceDirectory) {
  throw new Error('isolated staging handoff browser-test environment is required');
}

await mkdir(evidenceDirectory, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath: browserPath });

async function login(page) {
  await page.goto(baseURL, { waitUntil: 'domcontentloaded' });
  await page.locator('#email').fill(adminEmail);
  await page.locator('#password').fill((await readFile(passwordPath, 'utf8')).trim());
  await Promise.all([
    page.waitForURL(/\/admin$/),
    page.getByRole('button', { name: 'Sign in' }).click(),
  ]);
}

async function request(page, path, options = {}) {
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

async function importTemplate(page) {
  const bytes = (await readFile('examples/professor-demo/dist/main-template.zip')).toString('base64');
  return page.evaluate(async (archive) => {
    const body = new FormData();
    body.set('name', `Handoff template ${crypto.randomUUID()}`);
    body.set('description', 'Synthetic isolated staging handoff fixture');
    body.set('main', 'main.tex');
    const data = Uint8Array.from(atob(archive), (character) => character.charCodeAt(0));
    body.set('archive', new File([data], 'main-template.zip', { type: 'application/zip' }));
    const response = await fetch('/api/admin/v2/templates/import', { method: 'POST', credentials: 'same-origin', body });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, bytes);
}

function fixtures(suffix, department) {
  const faculty = [
    [`FAC-${suffix}-1`, 'Synthetic Mentor One', `mentor-one-${suffix}@example.invalid`, department, 'Dr', 'Mentor', 'ACTIVE'],
    [`FAC-${suffix}-2`, 'Synthetic Mentor Two', `mentor-two-${suffix}@example.invalid`, department, 'Dr', 'Mentor', 'ACTIVE'],
  ];
  const students = [
    [`STU-${suffix}-1`, 'Synthetic Writer One', `writer-one-${suffix}@example.invalid`, `CSE${suffix}`],
    [`STU-${suffix}-2`, 'Synthetic Writer Two', `writer-two-${suffix}@example.invalid`, `CSE${suffix}`],
    [`STU-${suffix}-3`, 'Synthetic Writer Three', `writer-three-${suffix}@example.invalid`, `ECE${suffix}`],
    [`STU-${suffix}-4`, 'Synthetic Writer Four', `writer-four-${suffix}@example.invalid`, `ECE${suffix}`],
    [`STU-${suffix}-5`, 'Synthetic Writer Five', `writer-five-${suffix}@example.invalid`, `CSE${suffix}`],
  ];
  const teams = [
    [`TEAM-${suffix}-MAJORITY`, `Majority Team ${suffix}`, '2026-2027', '1', 'ACTIVE'],
    [`TEAM-${suffix}-TIE`, `Tie Team ${suffix}`, '2026-2027', '1', 'ACTIVE'],
    [`TEAM-${suffix}-THIRD`, `Third Team ${suffix}`, '2026-2027', '1', 'ACTIVE'],
  ];
  const rows = (header, values) => `${header}\n${values.map((row) => row.join(',')).join('\n')}\n`;
  return {
    names: teams.map((team) => team[1]),
    programmes: [`CSE${suffix}`, `ECE${suffix}`],
    identityFiles: {
      'departments.csv': `department_id\n${department}\n`,
      'faculty.csv': rows('faculty_id,name,email,dept_id,honorific,designation,status', faculty),
      'programmes.csv': rows('programme_code,hod_id', [[`CSE${suffix}`, faculty[0][0]], [`ECE${suffix}`, faculty[1][0]]]),
      'students.csv': rows('reg_no,name,email,programme_code', students),
    },
    teamFiles: {
      'paper_teams.csv': rows('external_team_key,team_name,academic_year,semester,status', teams),
      'paper_team_writers.csv': rows('external_team_key,student_reg_no,writer_order,is_leader', [
        [teams[0][0], students[0][0], '1', 'true'], [teams[0][0], students[1][0], '2', 'false'], [teams[0][0], students[2][0], '3', 'false'],
        [teams[1][0], students[2][0], '1', 'true'], [teams[1][0], students[0][0], '2', 'false'],
        [teams[2][0], students[3][0], '1', 'true'], [teams[2][0], students[4][0], '2', 'false'],
      ]),
      'paper_team_mentors.csv': rows('external_team_key,faculty_id', [
        [teams[0][0], faculty[0][0]], [teams[1][0], faculty[0][0]], [teams[1][0], faculty[1][0]], [teams[2][0], faculty[1][0]],
      ]),
    },
  };
}

async function importBatch(page, files) {
  return page.evaluate(async (files) => {
    const body = new FormData();
    body.set('operation', 'ADD');
    for (const [name, content] of Object.entries(files)) {
      body.append('files[]', new File([content], name, { type: 'text/csv' }), name);
    }
    const response = await fetch('/api/admin/v2/institution/import-batches/validate', { method: 'POST', credentials: 'same-origin', body });
    return { status: response.status, payload: await response.json().catch(() => null) };
  }, files);
}

async function matchingTeams(page, names) {
  const result = await request(page, '/api/admin/v2/paper-teams/query?limit=100&page=1');
  assert.equal(result.status, 200, JSON.stringify(result.payload));
  return result.payload.items.filter((team) => names.includes(team.name));
}

try {
  const context = await browser.newContext();
  const page = await context.newPage();
  await login(page);
  if (phase === 'populate') {
    const suffix = randomBytes(3).toString('hex').toUpperCase();
    const template = await importTemplate(page);
    assert.equal(template.status, 201, JSON.stringify(template.payload));
    const fallback = await request(page, '/api/admin/v2/institution/template-defaults/global-fallback', {
      method: 'PUT', body: { template_id: template.payload.id },
    });
    assert.equal(fallback.status, 204, JSON.stringify(fallback.payload));
    const fixture = fixtures(suffix, randomUUID());
    const identities = await importBatch(page, fixture.identityFiles);
    assert.equal(identities.status, 201, JSON.stringify(identities.payload));
    assert.equal(identities.payload.batch.status, 'VALIDATED');
    assert.equal(identities.payload.batch.error_rows, 0);
    const identityBatchId = identities.payload.batch.id;
    const identitiesApplied = await request(page, `/api/admin/v2/institution/import-batches/${identityBatchId}/apply`, { method: 'POST', body: {} });
    assert.equal(identitiesApplied.status, 200, JSON.stringify(identitiesApplied.payload));
    assert.equal(identitiesApplied.payload.account_provisioning.created, 5);
    for (const programme of fixture.programmes) {
      const mapped = await request(page, `/api/admin/v2/institution/template-defaults/programmes/${programme}`, {
        method: 'PUT', body: { template_id: template.payload.id },
      });
      assert.equal(mapped.status, 204, JSON.stringify(mapped.payload));
    }
    const teamBatch = await importBatch(page, fixture.teamFiles);
    assert.equal(teamBatch.status, 201, JSON.stringify(teamBatch.payload));
    assert.equal(teamBatch.payload.batch.status, 'VALIDATED');
    assert.equal(teamBatch.payload.batch.error_rows, 0);
    const teamBatchId = teamBatch.payload.batch.id;
    const teamsApplied = await request(page, `/api/admin/v2/institution/import-batches/${teamBatchId}/apply`, { method: 'POST', body: {} });
    assert.equal(teamsApplied.status, 200, JSON.stringify(teamsApplied.payload));
    assert.equal(teamsApplied.payload.account_provisioning.created, 2);
    const teams = await matchingTeams(page, fixture.names);
    assert.equal(teams.length, 3, JSON.stringify(teams));
    const majority = teams.find((team) => team.name.startsWith('Majority Team'));
    const tie = teams.find((team) => team.name.startsWith('Tie Team'));
    assert.equal(majority.dominant_programme_code, `CSE${suffix}`);
    assert.equal(majority.template_resolution_method, 'MODE');
    assert.equal(tie.dominant_programme_code, `ECE${suffix}`);
    assert.equal(tie.template_resolution_method, 'TIE_FIRST_WRITER');
    assert.deepEqual(teams.map((team) => team.writer_count).sort(), [2, 2, 3]);
    assert.deepEqual(teams.map((team) => team.mentors.length).sort(), [1, 1, 2]);
    const state = { schemaVersion: 1, batchIds: [identityBatchId, teamBatchId], names: fixture.names, teamIds: teams.map((team) => team.id), suffix };
    await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
    await chmod(statePath, 0o600);
    console.log(JSON.stringify({ phase, status: 'PASS', programmes: 2, writers: 5, mentors: 2, teams: 3, majority: majority.dominant_programme_code, tie: tie.dominant_programme_code }));
  } else {
    const state = JSON.parse(await readFile(statePath, 'utf8'));
    const teams = await matchingTeams(page, state.names);
    assert.equal(teams.length, 3);
    assert.deepEqual(teams.map((team) => team.id).sort(), [...state.teamIds].sort());
    for (const batchId of state.batchIds) {
      const batch = await request(page, `/api/admin/v2/institution/import-batches/${batchId}`);
      assert.equal(batch.status, 200);
      assert.equal(batch.payload.batch.status, 'APPLIED');
    }
    const evidence = { phase, status: 'PASS', batchIds: state.batchIds, teamIds: state.teamIds, stableIds: true, appliedImportHistory: true };
    const evidencePath = `${evidenceDirectory}/populated-import-preservation.json`;
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
    await chmod(evidencePath, 0o600);
    console.log(JSON.stringify({ ...evidence, evidencePath }));
  }
  await context.close();
} finally {
  await browser.close();
}
