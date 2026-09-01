import { api } from '/static/api.js?v=control-plane-groups-1';

const nav = document.querySelector('#adminNav');
const content = document.querySelector('#adminContent');
const status = document.querySelector('#adminStatus');

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

function announce(message, failed = false) {
  status.textContent = message;
  status.classList.toggle('danger', failed);
}

function showError(error) {
  announce(error.message || 'The administrative request failed.', true);
}

function setActive(section) {
  nav.querySelectorAll('button[data-section]').forEach((button) => {
    if (button.dataset.section === section) button.setAttribute('aria-current', 'page');
    else button.removeAttribute('aria-current');
  });
}

function renderJson(value) {
  const pre = element('pre', 'admin-data');
  pre.textContent = JSON.stringify(value, null, 2);
  return pre;
}

function renderOverview(data) {
  const grid = element('div', 'admin-metrics');
  Object.entries(data).forEach(([key, value]) => {
    const metric = element('div', 'admin-metric');
    metric.append(element('strong', '', String(value)), element('span', '', key.replaceAll('_', ' ')));
    grid.append(metric);
  });
  content.append(grid);
}

function renderAdminReviews(reviews) {
  if (!reviews.length) return content.append(element('p', 'empty-copy', 'No review activity yet.'));
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Paper Team</th><th>Type</th><th>State</th><th>Severity</th><th>Mentor</th><th>Assigned Writer</th><th>Updated</th></tr></thead>';
  const body = document.createElement('tbody');
  reviews.forEach((review) => {
    const row = document.createElement('tr');
    row.append(
      element('td', '', review.paper_name), element('td', '', review.thread_type.replaceAll('_', ' ')),
      element('td', '', review.state), element('td', '', review.severity), element('td', '', review.mentor),
      element('td', '', review.assigned_writer || 'Unassigned'),
      element('td', '', new Date(review.updated_at || review.created_at).toLocaleString()),
    );
    body.append(row);
  });
  table.append(body); wrap.append(table); content.append(wrap);
}

