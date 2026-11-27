import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { inflateRawSync } from 'node:zlib';

const admin = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
const csvCode = admin.slice(admin.indexOf('function csvCell('), admin.indexOf('function downloadCredentials('));
const writerCode = admin.slice(admin.indexOf('// Dependency-free ZIP'), admin.indexOf('async function fetchAllPages('));
const api = runInNewContext(`${csvCode}\n${writerCode}\n({ zipBytes, xlsxBytes, credentialsByTeam, credentialBundle, credentialsCsv, sheetNames, columnName })`, { TextEncoder });

// Reads stored or deflated ZIP members through the central directory, as spreadsheet tools do.
function unzip(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength); const decoder = new TextDecoder();
  const end = bytes.length - 22; assert.equal(view.getUint32(end, true), 0x06054b50, 'end of central directory');
  const count = view.getUint16(end + 10, true); let cursor = view.getUint32(end + 16, true); const files = new Map();
  for (let index = 0; index < count; index += 1) {
    assert.equal(view.getUint32(cursor, true), 0x02014b50, 'central header');
    const size = view.getUint32(cursor + 20, true); const nameLength = view.getUint16(cursor + 28, true); const local = view.getUint32(cursor + 42, true);
    const name = decoder.decode(bytes.subarray(cursor + 46, cursor + 46 + nameLength));
    assert.equal(view.getUint32(local, true), 0x04034b50, 'local header');
    const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const data = bytes.subarray(start, start + size); files.set(name, view.getUint16(local + 8, true) ? inflateRawSync(data) : data);
    cursor += 46 + nameLength + view.getUint16(cursor + 30, true) + view.getUint16(cursor + 32, true);
  }
  return files;
}

const team = (name, id, role, order = null, leader = false) => ({ team_name: name, team_id: id, team_key: `key-${id}`, team_role: role, writer_order: order, is_leader: leader });
const credential = (email, teams) => ({ email, teams, temporary_password: `Pw-${email}`, credential_role: 'student' });

test('credential bundle is a ZIP with all-Teams CSV/XLSX, Team-grouped JSON and one CSV per Team', () => {
  const source = [credential('w2@x.invalid', [team('Beta', 'b', 'writer', 2)]), credential('lead@x.invalid', [team('Beta', 'b', 'writer', 1, true)]), credential('a@x.invalid', [team('Alpha', 'a', 'writer', 1, true)]), credential('none@x.invalid', [])];
  const files = unzip(api.credentialBundle(source, 'batch-1', new Date('2026-11-27T10:00:00Z')));
  assert.deepEqual([...files.keys()], ['credentials-all-teams.csv', 'credentials-all-teams.xlsx', 'credentials-by-team.json', 'teams/001 Alpha.csv', 'teams/002 Beta.csv', 'teams/003 Unassigned.csv']);
  assert.equal(new TextDecoder().decode(files.get('credentials-all-teams.csv')), api.credentialsCsv(source));
  const json = JSON.parse(new TextDecoder().decode(files.get('credentials-by-team.json')));
  assert.deepEqual(json.teams.map((item) => item.team_name), ['Alpha', 'Beta', 'Unassigned']);
  assert.deepEqual(json.teams[1].members.map((member) => member.email), ['lead@x.invalid', 'w2@x.invalid']);
  assert.equal(json.teams[1].members[0].is_team_leader, true);
  const beta = new TextDecoder().decode(files.get('teams/002 Beta.csv')).trimEnd().split('\r\n');
  assert.equal(beta.length, 3); assert.match(beta[1], /^lead@x\.invalid,Pw-lead@x\.invalid,student,Beta,b,key-b,writer,1,true$/);
  const workbook = unzip(files.get('credentials-all-teams.xlsx'));
  assert.match(new TextDecoder().decode(workbook.get('xl/workbook.xml')), /<sheet name="All Teams"/);
});

test('XLSX writer escapes text, keeps leading zeros, never emits formulas and names sheets safely', () => {
  const files = unzip(api.xlsxBytes([{ name: 'students', rows: [['reg_no', 'name'], ['0042', '=HYPERLINK("x") & <b>']] }, { name: 'a/b:c*?[d]', rows: [['x']] }, { name: 'students', rows: [['y']] }]));
  const sheet = new TextDecoder().decode(files.get('xl/worksheets/sheet1.xml'));
  assert.match(sheet, /<t xml:space="preserve">0042<\/t>/);
  assert.match(sheet, /=HYPERLINK\(&quot;x&quot;\) &amp; &lt;b&gt;/);
  assert.doesNotMatch(sheet, /<f>/);
  assert.deepEqual(api.sheetNames([{ name: 'students' }, { name: 'a/b:c*?[d]' }, { name: 'students' }]), ['students', 'a b c   d', 'students~2']);
  assert.equal(api.columnName(0), 'A'); assert.equal(api.columnName(25), 'Z'); assert.equal(api.columnName(26), 'AA'); assert.equal(api.columnName(701), 'ZZ');
});

test('Admin action buttons ignore repeated clicks while a request is running', () => {
  assert.match(admin, /function buttonAction\(label, handler, className = ''\) \{[^]*?if \(running\) return;/);
  for (const label of ['Export all institution data (Excel)', 'Export all imported data (Excel)', 'Export all Paper Teams (Excel)', 'Reset Team passwords', 'Reset passwords']) assert.ok(admin.includes(`'${label}'`), label);
});
