import { api } from '/static/api.js?v=control-plane-groups-1';

const nav = document.querySelector('#adminNav');
const content = document.querySelector('#adminContent');
const status = document.querySelector('#adminStatus');
const sectionTitle = document.querySelector('#adminSectionTitle');
const adminDialog = document.querySelector('#adminDialog');
const adminDialogTitle = document.querySelector('#adminDialogTitle');
let dialogReturnFocus = null;

const endpoints = {
  Overview: '/api/admin/overview',
  Templates: '/api/admin/templates',
  Versions: '/api/admin/v2/versions',
  Reviews: '/api/admin/v2/reviews',
  'Build Queue': '/api/admin/jobs',
  Audit: '/api/admin/audit',
  System: '/api/admin/system',
};

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// Visible, dismissible notice; successes fade after a few seconds, failures stay until closed.
function announce(message, failed = false) {
  clearTimeout(status.noticeTimer); status.classList.toggle('danger', failed);
  if (!message) { status.replaceChildren(); return; }
  const text = element('span', 'status-message', message); const close = element('button', 'status-close', '×'); close.type = 'button'; close.setAttribute('aria-label', 'Dismiss notification'); close.title = 'Dismiss';
  close.addEventListener('click', () => { clearTimeout(status.noticeTimer); status.replaceChildren(); });
  status.replaceChildren(text, close); status.setAttribute('role', failed ? 'alert' : 'status');
  if (!failed) status.noticeTimer = setTimeout(() => status.replaceChildren(), 8000);
}

function showError(error) {
  announce(error.message || 'The administrative request failed.', true);
}

function openAdminDialog(title) {
  adminDialogTitle.textContent = title;
  dialogReturnFocus = document.activeElement;
  adminDialog.showModal();
  queueMicrotask(() => {
    const firstControl = adminDialog.querySelector('.admin-dialog-body input:not([disabled]), .admin-dialog-body select:not([disabled]), .admin-dialog-body button:not([disabled])') || adminDialog.querySelector('.admin-dialog-header button');
    firstControl?.focus();
  });
}

adminDialog.addEventListener('close', () => {
  if (dialogReturnFocus instanceof HTMLElement) dialogReturnFocus.focus();
  dialogReturnFocus = null;
});

function humanLabel(value) {
  return String(value ?? '').replaceAll('_', ' ').toLowerCase().replace(/(^|\s)\S/g, (letter) => letter.toUpperCase());
}

function adminReviewRoundLabel(reviewOpen, roundStatus) {
  return reviewOpen || roundStatus === 'OPEN_FOR_REVIEW' ? 'In review' : 'Review closed';
}

function statusChip(label, tone = '') {
  return element('span', `status-chip ${tone}`.trim(), label);
}

const iconPaths = {
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  close: 'M6 6l12 12M18 6L6 18',
  delete: 'M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5',
  details: 'M12 9h.01M11 12h1v4h1m8-4a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z',
};

function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('width', '16'); svg.setAttribute('height', '16'); svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', iconPaths[name]); path.setAttribute('fill', 'none'); path.setAttribute('stroke', 'currentColor'); path.setAttribute('stroke-width', '1.8'); path.setAttribute('stroke-linecap', 'round'); path.setAttribute('stroke-linejoin', 'round');
  svg.append(path); return svg;
}

function iconAction(label, iconName, handler, className = '') {
  const action = buttonAction('', handler, `icon-button ${className}`.trim());
  action.append(icon(iconName)); action.setAttribute('aria-label', label); action.title = label; return action;
}

function rowMenu(label, actions) {
  const menu = element('details', 'admin-row-menu');
  const summary = element('summary'); summary.append(icon('more'));
  summary.setAttribute('aria-label', label);
  summary.title = label;
  const items = element('div', 'admin-row-menu-items');
  actions.filter(Boolean).forEach((action) => items.append(action));
  menu.append(summary, items);
  return menu;
}

// Keep the legacy first three columns and append Team presentation metadata.
function csvCell(value) {
  const text = String(value ?? '');
  // Prefix untrusted spreadsheet formulas; ordinary credentials are unchanged.
  const safe = /^[=+@\-\t\r\n]/.test(text.trimStart()) ? `'${text}` : text;
  return /[",\n\r]/.test(safe) ? `"${safe.replaceAll('"', '""')}"` : safe;
}

function credentialsCsv(credentials) {
  if (credentials.some(item => !Array.isArray(item.teams))) {
    // Manual account handoffs do not carry an import's membership scope.
    return ['email,password,role', ...credentials.map(item => [item.email, item.temporary_password, item.credential_role].map(csvCell).join(','))].join('\r\n') + '\r\n';
  }
  const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
  const rows = credentials.filter(item => typeof item.temporary_password === 'string' && item.temporary_password.length).flatMap(item =>
    (item.teams?.length ? item.teams : [{ team_name: 'Unassigned', team_id: '', team_key: '', team_role: '', writer_order: null, is_leader: false }])
      .map(team => ({ item, team })));
  const rank = team => team.is_leader ? 0 : team.team_role === 'writer' ? 1 : team.team_role === 'mentor' ? 2 : 3;
  rows.sort((a, b) => compare(a.team.team_name, b.team.team_name) || compare(a.team.team_id, b.team.team_id)
    || rank(a.team) - rank(b.team) || (a.team.writer_order ?? Number.MAX_SAFE_INTEGER) - (b.team.writer_order ?? Number.MAX_SAFE_INTEGER)
    || compare(a.item.email, b.item.email));
  return ['email,password,role,team_name,team_id,team_key,team_role,writer_order,is_team_leader', ...rows.map(({item, team}) =>
    [item.email, item.temporary_password, item.credential_role, team.team_name, team.team_id, team.team_key, team.team_role, team.writer_order, team.is_leader].map(csvCell).join(','))].join('\r\n') + '\r\n';
}

function downloadCredentials(credentials, batchId) {
  // Team-scoped handoffs download one ZIP: all-Teams CSV/XLSX, a Team-grouped JSON and one CSV per Team.
  if (credentials.length && credentials.every(item => Array.isArray(item.teams))) {
    downloadBlob(new Blob([credentialBundle(credentials, batchId)], { type: 'application/zip' }), `latex-core-credentials-${batchId}.zip`);
    return;
  }
  downloadBlob(new Blob([credentialsCsv(credentials)], { type: 'text/csv;charset=utf-8' }), `latex-core-generated-credentials-${batchId}.csv`);
}

function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob); const link = document.createElement('a');
  link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// Dependency-free ZIP (stored entries) and XLSX writers for Admin exports.