function renderTemplates(templates) {
  const intro = element('p', 'empty-copy', 'Import a bounded local ZIP, inspect its safe file tree, select Main when detection is ambiguous, then save an immutable template. Existing-Team changes use the separate conflict-safe preview workflow.');
  const form = element('form', 'admin-template-form');
  form.innerHTML = '<label>Name<input name="name" required maxlength="200"></label><label>Description (optional)<input name="description" maxlength="2000"></label><label>Template ZIP<input name="archive" type="file" accept=".zip,application/zip" required></label><label>Main .tex file<select name="main" required disabled><option value="">Validate a ZIP first…</option></select></label><button type="button" data-action="validate-template">Validate</button><button class="primary" type="submit" disabled>Import Template</button>';
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
    const heading = element('strong', '', `${preview.files.length} safe files`);
    const tree = element('ul', 'template-file-tree');
    preview.files.forEach((entry) => tree.append(element('li', '', `${entry.path} · ${entry.size_bytes} bytes`)));
    const hint = element('p', 'empty-copy', preview.detected_main ? `Detected Main: ${preview.detected_main}` : 'Main is ambiguous; choose one TeX file.');
    previewHost.replaceChildren(heading, hint, tree);
  };
  archiveInput.addEventListener('change', () => { preview = null; mainSelect.disabled = true; submit.disabled = true; previewHost.replaceChildren(); });
  validate.addEventListener('click', () => previewArchive().catch(showError));
  mainSelect.addEventListener('change', () => { submit.disabled = !preview || !mainSelect.value; });
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
      if (!preview) await previewArchive();
      if (!mainSelect.value) throw new Error('Select the Main TeX file.');
      const body = new FormData();
      body.set('name', form.elements.name.value); body.set('description', form.elements.description.value);
      body.set('main', mainSelect.value); body.set('archive', archiveInput.files[0]);
      const imported = await api('/api/admin/v2/templates/import', {
        method: 'POST', body,
      });
      announce(`Imported ${imported.name}: ${imported.files.length} immutable files · ${imported.source_identity}`);
      await showSection('Templates');
    } catch (error) { showError(error); }
  });
  content.append(intro, form, previewHost);
  if (!templates.length) return content.append(element('p', 'empty-copy', 'No templates yet.'));
  const wrap = element('div', 'admin-table-wrap');
  const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Name / description</th><th>Main file</th><th>Imported</th><th>Usage</th><th>Actions</th></tr></thead>';
  const body = document.createElement('tbody');
  templates.forEach((template) => {
    const row = document.createElement('tr');
    const metadata = element('td');
    const name = document.createElement('input'); name.value = template.name; name.maxLength = 200; name.setAttribute('aria-label', `Name for ${template.name}`);
    const description = document.createElement('input'); description.value = template.description || ''; description.maxLength = 2000; description.placeholder = 'Optional description'; description.setAttribute('aria-label', `Description for ${template.name}`);
    metadata.append(name, description);
    const mainCell = element('td'); const main = document.createElement('select'); main.setAttribute('aria-label', `Main file for ${template.name}`);
    template.tex_files.forEach((path) => main.append(new Option(path, path, false, path === template.main_file))); mainCell.append(main);
    const usage = template.pinned ? `${template.usage_count} Paper Team pin${template.usage_count === 1 ? '' : 's'}` : 'Unused';
    const actions = element('td');
    actions.append(
      buttonAction('Edit', async () => {
        await api(`/api/admin/v2/templates/${template.id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: name.value, description: description.value || null, main_file: main.value }) });
        announce(`Updated ${name.value}. Existing Paper Team sources were not changed.`); await showSection('Templates');
      }),
      buttonAction('Remove', async () => {
        if (!confirm(`Remove template “${template.name}”?`)) return;
        await api(`/api/admin/v2/templates/${template.id}`, { method: 'DELETE' }); announce(`Removed ${template.name}.`); await showSection('Templates');
      }, 'danger'),
    );
    row.append(metadata, mainCell, element('td', '', new Date(template.created_at).toLocaleString()), element('td', '', usage), actions);
    body.append(row);
  });
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
  const intro = element('p', 'empty-copy', 'Writer, Mentor, and Admin are mutually exclusive global V2 roles. Institutional compatibility fields do not authorize V2 access.');
  const form = element('form', 'admin-user-form');
  form.innerHTML = '<label>Email<input name="email" type="email" required></label><label>Password<input name="password" type="password" minlength="12" maxlength="256" required></label><label>Exclusive V2 role<select name="role"><option value="writer">Writer</option><option value="mentor">Mentor</option><option value="admin">Admin</option></select></label><button class="primary" type="submit">Create V2 user</button>';
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
      await api('/api/admin/v2/users', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(Object.fromEntries(new FormData(form))),
      });
      announce('V2 user created.');
      await showSection('V2 Users');
    } catch (error) { showError(error); }
  });
  const wrap = element('div', 'admin-table-wrap');
  const table = element('table', 'admin-table');
  table.innerHTML = '<thead><tr><th>Email</th><th>Legacy type</th><th>Exclusive V2 role</th><th>Migration</th><th>Status</th><th>Created</th></tr></thead>';
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
    const roleCell = document.createElement('td');
    roleCell.append(select);
    row.append(
      element('td', '', user.email),
      element('td', '', user.legacy_account_type),
      roleCell,
      element('td', '', user.migration_state),
      element('td', '', user.enabled ? 'enabled' : 'disabled'),
      element('td', '', user.created_at),
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
  fields.forEach(({ name, label, type = 'text', options = [] }) => {
    const field = element('label', '', label); let input;
    if (type === 'select') { input = document.createElement('select'); input.append(new Option('All', '')); options.forEach(([text, value = text]) => input.append(new Option(text, value))); }
    else { input = document.createElement('input'); input.type = type; }
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
  const section = element('section', 'admin-section'); section.append(element('h2', '', ordered ? 'Ordered Writers' : 'Mentors'));
  const search = element('input'); search.type = 'search'; search.placeholder = `Search V2 ${role} email`; search.setAttribute('aria-label', `Search V2 ${role}s`);
  const results = document.createElement('select'); results.setAttribute('aria-label', `${role} search results`); results.append(new Option('Search to find accounts…', ''));
  let timer;
  search.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(async () => { try { const users = await searchPeople(role, search.value); results.replaceChildren(new Option('Select an account…', '')); users.forEach((user) => results.append(new Option(user.email, user.user_id))); } catch (error) { showError(error); } }, 250); });
  const selected = []; const host = element('div', 'ordered-people');
  const draw = () => {
    host.replaceChildren(); selected.forEach((person, index) => {
      const row = element('div', 'ordered-person'); row.append(element('strong', '', ordered ? `${index + 1}.` : '•'), element('span', '', person.email));
      const up = buttonAction('Up', () => { if (index) [selected[index - 1], selected[index]] = [selected[index], selected[index - 1]]; draw(); changed(selected); }); up.disabled = !ordered || index === 0; up.title = 'Move up'; up.setAttribute('aria-label', `Move ${person.email} up`);
      const remove = buttonAction('Remove', () => { selected.splice(index, 1); draw(); changed(selected); }, 'danger'); remove.title = 'Remove account'; remove.setAttribute('aria-label', `Remove ${person.email}`);
      row.append(up, remove); host.append(row);
    });
  };
  const add = buttonAction('Add', () => { const option = results.selectedOptions[0]; if (!option?.value || selected.some((person) => person.user_id === option.value)) return; selected.push({ user_id: option.value, email: option.textContent }); draw(); changed(selected); });
  section.append(search, results, add, host);
  return { section, selected, draw };
}

async function renderManualTeamForm(templates) {
  const details = element('details', 'admin-section'); const summary = element('summary', '', 'Create Team Manually'); details.append(summary);
  const form = element('form', 'manual-team-form'); form.innerHTML = '<label>Team name<input name="name" required maxlength="200"></label>';
  const leader = element('label', '', 'Leader'); const leaderSelect = document.createElement('select'); leaderSelect.required = true; leader.append(leaderSelect);
  const template = element('label', '', 'Template'); const templateSelect = document.createElement('select'); templateSelect.append(new Option('Use suggested template automatically', '')); templates.forEach((item) => templateSelect.append(new Option(`Override: ${item.name}`, item.id))); template.append(templateSelect);
  const suggestion = element('p', 'muted-note', 'Add Writers to resolve the programme default.');
  let timer;
  const writers = peopleBuilder('writer', true, (people) => {
    const previous = leaderSelect.value; leaderSelect.replaceChildren(new Option('Select one of the ordered Writers…', ''));
    people.forEach((person) => leaderSelect.append(new Option(person.email, person.user_id))); if (people.some((person) => person.user_id === previous)) leaderSelect.value = previous;
    clearTimeout(timer); timer = setTimeout(async () => { if (!people.length) return; try { const preview = await api('/api/admin/v2/institution/template-defaults/resolve-preview', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ordered_writer_user_ids: people.map((person) => person.user_id) }) }); const found = templates.find((item) => item.id === preview.selected_template_id); suggestion.textContent = `Suggested: ${found?.name || preview.selected_template_id} · ${preview.dominant_programme_code || 'no linked programme'} · ${preview.resolution_method}${preview.warnings.length ? ` · ${preview.warnings.join('; ')}` : ''}`; } catch (error) { suggestion.textContent = error.message; } }, 300);
  });
  const mentors = peopleBuilder('mentor', false, () => {});
  form.append(leader, template, suggestion);
  const submit = element('button', 'primary', 'Create Paper Team'); submit.type = 'submit'; form.append(submit);
  form.addEventListener('submit', async (event) => { event.preventDefault(); if (!writers.selected.some((person) => person.user_id === leaderSelect.value)) return showError(new Error('Leader must be one of the selected Writers.')); try { await api('/api/admin/v2/paper-teams', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: form.elements.name.value, writer_ids: writers.selected.map((person) => person.user_id), leader_writer_id: leaderSelect.value, mentor_ids: mentors.selected.map((person) => person.user_id), template_id: templateSelect.value || null }) }); announce(`Paper Team created with ${templateSelect.value ? 'MANUAL_OVERRIDE' : 'automatic template resolution'}.`); await showSection('Paper Teams'); } catch (error) { showError(error); } });
  details.append(writers.section, mentors.section, form); content.append(details);
}

async function openTeamDetail(team, templates) {
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody');
  if (team.unresolved) {
    host.replaceChildren(element('h2', '', `${team.name} · unresolved`), element('p', 'danger-box', `${team.error_code || 'TEAM_UNRESOLVED'}: ${team.error_message || 'Resolve linked identities and template defaults, then re-apply the source import job.'}`), element('p', '', `Students: ${(team.unresolved_student_ids || []).join(', ') || 'None'} · Faculty: ${(team.unresolved_faculty_ids || []).join(', ') || 'None'}`));
    const links = buttonAction('Open Identity Links', () => { dialog.close(); showSection('Institution Data', { tab: 'Identity Links' }); }); const defaults = buttonAction('Open Programme Templates', () => { dialog.close(); showSection('Programme Templates'); }); const job = buttonAction('Open Import Job', () => { dialog.close(); showSection('Imports', { jobId: team.source_import_job_id }); }); host.append(element('p', 'muted-note', 'Imports are additive; missing rows do not remove existing members.'), links, defaults, job); dialog.showModal(); return;
  }
  const detail = await api(`/api/admin/v2/paper-teams/${team.id}`); const leader = detail.members.find((member) => member.is_leader); const writers = detail.members.filter((member) => member.role === 'writer'); const mentors = detail.members.filter((member) => member.role === 'mentor');
  host.replaceChildren(element('h2', '', detail.team.name), element('p', '', `${detail.team.status} · ${team.source} · updated ${new Date(detail.team.updated_at).toLocaleString()}`), element('p', '', `Writers: ${writers.map((member) => `${member.writer_order}. ${member.email}`).join(', ') || 'None'}`), element('p', '', `Leader: ${leader?.email || 'Missing'} · Mentors: ${mentors.map((member) => member.email).join(', ') || 'None'}`), element('p', '', `Dominant programme: ${detail.summary.dominant_programme_code || '—'} · Review: ${detail.summary.review_state} · Files: ${detail.summary.file_count} · Build: ${detail.summary.current_build.status || 'none'}`), element('p', '', `Team pinned template: ${detail.template_pin?.template_name || 'None'} · Team source: ${detail.summary.resolution_method || 'unrecorded'}`));
  if (team.source === 'imported') host.append(element('p', 'muted-note', `External key ${team.external_team_key} · source import ${team.source_import_job_id} · last imported ${team.last_imported_at || 'unknown'}. Imports are additive; missing rows do not remove existing members.`));
  const management = element('section', 'admin-section'); management.append(element('h2', '', 'Team management'));
  const leaderChoice = document.createElement('select'); leaderChoice.setAttribute('aria-label', 'Assigned Writer Leader'); writers.forEach((member) => leaderChoice.append(new Option(member.email, member.user_id, false, member.is_leader)));
  management.append(leaderChoice, buttonAction('Change Leader', async () => { await api(`/api/admin/v2/paper-teams/${team.id}/leader`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ user_id: leaderChoice.value }) }); announce(`Leader changed for ${team.name}.`); dialog.close(); await showSection('Paper Teams'); }));
  [['Freeze', 'frozen'], ['Activate', 'active'], ['Archive', 'archived']].forEach(([label, value]) => management.append(buttonAction(label, async () => { await api(`/api/admin/v2/paper-teams/${team.id}/status`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ status: value }) }); dialog.close(); await showSection('Paper Teams'); }, value === 'archived' ? 'danger' : ''))); host.append(management);
  const section = element('section', 'admin-section'); section.append(element('h2', '', 'Safe Template Override'));
  const select = document.createElement('select'); select.setAttribute('aria-label', 'New template'); templates.filter((item) => item.id !== detail.template_pin?.template_id).forEach((item) => select.append(new Option(item.name, item.id)));
  const confirmMain = element('label', '', ' Confirm Main file change'); const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; confirmMain.prepend(checkbox);
  const previewHost = element('div'); let preview;
  const previewButton = buttonAction('Preview Template Change', async () => { preview = await api(`/api/admin/v2/paper-teams/${team.id}/template-change/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, confirm_main_file_change: checkbox.checked }) }); previewHost.replaceChildren(renderJson(preview)); apply.disabled = !preview.can_apply; });
  const apply = buttonAction('Apply Template Change', async () => { if (!preview || !confirm('Apply this safe template change and create checkpoints?')) return; const result = await api(`/api/admin/v2/paper-teams/${team.id}/template-change/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, preview_token: preview.preview_token, confirm_main_file_change: checkbox.checked }) }); announce(`Template changed; PRE_TEMPLATE_CHANGE and TEMPLATE_UPDATE retained (${result.template_update_version_id}).`); dialog.close(); await showSection('Paper Teams'); }, 'primary'); apply.disabled = true;
  section.append(select, confirmMain, previewButton, apply, previewHost); host.append(section); dialog.showModal();
}

async function renderPaperTeamGrid() {
  const templates = await api('/api/admin/templates'); await renderManualTeamForm(templates);
  const state = { page: 1, limit: 25, search: '', status: '', programme_code: '', mentor_user_id: '', leader_user_id: '', template_id: '', review_state: '', source: '', unresolved: '' };
  const filters = filterForm([
    { name: 'search', label: 'Search' }, { name: 'status', label: 'Status', type: 'select', options: [['Active', 'active'], ['Frozen', 'frozen'], ['Submitted', 'submitted'], ['Archived', 'archived']] },
    { name: 'programme_code', label: 'Programme' }, { name: 'mentor_user_id', label: 'Mentor user ID' }, { name: 'leader_user_id', label: 'Leader user ID' },
    { name: 'template_id', label: 'Template', type: 'select', options: templates.map((item) => [item.name, item.id]) }, { name: 'review_state', label: 'Review state' }, { name: 'source', label: 'Source', type: 'select', options: [['Manual', 'manual'], ['Imported', 'imported']] }, { name: 'unresolved', label: 'Unresolved', type: 'select', options: [['Only unresolved', 'true'], ['Resolved only', 'false']] },
  ], (values) => { Object.assign(state, values, { page: 1 }); load(); });
  const pageSize = document.createElement('select'); [25, 50, 100].forEach((size) => pageSize.append(new Option(`${size} per page`, size))); pageSize.addEventListener('change', () => { state.limit = Number(pageSize.value); state.page = 1; load(); }); filters.append(pageSize); content.append(filters);
  const bulk = element('div', 'admin-toolbar'); const gridHost = element('div'); content.append(bulk, gridHost);
  const load = async (page = state.page) => { state.page = page; gridHost.replaceChildren(element('p', 'empty-copy', 'Loading requested Team page…')); const data = await api(`/api/admin/v2/paper-teams/query?${queryString(state)}`); const selected = new Set();
    const runBulk = async (statusValue) => { if (!selected.size || !confirm(`${statusValue} ${selected.size} selected Teams?`)) return; const result = await api('/api/admin/v2/paper-teams/bulk-lifecycle', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ team_ids: [...selected], status: statusValue, confirmed: true }) }); announce(`Bulk lifecycle: ${result.succeeded} succeeded, ${result.failed} failed.`, result.failed > 0); await load(); };
    bulk.replaceChildren(element('strong', '', 'Selected rows:'), buttonAction('Freeze', () => runBulk('frozen')), buttonAction('Activate', () => runBulk('active')), buttonAction('Archive', () => runBulk('archived'), 'danger'));
    if (!data.items.length) { gridHost.replaceChildren(element('p', 'empty-copy', state.unresolved === 'true' ? 'No unresolved imported Teams.' : 'No Paper Teams match these server-side filters.'), pager(data, load)); return; }
    const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Select</th><th>Team</th><th>Status</th><th>Writers</th><th>Leader</th><th>Mentors</th><th>Programme</th><th>Template / resolution</th><th>Review</th><th>Updated</th><th>Source</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody');
    data.items.forEach((team) => { const row = document.createElement('tr'); const selection = document.createElement('input'); selection.type = 'checkbox'; selection.disabled = !team.id; selection.setAttribute('aria-label', `Select ${team.name}`); selection.addEventListener('change', () => { if (selection.checked) selected.add(team.id); else selected.delete(team.id); row.classList.toggle('row-selected', selection.checked); }); const selectCell = element('td'); selectCell.append(selection); const source = team.source === 'imported' ? `Imported${team.external_team_key ? ` · ${team.external_team_key}` : ''}` : 'Manual'; const action = element('td'); action.append(buttonAction('View / Manage', () => openTeamDetail(team, templates))); row.append(selectCell, element('td', '', team.name), element('td', '', team.status), element('td', '', (team.writers || []).map((writer) => `${writer.writer_order}. ${writer.email}`).join(', ') || String(team.writer_count)), element('td', '', team.leader?.email || 'Missing'), element('td', '', (team.mentors || []).map((mentor) => mentor.email).join(', ') || 'None'), element('td', '', team.dominant_programme_code || '—'), element('td', '', `${team.template?.name || '—'} · ${team.template_resolution_method || '—'}`), element('td', '', team.review_state), element('td', '', new Date(team.updated_at).toLocaleString()), element('td', '', source), action); body.append(row); }); table.append(body); wrap.append(table); gridHost.replaceChildren(wrap, pager(data, load));
  };
  await load();
}

async function renderImportDetail(jobId) {
  const detail = await api(`/api/admin/v2/institution/imports/${jobId}`); const { job } = detail;
  const section = element('section', 'admin-section'); section.append(element('h2', '', `Validation result · ${job.id}`));
  const metrics = element('div', 'admin-metrics'); Object.entries({ filename: job.original_filename, sha_256: job.content_sha256, mode: job.mode, file_type: job.file_type, status: job.status, total_rows: job.total_rows, insert_rows: job.inserted_rows, update_rows: job.updated_rows, skip_rows: job.skipped_rows, error_rows: job.error_rows }).forEach(([label, value]) => { const metric = element('div', 'admin-metric'); metric.append(element('strong', '', value), element('span', '', label.replaceAll('_', ' '))); metrics.append(metric); }); section.append(metrics);
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Sheet / table</th><th>Total</th><th>Insert</th><th>Update</th><th>Skip</th><th>Error</th></tr></thead>'; const body = document.createElement('tbody'); detail.summaries.forEach((item) => { const row = document.createElement('tr'); row.append(element('td', '', item.source), element('td', '', item.total), element('td', '', item.insert), element('td', '', item.update), element('td', '', item.skip), element('td', item.error ? 'danger' : '', item.error)); body.append(row); }); table.append(body); wrap.append(table); section.append(wrap);
  const errors = detail.rows.filter((row) => row.status === 'ERROR' || row.status === 'UNRESOLVED'); if (errors.length) { const errorBox = element('div', 'danger-box', `${errors.length} blocking/unresolved rows shown in the bounded preview.`); const list = element('ul', 'compact-list'); errors.slice(0, 100).forEach((row) => list.append(element('li', '', `${row.source_table_or_sheet}:${row.row_number} · ${row.error_code} · ${row.error_message}`))); errorBox.append(list); section.append(errorBox); }
  const actions = element('div', 'admin-actions'); const download = element('a', 'shell-link', 'Download errors.csv'); download.href = `/api/admin/v2/institution/imports/${job.id}/errors.csv`; download.download = 'errors.csv';
  const apply = buttonAction('Apply validated import', async () => { if (!confirm(`Apply ${job.mode} import ${job.original_filename}? Imports never remove absent rows.`)) return; const result = await api(`/api/admin/v2/institution/imports/${job.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Import applied: ${result.materialized_teams} Teams created, ${result.merged_teams} merged, ${result.unresolved_teams} unresolved.`); await showSection('Imports', { jobId: job.id }); }, 'primary'); apply.disabled = job.mode === 'VALIDATE_ONLY' || job.status !== 'VALIDATED' || job.error_rows > 0;
  const retry = buttonAction('Retry Team Materialization', async () => { const result = await api(`/api/admin/v2/institution/imports/${job.id}/retry-teams`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Retry: ${result.succeeded} succeeded, ${result.failed} remain unresolved.`, result.failed > 0); await showSection('Imports', { jobId: job.id }); }); retry.disabled = job.status !== 'PARTIAL';
  actions.append(download, apply, retry); section.append(actions); content.append(section);
}