const crcTable = (() => { const table = new Uint32Array(256); for (let n = 0; n < 256; n += 1) { let c = n; for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1; table[n] = c >>> 0; } return table; })();
function crc32(bytes) { let crc = 0xFFFFFFFF; for (let index = 0; index < bytes.length; index += 1) crc = crcTable[(crc ^ bytes[index]) & 0xFF] ^ (crc >>> 8); return (crc ^ 0xFFFFFFFF) >>> 0; }
function zipBytes(entries, now = new Date()) {
  const encoder = new TextEncoder(); const parts = []; const central = []; let offset = 0;
  const time = (now.getHours() << 11) | (now.getMinutes() << 5) | Math.floor(now.getSeconds() / 2);
  const date = ((Math.max(now.getFullYear(), 1980) - 1980) << 9) | ((now.getMonth() + 1) << 5) | now.getDate();
  for (const entry of entries) {
    const name = encoder.encode(entry.name); const data = typeof entry.data === 'string' ? encoder.encode(entry.data) : entry.data; const crc = crc32(data);
    const local = new DataView(new ArrayBuffer(30));
    [[0, 0x04034b50, 4], [4, 20, 2], [6, 0x0800, 2], [8, 0, 2], [10, time, 2], [12, date, 2], [14, crc, 4], [18, data.length, 4], [22, data.length, 4], [26, name.length, 2], [28, 0, 2]].forEach(([at, value, size]) => size === 4 ? local.setUint32(at, value, true) : local.setUint16(at, value, true));
    const header = new DataView(new ArrayBuffer(46));
    [[0, 0x02014b50, 4], [4, 20, 2], [6, 20, 2], [8, 0x0800, 2], [10, 0, 2], [12, time, 2], [14, date, 2], [16, crc, 4], [20, data.length, 4], [24, data.length, 4], [28, name.length, 2], [30, 0, 2], [32, 0, 2], [34, 0, 2], [36, 0, 2], [38, 0, 4], [42, offset, 4]].forEach(([at, value, size]) => size === 4 ? header.setUint32(at, value, true) : header.setUint16(at, value, true));
    parts.push(new Uint8Array(local.buffer), name, data); central.push(new Uint8Array(header.buffer), name);
    offset += 30 + name.length + data.length;
  }
  const centralSize = central.reduce((sum, part) => sum + part.length, 0); const end = new DataView(new ArrayBuffer(22));
  [[0, 0x06054b50, 4], [4, 0, 2], [6, 0, 2], [8, entries.length, 2], [10, entries.length, 2], [12, centralSize, 4], [16, offset, 4], [20, 0, 2]].forEach(([at, value, size]) => size === 4 ? end.setUint32(at, value, true) : end.setUint16(at, value, true));
  const output = new Uint8Array(offset + centralSize + 22); let cursor = 0;
  [...parts, ...central, new Uint8Array(end.buffer)].forEach((part) => { output.set(part, cursor); cursor += part.length; });
  return output;
}
function xmlText(value) {
  return String(value ?? '').replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]/g, '').replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
}
function columnName(index) { let name = ''; for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) name = String.fromCharCode(65 + ((n - 1) % 26)) + name; return name; }
function sheetNames(sheets) {
  const used = new Set();
  return sheets.map((sheet, index) => {
    const base = String(sheet.name || `Sheet${index + 1}`).replace(/[\[\]:*?/\\]/g, ' ').replace(/^'+|'+$/g, '').trim().slice(0, 31) || `Sheet${index + 1}`;
    let name = base; for (let suffix = 2; used.has(name.toLowerCase()); suffix += 1) name = `${base.slice(0, 31 - String(suffix).length - 1)}~${suffix}`;
    used.add(name.toLowerCase()); return name;
  });
}
// Every cell is an inline string, so registration numbers keep leading zeros and no value is evaluated as a formula.
function xlsxBytes(sheets) {
  const names = sheetNames(sheets); const ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'; const rel = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
  const worksheet = (rows) => `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="${ns}"><sheetViews><sheetView workbookViewId="0">${rows.length > 1 ? '<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>' : ''}</sheetView></sheetViews><sheetData>${rows.map((row, rowIndex) => `<row r="${rowIndex + 1}">${row.map((value, columnIndex) => `<c r="${columnName(columnIndex)}${rowIndex + 1}" t="inlineStr"${rowIndex === 0 ? ' s="1"' : ''}><is><t xml:space="preserve">${xmlText(value)}</t></is></c>`).join('')}</row>`).join('')}</sheetData></worksheet>`;
  const entries = [
    { name: '[Content_Types].xml', data: `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>${names.map((_, index) => `<Override PartName="/xl/worksheets/sheet${index + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join('')}</Types>` },
    { name: '_rels/.rels', data: `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>` },
    { name: 'xl/workbook.xml', data: `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="${ns}" xmlns:r="${rel}"><sheets>${names.map((name, index) => `<sheet name="${xmlText(name)}" sheetId="${index + 1}" r:id="rId${index + 1}"/>`).join('')}</sheets></workbook>` },
    { name: 'xl/_rels/workbook.xml.rels', data: `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${names.map((_, index) => `<Relationship Id="rId${index + 1}" Type="${rel}/worksheet" Target="worksheets/sheet${index + 1}.xml"/>`).join('')}<Relationship Id="rId${names.length + 1}" Type="${rel}/styles" Target="styles.xml"/></Relationships>` },
    { name: 'xl/styles.xml', data: `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="${ns}"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>` },
    ...sheets.map((sheet, index) => ({ name: `xl/worksheets/sheet${index + 1}.xml`, data: worksheet(sheet.rows) })),
  ];
  return zipBytes(entries);
}

const credentialColumns = ['email', 'password', 'role', 'team_name', 'team_id', 'team_key', 'team_role', 'writer_order', 'is_team_leader'];
// Groups one-time credentials by Team in the same deterministic order as the CSV.
function credentialsByTeam(credentials) {
  const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0; const rank = team => team.is_leader ? 0 : team.team_role === 'writer' ? 1 : team.team_role === 'mentor' ? 2 : 3;
  const groups = new Map();
  credentials.filter(item => typeof item.temporary_password === 'string' && item.temporary_password.length).forEach((item) => {
    (item.teams?.length ? item.teams : [{ team_name: 'Unassigned', team_id: '', team_key: '', team_role: '', writer_order: null, is_leader: false }]).forEach((team) => {
      const key = `${team.team_name}\u0000${team.team_id}`;
      if (!groups.has(key)) groups.set(key, { team_name: team.team_name, team_id: team.team_id || null, team_key: team.team_key || null, members: [] });
      groups.get(key).members.push({ email: item.email, login_id: item.email, password: item.temporary_password, role: item.credential_role, team_role: team.team_role || null, writer_order: team.writer_order ?? null, is_team_leader: Boolean(team.is_leader) });
    });
  });
  const teams = [...groups.values()].sort((a, b) => compare(a.team_name, b.team_name) || compare(a.team_id || '', b.team_id || ''));
  teams.forEach((team) => team.members.sort((a, b) => rank({ is_leader: a.is_team_leader, team_role: a.team_role }) - rank({ is_leader: b.is_team_leader, team_role: b.team_role }) || (a.writer_order ?? Number.MAX_SAFE_INTEGER) - (b.writer_order ?? Number.MAX_SAFE_INTEGER) || compare(a.email, b.email)));
  return teams;
}

function credentialBundle(credentials, batchId, now = new Date()) {
  const teams = credentialsByTeam(credentials);
  const rows = teams.flatMap((team) => team.members.map((member) => [member.email, member.password, member.role, team.team_name, team.team_id ?? '', team.team_key ?? '', member.team_role ?? '', member.writer_order ?? '', member.is_team_leader]));
  const json = JSON.stringify({ schema_version: 1, generated_at: now.toISOString(), handoff: String(batchId), warning: 'Administrator-only one-time temporary passwords. Users must change them at first sign-in.', team_count: teams.length, teams }, null, 2);
  const safeName = (value) => String(value || 'Team').replace(/[^A-Za-z0-9._ -]+/g, '_').replace(/\s+/g, ' ').trim().slice(0, 80) || 'Team';
  const teamFiles = teams.map((team, index) => ({ name: `teams/${String(index + 1).padStart(3, '0')} ${safeName(team.team_name)}.csv`, data: [credentialColumns.join(','), ...team.members.map((member) => [member.email, member.password, member.role, team.team_name, team.team_id ?? '', team.team_key ?? '', member.team_role ?? '', member.writer_order ?? '', member.is_team_leader].map(csvCell).join(','))].join('\r\n') + '\r\n' }));
  return zipBytes([
    { name: 'credentials-all-teams.csv', data: credentialsCsv(credentials) },
    { name: 'credentials-all-teams.xlsx', data: xlsxBytes([{ name: 'All Teams', rows: [credentialColumns, ...rows] }]) },
    { name: 'credentials-by-team.json', data: json },
    ...teamFiles,
  ], now);
}

async function fetchAllPages(path, limit = 100, progress = () => {}) {
  const items = []; const separator = path.includes('?') ? '&' : '?';
  for (let page = 1; page <= 10000; page += 1) {
    const data = await api(`${path}${separator}${queryString({ page, limit })}`);
    items.push(...(data.items || [])); progress(items.length, data.total);
    if (!data.has_more || !(data.items || []).length) break;
  }
  return items;
}

function exportStamp(now = new Date()) { return now.toISOString().slice(0, 19).replace(/[:T]/g, '-'); }

// One worksheet per import dataset, named and ordered exactly like the import format, so the file can be re-imported.
async function exportInstitutionData() {
  const sheets = []; const writers = []; const mentors = [];
  for (const config of institutionDatasets) {
    const items = await fetchAllPages(`/api/admin/v2/institution/data/${config.dataset}`, 100, (count, total) => announce(`Exporting ${config.label}: ${count.toLocaleString()} of ${Number(total || 0).toLocaleString()}…`));
    const columns = targetColumns[config.dataset];
    sheets.push({ name: config.dataset, rows: [columns, ...items.map((item) => columns.map((column) => item[column] ?? ''))] });
    if (config.dataset === 'paper_teams') items.forEach((item) => { (item.writers || []).forEach((writer) => writers.push([item.external_team_key, writer.student_reg_no, writer.writer_order ?? '', Boolean(writer.is_leader)])); (item.mentors || []).forEach((mentor) => mentors.push([item.external_team_key, mentor.faculty_id])); });
  }
  sheets.push({ name: 'paper_team_writers', rows: [targetColumns.paper_team_writers, ...writers] }, { name: 'paper_team_mentors', rows: [targetColumns.paper_team_mentors, ...mentors] });
  downloadBlob(new Blob([xlsxBytes(sheets)], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' }), `latex-core-institution-data-${exportStamp()}.xlsx`);
  announce(`Institution data exported: ${sheets.reduce((sum, sheet) => sum + sheet.rows.length - 1, 0).toLocaleString()} records in ${sheets.length} sheets.`);
}

async function exportPaperTeams() {
  const teams = new Map();
  (await fetchAllPages('/api/admin/v2/paper-teams/query', 200, (count, total) => announce(`Exporting Paper Teams: ${count.toLocaleString()} of ${Number(total || 0).toLocaleString()}…`))).forEach((team) => teams.set(team.id, team));
  const unresolved = await fetchAllPages('/api/admin/v2/paper-teams/query?unresolved=true', 200).catch(() => []);
  const people = new Map();
  const remember = (email, values) => { if (email) people.set(String(email).trim().toLowerCase(), values); };
  try {
    announce('Exporting Paper Teams: matching institutional names and IDs…');
    (await fetchAllPages('/api/admin/v2/institution/data/students')).forEach((student) => { const values = { id: student.reg_no, name: student.name }; remember(student.email, values); remember(student.v2_account, values); });
    (await fetchAllPages('/api/admin/v2/institution/data/faculty')).forEach((faculty) => { const values = { id: faculty.faculty_id, name: faculty.name }; remember(faculty.email, values); remember(faculty.v2_account, values); });
  } catch (error) { showError(error); }
  const who = (email) => people.get(String(email || '').trim().toLowerCase()) || {};
  const ordered = [...teams.values()].sort((a, b) => String(a.name).localeCompare(String(b.name)) || String(a.id).localeCompare(String(b.id)));
  const teamRows = [['team_name', 'team_status', 'external_team_key', 'source', 'programme', 'main_template', 'review_state', 'leader_email', 'leader_name', 'leader_id', 'writers', 'mentors', 'writer_count', 'mentor_count', 'updated_at', 'team_id']];
  const memberRows = [['team_name', 'external_team_key', 'team_status', 'role', 'writer_order', 'is_team_leader', 'email', 'name', 'institution_id', 'team_id', 'user_id']];
  ordered.forEach((team) => {
    const leader = who(team.leader?.email);
    teamRows.push([team.name, team.status, team.external_team_key || '', team.source, team.dominant_programme_code || '', team.template?.name || '', team.review_state, team.leader?.email || '', leader.name || '', leader.id || '', (team.writers || []).map((writer) => `${writer.writer_order}. ${writer.email}`).join('; '), (team.mentors || []).map((mentor) => mentor.email).join('; '), team.writer_count ?? (team.writers || []).length, (team.mentors || []).length, team.updated_at, team.id]);
    (team.writers || []).forEach((writer) => { const person = who(writer.email); memberRows.push([team.name, team.external_team_key || '', team.status, 'writer', writer.writer_order ?? '', team.leader?.user_id === writer.user_id, writer.email, person.name || '', person.id || '', team.id, writer.user_id]); });
    (team.mentors || []).forEach((mentor) => { const person = who(mentor.email); memberRows.push([team.name, team.external_team_key || '', team.status, 'mentor', '', false, mentor.email, person.name || '', person.id || '', team.id, mentor.user_id]); });
  });
  const sheets = [{ name: 'Paper Teams', rows: teamRows }, { name: 'Members', rows: memberRows }];
  if (unresolved.length) sheets.push({ name: 'Unresolved imports', rows: [['team_name', 'external_team_key', 'status', 'writer_count', 'unresolved_student_ids', 'unresolved_faculty_ids', 'error_code', 'error_message', 'source_import_job_id'], ...unresolved.map((team) => [team.name, team.external_team_key || '', team.status || '', team.writer_count ?? '', (team.unresolved_student_ids || []).join('; '), (team.unresolved_faculty_ids || []).join('; '), team.error_code || '', team.error_message || '', team.source_import_job_id || ''])] });
  downloadBlob(new Blob([xlsxBytes(sheets)], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' }), `latex-core-paper-teams-${exportStamp()}.xlsx`);
  announce(`Paper Teams exported: ${ordered.length.toLocaleString()} Teams, ${(memberRows.length - 1).toLocaleString()} memberships${unresolved.length ? `, ${unresolved.length} unresolved imports` : ''}.`);
}

// Issues new one-time passwords for Team members through the per-account Admin endpoint and hands them off Team-wise.
async function resetTeamPasswords(teams) {
  const members = teams.flatMap((team) => [...(team.writers || []).map((writer) => ({ ...writer, role: 'writer' })), ...(team.mentors || []).map((mentor) => ({ ...mentor, role: 'mentor' }))].map((member) => ({ team, member })));
  if (!members.length) throw new Error('The selected Teams have no members.');
  const accounts = new Set(members.map(({ member }) => member.user_id));
  if (!confirm(`Generate new temporary passwords for ${accounts.size} account${accounts.size === 1 ? '' : 's'} in ${teams.length} Team${teams.length === 1 ? '' : 's'}? Their current passwords and sessions stop working.`)) return;
  const issued = new Map(); let queued = 0; let failed = 0;
  for (const userId of accounts) {
    try { const reset = await api(`/api/admin/v2/users/${encodeURIComponent(userId)}/temporary-password`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); issued.set(userId, reset); if (reset.credential_email_queued) queued += 1; }
    catch (error) { failed += 1; showError(error); }
    announce(`Resetting Team passwords: ${issued.size + failed} of ${accounts.size}…`);
  }
  const credentials = new Map();
  members.forEach(({ team, member }) => {
    const reset = issued.get(member.user_id); if (!reset) return;
    if (!credentials.has(member.user_id)) credentials.set(member.user_id, { email: reset.email, temporary_password: reset.temporary_password, credential_role: member.role === 'writer' ? 'student' : 'mentor', teams: [] });
    credentials.get(member.user_id).teams.push({ team_name: team.name, team_id: team.id, team_key: team.external_team_key || '', team_role: member.role, writer_order: member.role === 'writer' ? member.writer_order ?? null : null, is_leader: member.role === 'writer' && (member.is_leader ?? team.leader?.user_id === member.user_id) });
  });
  showCredentialHandoff({ created: credentials.size, credential_emails_queued: queued, reused: 0, needs_attention: failed, credentials: [...credentials.values()] }, `team-reset-${exportStamp()}`, { title: 'Team password reset', createdLabel: 'temporary passwords generated' });
}

function showCredentialHandoff(provisioning, batchId, options = {}) {
  if (!provisioning) return;
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody');
  const credentials = provisioning.credentials || [];
  const grouped = credentials.every(item => Array.isArray(item.teams));
  host.replaceChildren(
    element('h2', '', options.title || 'Account setup'),
    element('p', '', `${provisioning.created || 0} ${options.createdLabel || 'accounts created'}`),
    element('p', '', `${provisioning.credential_emails_queued || 0} credential emails queued`),
    element('p', '', `${provisioning.reused || 0} existing accounts reused`),
    element('p', '', `${provisioning.needs_attention || 0} need attention`),
    element('p', 'muted-note', 'Email is the primary credential delivery method. The download is an administrator fallback.'),
    element('p', 'danger-box', 'Save this file now. Temporary passwords cannot be viewed again. Administrator-only: do not distribute the all-Teams files to students; hand each Team only its own file from the teams folder.'),
    element('p', 'muted-note', grouped ? 'ZIP contents: all-Teams CSV and Excel, a Team-grouped JSON, and one CSV per Team, sorted Team-wise (Leader, Writers in order, then Mentors). Accounts without a current in-scope assignment appear as Unassigned; later assignments cannot update this one-time export.' : 'Team assignments are unavailable for this account handoff. This download retains the original account-only CSV format.'),
  );
  const download = buttonAction(grouped && credentials.length ? 'Download Team-wise credentials (ZIP)' : 'Download generated credentials', () => downloadCredentials(credentials, batchId), 'primary');
  download.disabled = credentials.length === 0; host.append(download); openAdminDialog(options.title || 'Account setup');
  if (credentials.length) downloadCredentials(credentials, batchId);
}

function setActive(section) {
  nav.querySelectorAll('button[data-section]').forEach((button) => {
    if (button.dataset.section === section) button.setAttribute('aria-current', 'page');
    else button.removeAttribute('aria-current');
  });
  sectionTitle.textContent = section;
}

function renderJson(value) {
  const pre = element('pre', 'admin-data');
  pre.textContent = JSON.stringify(value, null, 2);
  return pre;
}

async function renderRuntimeLogs() {
  const controls = element('div', 'admin-toolbar');
  const service = document.createElement('select'); service.setAttribute('aria-label', 'Service');
  const search = document.createElement('input'); search.type = 'search'; search.placeholder = 'Search retained logs'; search.setAttribute('aria-label', 'Search available retained container logs');
  const refresh = buttonAction('Refresh', () => load());
  const host = element('div');
  controls.append(element('label', '', 'Service'), service, search, refresh);
  content.append(element('p', 'muted-note', 'Search available retained container logs. Docker retention may exclude older records.'), controls, host);
  const load = async () => {
    refresh.disabled = true;
    try {
      const params = new URLSearchParams({ limit: '100' });
      if (service.value) params.set('service', service.value);
      if (search.value.trim()) params.set('query', search.value.trim());
      const data = await api(`/api/admin/v2/runtime-logs?${params}`);
      const services = [...new Set(data.services || (data.records || []).map((record) => record.service))].sort();
      const selected = service.value;
      service.replaceChildren(new Option('All services', '')); services.forEach((name) => service.append(new Option(name, name)));
      service.value = services.includes(selected) ? selected : '';
      if (!data.records?.length) { host.replaceChildren(element('p', 'empty-copy', data.source_status === 'empty' ? 'No retained log records matched.' : 'The runtime log source returned no records.')); return; }
      const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table');
      table.innerHTML = '<thead><tr><th>Timestamp</th><th>Service / container</th><th>Message</th></tr></thead>';
      const body = document.createElement('tbody');
      data.records.forEach((record) => { const row = document.createElement('tr'); const message = element('pre', 'admin-data', record.message || ''); message.className = 'admin-log-message'; const messageCell = element('td'); messageCell.append(message); row.append(element('td', '', record.timestamp), element('td', '', `${record.service} · ${record.container || 'container'}`), messageCell); body.append(row); });
      table.append(body); wrap.append(table); host.replaceChildren(wrap);
    } catch (error) {
      const message = /permission denied/i.test(error.message) ? 'Runtime log source permission denied.'
        : /source unavailable|command unavailable/i.test(error.message) ? 'Runtime log source unavailable.'
          : `Runtime log request failed: ${error.message}`;
      const alert = element('p', 'danger', message); alert.setAttribute('role', 'alert'); host.replaceChildren(alert);
    } finally { refresh.disabled = false; }
  };
  service.addEventListener('change', () => load().catch(showError)); search.addEventListener('keydown', (event) => { if (event.key === 'Enter') load().catch(showError); });
  await load();
}

function renderOverview(data) {
  const metrics = [
    ['active_paper_teams', 'Paper Teams'], ['teams_in_review', 'Teams in Review'],
    ['writers', 'Writers'], ['mentors', 'Mentors'],
    ['institutional_students', 'Institutional Students'], ['institutional_faculty', 'Institutional Faculty'],
    ['unlinked_total', 'Unlinked / needs attention'], ['email_attention', 'Pending / failed emails'],
    ['build_activity', 'Queued / running builds'],
  ];
  const values = {
    ...data,
    unlinked_total: Number(data.unlinked_students || 0) + Number(data.unlinked_faculty || 0) + Number(data.unresolved_imported_teams || 0),
    email_attention: `${Number(data.pending_email_count || 0).toLocaleString()} / ${Number(data.failed_email_count || 0).toLocaleString()}`,
    build_activity: `${Number(data.queued_builds || 0).toLocaleString()} / ${Number(data.running_builds || 0).toLocaleString()}`,
  };
  const grid = element('div', 'admin-metrics');
  metrics.forEach(([key, label]) => {
    const metric = element('div', 'admin-metric');
    const value = values[key] ?? 0;
    metric.append(element('strong', '', typeof value === 'number' ? value.toLocaleString() : String(value)), element('span', '', label));
    grid.append(metric);
  });
  content.append(grid);

  const attention = [];
  if (Number(data.unresolved_imported_teams || 0)) attention.push([`${data.unresolved_imported_teams} imported Teams need resolution`, 'Paper Teams']);
  if (Number(data.failed_email_count || 0)) attention.push([`${data.failed_email_count} credential emails failed`, 'V2 Users']);
  if (String(data.latest_import_status || '').match(/FAIL|PARTIAL|ERROR/i)) attention.push([`Latest import: ${humanLabel(data.latest_import_status)}`, 'Imports']);
  if (attention.length) {
    content.append(element('h2', '', 'Needs attention'));
    const list = element('ul', 'admin-attention');
    attention.forEach(([label, section]) => {
      const action = buttonAction(label, () => showSection(section));
      action.append(element('span', '', '→'));
      const item = element('li'); item.append(action); list.append(item);
    });
    content.append(list);
  }
}

function renderAdminReviews(reviews) {
  if (!reviews.length) return content.append(element('p', 'empty-copy', 'No Team reviews have been submitted yet.'));
  const grouped = new Map();
  reviews.forEach((review) => {
    const item = grouped.get(review.paper_id) || { ...review, open: 0, blocking: 0, last_activity: review.updated_at || review.created_at };
    const belongsToCurrentRound = review.review_round_id === review.current_review_round_id;
    if (belongsToCurrentRound && review.state !== 'RESOLVED') item.open += 1;
    if (belongsToCurrentRound && review.severity === 'BLOCKING' && review.state !== 'RESOLVED') item.blocking += 1;
    if (new Date(review.updated_at || review.created_at) > new Date(item.last_activity)) item.last_activity = review.updated_at || review.created_at;
    grouped.set(review.paper_id, item);
  });
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Team</th><th>Review state</th><th>Mentor</th><th>Open comments</th><th>Blocking comments</th><th>Submitted</th><th>Last activity</th></tr></thead>';
  const body = document.createElement('tbody');
  grouped.forEach((review) => {
    const row = document.createElement('tr');
    row.append(
      element('td', '', review.paper_name), element('td', '', adminReviewRoundLabel(review.review_open, review.round_status)),
      element('td', '', review.mentor), element('td', '', String(review.open)), element('td', '', String(review.blocking)),
      element('td', '', new Date(review.created_at).toLocaleString()), element('td', '', new Date(review.last_activity).toLocaleString()),
    );
    body.append(row);
  });
  table.append(body); wrap.append(table); content.append(wrap);
}

function renderVersions(versions) {
  if (!versions.length) return content.append(element('p', 'empty-copy', 'No Team versions have been recorded yet.'));
  const host = element('div');
  const filters = filterForm([
    { name: 'search', label: 'Team or actor', type: 'search', placeholder: 'Filter history…' },
    { name: 'version_type', label: 'Version type', type: 'select', options: [...new Set(versions.map((item) => item.version_type))].map((value) => [humanLabel(value), value]) },
  ], ({ search, version_type }) => draw(search, version_type));
  const draw = (search = '', versionType = '') => {
    const visible = versions.filter((item) => (!versionType || item.version_type === versionType) && (!search || `${item.paper_name} ${item.author}`.toLowerCase().includes(search.toLowerCase())));
    if (!visible.length) { host.replaceChildren(element('p', 'empty-copy', 'No Team versions match these filters.')); return; }
    const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table');
    table.innerHTML = '<thead><tr><th>Team</th><th>Version type</th><th>Actor</th><th>Created</th></tr></thead>';
    const body = document.createElement('tbody'); visible.forEach((item) => { const row = element('tr'); row.append(element('td', '', item.paper_name), element('td', '', humanLabel(item.version_type)), element('td', '', item.author), element('td', '', new Date(item.created_at).toLocaleString())); body.append(row); });
    table.append(body); wrap.append(table); host.replaceChildren(wrap);
  };
  content.append(element('p', 'muted-note', 'Inspection only. Team version and revert authority remains with the Writer Leader.'), filters, host); draw();
}

function renderBuildQueue(jobs) {
  const host = element('div');
  const filters = filterForm([{ name: 'state', label: 'State', type: 'select', options: [['Queued', 'queued'], ['Running', 'running'], ['Failed', 'failed'], ['Succeeded', 'succeeded']] }], ({ state }) => draw(state));
  const draw = (state = '') => {
    const visible = jobs.filter((job) => !state || job.state === state || (state === 'running' && job.state === 'claimed'));
    if (!visible.length) { host.replaceChildren(element('p', 'empty-copy', state ? `No ${state} builds.` : 'No builds have been queued yet.')); return; }
    const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Team / workspace</th><th>Trigger</th><th>State</th><th>Queued</th><th>Started</th><th>Duration</th></tr></thead>';
    const body = document.createElement('tbody'); visible.forEach((job) => { const row = element('tr'); const start = job.started_at ? new Date(job.started_at) : null; const end = job.finished_at ? new Date(job.finished_at) : new Date(); const duration = start ? `${Math.max(0, Math.round((end - start) / 1000))}s` : '—'; row.append(element('td', '', job.workspace_id), element('td', '', job.requester || 'System'), element('td', '', humanLabel(job.state)), element('td', '', new Date(job.created_at).toLocaleString()), element('td', '', start ? start.toLocaleString() : '—'), element('td', '', duration)); body.append(row); }); table.append(body); wrap.append(table); host.replaceChildren(wrap);
  };
  content.append(filters, host); draw();
}

const auditLabels = {
  'review.round.opened': 'Send for Review',
  'review.feedback.published': 'Review feedback published',
  'account.credential_email.sent': 'Credential email sent',
  'paper_team.member.changed': 'Team membership changed',
};

function renderAudit(events) {
  content.append(element('p', '', 'Audit records important administrative and security actions: who did what, when, and to which object.'));
  if (!events.length) return content.append(element('p', 'empty-copy', 'No administrative or security actions have been recorded yet.'));
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Time</th><th>Actor</th><th>Action</th><th>Target</th><th>Details</th></tr></thead>';
  const body = document.createElement('tbody'); events.forEach((event) => { const row = element('tr'); const details = element('details'); details.append(element('summary', '', 'View'), element('p', 'muted-note', `Event type: ${event.event}`), element('p', 'muted-note', `Object ID: ${event.resource_id || 'Not recorded'}`)); if (event.paper_id) details.append(element('p', 'muted-note', `Paper/Team ID: ${event.paper_id}`), element('p', 'muted-note', `Round ID: ${event.round_id}`), element('p', 'muted-note', `Outcome: ${event.outcome}`), element('p', 'muted-note', `Actor ID: ${event.actor_user_id}`)); const detailCell = element('td'); detailCell.append(details); row.append(element('td', '', new Date(event.created_at).toLocaleString()), element('td', '', event.actor || 'System'), element('td', '', auditLabels[event.event] || humanLabel(event.event.replaceAll('.', ' '))), element('td', '', humanLabel(event.resource)), detailCell); body.append(row); });
  table.append(body); wrap.append(table); content.append(wrap);
}

function renderSystem(data, overview, branding) {
  content.append(element('p', 'muted-note', 'Read-only operational summary. Administrative actions remain in the operator service.'));
  const support = element('form', 'branding-form');
  const supportLabel = document.createElement('label'); supportLabel.textContent = 'Support email';
  const supportEmail = document.createElement('input'); supportEmail.type = 'email'; supportEmail.name = 'support_email'; supportEmail.maxLength = 254;
  supportLabel.append(supportEmail); const supportSave = document.createElement('button'); supportSave.type = 'submit'; supportSave.textContent = 'Save support email';
  support.append(supportLabel, supportSave); content.append(support);
  api('/api/admin/v2/institution/support').then((config) => { supportEmail.value = config.support_email || ''; }).catch(showError);
  support.addEventListener('submit', async (event) => { event.preventDefault(); try { await api('/api/admin/v2/institution/support', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ support_email: supportEmail.value || null }) }); announce('Support email saved.'); } catch (error) { showError(error); } });
  const systems = [
    ['API', 'Healthy', data.version],
    ['PostgreSQL', 'Healthy', data.database],
    ['Blob storage', 'Healthy', 'Content-addressed storage configured'],
    ['Worker', 'Healthy', data.queue],
    ['Compiler M7', 'Healthy', data.compiler],
    ['Mail delivery', overview.mail_delivery_enabled ? 'Healthy' : 'Disabled', overview.mail_delivery_enabled ? 'Credential delivery enabled' : 'Disabled in this environment'],
  ];
  const grid = element('div', 'system-grid'); systems.forEach(([name, state, detail]) => { const card = element('article', 'system-card'); card.append(element('strong', '', name), statusChip(state, state === 'Healthy' ? 'good' : state === 'Disabled' ? '' : 'warning'), element('span', '', detail)); grid.append(card); }); content.append(grid);
  const backup = data.backup || {};
  const recovery = element('section', 'admin-section');
  const recoveryDetails = element('dl', 'detail-list');
  recoveryDetails.append(
    element('dt', '', 'Last successful backup'), element('dd', '', backup.last_successful_backup_at ? new Date(backup.last_successful_backup_at).toLocaleString() : 'No successful backup recorded'),
    element('dt', '', 'Last backup failure'), element('dd', '', backup.last_failure_at ? `${new Date(backup.last_failure_at).toLocaleString()} · ${backup.last_failure_phase || 'unknown phase'}` : 'No failure recorded'),
    element('dt', '', 'Schedule'), element('dd', '', backup.schedule_configured ? 'Configured' : 'Not configured'),
    element('dt', '', 'Off-host copy'), element('dd', '', backup.off_host_copy_configured ? 'Configured' : 'Not configured'),
    element('dt', '', 'Last successful restore drill'), element('dd', '', backup.last_successful_restore_drill_at ? new Date(backup.last_successful_restore_drill_at).toLocaleString() : 'No successful restore drill recorded'),
  );
  recovery.append(
    element('h2', '', 'Backup and recovery'),
    element('p', 'muted-note', 'Read-only operator status. Backup contents, keys, and restore actions are never exposed here.'),
    recoveryDetails,
  );
  content.append(recovery);
  const section = element('section', 'admin-section branding-settings');
  section.append(element('h2', '', 'Branding'), element('p', 'muted-note', 'Application header only. PNG, JPEG, or WebP; maximum 512 KiB and 2048 × 2048 px. PDF and Front Matter logos are unchanged.'));
  const preview = element('div', 'branding-preview');
  if (branding.logo_url) { const image = document.createElement('img'); image.src = `${branding.logo_url}?v=${Date.now()}`; image.alt = 'Current institution logo'; preview.append(image, element('span', '', `${branding.width} × ${branding.height} · ${branding.media_type}`)); }
  else preview.append(element('strong', '', 'LaTeX Core'), element('span', '', 'Default wordmark'));
  const form = element('form', 'branding-form'); const input = document.createElement('input'); input.type = 'file'; input.accept = 'image/png,image/jpeg,image/webp'; input.required = true; input.setAttribute('aria-label', 'Upload institution logo');
  const upload = buttonAction(branding.logo_url ? 'Replace logo' : 'Upload logo', () => {}, 'primary'); upload.type = 'submit';
  const remove = buttonAction('Remove / reset to default', async () => { if (!confirm('Remove the application logo and restore the LaTeX Core wordmark?')) return; await api('/api/admin/v2/branding', { method: 'DELETE' }); announce('Application logo reset to default.'); await showSection('System'); }); remove.disabled = !branding.logo_url;
  form.append(input, upload, remove); form.addEventListener('submit', async (event) => { event.preventDefault(); try { const file = input.files[0]; if (!file) return; if (file.size > 512 * 1024) throw new Error('Logo exceeds the 512 KiB limit.'); await api('/api/admin/v2/branding', { method: 'POST', headers: { 'Content-Type': file.type || 'application/octet-stream' }, body: file }); announce('Application logo saved.'); await showSection('System'); } catch (error) { showError(error); } });
  section.append(preview, form); content.append(section);
}

async function renderTemplates(templates, frontMatterPacks) {
  const heading = element('h2', 'admin-section-anchor', 'Complete report templates'); heading.id = 'main-templates'; content.append(heading);
  const intro = element('p', 'empty-copy', 'Import one complete report ZIP when the selected source controls cover, front matter, and body. Existing separate-file Front Matter support remains available for older templates.');
  const form = element('form', 'admin-template-form');
  form.innerHTML = '<label>Name<input name="name" required maxlength="200"></label><label>Description (optional)<input name="description" maxlength="2000"></label><label>Template type<select name="arrangement"><option value="SINGLE_SOURCE">Complete report template (recommended)</option><option value="REPORT_CONTENT_ONLY">Report content only</option><option value="SEPARATE_FILES">Legacy separate Front Matter files</option></select></label><label>Template ZIP<input name="archive" type="file" accept=".zip,application/zip" required></label><label>Main .tex file<select name="main" required disabled><option value="">Validate a ZIP first…</option></select></label><button type="button" data-action="validate-template">Validate</button><button class="primary" type="submit" disabled>+ Import Complete Report Template</button>';
  const archiveInput = form.elements.archive;
  const mainSelect = form.elements.main;
  const validate = form.querySelector('[data-action="validate-template"]');
  const submit = form.querySelector('button[type="submit"]');
  const previewHost = element('div', 'template-preview');
  let preview = null;

  const previewArchive = async () => {
    const file = archiveInput.files[0];
    if (!file) throw new Error('Choose a local template ZIP.');
    const body = new FormData(); body.set('archive', file);
    preview = await api('/api/admin/v2/templates/preview', {
      method: 'POST', body,
    });
    const texFiles = preview.files.filter((entry) => entry.is_tex);
    mainSelect.replaceChildren(new Option('Select Main…', ''));
    texFiles.forEach((entry) => mainSelect.append(new Option(entry.path, entry.path)));
    mainSelect.value = preview.detected_main || '';
    mainSelect.disabled = false;
    submit.disabled = !mainSelect.value;
    const selectedEntry = preview.files.find((entry) => entry.path === mainSelect.value);
    if (selectedEntry?.single_source_compatible) form.elements.arrangement.value = 'SINGLE_SOURCE';
    const heading = element('strong', '', `${preview.files.length} safe files`);
    const tree = element('ul', 'template-file-tree');
    preview.files.forEach((entry) => tree.append(element('li', '', `${entry.path} · ${entry.size_bytes} bytes`)));
    const selected = preview.files.find((entry) => entry.path === mainSelect.value);
    const hint = element('p', 'empty-copy', `${preview.detected_main ? `Detected Main: ${preview.detected_main}` : 'Main is ambiguous; choose one TeX file.'} ${selected?.single_source_compatible ? 'Complete report binding is safe.' : selected?.front_matter_compatible ? 'Legacy separate Front Matter marker detected.' : 'No metadata binding detected.'}`);
    previewHost.replaceChildren(heading, hint, tree);
  };
  archiveInput.addEventListener('change', () => { preview = null; mainSelect.disabled = true; submit.disabled = true; previewHost.replaceChildren(); });
  validate.addEventListener('click', () => previewArchive().catch(showError));
  mainSelect.addEventListener('change', () => { submit.disabled = !preview || !mainSelect.value; const selected = preview?.files.find((entry) => entry.path === mainSelect.value); if (selected?.single_source_compatible) form.elements.arrangement.value = 'SINGLE_SOURCE'; });
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
      if (!preview) await previewArchive();
      if (!mainSelect.value) throw new Error('Select the Main TeX file.');
      const body = new FormData();
      body.set('name', form.elements.name.value); body.set('description', form.elements.description.value);
      body.set('main', mainSelect.value); body.set('arrangement', form.elements.arrangement.value); body.set('archive', archiveInput.files[0]);
      const imported = await api('/api/admin/v2/templates/import', {
        method: 'POST', body,
      });
      announce(imported.binding_warning || `Imported ${imported.name}: ${imported.files.length} immutable files · ${imported.source_identity}`, Boolean(imported.binding_warning));
      await showSection('Templates');
    } catch (error) { showError(error); }
  });
  content.append(intro, form, previewHost);
  if (!templates.length) { content.append(element('p', 'empty-copy', 'No templates yet.')); await renderFrontMatterPacks(frontMatterPacks); await renderAutomaticDefaults(templates, frontMatterPacks); return; }
  const wrap = element('div', 'admin-table-wrap');
  const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Name / description</th><th>Main file</th><th>Front Matter</th><th>Imported</th><th>Usage</th><th>Actions</th></tr></thead>';
  const body = document.createElement('tbody');
  templates.forEach((template) => {
    const row = document.createElement('tr');
    const metadata = element('td');
    const name = document.createElement('input'); name.value = template.name; name.maxLength = 200; name.setAttribute('aria-label', `Name for ${template.name}`);
    const description = document.createElement('input'); description.value = template.description || ''; description.maxLength = 2000; description.placeholder = 'Optional description'; description.setAttribute('aria-label', `Description for ${template.name}`);
    metadata.append(name, description);
    const mainCell = element('td'); const main = document.createElement('select'); main.setAttribute('aria-label', `Main file for ${template.name}`);
    template.tex_files.forEach((path) => main.append(new Option(path, path, false, path === template.main_file))); mainCell.append(main);
    const arrangement = document.createElement('select'); arrangement.setAttribute('aria-label', `Front Matter arrangement for ${template.name}`); [['Report content only', 'REPORT_CONTENT_ONLY'], ['Separate files', 'SEPARATE_FILES'], ['One complete source', 'SINGLE_SOURCE']].forEach(([label, value]) => arrangement.append(new Option(label, value, false, value === template.front_matter_arrangement))); mainCell.append(arrangement);
    const usage = template.pinned ? `${template.usage_count} Paper Team pin${template.usage_count === 1 ? '' : 's'}` : 'Unused';
    const actions = element('td');
    actions.append(
      buttonAction('Edit', async () => {
        await api(`/api/admin/v2/templates/${template.id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: name.value, description: description.value || null, main_file: main.value, front_matter_arrangement: arrangement.value }) });
        announce(`Updated ${name.value}. Existing Paper Team sources were not changed.`); await showSection('Templates');
      }),
      buttonAction('Remove', async () => {
        if (!confirm(`Remove template “${template.name}”?`)) return;
        await api(`/api/admin/v2/templates/${template.id}`, { method: 'DELETE' }); announce(`Removed ${template.name}.`); await showSection('Templates');
      }, 'danger'),
    );
    row.append(metadata, mainCell, element('td', '', template.front_matter_arrangement === 'SEPARATE_FILES' ? 'Legacy separate Front Matter' : template.front_matter_arrangement === 'SINGLE_SOURCE' ? 'Complete report · single source' : 'Report content only'), element('td', '', new Date(template.created_at).toLocaleString()), element('td', '', usage), actions);
    body.append(row);
  });
  table.append(body); wrap.append(table); content.append(wrap); await renderFrontMatterPacks(frontMatterPacks); await renderAutomaticDefaults(templates, frontMatterPacks);
}

async function renderFrontMatterPacks(packs) {
  const heading = element('h2', 'admin-section-anchor', 'Front Matter'); heading.id = 'front-matter'; content.append(heading);
  const intro = element('p', 'empty-copy', 'Import a safe ZIP with frontmatter.json, or VIT cover/certificate/declaration/acknowledgement section files. Pack content never changes after import.');
  const form = element('form', 'admin-template-form');
  form.innerHTML = '<label>Name<input name="name" required maxlength="200"></label><label>Description (optional)<input name="description" maxlength="2000"></label><label>Front Matter ZIP<input name="archive" type="file" accept=".zip,application/zip" required></label><button type="button" data-action="validate-front-matter">Validate</button><button class="primary" type="submit" disabled>+ Import Front Matter Pack</button>';
  const previewHost = element('div', 'template-preview'); let preview = null;
  const validate = async () => { const file = form.elements.archive.files[0]; if (!file) throw new Error('Choose a Front Matter ZIP.'); const body = new FormData(); body.set('archive', file); preview = await api('/api/admin/v2/front-matter-packs/preview', { method: 'POST', body }); form.querySelector('button[type="submit"]').disabled = false; const sections = element('ul', 'compact-list'); preview.sections.forEach((section) => sections.append(element('li', '', `${section.label} · ${section.required ? 'Required' : 'Optional'} · ${section.file}`))); const fields = element('ul', 'compact-list'); preview.fields.forEach((field) => fields.append(element('li', '', `${field.label} · ${field.type}${field.source ? ` · ${field.source}` : field.default !== null && field.default !== undefined ? ' · Pack default' : ' · Team value'}`))); const tree = element('ul', 'template-file-tree'); preview.files.forEach((fileEntry) => tree.append(element('li', '', `${fileEntry.path} · ${fileEntry.size_bytes} bytes`))); previewHost.replaceChildren(...(preview.warnings || []).map((warning) => element('p', 'muted-note', warning)), element('strong', '', `Entry: ${preview.entry_file}`), element('h3', '', 'Sections'), sections, element('h3', '', 'Fields'), fields, element('h3', '', 'Files'), tree); };
  form.elements.archive.addEventListener('change', () => { preview = null; form.querySelector('button[type="submit"]').disabled = true; previewHost.replaceChildren(); });
  form.querySelector('[data-action="validate-front-matter"]').addEventListener('click', () => validate().catch(showError));
  form.addEventListener('submit', async (event) => { event.preventDefault(); try { if (!preview) await validate(); const body = new FormData(); body.set('name', form.elements.name.value); body.set('description', form.elements.description.value); body.set('archive', form.elements.archive.files[0]); const imported = await api('/api/admin/v2/front-matter-packs/import', { method: 'POST', body }); announce(`Imported ${imported.name}: immutable Front Matter Pack ${imported.content_hash}.`); await showSection('Templates'); } catch (error) { showError(error); } });
  content.append(intro, form, previewHost);
  if (!packs.length) return content.append(element('p', 'empty-copy', 'No Front Matter Packs yet.'));
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Pack</th><th>Sections</th><th>Fields</th><th>Created</th><th>Usage</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody');
  packs.forEach((pack) => { const row = document.createElement('tr'); const identity = element('td'); const name = document.createElement('input'); name.value = pack.name; name.maxLength = 200; name.setAttribute('aria-label', `Name for ${pack.name}`); const description = document.createElement('input'); description.value = pack.description || ''; description.maxLength = 2000; description.placeholder = 'Optional description'; identity.append(name, description); const actions = element('td'); actions.append(buttonAction('View', async () => { const detail = await api(`/api/admin/v2/front-matter-packs/${pack.id}`); const host = document.querySelector('#adminDialogBody'); const sectionList = element('ul', 'compact-list'); detail.pack.manifest_json.sections.forEach((section) => sectionList.append(element('li', '', `${section.label} · ${section.required ? 'Required' : 'Optional'} · ${section.file}`))); const fieldList = element('ul', 'compact-list'); detail.pack.manifest_json.fields.forEach((field) => fieldList.append(element('li', '', `${field.label} · ${field.type}${field.source ? ` · ${field.source}` : ''}`))); const fileList = element('ul', 'template-file-tree'); detail.files.forEach((file) => fileList.append(element('li', '', `${file.path} · ${file.media_type}`))); host.replaceChildren(element('p', '', detail.pack.description || 'No description'), element('p', '', `Entry file: ${detail.pack.manifest_json.entry_file}`), element('h3', '', 'Section order'), sectionList, element('h3', '', 'Fields and automatic sources'), fieldList, element('h3', '', 'Asset / file tree'), fileList); openAdminDialog(detail.pack.name); }), buttonAction('Edit metadata', async () => { await api(`/api/admin/v2/front-matter-packs/${pack.id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: name.value, description: description.value || null }) }); announce(`Updated ${name.value}. Pack content was not changed.`); await showSection('Templates'); }), buttonAction('Remove', async () => { if (!confirm(`Remove Front Matter Pack “${pack.name}”?`)) return; await api(`/api/admin/v2/front-matter-packs/${pack.id}`, { method: 'DELETE' }); await showSection('Templates'); }, 'danger')); row.append(identity, element('td', '', String(pack.manifest_json.sections.length)), element('td', '', String(pack.manifest_json.fields.length)), element('td', '', new Date(pack.created_at).toLocaleString()), element('td', '', pack.usage_count ? `${pack.usage_count} Team${pack.usage_count === 1 ? '' : 's'}` : 'Unused'), actions); body.append(row); });
  table.append(body); wrap.append(table); content.append(wrap);
}

async function patchUser(email, values) {
  await api(`/api/admin/users/${encodeURIComponent(email)}`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(values),
  });
  await showSection('Users');
}

function userActions(user) {
  const actions = element('div', 'admin-actions');
  const type = document.createElement('select');
  type.setAttribute('aria-label', `Institutional account type for ${user.email}`);
  ['student', 'professor', 'admin'].forEach((value) => {
    const option = element('option', '', value);
    option.value = value;
    option.selected = value === user.account_type;
    type.append(option);
  });
  type.addEventListener('change', () => patchUser(user.email, { account_type: type.value }).catch(showError));
  const toggle = element('button', '', user.enabled ? 'Disable' : 'Enable');
  toggle.type = 'button';
  toggle.addEventListener('click', () => patchUser(user.email, { enabled: !user.enabled }).catch(showError));
  const reset = element('button', '', 'Reset password');
  reset.type = 'button';
  reset.addEventListener('click', async () => {
    try {
      const result = await api(`/api/admin/users/${encodeURIComponent(user.email)}/reset-password`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{}',
      });
      announce(`Temporary password for ${result.email}: ${result.temporary_password}`);
    } catch (error) { showError(error); }
  });
  const remove = element('button', 'danger', 'Delete');
  remove.type = 'button';
  remove.addEventListener('click', async () => {
    try {
      await api(`/api/admin/users/${encodeURIComponent(user.email)}`, { method: 'DELETE' });
      await showSection('Users');
    } catch (error) { showError(error); }
  });
  actions.append(type, toggle, reset, remove);
  return actions;
}