async function renderImports(options = {}) {
  const steps = element('div', 'wizard-steps'); ['Select file and mode', 'Validate', 'Review results', 'Apply'].forEach((label, index) => steps.append(element('div', `wizard-step${index === 0 ? ' current' : ''}`, label))); content.append(steps);
  const form = element('form', 'import-form'); form.enctype = 'multipart/form-data'; form.innerHTML = '<label>CSV or XLSX file<input name="file" type="file" accept=".csv,.xlsx,text/csv,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" required></label><label>Import mode<select name="mode"><option value="VALIDATE_ONLY">Validate only — no canonical changes</option><option value="MERGE">Merge / Update — insert and update, never delete</option><option value="ADD_ONLY">Import More — Add Only</option></select></label>';
  const targets = ['departments', 'admins', 'faculty', 'programmes', 'schools', 'students', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles', 'paper_teams', 'paper_team_writers', 'paper_team_mentors']; const target = element('label', '', 'CSV target table'); const targetSelect = document.createElement('select'); targetSelect.name = 'target_table'; targetSelect.append(new Option('Select target…', '')); targets.forEach((name) => targetSelect.append(new Option(name, name))); target.append(targetSelect); form.append(target);
  const fileInput = form.elements.file; fileInput.addEventListener('change', () => { const file = fileInput.files[0]; const xlsx = file?.name.toLowerCase().endsWith('.xlsx'); target.hidden = xlsx; targetSelect.required = !xlsx; if (!xlsx) { const base = file?.name.replace(/\.csv$/i, ''); targetSelect.value = targets.includes(base) ? base : ''; } else targetSelect.value = ''; });
  const validate = element('button', 'primary', 'Validate upload'); validate.type = 'submit'; form.append(validate); form.addEventListener('submit', async (event) => { event.preventDefault(); try { const file = fileInput.files[0]; if (!file) throw new Error('Choose a CSV or XLSX file.'); if (file.name.toLowerCase().endsWith('.csv') && !targetSelect.value) throw new Error('Select the CSV target table; unknown filenames are never guessed.'); const body = new FormData(form); const job = await api('/api/admin/v2/institution/imports/validate', { method: 'POST', body }); announce(`Validation completed for ${job.original_filename}.`); await showSection('Imports', { jobId: job.id }); } catch (error) { showError(error); } }); content.append(form);
  if (options.jobId) await renderImportDetail(options.jobId);
  const history = element('section', 'admin-section'); history.append(element('h2', '', 'Import history')); const state = { page: 1, limit: 25, search: '', status: '', mode: '', file_type: '' }; const filters = filterForm([{ name: 'search', label: 'Filename or Job ID' }, { name: 'status', label: 'Status', type: 'select', options: ['VALIDATED', 'APPLIED', 'PARTIAL', 'FAILED'].map((value) => [value]) }, { name: 'mode', label: 'Mode', type: 'select', options: [['Validate only', 'VALIDATE_ONLY'], ['Merge', 'MERGE'], ['Import More', 'ADD_ONLY']] }, { name: 'file_type', label: 'File type', type: 'select', options: [['CSV'], ['XLSX']] }], (values) => { Object.assign(state, values, { page: 1 }); load(); }); const host = element('div'); history.append(filters, host); content.append(history);
  const load = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/imports?${queryString(state)}`); if (!data.items.length) { host.replaceChildren(element('p', 'empty-copy', 'No institution imports yet.'), pager(data, load)); return; } const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Submitted</th><th>Filename</th><th>Type</th><th>Mode</th><th>Status</th><th>Total</th><th>Inserted</th><th>Updated</th><th>Skipped</th><th>Errors</th><th>Submitted by</th></tr></thead>'; const body = document.createElement('tbody'); data.items.forEach((job) => { const row = document.createElement('tr'); row.tabIndex = 0; row.title = 'Open import detail'; row.addEventListener('click', () => showSection('Imports', { jobId: job.id })); row.append(element('td', '', new Date(job.created_at).toLocaleString()), element('td', '', job.original_filename), element('td', '', job.file_type), element('td', '', job.mode === 'ADD_ONLY' ? 'Import More' : job.mode), element('td', '', job.status), element('td', '', job.total_rows), element('td', '', job.inserted_rows), element('td', '', job.updated_rows), element('td', '', job.skipped_rows), element('td', job.error_rows ? 'danger' : '', job.error_rows), element('td', '', job.submitted_by)); body.append(row); }); table.append(body); wrap.append(table); host.replaceChildren(wrap, pager(data, load)); }; await load();
}

async function renderInstitutionData(options = {}) {
  const tabs = ['Students', 'Faculty', 'Programmes', 'Identity Links']; let active = options.tab || 'Students'; const tabsHost = element('div', 'admin-tabs'); const host = element('div'); tabs.forEach((tab) => { const button = buttonAction(tab, () => load(tab)); tabsHost.append(button); }); content.append(tabsHost, host);
  const load = async (tab = active) => { active = tab; [...tabsHost.children].forEach((button) => button.setAttribute('aria-current', button.textContent === tab ? 'page' : 'false')); host.replaceChildren(element('p', 'empty-copy', `Loading ${tab} page…`)); const state = { page: 1, limit: 50, search: '', link_status: '', programme_code: '', external_type: '' }; const filters = filterForm([{ name: 'search', label: 'Search' }, ...(tab === 'Students' ? [{ name: 'programme_code', label: 'Programme' }] : []), ...(tab !== 'Programmes' ? [{ name: 'link_status', label: 'Link status', type: 'select', options: ['LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE'].map((value) => [value]) }] : []), ...(tab === 'Identity Links' ? [{ name: 'external_type', label: 'Identity type', type: 'select', options: [['Student', 'STUDENT'], ['Faculty', 'FACULTY'], ['Admin', 'ADMIN']] }] : [])], (values) => { Object.assign(state, values, { page: 1 }); pageLoad(); }); const pageHost = element('div'); host.replaceChildren(filters, pageHost);
    const endpoint = tab === 'Identity Links' ? 'identity-links' : tab.toLowerCase(); const pageLoad = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/${endpoint}?${queryString(state)}`); if (!data.items.length) { pageHost.replaceChildren(element('p', 'empty-copy', `No institutional ${tab.toLowerCase()} records.`), pager(data, pageLoad)); return; } const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); const headers = tab === 'Students' ? ['Registration number', 'Name', 'Email', 'Programme', 'V2 account', 'Link status'] : tab === 'Faculty' ? ['Faculty ID', 'Name', 'Email', 'Department', 'Designation', 'Status', 'V2 account', 'Link status', 'Guide capacity'] : tab === 'Programmes' ? ['Programme', 'HOD', 'Students', 'Default template'] : ['Type', 'External ID', 'Name / email', 'V2 account', 'Status / method', 'Actions']; table.innerHTML = `<thead><tr>${headers.map((value) => `<th>${value}</th>`).join('')}</tr></thead>`; const body = document.createElement('tbody');
      data.items.forEach((item) => { const row = document.createElement('tr'); if (tab === 'Students') row.append(element('td', '', item.registration_number), element('td', '', item.name || '—'), element('td', '', item.email || '—'), element('td', '', item.programme_code || '—'), element('td', '', item.v2_account || '—'), element('td', '', item.link_status)); else if (tab === 'Faculty') row.append(element('td', '', item.faculty_id), element('td', '', item.name || '—'), element('td', '', item.email || '—'), element('td', '', item.department_id || '—'), element('td', '', item.designation || '—'), element('td', '', item.status || '—'), element('td', '', item.v2_account || '—'), element('td', '', item.link_status), element('td', '', item.guide_capacity.map((entry) => `${entry.academic_year || ''}: UG ${entry.ug ?? '—'} / PG ${entry.pg ?? '—'}`).join('; ') || '—')); else if (tab === 'Programmes') row.append(element('td', '', item.programme_code), element('td', '', item.hod_name || item.hod_id || '—'), element('td', '', item.student_count), element('td', '', item.template_name || '—')); else { const actions = element('td'); if (item.status === 'LINKED') actions.append(buttonAction('Unlink', async () => { if (!confirm(`Unlink ${item.external_type} ${item.external_id}?`)) return; await api(`/api/admin/v2/institution/identity-links/${encodeURIComponent(item.external_type)}/${encodeURIComponent(item.external_id)}`, { method: 'DELETE' }); announce('Identity unlinked.'); await pageLoad(); }, 'danger')); else actions.append(buttonAction('Manual link', async () => { const q = prompt('Search existing V2 account by email:'); if (!q) return; const requiredRole = item.external_type === 'STUDENT' ? 'writer' : item.external_type === 'FACULTY' ? 'mentor' : ''; const users = await searchPeople(requiredRole, q); if (!users.length) throw new Error('No compatible V2 user found.'); const chosen = users.length === 1 ? users[0] : users.find((user) => user.email === prompt(`Enter exact email:\n${users.map((user) => user.email).join('\n')}`)); if (!chosen) throw new Error('Select an exact account from the server search results.'); await api('/api/admin/v2/institution/identity-links', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ external_type: item.external_type, external_id: item.external_id, user_id: chosen.user_id }) }); announce('Manual identity link saved. Imported institutional Admin linkage grants no V2 role.'); await pageLoad(); })); row.append(element('td', '', item.external_type), element('td', '', item.external_id), element('td', '', `${item.name || '—'} · ${item.email || '—'}`), element('td', '', item.v2_account || '—'), element('td', '', `${item.status} · ${item.match_method || 'none'}`), actions); } body.append(row); }); table.append(body); wrap.append(table); pageHost.replaceChildren(wrap, pager(data, pageLoad)); }; await pageLoad(); };
  await load(active);
}