function renderLegacyUsers(users) {
  const form = element('form', 'admin-user-form');
  form.innerHTML = '<label>Email<input name="email" type="email" required></label><label>Institutional type<select name="account_type"><option>student</option><option>professor</option><option>admin</option></select></label><label>Temporary password (optional)<input name="password" type="password" minlength="12"></label><button class="primary" type="submit">Create user</button>';
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const values = Object.fromEntries(new FormData(form));
    if (!values.password) delete values.password;
    try {
      const result = await api('/api/admin/users', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(values),
      });
      announce(`Temporary password for ${result.email}: ${result.temporary_password}`);
      await showSection('Users');
    } catch (error) { showError(error); }
  });
  const wrap = element('div', 'admin-table-wrap');
  const table = element('table', 'admin-table');
  const head = document.createElement('thead');
  head.innerHTML = '<tr><th>Email</th><th>Institutional type</th><th>Status</th><th>Created</th><th>Actions</th></tr>';
  const body = document.createElement('tbody');
  users.forEach((user) => {
    const row = document.createElement('tr');
    row.append(
      element('td', '', user.email),
      element('td', '', user.account_type),
      element('td', '', user.enabled ? 'enabled' : 'disabled'),
      element('td', '', user.created_at),
    );
    const actionCell = document.createElement('td');
    actionCell.append(userActions(user));
    row.append(actionCell);
    body.append(row);
  });
  table.append(head, body);
  wrap.append(table);
  content.append(form, wrap);
}

async function patchV2Role(user, role) {
  await api(`/api/admin/v2/users/${encodeURIComponent(user.user_id)}/role`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ role }),
  });
  announce(`${user.email} is now an exclusive V2 ${role}.`);
  await showSection('V2 Users');
}

function renderV2Users(users) {
  const intro = element('p', 'empty-copy', 'Writer, Mentor, and Admin are mutually exclusive V2 roles. Manual provisioning remains available.');
  const form = element('form', 'admin-user-form');
  form.innerHTML = '<label>Email<input name="email" type="email" required></label><label>Password<input name="password" type="password" minlength="12" maxlength="256"></label><label>V2 role<select name="role"><option value="writer">Writer</option><option value="mentor">Mentor</option><option value="admin">Admin</option></select></label><label><input name="generate_temporary_password" type="checkbox"> Generate temporary password</label><button class="primary" type="submit">Create V2 user</button>';
  const generated = form.elements.generate_temporary_password; const password = form.elements.password;
  generated.addEventListener('change', () => { password.disabled = generated.checked; password.required = !generated.checked; }); password.required = true;
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
      const payload = Object.fromEntries(new FormData(form)); payload.generate_temporary_password = generated.checked;
      if (generated.checked) delete payload.password;
      const created = await api('/api/admin/v2/users', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (created.temporary_password) showCredentialHandoff({ created: 1, credential_emails_queued: created.credential_email_queued ? 1 : 0, reused: 0, needs_attention: 0, credentials: [{ email: created.email, temporary_password: created.temporary_password, credential_role: created.role === 'writer' ? 'student' : 'mentor' }] }, created.user_id);
      announce('V2 user created.');
      await showSection('V2 Users');
    } catch (error) { showError(error); }
  });
  const wrap = element('div', 'admin-table-wrap');
  const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Email</th><th>Role</th><th>Status</th><th>Account state</th><th>Email delivery</th><th>Actions</th></tr></thead>';
  const body = document.createElement('tbody');
  users.forEach((user) => {
    const row = document.createElement('tr');
    const select = document.createElement('select');
    select.setAttribute('aria-label', `Exclusive V2 role for ${user.email}`);
    const unassigned = element('option', '', 'Unassigned');
    unassigned.value = '';
    unassigned.disabled = true;
    unassigned.selected = user.v2_role === null;
    select.append(unassigned);
    ['writer', 'mentor', 'admin'].forEach((role) => {
      const option = element('option', '', role[0].toUpperCase() + role.slice(1));
      option.value = role;
      option.selected = role === user.v2_role;
      select.append(option);
    });
    select.addEventListener('change', () => patchV2Role(user, select.value).catch(showError));
    const toggle = buttonAction(user.enabled ? 'Disable' : 'Enable', async () => { await api(`/api/admin/users/${encodeURIComponent(user.email)}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled: !user.enabled }) }); await showSection('V2 Users'); });
    const temporaryPassword = (user.v2_role === 'writer' || user.v2_role === 'mentor') ? buttonAction('Generate temporary password', async () => { if (!confirm(`Replace the current password for ${user.email}? Current sessions will end.`)) return; const reset = await api(`/api/admin/v2/users/${encodeURIComponent(user.user_id)}/temporary-password`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); showCredentialHandoff({ created: 1, credential_emails_queued: reset.credential_email_queued ? 1 : 0, reused: 0, needs_attention: 0, credentials: [{ email: reset.email, temporary_password: reset.temporary_password, credential_role: user.v2_role === 'writer' ? 'student' : 'mentor' }] }, `reset-${user.user_id}`); await showSection('V2 Users'); }) : null;
    const retry = user.email_delivery_status === 'FAILED' && !user.email_delivery_expired && user.email_delivery_id ? buttonAction('Retry email', async () => { await api(`/api/admin/v2/credential-emails/${encodeURIComponent(user.email_delivery_id)}/retry`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Credential email retry queued for ${user.email}.`); await showSection('V2 Users'); }) : null;
    const details = buttonAction('Details', () => {
      const host = document.querySelector('#adminDialogBody');
      host.replaceChildren(
        element('p', '', user.email),
        element('p', '', `Created ${new Date(user.created_at).toLocaleString()}`),
        element('p', 'muted-note', `Legacy account type: ${user.legacy_account_type || 'None'}`),
        element('p', 'muted-note', `Migration state: ${humanLabel(user.migration_state || 'Not recorded')}`),
      );
      openAdminDialog('User details');
    });
    const roleCell = element('td'); roleCell.append(select);
    const actionCell = element('td'); actionCell.append(rowMenu(`Actions for ${user.email}`, [temporaryPassword, retry, toggle, details]));
    const emailDelivery = !user.must_change_password ? 'Not required' : user.email_delivery_status === 'SENT' ? 'Email sent' : user.email_delivery_status === 'SENDING' ? 'Sending' : user.email_delivery_status === 'PENDING' ? 'Pending' : user.email_delivery_status === 'FAILED' ? (user.email_delivery_expired ? 'Expired' : 'Failed') : user.email_delivery_status === 'EXPIRED' ? 'Expired' : 'Unavailable';
    row.append(
      element('td', '', user.email),
      roleCell,
      element('td', '', user.enabled ? 'Enabled' : 'Disabled'),
      element('td', '', user.must_change_password ? 'Setup required' : 'Active'),
      element('td', '', emailDelivery),
      actionCell,
    );
    body.append(row);
  });
  table.append(body);
  wrap.append(table);
  content.append(intro, form, wrap);
}

function queryString(values) {
  const query = new URLSearchParams();
  Object.entries(values).forEach(([key, value]) => { if (value !== '' && value !== null && value !== undefined) query.set(key, value); });
  return query.toString();
}

function pager(page, reload) {
  const controls = element('div', 'admin-pagination');
  const previous = buttonAction('Previous', () => reload(Math.max(1, page.page - 1)));
  previous.disabled = page.page <= 1;
  const next = buttonAction('Next', () => reload(page.page + 1)); next.disabled = !page.has_more;
  controls.append(previous, element('span', '', `Page ${page.page} · ${page.total} records`), next);
  return controls;
}

function filterForm(fields, onSubmit) {
  const form = element('form', 'admin-filters');
  fields.forEach(({ name, label, type = 'text', options = [], placeholder = '' }) => {
    const field = element('label', '', label); let input;
    if (type === 'select') { input = document.createElement('select'); input.append(new Option('All', '')); options.forEach(([text, value = text]) => input.append(new Option(text, value))); }
    else { input = document.createElement('input'); input.type = type; input.placeholder = placeholder; }
    input.name = name; field.append(input); form.append(field);
  });
  const apply = element('button', 'primary', 'Apply filters'); apply.type = 'submit'; form.append(apply);
  form.addEventListener('submit', (event) => { event.preventDefault(); onSubmit(Object.fromEntries(new FormData(form))); });
  return form;
}

async function searchPeople(role, q) {
  return api(`/api/admin/v2/institution/users/search?${queryString({ q, role, limit: 20 })}`);
}

function peopleBuilder(role, ordered, changed) {
  const section = element('section', 'admin-section people-builder'); section.append(element('h2', '', ordered ? 'Ordered Writers' : 'Mentors'));
  const search = element('input'); search.type = 'search'; search.placeholder = `Search V2 ${role} email`; search.setAttribute('aria-label', `Search V2 ${role}s`);
  const results = document.createElement('select'); results.setAttribute('aria-label', `${role} search results`); results.append(new Option('Search to find accounts…', ''));
  let timer;
  search.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(async () => { try { const users = await searchPeople(role, search.value); results.replaceChildren(new Option('Select an account…', '')); users.forEach((user) => results.append(new Option(user.email, user.user_id))); } catch (error) { showError(error); } }, 250); });
  const selected = []; const host = element('div', 'ordered-people');
  const draw = () => {
    host.replaceChildren(); selected.forEach((person, index) => {
      const row = element('div', 'ordered-person'); row.append(element('strong', '', ordered ? `${index + 1}.` : '•'), element('span', '', person.email));
      const up = buttonAction('Up', () => { if (index) [selected[index - 1], selected[index]] = [selected[index], selected[index - 1]]; draw(); changed(selected); }); up.disabled = !ordered || index === 0; up.title = 'Move up'; up.setAttribute('aria-label', `Move ${person.email} up`);
      const down = buttonAction('Down', () => { if (index < selected.length - 1) [selected[index], selected[index + 1]] = [selected[index + 1], selected[index]]; draw(); changed(selected); }); down.disabled = !ordered || index === selected.length - 1; down.title = 'Move down'; down.setAttribute('aria-label', `Move ${person.email} down`);
      const remove = buttonAction('Remove', () => { selected.splice(index, 1); draw(); changed(selected); }, 'danger'); remove.title = 'Remove account'; remove.setAttribute('aria-label', `Remove ${person.email}`);
      const actions = element('div', 'ordered-person-actions');
      if (ordered) actions.append(up, down);
      actions.append(remove); row.append(actions); host.append(row);
    });
  };
  const add = buttonAction('Add', () => { const option = results.selectedOptions[0]; if (!option?.value || selected.some((person) => person.user_id === option.value)) return; selected.push({ user_id: option.value, email: option.textContent }); draw(); changed(selected); });
  const controls = element('div', 'people-search-controls'); controls.append(search, results, add);
  section.append(controls, host);
  return { section, selected, draw };
}

async function renderManualTeamForm(templates, frontMatterPacks) {
  const details = element('details', 'admin-section'); const summary = element('summary', 'admin-section-summary', '+ Create Team'); details.append(summary);
  const form = element('form', 'manual-team-form'); form.innerHTML = '<label>Team name<input name="name" required maxlength="200"></label>';
  const leader = element('label', '', 'Leader'); const leaderSelect = document.createElement('select'); leaderSelect.required = true; leader.append(leaderSelect);
  const template = element('label', '', 'Template'); const templateSelect = document.createElement('select'); templateSelect.append(new Option('Use suggested template automatically', '')); templates.forEach((item) => templateSelect.append(new Option(`Override: ${item.name}`, item.id))); template.append(templateSelect);
  const frontMatter = element('label', '', 'Front Matter'); const frontMatterSelect = document.createElement('select'); frontMatterSelect.append(new Option('Use automatic suggestion', 'automatic')); frontMatterSelect.append(new Option('None', 'none')); frontMatterPacks.forEach((item) => frontMatterSelect.append(new Option(`Override: ${item.name}`, item.id))); frontMatter.append(frontMatterSelect);
  const suggestion = element('p', 'muted-note', 'Add Writers to resolve the programme default.');
  let timer;
  const writers = peopleBuilder('writer', true, (people) => {
    const previous = leaderSelect.value; leaderSelect.replaceChildren(new Option('Select one of the ordered Writers…', ''));
    people.forEach((person) => leaderSelect.append(new Option(person.email, person.user_id))); if (people.some((person) => person.user_id === previous)) leaderSelect.value = previous;
    clearTimeout(timer); timer = setTimeout(async () => { if (!people.length) return; try { const preview = await api('/api/admin/v2/institution/template-defaults/resolve-preview', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ordered_writer_user_ids: people.map((person) => person.user_id) }) }); const found = templates.find((item) => item.id === preview.selected_template_id); const front = frontMatterPacks.find((item) => item.id === preview.front_matter_pack_id); const context = preview.resolution_method === 'TIE_FIRST_WRITER' ? `Selected using Writer-order tie-break (${preview.dominant_programme_code})` : preview.resolution_method === 'GLOBAL_FALLBACK' ? 'Automatically selected from Global fallback' : `Automatically selected from ${preview.dominant_programme_code}`; suggestion.textContent = `Main template: ${found?.name || preview.selected_template_id} · ${context}. Front Matter: ${front?.name || 'None'}.`; } catch (error) { suggestion.textContent = error.message; } }, 300);
  });
  const mentors = peopleBuilder('mentor', false, () => {});
  form.append(leader, template, frontMatter, suggestion);
  const submit = element('button', 'primary', 'Create Paper Team'); submit.type = 'submit'; form.append(submit);
  form.addEventListener('submit', async (event) => { event.preventDefault(); if (!writers.selected.some((person) => person.user_id === leaderSelect.value)) return showError(new Error('Leader must be one of the selected Writers.')); try { await api('/api/admin/v2/paper-teams', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: form.elements.name.value, writer_ids: writers.selected.map((person) => person.user_id), leader_writer_id: leaderSelect.value, mentor_ids: mentors.selected.map((person) => person.user_id), template_id: templateSelect.value || null, front_matter_pack_id: !['automatic', 'none'].includes(frontMatterSelect.value) ? frontMatterSelect.value : null, use_front_matter_default: frontMatterSelect.value === 'automatic' }) }); announce(`Paper Team created with ${templateSelect.value ? 'an Admin template override' : 'automatic template selection'}.`); await showSection('Paper Teams'); } catch (error) { showError(error); } });
  details.append(writers.section, mentors.section, form); content.append(details);
}

async function openTeamDetail(team, templates, frontMatterPacks) {
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody');
  if (team.unresolved) {
    host.replaceChildren(element('h2', '', `${team.name} · unresolved`), element('p', 'danger-box', `${team.error_code || 'TEAM_UNRESOLVED'}: ${team.error_message || 'Resolve linked identities and template defaults, then re-apply the source import job.'}`), element('p', '', `Students: ${(team.unresolved_student_ids || []).join(', ') || 'None'} · Faculty: ${(team.unresolved_faculty_ids || []).join(', ') || 'None'}`));
    const links = buttonAction('Open Identity Links', () => { dialog.close(); showSection('Institution Data', { tab: 'Identity Links' }); }); const defaults = buttonAction('Open Templates', () => { dialog.close(); showSection('Templates'); }); const job = buttonAction('Open Import Job', () => { dialog.close(); showSection('Imports', { jobId: team.source_import_job_id }); }); host.append(element('p', 'muted-note', 'Imports are additive; missing rows do not remove existing members.'), links, defaults, job); openAdminDialog(team.name); return;
  }
  const detail = await api(`/api/admin/v2/paper-teams/${team.id}`); const leader = detail.members.find((member) => member.is_leader); const writers = detail.members.filter((member) => member.role === 'writer'); const mentors = detail.members.filter((member) => member.role === 'mentor');
  const sourceLabel = detail.summary.resolution_method === 'MANUAL_OVERRIDE' ? 'Admin override' : detail.summary.resolution_method === 'GLOBAL_FALLBACK' ? 'Global fallback' : detail.summary.dominant_programme_code || 'Programme default';
  const sourcePrefix = detail.summary.resolution_method === 'MANUAL_OVERRIDE' ? 'Selected by:' : 'Selected automatically from:';
  const summary = element('div', 'admin-summary-grid');
  [['Programme', detail.summary.dominant_programme_code || '—'], ['Main Template', detail.template_pin?.template_name || 'None'], ['Front Matter', detail.front_matter.pack_name || 'None'], ['Review', humanLabel(detail.summary.review_state)]].forEach(([label, value]) => { const item = element('div'); item.append(element('small', '', label), element('strong', '', value)); summary.append(item); });
  const badges = element('p', 'admin-actions'); badges.append(statusChip(humanLabel(detail.team.status)), statusChip(team.source === 'imported' ? 'Imported' : 'Manual'));
  const membersSummary = element('section', 'admin-section'); membersSummary.append(element('h2', '', 'Members'), element('p', '', `Writers: ${writers.map((member) => `${member.writer_order}. ${member.email}${member.is_leader ? ' (Leader)' : ''}`).join(', ') || 'None'}`), element('p', '', `Leader: ${leader?.email || 'Missing'}`), element('p', '', `Mentors: ${mentors.map((member) => member.email).join(', ') || 'None'}`));
  const resetTeam = buttonAction('Reset Team passwords', () => resetTeamPasswords([{ id: team.id, name: detail.team.name, external_team_key: team.external_team_key, writers: writers.map(({ user_id, email, writer_order, is_leader }) => ({ user_id, email, writer_order, is_leader })), mentors: mentors.map(({ user_id, email }) => ({ user_id, email })), leader }]));
  resetTeam.title = 'Forgotten passwords: issue new temporary passwords to this Team and download them Team-wise'; membersSummary.append(resetTeam);
  host.replaceChildren(badges, summary, membersSummary);
  const advanced = element('details', 'admin-section'); advanced.append(element('summary', 'admin-section-summary', 'Advanced details'), element('p', 'muted-note', `Dominant programme: ${detail.summary.dominant_programme_code || '—'} · Resolution: ${detail.summary.resolution_method || 'unrecorded'} · Files: ${detail.summary.file_count} · Build: ${detail.summary.current_build.status || 'none'} · Updated: ${new Date(detail.team.updated_at).toLocaleString()}`));
  if (team.source === 'imported') advanced.append(element('p', 'muted-note', `External key ${team.external_team_key} · source import ${team.source_import_job_id} · last imported ${team.last_imported_at || 'unknown'}.`));
  const management = element('section', 'admin-section'); management.append(element('h2', '', 'Edit Team'));
  const edit = element('details'); edit.append(element('summary', '', 'Edit Team')); const editForm = element('form', 'manual-team-form'); const nameLabel = element('label', '', 'Team name'); const nameInput = document.createElement('input'); nameInput.name = 'name'; nameInput.required = true; nameInput.maxLength = 200; nameInput.value = detail.team.name; nameLabel.append(nameInput);
  const leaderLabel = element('label', '', 'Team Leader'); const leaderSelect = document.createElement('select'); leaderSelect.required = true; leaderLabel.append(leaderSelect);
  const updateLeaderChoices = (people) => { const previous = leaderSelect.value || leader?.user_id; leaderSelect.replaceChildren(new Option('Select one Writer…', '')); people.forEach((person) => leaderSelect.append(new Option(person.email, person.user_id))); if (people.some((person) => person.user_id === previous)) leaderSelect.value = previous; };
  const editWriters = peopleBuilder('writer', true, updateLeaderChoices); editWriters.selected.push(...writers.map(({ user_id, email }) => ({ user_id, email }))); editWriters.draw(); updateLeaderChoices(editWriters.selected);
  const editMentors = peopleBuilder('mentor', false, () => {}); editMentors.selected.push(...mentors.map(({ user_id, email }) => ({ user_id, email }))); editMentors.draw();
  const saveTeam = element('button', 'primary', 'Save Team'); saveTeam.type = 'submit'; editForm.append(nameLabel, editWriters.section, leaderLabel, editMentors.section, saveTeam); editForm.addEventListener('submit', async (event) => { event.preventDefault(); try { if (!editWriters.selected.length) throw new Error('Team must retain at least one Writer.'); if (!editWriters.selected.some((person) => person.user_id === leaderSelect.value)) throw new Error('Leader must be a selected Writer.'); await api(`/api/admin/v2/paper-teams/${team.id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: nameInput.value, writer_ids: editWriters.selected.map((person) => person.user_id), leader_writer_id: leaderSelect.value, mentor_ids: editMentors.selected.map((person) => person.user_id) }) }); announce(`Saved ${nameInput.value}.`); dialog.close(); await showSection('Paper Teams'); } catch (error) { showError(error); } }); edit.append(editForm); management.append(edit);
  management.append(element('h3', '', 'Lifecycle'));
  [['Freeze', 'frozen'], ['Activate', 'active'], ['Archive', 'archived']].forEach(([label, value]) => management.append(buttonAction(label, async () => { await api(`/api/admin/v2/paper-teams/${team.id}/status`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ status: value }) }); dialog.close(); await showSection('Paper Teams'); }, value === 'archived' ? 'danger' : ''))); host.append(management);
  const section = element('section', 'admin-section'); section.append(element('h2', '', 'Document setup'), element('h3', '', 'Main Template'), element('p', '', `Current: ${detail.template_pin?.template_name || 'None'}`), element('p', 'muted-note', `${sourcePrefix} ${sourceLabel}`));
  const select = document.createElement('select'); select.setAttribute('aria-label', 'New template'); templates.filter((item) => item.id !== detail.template_pin?.template_id).forEach((item) => select.append(new Option(item.name, item.id)));
  const confirmation = element('div'); const change = buttonAction('Change template', async () => { if (!select.value) throw new Error('No alternative template is available.'); const preview = await api(`/api/admin/v2/paper-teams/${team.id}/template-change/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, confirm_main_file_change: false }) }); const fileConflicts = preview.writer_modified_conflicts.filter((item) => !String(item.reason).startsWith('Main file change')); if (fileConflicts.length) { const list = element('ul', 'compact-list'); fileConflicts.forEach((item) => list.append(element('li', '', item.path))); confirmation.replaceChildren(element('h3', '', 'Template cannot be changed yet.'), element('p', 'danger-box', 'Writer-edited files conflict:'), list, element('p', '', 'No files were changed.')); return; } const selectedTemplate = templates.find((item) => item.id === select.value); const mainChanged = preview.main_file_change.changed; const confirmBox = element('div', 'template-change-confirmation'); confirmBox.append(element('h3', '', 'Change template?'), element('p', '', `${detail.template_pin?.template_name || 'None'} → ${selectedTemplate?.name || 'Selected template'}`), element('p', '', `${preview.template_managed_files_to_update.length + preview.unchanged_old_template_files_to_update.length} files will update`), element('p', '', `${preview.files_to_add.length} files will be added`), element('p', '', 'Paper history will be preserved')); let mainConfirm = null; if (mainChanged) { const label = element('label', '', ` Confirm Main document change (${preview.main_file_change.from} → ${preview.main_file_change.to})`); mainConfirm = document.createElement('input'); mainConfirm.type = 'checkbox'; label.prepend(mainConfirm); confirmBox.append(label); } const apply = buttonAction('Change', async () => { const confirmedMain = Boolean(mainConfirm?.checked); if (mainChanged && !confirmedMain) return; const ready = mainChanged ? await api(`/api/admin/v2/paper-teams/${team.id}/template-change/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, confirm_main_file_change: true }) }) : preview; await api(`/api/admin/v2/paper-teams/${team.id}/template-change/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, preview_token: ready.preview_token, confirm_main_file_change: confirmedMain }) }); announce('Template changed. Paper history was preserved.'); dialog.close(); await showSection('Paper Teams'); }, 'primary'); if (mainConfirm) { apply.disabled = true; mainConfirm.addEventListener('change', () => { apply.disabled = !mainConfirm.checked; }); } confirmBox.append(buttonAction('Cancel', () => confirmation.replaceChildren()), apply); confirmation.replaceChildren(confirmBox); });
  section.append(select, change, confirmation); host.append(section);
  const frontSection = element('section', 'admin-section'); frontSection.append(element('h2', '', 'Front Matter'), element('p', '', `Current: ${detail.front_matter.pack_name || 'None'}`), element('p', 'muted-note', `Status: ${detail.front_matter.status === 'READY' ? 'Ready' : detail.front_matter.status === 'INCOMPATIBLE' ? 'Incompatible' : 'Needs information'}`));
  if ((detail.front_matter.missing_required_fields || []).length) frontSection.append(element('p', 'danger-box', `Needs information: ${detail.front_matter.missing_required_fields.join(', ')}`));
  const frontSelect = document.createElement('select'); frontSelect.setAttribute('aria-label', 'Front Matter Pack'); frontSelect.append(new Option('None', '')); frontMatterPacks.forEach((pack) => frontSelect.append(new Option(pack.name, pack.id, false, pack.id === detail.front_matter.pack_id)));
  frontSection.append(frontSelect, buttonAction('Change', async () => { if (!confirm(`Change Front Matter to ${frontSelect.selectedOptions[0].textContent}? Writer files will be preserved.`)) return; await api(`/api/admin/v2/paper-teams/${team.id}/front-matter`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ front_matter_pack_id: frontSelect.value || null }) }); announce('Front Matter changed. A safety checkpoint preserved paper history.'); dialog.close(); await showSection('Paper Teams'); }, 'primary'));
  host.insertBefore(frontSection, section); host.append(advanced); openAdminDialog(detail.team.name);
}