async function renderProgrammeTemplates() {
  const [mappings, fallback, templates] = await Promise.all([api('/api/admin/v2/institution/template-defaults/programmes'), api('/api/admin/v2/institution/template-defaults/global-fallback'), api('/api/admin/templates')]); content.append(element('p', 'muted-note', 'Changing a programme default or global fallback affects future Teams only. Existing pinned Teams never change.'));
  const fallbackSection = element('section', 'admin-section'); fallbackSection.append(element('h2', '', 'Global fallback'), element('p', '', `Current fallback: ${fallback.template_name || 'Not configured'}`)); const fallbackSelect = document.createElement('select'); templates.forEach((item) => fallbackSelect.append(new Option(item.name, item.id, false, item.id === fallback.template_id))); fallbackSection.append(fallbackSelect, buttonAction('Change global fallback', async () => { if (!confirm('Change fallback for future Teams only?')) return; await api('/api/admin/v2/institution/template-defaults/global-fallback', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: fallbackSelect.value }) }); announce('Global fallback changed for future Teams only.'); await showSection('Programme Templates'); }, 'primary')); content.append(fallbackSection);
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Programme</th><th>Students</th><th>Current default</th><th>Last updated</th><th>Updated by</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody'); mappings.forEach((mapping) => { const row = document.createElement('tr'); const select = document.createElement('select'); select.append(new Option('No programme default', '')); templates.forEach((item) => select.append(new Option(item.name, item.id, false, item.id === mapping.template_id))); const actions = element('td'); actions.append(buttonAction(mapping.template_id ? 'Change default' : 'Set default', async () => { if (!select.value) throw new Error('Select a template.'); await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: select.value }) }); announce(`${mapping.programme_code} default saved for future Teams.`); await showSection('Programme Templates'); }), buttonAction('Remove mapping', async () => { if (!mapping.template_id || !confirm(`Remove ${mapping.programme_code} default?`)) return; await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'DELETE' }); await showSection('Programme Templates'); }, 'danger')); const templateCell = element('td'); templateCell.append(select); row.append(element('td', '', mapping.programme_code), element('td', '', mapping.student_count), templateCell, element('td', '', mapping.updated_at ? new Date(mapping.updated_at).toLocaleString() : '—'), element('td', '', mapping.updated_by || '—'), actions); body.append(row); }); table.append(body); wrap.append(table); content.append(wrap);
  const preview = element('section', 'admin-section'); preview.append(element('h2', '', 'Resolution preview'), element('p', 'muted-note', 'Writer order controls tie-breaking. Preview never mutates a Team.')); const result = element('div'); const writers = peopleBuilder('writer', true, () => {}); preview.append(writers.section, buttonAction('Resolve preview', async () => { if (!writers.selected.length) throw new Error('Add at least one Writer.'); const data = await api('/api/admin/v2/institution/template-defaults/resolve-preview', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ordered_writer_user_ids: writers.selected.map((person) => person.user_id) }) }); const selected = templates.find((item) => item.id === data.selected_template_id); result.replaceChildren(element('p', '', `Writer order: ${writers.selected.map((person, index) => `${index + 1}. ${person.email}`).join(' · ')}`), element('p', '', `Programme counts: ${JSON.stringify(data.counts)} · dominant ${data.dominant_programme_code || 'none'}`), element('p', '', `Selected: ${selected?.name || data.selected_template_id} · ${data.resolution_method}${data.tie_break ? ` · ${data.tie_break}` : ''}`), element('p', 'muted-note', data.warnings.join('; ') || 'No warnings.')); }), result); content.append(preview);
}