async function renderPaperTeamGrid() {
  const [templates, frontMatterPacks] = await Promise.all([api('/api/admin/templates'), api('/api/admin/v2/front-matter-packs')]); await renderManualTeamForm(templates, frontMatterPacks);
  const state = { page: 1, limit: 25, search: '', status: '', programme_code: '', mentor_email: '', leader_email: '', template_id: '', review_state: '', source: '', unresolved: '' };
  const filters = filterForm([
    { name: 'search', label: 'Search Teams', type: 'search', placeholder: 'Team name or Leader/Mentor email…' }, { name: 'status', label: 'Status', type: 'select', options: [['Active', 'active'], ['Frozen', 'frozen'], ['Submitted', 'submitted'], ['Archived', 'archived']] },
    { name: 'programme_code', label: 'Programme' }, { name: 'mentor_email', label: 'Mentor email', type: 'search' }, { name: 'leader_email', label: 'Leader email', type: 'search' },
    { name: 'template_id', label: 'Template', type: 'select', options: templates.map((item) => [item.name, item.id]) }, { name: 'review_state', label: 'Review state' }, { name: 'source', label: 'Source', type: 'select', options: [['Manual', 'manual'], ['Imported', 'imported']] }, { name: 'unresolved', label: 'Unresolved', type: 'select', options: [['Only unresolved', 'true'], ['Resolved only', 'false']] },
  ], (values) => { Object.assign(state, values, { page: 1 }); load(); });
  const pageSize = document.createElement('select'); [25, 50, 100].forEach((size) => pageSize.append(new Option(`${size} per page`, size))); pageSize.addEventListener('change', () => { state.limit = Number(pageSize.value); state.page = 1; load(); }); filters.append(pageSize); content.append(filters);
  const exportBar = element('div', 'admin-toolbar'); exportBar.append(buttonAction('Export all Paper Teams (Excel)', exportPaperTeams, 'primary')); content.append(exportBar);
  const bulk = element('div', 'admin-toolbar'); const gridHost = element('div'); content.append(bulk, gridHost);
  const load = async (page = state.page) => { state.page = page; gridHost.replaceChildren(element('p', 'empty-copy', 'Loading requested Team page…')); const data = await api(`/api/admin/v2/paper-teams/query?${queryString(state)}`); const selected = new Set();
    const runBulk = async (statusValue) => { if (!selected.size || !confirm(`${statusValue} ${selected.size} selected Teams?`)) return; const result = await api('/api/admin/v2/paper-teams/bulk-lifecycle', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ team_ids: [...selected], status: statusValue, confirmed: true }) }); announce(`Bulk lifecycle: ${result.succeeded} succeeded, ${result.failed} failed.`, result.failed > 0); await load(); };
    const selectedTeams = () => data.items.filter((team) => selected.has(team.id));
    bulk.replaceChildren(element('strong', '', 'Selected rows:'), buttonAction('Freeze', () => runBulk('frozen')), buttonAction('Activate', () => runBulk('active')), buttonAction('Archive', () => runBulk('archived'), 'danger'), buttonAction('Reset passwords', () => { if (!selected.size) throw new Error('Select one or more Teams first.'); return resetTeamPasswords(selectedTeams()); }));
    if (!data.items.length) { gridHost.replaceChildren(element('p', 'empty-copy', state.unresolved === 'true' ? 'No unresolved imported Teams.' : 'No Paper Teams match these server-side filters.'), pager(data, load)); return; }
    const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Select</th><th>Team</th><th>Status</th><th>Writers</th><th>Leader</th><th>Mentor</th><th>Programme</th><th>Template</th><th>Review</th><th>Source</th><th>Updated</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody');
    data.items.forEach((team) => { const row = document.createElement('tr'); const selection = document.createElement('input'); selection.type = 'checkbox'; selection.disabled = !team.id; selection.setAttribute('aria-label', `Select ${team.name}`); selection.addEventListener('change', () => { if (selection.checked) selected.add(team.id); else selected.delete(team.id); row.classList.toggle('row-selected', selection.checked); }); const selectCell = element('td'); selectCell.append(selection); const source = team.source === 'imported' ? 'Imported' : 'Manual'; const action = element('td'); action.append(rowMenu(`Actions for ${team.name}`, [buttonAction('View / Manage', () => openTeamDetail(team, templates, frontMatterPacks))])); const statusCell = element('td'); statusCell.append(statusChip(humanLabel(team.status))); row.append(selectCell, element('td', '', team.name), statusCell, element('td', '', (team.writers || []).map((writer) => `${writer.writer_order}. ${writer.email}`).join(', ') || String(team.writer_count)), element('td', '', team.leader?.email || 'Missing'), element('td', '', (team.mentors || []).map((mentor) => mentor.email).join(', ') || 'None'), element('td', '', team.dominant_programme_code || '—'), element('td', '', team.template?.name || '—'), element('td', '', humanLabel(team.review_state)), element('td', '', source), element('td', '', new Date(team.updated_at).toLocaleString()), action); body.append(row); }); table.append(body); wrap.append(table); gridHost.replaceChildren(wrap, pager(data, load));
  };
  await load();
}

async function renderImportDetail(jobId) {
  const detail = await api(`/api/admin/v2/institution/imports/${jobId}`); const { job } = detail;
  const section = element('section', 'admin-section'); section.append(element('h2', '', `Validation result · ${job.id}`));
  (job.warnings || []).forEach((warning) => section.append(element('p', 'muted-note', warning)));
  const metrics = element('div', 'admin-metrics'); Object.entries({ filename: job.original_filename, sha_256: job.content_sha256, mode: job.mode, file_type: job.file_type, status: job.status, total_rows: job.total_rows, insert_rows: job.inserted_rows, update_rows: job.updated_rows, skip_rows: job.skipped_rows, error_rows: job.error_rows }).forEach(([label, value]) => { const metric = element('div', 'admin-metric'); metric.append(element('strong', '', value), element('span', '', label.replaceAll('_', ' '))); metrics.append(metric); }); section.append(metrics);
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Sheet / table</th><th>Total</th><th>Insert</th><th>Update</th><th>Skip</th><th>Error</th></tr></thead>'; const body = document.createElement('tbody'); detail.summaries.forEach((item) => { const row = document.createElement('tr'); row.append(element('td', '', item.source), element('td', '', item.total), element('td', '', item.insert), element('td', '', item.update), element('td', '', item.skip), element('td', item.error ? 'danger' : '', item.error)); body.append(row); }); table.append(body); wrap.append(table); section.append(wrap);
  const errors = detail.rows.filter((row) => row.status === 'ERROR' || row.status === 'UNRESOLVED'); if (errors.length) { const errorBox = element('div', 'danger-box', `${errors.length} blocking/unresolved rows shown in the bounded preview.`); const list = element('ul', 'compact-list'); errors.slice(0, 100).forEach((row) => list.append(element('li', '', `${row.source_table_or_sheet}:${row.row_number} · ${row.error_code} · ${row.error_message}`))); errorBox.append(list); section.append(errorBox); }
  const actions = element('div', 'admin-actions'); const download = element('a', 'shell-link', 'Download errors.csv'); download.href = `/api/admin/v2/institution/imports/${job.id}/errors.csv`; download.download = 'errors.csv';
  const apply = buttonAction('Apply validated import', async () => { if (!confirm(`Apply ${job.mode} import ${job.original_filename}? Imports never remove absent rows.`)) return; const result = await api(`/api/admin/v2/institution/imports/${job.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Import applied: ${result.materialized_teams} Teams created, ${result.merged_teams} merged, ${result.unresolved_teams} unresolved.`); await showSection('Imports', { jobId: job.id }); showCredentialHandoff(result.account_provisioning, job.id); }, 'primary'); apply.disabled = job.mode === 'VALIDATE_ONLY' || job.status !== 'VALIDATED' || job.error_rows > 0;
  const retry = buttonAction('Retry Team Materialization', async () => { const result = await api(`/api/admin/v2/institution/imports/${job.id}/retry-teams`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Retry: ${result.succeeded} succeeded, ${result.failed} remain unresolved.`, result.failed > 0); await showSection('Imports', { jobId: job.id }); }); retry.disabled = job.status !== 'PARTIAL';
  actions.append(download, apply, retry); section.append(actions); content.append(section);
}

const institutionTargets = ['departments', 'admins', 'faculty', 'programmes', 'schools', 'students', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles', 'paper_teams', 'paper_team_writers', 'paper_team_mentors'];
const targetColumns = {
  departments: ['department_id', 'department_name'], admins: ['admin_id', 'email', 'name', 'pfp'], faculty: ['faculty_id', 'name', 'email', 'dept_id', 'honorific', 'designation', 'status'], programmes: ['programme_code', 'programme_name', 'degree_name', 'specialization', 'hod_id'], schools: ['school_id', 'school_name'], students: ['reg_no', 'name', 'email', 'programme_code'], student_course_registrations: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'], faculty_guide_capacity: ['capacity_id', 'faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'integrated_pg_max_projects', 'status'], department_roles: ['id', 'dept_id', 'role_type', 'faculty_id'], faculty_roles: ['role_id', 'faculty_id', 'role_type', 'school_id', 'department_id', 'programme_code', 'status'], paper_teams: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status'], paper_team_writers: ['external_team_key', 'student_reg_no', 'writer_order', 'is_leader'], paper_team_mentors: ['external_team_key', 'faculty_id'],
};

const teamGenerationDatasets = [
  ['departments', 'Identifies institutional departments referenced by Faculty.'],
  ['faculty', 'Identifies institutional Faculty/Mentors.'],
  ['programmes', 'Defines CSE/ECE/etc and drives programme-based template resolution.'],
  ['students', 'Identifies institutional Students/Writers and their programme.'],
  ['paper_teams', 'Defines the institutional paper/project Team.'],
  ['paper_team_writers', 'Maps Students into the Team, establishes deterministic Writer order, and explicitly selects one Team Leader.'],
  ['paper_team_mentors', 'Assigns Faculty Mentor(s) to the Team.'],
];

function createImportGuide() {
  const trigger = element('button', 'import-guide-trigger', 'ⓘ');
  trigger.type = 'button';
  trigger.setAttribute('aria-label', 'What data do I need?');
  trigger.setAttribute('title', 'What data do I need?');
  trigger.setAttribute('aria-haspopup', 'dialog');
  trigger.setAttribute('aria-expanded', 'false');

  const guide = element('dialog', 'import-guide-dialog');
  guide.setAttribute('aria-labelledby', 'importGuideTitle');
  const shell = element('div', 'import-guide-shell');
  const header = element('header', 'import-guide-header');
  const title = element('h2', '', 'What data do I need?');
  title.id = 'importGuideTitle';
  const close = element('button', '', '×');
  close.type = 'button';
  close.setAttribute('aria-label', 'Close import guide');
  close.setAttribute('title', 'Close import guide');
  close.addEventListener('click', () => guide.close());
  header.append(title, close);

  const body = element('div', 'import-guide-body');
  body.append(element('p', '', "To automatically create a Paper Team, LaTeX Core needs institutional people records plus the Team's Writer and Mentor assignments. Drop the files together; dependency order is handled automatically."));

  const datasetHeading = element('h3', '', 'Team-generation datasets');
  const datasetWrap = element('div', 'admin-table-wrap');
  const datasetTable = element('table', 'admin-table import-guide-table');
  datasetTable.innerHTML = '<thead><tr><th>Dataset</th><th>Fields</th><th>Purpose</th></tr></thead>';
  const datasetBody = document.createElement('tbody');
  teamGenerationDatasets.forEach(([dataset, purpose]) => {
    const fields = element('code', '', targetColumns[dataset].join(', '));
    const fieldCell = element('td');
    fieldCell.append(fields);
    const row = document.createElement('tr');
    row.append(element('td', '', dataset), fieldCell, element('td', '', purpose));
    datasetBody.append(row);
  });
  datasetTable.append(datasetBody);
  datasetWrap.append(datasetTable);
  body.append(datasetHeading, datasetWrap, element('p', 'muted-note', 'Include department_name, school_name, programme_name and degree_name for readable document bindings. Legacy ID-only imports remain supported and preserve existing names, but cannot supply a missing display name.'));

  const prerequisites = element('section', 'import-guide-section');
  prerequisites.append(
    element('h3', '', 'Before Teams can be created'),
    element('h4', '', 'Student account link'),
    element('p', '', 'A valid Student.email automatically creates or reuses a V2 WRITER account. Newly created accounts receive their temporary password by email; the one-time credentials CSV remains available as a fallback.'),
    element('h4', '', 'Faculty account link'),
    element('p', '', 'Faculty assigned in paper_team_mentors automatically receive or reuse a V2 MENTOR account. Unassigned Faculty and institutional Admins are never provisioned.'),
    element('h4', '', 'Team Leader'),
    element('p', '', 'Exactly one paper_team_writers row per Team must be marked is_leader = true.'),
    element('h4', '', 'Template'),
    element('p', '', 'Programme defaults may be configured, for example CSE → CSE Template and ECE → ECE Template. If no programme mapping is available, the configured global fallback template is used. Existing Team template pins do not silently change later.'),
  );

  const templateExample = element('section', 'import-guide-section');
  templateExample.append(
    element('h3', '', 'Template selection'),
    element('pre', 'import-guide-example', 'Writer 1 → CSE\nWriter 2 → CSE\nWriter 3 → ECE\nResult: CSE template · MODE\n\nTie:\nWriter 1 → ECE\nWriter 2 → CSE\nResult: ECE template · TIE_FIRST_WRITER'),
  );

  const optional = element('details', 'import-guide-section');
  optional.append(element('summary', '', 'Additional institutional data (optional)'));
  const optionalList = element('ul', 'compact-list');
  ['schools', 'admins', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles'].forEach((dataset) => optionalList.append(element('li', '', `${dataset}: ${targetColumns[dataset].join(', ')}`)));
  optional.append(
    element('p', '', 'These are optional for basic Paper Team creation. They may be imported when the institution wants the additional People & Roles metadata.'),
    optionalList,
    element('p', 'muted-note', 'Optional does not mean invalid parent references are ignored. If an optional child row references another entity, its referenced parent must exist in the database or the same import batch.'),
  );

  const formats = element('section', 'import-guide-section');
  const formatList = element('ul', 'compact-list');
  formatList.append(
    element('li', '', 'CSV: one dataset per file; filename/header detection identifies the dataset automatically in normal cases.'),
    element('li', '', 'XLSX: one workbook may contain all datasets as named worksheets and is recommended for a full institutional import.'),
  );
  formats.append(
    element('h3', '', 'File format'),
    formatList,
    element('p', '', 'Recommended Team-creation workbook sheets:'),
    element('code', 'import-guide-fields', teamGenerationDatasets.map(([dataset]) => dataset).join(', ')),
  );

  body.append(prerequisites, templateExample, optional, formats);
  shell.append(header, body);
  guide.append(shell);
  trigger.addEventListener('click', () => {
    trigger.setAttribute('aria-expanded', 'true');
    guide.showModal();
  });
  guide.addEventListener('close', () => trigger.setAttribute('aria-expanded', 'false'));
  guide.addEventListener('click', (event) => {
    if (event.target === guide) guide.close();
  });
  return { trigger, guide };
}

function datasetLabel(value) { return value === 'paper_teams' ? 'Paper Assignments' : value.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase()); }

async function detectInstitutionDataset(file) {
  if (file.name.toLowerCase().endsWith('.xlsx')) return { dataset: '', candidates: [], status: 'Sheets detected by server' };
  const normalized = file.name.replace(/\.csv$/i, '').toLowerCase().replace(/[^a-z0-9]+/g, '_');
  const named = institutionTargets.find((target) => normalized === target) || institutionTargets.filter((target) => normalized.startsWith(`${target}_`) || normalized.endsWith(`_${target}`) || normalized.includes(`_${target}_`)).sort((left, right) => right.length - left.length)[0];
  const firstLine = (await file.slice(0, 65536).text()).split(/\r?\n/, 1)[0];
  const headers = firstLine.split(',').map((value) => value.trim().replace(/^"|"$/g, ''));
  const candidates = institutionTargets.filter((target) => headers.length && headers.every((header) => targetColumns[target].includes(header)) && targetColumns[target][0] === headers[0]);
  const dataset = named || (candidates.length === 1 ? candidates[0] : '');
  return { dataset, candidates, status: dataset ? 'Ready' : candidates.length > 1 ? 'What data is this?' : 'Choose dataset' };
}

async function renderBatchDetail(batchId) {
  const detail = await api(`/api/admin/v2/institution/import-batches/${batchId}`); const { batch } = detail;
  const section = element('section', 'admin-section import-review');
  section.append(element('h2', '', `${batch.total_files} file${batch.total_files === 1 ? '' : 's'} · ${Number(batch.total_rows).toLocaleString()} records`));
  section.append(element('p', 'muted-note', `${batch.operation[0]}${batch.operation.slice(1).toLowerCase()} review · ${batch.error_rows ? `${batch.error_rows} need attention` : 'Ready to apply'}`));
  (detail.warnings || []).forEach((warning) => section.append(element('p', 'muted-note', warning)));
  const cards = element('div', 'import-summary-grid');
  detail.summaries.forEach((item) => { const card = element('article', 'import-summary-card'); const actionCount = batch.operation === 'ADD' ? item.add : batch.operation === 'EDIT' ? item.edit : item.delete; const action = batch.operation === 'ADD' ? 'to add' : batch.operation === 'EDIT' ? 'to edit' : 'to delete'; card.append(element('strong', '', item.dataset), element('span', '', `${Number(actionCount).toLocaleString()} ${action}`)); if (item.skip) card.append(element('small', '', `${item.skip} already exist`)); if (item.error) card.append(element('small', 'danger', `${item.error} need attention`)); cards.append(card); }); section.append(cards);
  if (detail.issues.length) { const issues = element('section', 'issue-list'); issues.append(element('h3', '', `${batch.error_rows} records need attention`)); detail.issues.forEach((issue) => { const item = element('article', 'issue-card'); item.append(element('strong', '', `${issue.dataset} · row ${issue.row}`), element('span', '', issue.problem || issue.code), element('small', '', `${issue.file}${issue.suggested_action ? ` · ${issue.suggested_action}` : ''}`)); issues.append(item); }); section.append(issues); }
  if (detail.changes?.length) { const changes = element('section', 'change-list'); changes.append(element('h3', '', 'Field changes')); detail.changes.forEach((change) => { const item = element('article', 'change-card'); item.append(element('strong', '', `${change.dataset} ${change.display_key}`)); Object.entries(change.fields).forEach(([field, values]) => item.append(element('p', '', `${field.replaceAll('_', ' ')}: ${values.old ?? '—'} → ${values.new ?? '—'}`))); if (change.template_effect) item.append(element('small', 'muted-note', change.template_effect)); changes.append(item); }); section.append(changes); }
  const actions = element('div', 'admin-actions'); const actionLabel = batch.operation === 'ADD' ? 'Add records' : batch.operation === 'EDIT' ? 'Edit records' : 'Delete records'; const apply = buttonAction(actionLabel, async () => { if (!confirm(`${actionLabel} from this reviewed batch? Only explicit rows are affected.`)) return; const result = await api(`/api/admin/v2/institution/import-batches/${batch.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`${actionLabel}: ${Number(result.batch.total_rows).toLocaleString()} records processed.`); await showSection('Imports', { batchId: batch.id }); if (batch.operation === 'ADD') showCredentialHandoff(result.account_provisioning, batch.id); }, batch.operation === 'DELETE' ? 'danger' : 'primary'); apply.disabled = batch.status !== 'VALIDATED' || batch.error_rows > 0; actions.append(apply); section.append(actions);
  const technical = element('details', 'technical-details'); technical.append(element('summary', '', 'Technical details'), element('p', '', `Batch ID: ${batch.id}`)); detail.files.forEach((file) => { const row = element('p', '', `${file.filename} · ${file.file_type} · ${file.status} · SHA-256 ${file.checksum}`); const errors = element('a', 'shell-link', ' Error CSV'); errors.href = `/api/admin/v2/institution/imports/${file.id}/errors.csv`; row.append(errors); technical.append(row); }); section.append(technical); content.append(section);
}

async function renderImports(options = {}) {
  const heading = content.querySelector('h1');
  const headingRow = element('div', 'import-heading');
  const { trigger: guideTrigger, guide } = createImportGuide();
  heading.replaceWith(headingRow);
  headingRow.append(heading, guideTrigger, guide);
  content.append(element('p', 'muted-note', 'Add new records, edit known records, or delete only the exact keys you provide. File order and database dependency order are handled automatically.'));
  const exportBar = element('div', 'admin-toolbar'); const exportAll = buttonAction('Export all imported data (Excel)', exportInstitutionData); exportAll.title = 'One workbook with every dataset, in the same sheet and column format used for imports'; exportBar.append(exportAll); content.append(exportBar);
  const form = element('form', 'batch-import-form'); form.enctype = 'multipart/form-data'; const operations = element('fieldset', 'operation-control'); operations.innerHTML = '<legend>Operation</legend><label><input type="radio" name="operation" value="ADD" checked>Add</label><label><input type="radio" name="operation" value="EDIT">Edit</label><label><input type="radio" name="operation" value="DELETE">Delete</label>';
  const drop = element('div', 'import-dropzone'); drop.tabIndex = 0; drop.setAttribute('role', 'button'); drop.setAttribute('aria-label', 'Browse for CSV or XLSX files'); drop.append(element('strong', '', 'Drop CSV or XLSX files here'), element('span', '', 'or browse · multiple files supported')); const fileInput = document.createElement('input'); fileInput.type = 'file'; fileInput.multiple = true; fileInput.accept = '.csv,.xlsx,text/csv,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'; fileInput.hidden = true; drop.append(fileInput); const fileHost = element('div', 'import-file-list'); const submit = element('button', 'primary', 'Review changes'); submit.type = 'submit'; submit.disabled = true; const selected = [];
  const drawFiles = () => { fileHost.replaceChildren(); selected.forEach((entry, index) => { const chip = element('div', 'import-file-chip'); chip.append(element('strong', '', entry.file.name)); if (entry.file.name.toLowerCase().endsWith('.xlsx')) chip.append(element('span', '', entry.status)); else if (entry.dataset) chip.append(element('span', '', datasetLabel(entry.dataset))); else { const select = document.createElement('select'); select.setAttribute('aria-label', `What data is ${entry.file.name}?`); select.append(new Option('What data is this?', '')); institutionTargets.forEach((target) => select.append(new Option(datasetLabel(target), target))); select.addEventListener('change', () => { entry.dataset = select.value; entry.status = entry.dataset ? 'Ready' : 'Choose dataset'; drawFiles(); }); chip.append(select); } chip.append(element('span', entry.status === 'Ready' ? 'ready' : 'muted-note', entry.status), iconAction(`Remove ${entry.file.name}`, 'delete', async () => { selected.splice(index, 1); drawFiles(); }, 'danger')); fileHost.append(chip); }); submit.disabled = !selected.length || selected.some((entry) => !entry.file.name.toLowerCase().endsWith('.xlsx') && !entry.dataset); };
  const addFiles = async (files) => { for (const file of files) { if (!/\.(csv|xlsx)$/i.test(file.name)) { announce(`${file.name} is not a supported CSV or XLSX file.`, true); continue; } if (selected.some((entry) => entry.file.name.toLowerCase() === file.name.toLowerCase() && entry.file.size === file.size && entry.file.lastModified === file.lastModified)) { announce(`${file.name} is already in this batch.`, true); continue; } selected.push({ file, ...(await detectInstitutionDataset(file)) }); } drawFiles(); };
  drop.addEventListener('click', () => fileInput.click()); drop.addEventListener('keydown', (event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); fileInput.click(); } }); fileInput.addEventListener('change', () => addFiles(fileInput.files).catch(showError)); let dragDepth = 0; drop.addEventListener('dragenter', (event) => { event.preventDefault(); dragDepth += 1; drop.classList.add('drag-active'); }); drop.addEventListener('dragover', (event) => { event.preventDefault(); event.dataTransfer.dropEffect = 'copy'; }); drop.addEventListener('dragleave', (event) => { event.preventDefault(); dragDepth -= 1; if (dragDepth <= 0) { dragDepth = 0; drop.classList.remove('drag-active'); } }); drop.addEventListener('drop', (event) => { event.preventDefault(); dragDepth = 0; drop.classList.remove('drag-active'); addFiles(event.dataTransfer.files).catch(showError); });
  form.append(drop, fileHost, operations, submit); form.addEventListener('submit', async (event) => { event.preventDefault(); try { const body = new FormData(); body.set('operation', new FormData(form).get('operation')); selected.forEach((entry) => { body.append('files[]', entry.file, entry.file.name); if (entry.dataset && !entry.file.name.toLowerCase().endsWith('.xlsx')) body.append('target_table', entry.dataset); }); const detail = await api('/api/admin/v2/institution/import-batches/validate', { method: 'POST', body }); announce(detail.batch.error_rows ? `${detail.batch.error_rows} records need attention.` : 'Batch is ready to apply.', detail.batch.error_rows > 0); await showSection('Imports', { batchId: detail.batch.id }); } catch (error) { showError(error); } }); content.append(form);
  if (options.batchId) await renderBatchDetail(options.batchId);
  if (options.jobId) await renderImportDetail(options.jobId);
  const history = element('section', 'admin-section'); history.append(element('h2', '', 'Recent imports')); const state = { page: 1, limit: 25, search: '', status: '', mode: '' }; const filters = filterForm([{ name: 'search', label: 'Filename' }, { name: 'status', label: 'State', type: 'select', options: ['VALIDATED', 'APPLIED', 'PARTIAL', 'FAILED'].map((value) => [value]) }, { name: 'mode', label: 'Operation', type: 'select', options: [['Add', 'ADD'], ['Edit', 'EDIT'], ['Delete', 'DELETE']] }], (values) => { Object.assign(state, values, { page: 1 }); load(); }); const host = element('div'); history.append(filters, host); content.append(history);
  const load = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/import-batches?${queryString(state)}`); if (!data.items.length) { host.replaceChildren(element('p', 'empty-copy', 'No batch imports yet.'), pager(data, load)); return; } const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Filename / batch</th><th>Operation</th><th>Result</th><th>Date</th><th>State</th><th></th></tr></thead>'; const body = document.createElement('tbody'); data.items.forEach((batch) => { const result = batch.operation === 'ADD' ? `Added ${batch.added_rows} records` : batch.operation === 'EDIT' ? `Edited ${batch.edited_rows} records` : `Deleted ${batch.deleted_rows} records`; const row = document.createElement('tr'); const action = element('td'); action.append(buttonAction('View details', () => showSection('Imports', { batchId: batch.id }))); row.append(element('td', '', batch.filenames), element('td', '', `${batch.operation[0]}${batch.operation.slice(1).toLowerCase()}`), element('td', '', `${result}${batch.skipped_rows ? ` · ${batch.skipped_rows} skipped` : ''}${batch.error_rows ? ` · ${batch.error_rows} need attention` : ''}`), element('td', '', new Date(batch.created_at).toLocaleString()), element('td', '', batch.status === 'APPLIED' ? 'Complete' : batch.status), action); body.append(row); }); table.append(body); wrap.append(table); host.replaceChildren(wrap, pager(data, load)); }; await load();
}

const institutionDatasets = [
  { label: 'Students', dataset: 'students', keys: ['reg_no'], fields: ['reg_no', 'name', 'email', 'programme_code'], columns: ['reg_no', 'name', 'email', 'programme_code', 'link_status'] },
  { label: 'Faculty', dataset: 'faculty', keys: ['faculty_id'], fields: ['faculty_id', 'name', 'email', 'dept_id', 'honorific', 'designation', 'status'], columns: ['faculty_id', 'honorific', 'name', 'email', 'dept_id', 'designation', 'link_status'] },
  { label: 'Programmes', dataset: 'programmes', keys: ['programme_code'], fields: ['programme_code', 'programme_name', 'degree_name', 'specialization', 'hod_id'], columns: ['programme_code', 'programme_name', 'degree_name', 'specialization', 'hod_id', 'template_name'] },
  { label: 'Departments', dataset: 'departments', keys: ['department_id'], fields: ['department_id', 'department_name'], columns: ['department_id', 'department_name'] },
  { label: 'Schools', dataset: 'schools', keys: ['school_id'], fields: ['school_id', 'school_name'], columns: ['school_id', 'school_name'] },
  { label: 'Course Registrations', dataset: 'student_course_registrations', keys: ['student_reg_no', 'course_id', 'academic_year', 'semester'], fields: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'], columns: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'] },
  { label: 'Guide Capacity', dataset: 'faculty_guide_capacity', keys: ['capacity_id'], fields: ['capacity_id', 'faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'integrated_pg_max_projects', 'status'], columns: ['faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'status'] },
  { label: 'Department Roles', dataset: 'department_roles', keys: ['id'], fields: ['id', 'dept_id', 'role_type', 'faculty_id'], columns: ['id', 'dept_id', 'role_type', 'faculty_id'] },
  { label: 'Faculty Roles', dataset: 'faculty_roles', keys: ['role_id'], fields: ['role_id', 'faculty_id', 'role_type', 'school_id', 'department_id', 'programme_code', 'status'], columns: ['faculty_id', 'role_type', 'programme_code', 'status'] },
  { label: 'Paper Assignments', dataset: 'paper_teams', keys: ['external_team_key'], fields: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status'], columns: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status', 'materialized_paper_team_id'] },
];

function canonicalRecord(config, item) { return Object.fromEntries(config.fields.filter((field) => Object.hasOwn(item, field)).map((field) => [field, item[field]])); }

async function manualInstitutionRecord(config, operation, item, reload) {
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody'); const form = element('form', 'manual-record-form'); const title = `${operation[0]}${operation.slice(1).toLowerCase()} ${config.label.replace(/s$/, '')}`; host.replaceChildren(element('h2', '', title)); const source = canonicalRecord(config, item || {}); const shownFields = operation === 'DELETE' ? config.keys : config.fields;
  shownFields.forEach((field) => { const label = element('label', '', humanLabel(field).replace(/\bId\b/g, 'ID')); let input; if (field === 'is_leader') { input = document.createElement('select'); input.append(new Option('False', 'false'), new Option('True', 'true')); } else { input = document.createElement('input'); input.type = field === 'email' ? 'email' : 'text'; } input.name = field; input.required = operation !== 'DELETE' && ['department_name', 'school_name', 'programme_name', 'degree_name'].includes(field); input.value = source[field] ?? ''; input.readOnly = operation !== 'ADD' && config.keys.includes(field); label.append(input); form.append(label); }); const result = element('div'); const submit = element('button', operation === 'DELETE' ? 'danger' : 'primary', operation === 'DELETE' ? 'Check dependencies' : `Review ${operation.toLowerCase()}`); submit.type = 'submit'; form.append(submit, result); host.append(form); openAdminDialog(title);
  form.addEventListener('submit', async (event) => { event.preventDefault(); try { const payload = {}; new FormData(form).forEach((value, key) => { if (key === 'specialization' && value === '') return; payload[key] = value === '' ? null : value; }); const detail = await api(`/api/admin/v2/institution/data/${config.dataset}/validate`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ operation, payload }) }); if (detail.batch.error_rows) { result.replaceChildren(element('p', 'danger-box', detail.issues.map((issue) => issue.problem).join('; '))); submit.disabled = true; return; } const question = operation === 'DELETE' ? `Delete this ${config.label.replace(/s$/, '')}? LaTeX Core users and paper history are retained.` : `${title}?`; if (!confirm(question)) return; const applied = await api(`/api/admin/v2/institution/import-batches/${detail.batch.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); dialog.close(); announce(`${title} complete.`); await reload(); if (operation === 'ADD') showCredentialHandoff(applied.account_provisioning, detail.batch.id); } catch (error) { result.replaceChildren(element('p', 'danger-box', error.message)); } });
}

async function renderIdentityLinkManager(host) {
  const state = { page: 1, limit: 50, search: '', external_type: '' }; const filters = filterForm([{ name: 'search', label: 'External ID, name, or email' }, { name: 'external_type', label: 'Identity type', type: 'select', options: [['Student', 'STUDENT'], ['Faculty', 'FACULTY'], ['Admin', 'ADMIN']] }], (values) => { Object.assign(state, values, { page: 1 }); load(); }); const pageHost = element('div'); host.replaceChildren(filters, pageHost);
  const load = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/identity-links?${queryString(state)}`); const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Type</th><th>External ID</th><th>Name / email</th><th>V2 account</th><th>Status</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody'); data.items.forEach((item) => { const row = document.createElement('tr'); const actions = element('td'); if (item.status === 'LINKED') actions.append(buttonAction('Unlink', async () => { if (!confirm(`Unlink ${item.external_type} ${item.external_id}? The V2 account is retained.`)) return; await api(`/api/admin/v2/institution/identity-links/${encodeURIComponent(item.external_type)}/${encodeURIComponent(item.external_id)}`, { method: 'DELETE' }); await load(); }, 'danger')); else actions.append(buttonAction('Manual link', async () => { const q = prompt('Search an existing compatible V2 account by email:'); if (!q) return; const role = item.external_type === 'STUDENT' ? 'writer' : item.external_type === 'FACULTY' ? 'mentor' : ''; const users = await searchPeople(role, q); if (!users.length) throw new Error('No compatible V2 account found.'); await api('/api/admin/v2/institution/identity-links', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ external_type: item.external_type, external_id: item.external_id, user_id: users[0].user_id }) }); await load(); })); row.append(element('td', '', item.external_type), element('td', '', item.external_id), element('td', '', `${item.name || '—'} · ${item.email || '—'}`), element('td', '', item.v2_account || '—'), element('td', '', `${item.status} · ${item.match_method || 'none'}`), actions); body.append(row); }); table.append(body); wrap.append(table); pageHost.replaceChildren(wrap, pager(data, load)); }; await load();
}

async function renderInstitutionData(options = {}) {
  let active = institutionDatasets.find((config) => config.label === options.tab) || institutionDatasets[0]; const tabsHost = element('div', 'admin-tabs'); const host = element('div'); institutionDatasets.forEach((config) => tabsHost.append(buttonAction(config.label, () => load(config)))); const identities = element('details', 'admin-section'); identities.append(element('summary', '', 'Identity Links')); const identityHost = element('div'); identities.append(identityHost); let identitiesLoaded = false; identities.addEventListener('toggle', () => { if (identities.open && !identitiesLoaded) { identitiesLoaded = true; renderIdentityLinkManager(identityHost).catch(showError); } }); const exportAll = buttonAction('Export all institution data (Excel)', exportInstitutionData, 'primary'); exportAll.title = 'Download every dataset as one import-compatible workbook'; const exportBar = element('div', 'admin-toolbar'); exportBar.append(exportAll); content.append(element('p', 'muted-note', 'Search stays server-side. Add, Edit, and Delete use the same validation and dependency rules as file imports.'), exportBar, tabsHost, host, identities);
  const load = async (config = active) => { active = config; [...tabsHost.children].forEach((button) => button.setAttribute('aria-current', button.textContent === config.label ? 'page' : 'false')); const state = { page: 1, limit: 50, search: '', programme_code: '', department_id: '', link_status: '' }; const filterFields = [{ name: 'search', label: `Search ${config.label}` }, ...(config.dataset === 'students' ? [{ name: 'programme_code', label: 'Programme' }] : []), ...(config.dataset === 'faculty' ? [{ name: 'department_id', label: 'Department UUID' }] : []), ...(['students', 'faculty'].includes(config.dataset) ? [{ name: 'link_status', label: 'Link status', type: 'select', options: ['LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE'].map((value) => [value]) }] : [])]; const filters = filterForm(filterFields, (values) => { Object.assign(state, values, { page: 1 }); pageLoad(); }); const add = buttonAction(`+ Add ${config.label.replace(/s$/, '')}`, () => manualInstitutionRecord(config, 'ADD', {}, pageLoad), 'primary'); const pageHost = element('div'); host.replaceChildren(element('div', 'admin-toolbar'), filters, pageHost); host.firstChild.append(add);
    const pageLoad = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/data/${config.dataset}?${queryString(state)}`); if (!data.items.length) { pageHost.replaceChildren(element('p', 'empty-copy', `No ${config.label.toLowerCase()} records.`), pager(data, pageLoad)); return; } const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = `<thead><tr>${config.columns.map((field) => `<th>${humanLabel(field).replace(/\bId\b/g, 'ID')}</th>`).join('')}<th>Actions</th></tr></thead>`; const body = document.createElement('tbody'); data.items.forEach((item) => { const row = document.createElement('tr'); config.columns.forEach((field) => { let value = item[field]; if (field === 'materialized_paper_team_id') value = value ? `Materialized → ${value}` : 'Not materialized'; row.append(element('td', '', value ?? '—')); }); const actions = element('td'); actions.append(buttonAction('Edit', () => manualInstitutionRecord(config, 'EDIT', item, pageLoad)), buttonAction('Delete', () => manualInstitutionRecord(config, 'DELETE', item, pageLoad), 'danger')); if (config.dataset === 'paper_teams') { const writerConfig = { label: 'Paper Team Writers', dataset: 'paper_team_writers', keys: ['external_team_key', 'student_reg_no'], fields: ['external_team_key', 'student_reg_no', 'writer_order', 'is_leader'] }; const mentorConfig = { label: 'Paper Team Mentors', dataset: 'paper_team_mentors', keys: ['external_team_key', 'faculty_id'], fields: ['external_team_key', 'faculty_id'] }; const assignment = element('details'); const writerList = element('div'); (item.writers || []).forEach((writer) => { const entry = element('p', '', `${writer.writer_order}. ${writer.student_reg_no}${writer.is_leader ? ' (Leader)' : ''} `); const values = { external_team_key: item.external_team_key, ...writer }; entry.append(buttonAction('Edit', () => manualInstitutionRecord(writerConfig, 'EDIT', values, pageLoad)), buttonAction('Remove', () => manualInstitutionRecord(writerConfig, 'DELETE', values, pageLoad), 'danger')); writerList.append(entry); }); const mentorList = element('div'); (item.mentors || []).forEach((mentor) => { const entry = element('p', '', `${mentor.faculty_id} `); const values = { external_team_key: item.external_team_key, ...mentor }; entry.append(buttonAction('Remove', () => manualInstitutionRecord(mentorConfig, 'DELETE', values, pageLoad), 'danger')); mentorList.append(entry); }); assignment.append(element('summary', '', 'Writers and Mentors'), writerList, mentorList, buttonAction('Add Writer', () => manualInstitutionRecord(writerConfig, 'ADD', { external_team_key: item.external_team_key }, pageLoad)), buttonAction('Add Mentor', () => manualInstitutionRecord(mentorConfig, 'ADD', { external_team_key: item.external_team_key }, pageLoad))); actions.prepend(assignment); } row.append(actions); body.append(row); }); table.append(body); wrap.append(table); pageHost.replaceChildren(wrap, pager(data, pageLoad)); };
    await pageLoad(); };
  await load(active);
}