function buttonAction(label, handler, className = '') {
  const action = element('button', className, label); action.type = 'button';
  action.addEventListener('click', () => handler().catch(showError)); return action;
}

async function renderFilePolicies(teams) {
  if (!teams.length) return content.append(element('p', 'empty-copy', 'No Paper Teams yet.'));
  const selector = document.createElement('select'); selector.setAttribute('aria-label', 'Paper Team');
  teams.forEach((team) => selector.append(new Option(team.name, team.id)));
  const tableHost = element('div', 'admin-table-wrap'); content.append(selector, tableHost);
  const load = async () => {
    const files = await api(`/api/admin/v2/paper-teams/${selector.value}/file-policies`);
    const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Path / stable file ID</th><th>Policy</th></tr></thead>';
    const body = document.createElement('tbody');
    files.forEach((file) => {
      const row = document.createElement('tr'); const identity = element('td', '', file.path); identity.append(element('small', '', ` ${file.file_id}`));
      const select = document.createElement('select');
      ['EDITABLE', 'CONTENT_READ_ONLY', 'STRUCTURE_LOCKED', 'TEMPLATE_MANAGED', 'HIDDEN_SYSTEM'].forEach((policy) => { const option = new Option(policy.replaceAll('_', ' '), policy); option.selected = file.policy === policy; select.append(option); });
      select.addEventListener('change', async () => { try { await api(`/api/admin/v2/paper-teams/${selector.value}/file-policies/${file.file_id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ policy: select.value }) }); announce(`Policy updated for ${file.path}. Active rooms were revalidated.`); } catch (error) { showError(error); } });
      const policyCell = document.createElement('td'); policyCell.append(select); row.append(identity, policyCell); body.append(row);
    }); table.append(body); tableHost.replaceChildren(table);
  };
  selector.addEventListener('change', () => load().catch(showError)); await load();
}

async function showSection(section, options = {}) {
  setActive(section);
  content.replaceChildren(element('h1', '', section.toUpperCase()), element('p', 'empty-copy', 'Loading operational data…'));
  content.focus();
  try {
    if (section === 'V2 Users') {
      const users = await api('/api/admin/v2/users');
      content.replaceChildren(element('h1', '', 'V2 USERS'));
      renderV2Users(users);
      return;
    }
    if (section === 'Paper Teams') {
      content.replaceChildren(element('h1', '', 'PAPER TEAMS'));
      await renderPaperTeamGrid();
      return;
    }
    if (section === 'Imports') {
      content.replaceChildren(element('h1', '', 'IMPORT CENTER'));
      await renderImports(options);
      return;
    }
    if (section === 'Institution Data') {
      content.replaceChildren(element('h1', '', 'INSTITUTION DATA'));
      await renderInstitutionData(options);
      return;
    }
    if (section === 'Programme Templates') {
      content.replaceChildren(element('h1', '', 'PROGRAMME TEMPLATES'));
      await renderProgrammeTemplates();
      return;
    }
    if (section === 'Templates') {
      const templates = await api('/api/admin/templates');
      content.replaceChildren(element('h1', '', 'TEMPLATES'));
      renderTemplates(templates);
      return;
    }
    if (section === 'File Policies') {
      const page = await api('/api/admin/v2/paper-teams/query?page=1&limit=100'); content.replaceChildren(element('h1', '', 'FILE POLICIES')); await renderFilePolicies(page.items);
      return;
    }
    const data = await api(endpoints[section]);
    content.replaceChildren(element('h1', '', section.toUpperCase()));
    if (section === 'Overview') renderOverview(data);
    else if (section === 'Reviews') renderAdminReviews(data);
    else if (section === 'System') { content.append(element('p', 'empty-copy', 'Host-level operational actions remain CLI-only in this release candidate.')); content.append(renderJson(data)); }
    else if (Array.isArray(data) && data.length === 0) content.append(element('p', 'empty-copy', 'No records available.'));
    else content.append(renderJson(data));
  } catch (error) {
    content.replaceChildren(element('h1', '', section.toUpperCase()), element('p', 'danger', error.message || 'Unable to load this section.'));
  }
}

nav.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-section]:not(:disabled)');
  if (button) showSection(button.dataset.section);
});

showSection('Overview');