async function renderAutomaticDefaults(templates, frontMatterPacks) {
  const [mappings, fallback] = await Promise.all([api('/api/admin/v2/institution/template-defaults/programmes'), api('/api/admin/v2/institution/template-defaults/global-fallback')]); const heading = element('h2', 'admin-section-anchor', 'Automatic defaults'); heading.id = 'automatic-defaults'; content.append(heading, element('p', 'muted-note', 'Defaults affect future Teams only. Existing template pins remain unchanged.'));
  const fallbackSection = element('section', 'admin-section'); fallbackSection.append(element('h3', '', 'Global fallback')); const fallbackSelect = document.createElement('select'); templates.forEach((item) => fallbackSelect.append(new Option(item.name, item.id, false, item.id === fallback.template_id))); const fallbackFrontMatter = document.createElement('select'); fallbackFrontMatter.append(new Option('None', '')); frontMatterPacks.forEach((item) => fallbackFrontMatter.append(new Option(item.name, item.id, false, item.id === fallback.front_matter_pack_id))); fallbackSection.append(element('span', '', 'Main template: '), fallbackSelect, buttonAction('Save main fallback', async () => { if (!fallbackSelect.value) throw new Error('Select a template.'); await api('/api/admin/v2/institution/template-defaults/global-fallback', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: fallbackSelect.value }) }); announce('Global Main template fallback saved for future Teams.'); await showSection('Templates'); }, 'primary'), element('span', '', 'Front Matter: '), fallbackFrontMatter, buttonAction('Save Front Matter fallback', async () => { await api('/api/admin/v2/institution/template-defaults/global-front-matter', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ front_matter_pack_id: fallbackFrontMatter.value || null }) }); announce('Global Front Matter fallback saved for future Teams.'); await showSection('Templates'); }, 'primary')); content.append(fallbackSection);
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Programme</th><th>Main Content Template</th><th>Front Matter Pack</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody'); mappings.forEach((mapping) => { const row = document.createElement('tr'); const select = document.createElement('select'); select.append(new Option('No programme default', '')); templates.forEach((item) => select.append(new Option(item.name, item.id, false, item.id === mapping.template_id))); const frontSelect = document.createElement('select'); frontSelect.append(new Option('None', '')); frontMatterPacks.forEach((item) => frontSelect.append(new Option(item.name, item.id, false, item.id === mapping.front_matter_pack_id))); const actions = element('td'); actions.append(buttonAction('Save', async () => { if (select.value) await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: select.value }) }); await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}/front-matter`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ front_matter_pack_id: frontSelect.value || null }) }); announce(`${mapping.programme_code} defaults saved for future Teams.`); await showSection('Templates'); }), buttonAction('Remove mapping', async () => { if ((!mapping.template_id && !mapping.front_matter_pack_id) || !confirm(`Remove ${mapping.programme_code} defaults?`)) return; await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'DELETE' }); await showSection('Templates'); }, 'danger')); const templateCell = element('td'); templateCell.append(select); const frontCell = element('td'); frontCell.append(frontSelect); row.append(element('td', '', mapping.programme_code), templateCell, frontCell, actions); body.append(row); }); table.append(body); wrap.append(table); content.append(wrap);
}

function buttonAction(label, handler, className = '') {
  const action = element('button', className, label); action.type = 'button'; let running = false;
  // Ignore repeated clicks while the previous request is still running (for example a second "Add records").
  action.addEventListener('click', () => { if (running) return; running = true; action.setAttribute('aria-busy', 'true'); Promise.resolve().then(handler).catch(showError).finally(() => { running = false; action.removeAttribute('aria-busy'); }); }); return action;
}

async function renderFilePolicies(teams) {
  const descriptions = {
    EDITABLE: 'Writers can change content and file structure.',
    CONTENT_READ_ONLY: 'Content is visible but cannot be edited.',
    STRUCTURE_LOCKED: 'Content can change, but the file cannot be moved or removed.',
    TEMPLATE_MANAGED: 'Safe template updates manage this file.',
    HIDDEN_SYSTEM: 'Reserved system content, hidden from ordinary lists.',
  };
  const guide = element('div', 'policy-guide'); Object.entries(descriptions).forEach(([policy, description]) => { const item = element('div'); item.append(element('strong', '', humanLabel(policy)), element('small', '', description)); guide.append(item); }); content.append(guide);
  const selector = document.createElement('select'); selector.setAttribute('aria-label', 'Paper Team');
  selector.append(new Option('All Teams', 'ALL_TEAMS'));
  teams.forEach((team) => selector.append(new Option(team.name, team.id)));
  const tableHost = element('div', 'admin-table-wrap'); content.append(selector, tableHost);
  let loadRequest = 0;
  const load = async () => {
    const request = ++loadRequest; const selectedTeam = selector.value;
    tableHost.replaceChildren(element('p', 'muted-note', 'Loading file policies…'));
    const allTeams = selector.value === 'ALL_TEAMS';
    if (allTeams) {
      const overview = await api('/api/admin/v2/file-policies');
      if (request !== loadRequest) return;
      const files = overview.files.filter((file) => file.policy !== 'HIDDEN_SYSTEM' && !file.path.startsWith('.latex-core/frontmatter/'));
      const changed = new Map();
      const count = element('p', 'muted-note', `${overview.team_count} Teams affected. Only selected existing file paths are changed; other policies stay as they are.`);
      if (!overview.team_count || !files.length) {
        tableHost.replaceChildren(count, element('p', 'empty-copy', 'No ordinary Team files are available.')); return;
      }
      const table = element('table', 'admin-table');
      table.innerHTML = '<thead><tr><th>File</th><th>Current policy</th><th>Teams with file</th><th>Change</th></tr></thead>';
      const body = document.createElement('tbody');
      const apply = buttonAction('Apply policies to All Teams', async () => {
        if (!changed.size || !confirm(`Apply these file policies to all ${overview.team_count} Teams? Only Teams containing the selected file paths are affected.`)) return;
        apply.disabled = true; selector.disabled = true;
        try {
          const result = await api('/api/admin/v2/file-policies', { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ expected_team_count: overview.team_count, confirmed: true, changes: [...changed].map(([path, policy]) => ({ path, policy })) }) });
          const exclusions = (result.excluded_files || []).map((item) => `${teams.find((team) => team.id === item.paper_team_id)?.name || item.paper_team_id}: ${item.path} (${item.reason})`);
          announce(`Updated ${result.updated_files.length} file policies in ${result.affected_team_count} of ${result.team_count} Teams.${exclusions.length ? ` Excluded: ${exclusions.join('; ')}` : ''}`);
          await load();
        } finally { selector.disabled = false; apply.disabled = !changed.size; }
      }, 'primary');
      apply.disabled = true;
      files.forEach((file) => {
        const row = document.createElement('tr'); const select = document.createElement('select');
        select.append(new Option('Keep unchanged', ''));
        ['EDITABLE', 'CONTENT_READ_ONLY', 'STRUCTURE_LOCKED', 'TEMPLATE_MANAGED'].forEach((policy) => select.append(new Option(humanLabel(policy), policy)));
        select.setAttribute('aria-label', `Change policy for ${file.path} across All Teams`);
        select.addEventListener('change', () => { if (select.value) changed.set(file.path, select.value); else changed.delete(file.path); apply.disabled = !changed.size; });
        const changeCell = element('td'); changeCell.append(select);
        row.append(element('td', '', file.path), element('td', '', file.policy === null ? 'Mixed / Varies' : humanLabel(file.policy)), element('td', '', `${file.team_count} of ${overview.team_count}`), changeCell); body.append(row);
      });
      table.append(body); tableHost.replaceChildren(count, table, apply); return;
    }
    const files = await api(`/api/admin/v2/paper-teams/${selectedTeam}/file-policies`);
    if (request !== loadRequest) return;
    const visibleFiles = files.filter((file) => file.policy !== 'HIDDEN_SYSTEM');
    if (!visibleFiles.length) { tableHost.replaceChildren(element('p', 'empty-copy', 'No ordinary Team files are available.')); return; }
    const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>File</th><th>Current policy</th><th>Change</th></tr></thead>';
    const body = document.createElement('tbody');
    visibleFiles.forEach((file) => {
      const row = document.createElement('tr'); const identity = element('td', '', file.path);
      const select = document.createElement('select');
      ['EDITABLE', 'CONTENT_READ_ONLY', 'STRUCTURE_LOCKED', 'TEMPLATE_MANAGED'].forEach((policy) => { const option = new Option(humanLabel(policy), policy); option.selected = file.policy === policy; select.append(option); });
      select.setAttribute('aria-label', `Change policy for ${file.path}`);
      select.addEventListener('change', async () => { try { await api(`/api/admin/v2/paper-teams/${selectedTeam}/file-policies/${file.file_id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ policy: select.value }) }); announce(`Policy updated for ${file.path}. Active rooms were revalidated.`); } catch (error) { showError(error); } });
      const changeCell = document.createElement('td'); changeCell.append(select); row.append(identity, element('td', '', humanLabel(file.policy)), changeCell); body.append(row);
    }); table.append(body); tableHost.replaceChildren(table);
  };
  selector.addEventListener('change', () => load().catch(showError)); await load();
}

function templateTabs() {
  const tabs = element('nav', 'admin-tabs'); tabs.setAttribute('aria-label', 'Template sections');
  [['Main templates', 'main-templates'], ['Front Matter', 'front-matter'], ['Automatic defaults', 'automatic-defaults']].forEach(([label, id], index) => {
    const action = buttonAction(label, () => document.querySelector(`#${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' }));
    if (index === 0) action.setAttribute('aria-current', 'page'); tabs.append(action);
  });
  return tabs;
}

async function showSection(section, options = {}) {
  setActive(section);
  content.replaceChildren(element('h1', '', section), element('p', 'empty-copy', `Loading ${section}…`));
  content.focus();
  try {
    if (section === 'V2 Users') {
      const users = await api('/api/admin/v2/users');
      content.replaceChildren(element('h1', '', 'V2 Users'));
      renderV2Users(users);
      return;
    }
    if (section === 'Paper Teams') {
      content.replaceChildren(element('h1', '', 'Paper Teams'));
      await renderPaperTeamGrid();
      return;
    }
    if (section === 'Imports') {
      content.replaceChildren(element('h1', '', 'Imports'));
      await renderImports(options);
      return;
    }
    if (section === 'Institution Data') {
      content.replaceChildren(element('h1', '', 'Institution Data'));
      await renderInstitutionData(options);
      return;
    }
    if (section === 'Templates') {
      const [templates, frontMatterPacks] = await Promise.all([api('/api/admin/templates'), api('/api/admin/v2/front-matter-packs')]);
      content.replaceChildren(element('h1', '', 'Templates'), templateTabs());
      await renderTemplates(templates, frontMatterPacks);
      return;
    }
    if (section === 'File Policies') {
      const teams = await api('/api/admin/v2/paper-teams'); content.replaceChildren(element('h1', '', 'File Policies')); await renderFilePolicies(teams);
      return;
    }
    if (section === 'System') {
      const [data, overview, branding] = await Promise.all([api(endpoints.System), api(endpoints.Overview), api('/api/admin/v2/branding')]);
      content.replaceChildren(element('h1', '', 'System')); renderSystem(data, overview, branding); return;
    }
    if (section === 'Runtime Logs') {
      content.replaceChildren(element('h1', '', 'Runtime Logs'));
      await renderRuntimeLogs();
      return;
    }
    const data = await api(endpoints[section]);
    content.replaceChildren(element('h1', '', section));
    if (section === 'Overview') renderOverview(data);
    else if (section === 'Reviews') renderAdminReviews(data);
    else if (section === 'Versions') renderVersions(data);
    else if (section === 'Build Queue') renderBuildQueue(data);
    else if (section === 'Audit') renderAudit(data);
    else if (Array.isArray(data) && data.length === 0) content.append(element('p', 'empty-copy', `No ${section.toLowerCase()} records are available.`));
    else content.append(renderJson(data));
  } catch (error) {
    content.replaceChildren(element('h1', '', section), element('p', 'danger-box', error.message || `Unable to load ${section}. Retry or check System status.`), buttonAction('Retry', () => showSection(section, options), 'primary'));
  }
}

nav.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-section]:not(:disabled)');
  if (button) showSection(button.dataset.section);
});

async function loadAdminIdentity() {
  let identity;
  try { identity = await api('/api/v2/me'); }
  catch { identity = await api('/api/auth/me'); }
  document.querySelector('#adminAccountName').textContent = identity.display_name || identity.email;
  const branding = await api('/api/branding');
  if (branding.logo_url) {
    const brand = document.querySelector('.branded-wordmark'); const image = brand.querySelector('.header-logo'); const fallback = brand.querySelector('span');
    image.src = branding.logo_url; image.hidden = false; fallback.hidden = true; image.onerror = () => { image.hidden = true; fallback.hidden = false; };
  }
}
loadAdminIdentity().catch(() => { document.querySelector('#adminAccountName').textContent = 'Admin'; });
showSection('Overview');
