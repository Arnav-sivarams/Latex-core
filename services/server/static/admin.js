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

function csvCell(value) {
  const text = String(value ?? '');
  return /[",\n\r]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function credentialsCsv(credentials) {
  return ['email,password,role', ...credentials.map((item) => [item.email, item.temporary_password, item.credential_role].map(csvCell).join(','))].join('\r\n') + '\r\n';
}

function downloadCredentials(credentials, batchId) {
  const blob = new Blob([credentialsCsv(credentials)], { type: 'text/csv;charset=utf-8' });
  const url = URL.createObjectURL(blob); const link = document.createElement('a');
  link.href = url; link.download = `latex-core-generated-credentials-${batchId}.csv`;
  document.body.append(link); link.click(); link.remove(); URL.revokeObjectURL(url);
}

function showCredentialHandoff(provisioning, batchId) {
  if (!provisioning) return;
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody');
  const credentials = provisioning.credentials || [];
  host.replaceChildren(
    element('h2', '', 'Account setup'),
    element('p', '', `${provisioning.created || 0} accounts created`),
    element('p', '', `${provisioning.credential_emails_queued || 0} credential emails queued`),
    element('p', '', `${provisioning.reused || 0} existing accounts reused`),
    element('p', '', `${provisioning.needs_attention || 0} need attention`),
    element('p', 'muted-note', 'Email is the primary credential delivery method. The CSV is an administrator fallback.'),
    element('p', 'danger-box', 'Save this file now. Temporary passwords cannot be viewed again.'),
  );
  const download = buttonAction('Download generated credentials', () => downloadCredentials(credentials, batchId), 'primary');
  download.disabled = credentials.length === 0; host.append(download); dialog.showModal();
  if (credentials.length) downloadCredentials(credentials, batchId);
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

async function renderTemplates(templates) {
  content.append(element('h2', '', 'Template Library'));
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
  if (!templates.length) { content.append(element('p', 'empty-copy', 'No templates yet.')); await renderAutomaticDefaults(templates); return; }
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
  table.append(body); wrap.append(table); content.append(wrap); await renderAutomaticDefaults(templates);
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
  table.innerHTML = '<thead><tr><th>Email</th><th>V2 role</th><th>Status</th><th>Account state</th><th>Actions</th></tr></thead>';
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
    const actions = element('div', 'admin-actions');
    select.addEventListener('change', () => patchV2Role(user, select.value).catch(showError)); actions.append(select);
    const toggle = buttonAction(user.enabled ? 'Disable' : 'Enable', async () => { await api(`/api/admin/users/${encodeURIComponent(user.email)}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled: !user.enabled }) }); await showSection('V2 Users'); }); actions.append(toggle);
    if (user.v2_role === 'writer' || user.v2_role === 'mentor') actions.append(buttonAction('Generate new temporary password', async () => { if (!confirm(`Replace the current password for ${user.email}? Current sessions will end.`)) return; const reset = await api(`/api/admin/v2/users/${encodeURIComponent(user.user_id)}/temporary-password`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); showCredentialHandoff({ created: 1, credential_emails_queued: reset.credential_email_queued ? 1 : 0, reused: 0, needs_attention: 0, credentials: [{ email: reset.email, temporary_password: reset.temporary_password, credential_role: user.v2_role === 'writer' ? 'student' : 'mentor' }] }, `reset-${user.user_id}`); await showSection('V2 Users'); }));
    if (user.email_delivery_status === 'FAILED' && !user.email_delivery_expired && user.email_delivery_id) actions.append(buttonAction('Retry email', async () => { await api(`/api/admin/v2/credential-emails/${encodeURIComponent(user.email_delivery_id)}/retry`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Credential email retry queued for ${user.email}.`); await showSection('V2 Users'); }));
    const details = element('details'); details.append(element('summary', '', 'Details'), element('p', 'muted-note', `Legacy type: ${user.legacy_account_type} · Migration: ${user.migration_state} · Created: ${new Date(user.created_at).toLocaleString()}`));
    const actionCell = element('td'); actionCell.append(actions, details);
    row.append(
      element('td', '', user.email),
      element('td', '', user.v2_role ? user.v2_role[0].toUpperCase() + user.v2_role.slice(1) : 'Unassigned'),
      element('td', '', user.enabled ? 'Enabled' : 'Disabled'),
      element('td', '', user.must_change_password ? `Temporary password · ${user.email_delivery_status === 'SENT' ? 'Email sent' : user.email_delivery_status === 'SENDING' ? 'Sending' : user.email_delivery_status === 'PENDING' ? 'Email pending' : user.email_delivery_status === 'FAILED' ? (user.email_delivery_expired ? 'Email expired' : 'Email failed') : user.email_delivery_status === 'EXPIRED' ? 'Email expired' : 'Email unavailable'}` : 'Active'),
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
      const down = buttonAction('Down', () => { if (index < selected.length - 1) [selected[index], selected[index + 1]] = [selected[index + 1], selected[index]]; draw(); changed(selected); }); down.disabled = !ordered || index === selected.length - 1; down.title = 'Move down'; down.setAttribute('aria-label', `Move ${person.email} down`);
      const remove = buttonAction('Remove', () => { selected.splice(index, 1); draw(); changed(selected); }, 'danger'); remove.title = 'Remove account'; remove.setAttribute('aria-label', `Remove ${person.email}`);
      row.append(up, down, remove); host.append(row);
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
    clearTimeout(timer); timer = setTimeout(async () => { if (!people.length) return; try { const preview = await api('/api/admin/v2/institution/template-defaults/resolve-preview', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ordered_writer_user_ids: people.map((person) => person.user_id) }) }); const found = templates.find((item) => item.id === preview.selected_template_id); const context = preview.resolution_method === 'TIE_FIRST_WRITER' ? `Selected using Writer-order tie-break (${preview.dominant_programme_code})` : preview.resolution_method === 'GLOBAL_FALLBACK' ? 'Automatically selected from Global fallback' : `Automatically selected from ${preview.dominant_programme_code}`; suggestion.textContent = `${found?.name || preview.selected_template_id} · ${context}`; } catch (error) { suggestion.textContent = error.message; } }, 300);
  });
  const mentors = peopleBuilder('mentor', false, () => {});
  form.append(leader, template, suggestion);
  const submit = element('button', 'primary', 'Create Paper Team'); submit.type = 'submit'; form.append(submit);
  form.addEventListener('submit', async (event) => { event.preventDefault(); if (!writers.selected.some((person) => person.user_id === leaderSelect.value)) return showError(new Error('Leader must be one of the selected Writers.')); try { await api('/api/admin/v2/paper-teams', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: form.elements.name.value, writer_ids: writers.selected.map((person) => person.user_id), leader_writer_id: leaderSelect.value, mentor_ids: mentors.selected.map((person) => person.user_id), template_id: templateSelect.value || null }) }); announce(`Paper Team created with ${templateSelect.value ? 'an Admin template override' : 'automatic template selection'}.`); await showSection('Paper Teams'); } catch (error) { showError(error); } });
  details.append(writers.section, mentors.section, form); content.append(details);
}

async function openTeamDetail(team, templates) {
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody');
  if (team.unresolved) {
    host.replaceChildren(element('h2', '', `${team.name} · unresolved`), element('p', 'danger-box', `${team.error_code || 'TEAM_UNRESOLVED'}: ${team.error_message || 'Resolve linked identities and template defaults, then re-apply the source import job.'}`), element('p', '', `Students: ${(team.unresolved_student_ids || []).join(', ') || 'None'} · Faculty: ${(team.unresolved_faculty_ids || []).join(', ') || 'None'}`));
    const links = buttonAction('Open Identity Links', () => { dialog.close(); showSection('Institution Data', { tab: 'Identity Links' }); }); const defaults = buttonAction('Open Templates', () => { dialog.close(); showSection('Templates'); }); const job = buttonAction('Open Import Job', () => { dialog.close(); showSection('Imports', { jobId: team.source_import_job_id }); }); host.append(element('p', 'muted-note', 'Imports are additive; missing rows do not remove existing members.'), links, defaults, job); dialog.showModal(); return;
  }
  const detail = await api(`/api/admin/v2/paper-teams/${team.id}`); const leader = detail.members.find((member) => member.is_leader); const writers = detail.members.filter((member) => member.role === 'writer'); const mentors = detail.members.filter((member) => member.role === 'mentor');
  const sourceLabel = detail.summary.resolution_method === 'MANUAL_OVERRIDE' ? 'Admin override' : detail.summary.resolution_method === 'GLOBAL_FALLBACK' ? 'Global fallback' : detail.summary.dominant_programme_code || 'Programme default';
  const sourcePrefix = detail.summary.resolution_method === 'MANUAL_OVERRIDE' ? 'Selected by:' : 'Selected automatically from:';
  host.replaceChildren(element('h2', '', detail.team.name), element('p', '', `${detail.team.status} · ${team.source} · updated ${new Date(detail.team.updated_at).toLocaleString()}`), element('p', '', `Writers: ${writers.map((member) => `${member.writer_order}. ${member.email}`).join(', ') || 'None'}`), element('p', '', `Leader: ${leader?.email || 'Missing'} · Mentors: ${mentors.map((member) => member.email).join(', ') || 'None'}`), element('h3', '', 'Template'), element('p', '', detail.template_pin?.template_name || 'None'), element('p', 'muted-note', `${sourcePrefix} ${sourceLabel}`));
  const advanced = element('details'); advanced.append(element('summary', '', 'Details'), element('p', 'muted-note', `Dominant programme: ${detail.summary.dominant_programme_code || '—'} · Resolution: ${detail.summary.resolution_method || 'unrecorded'} · Review: ${detail.summary.review_state} · Files: ${detail.summary.file_count} · Build: ${detail.summary.current_build.status || 'none'}`));
  if (team.source === 'imported') advanced.append(element('p', 'muted-note', `External key ${team.external_team_key} · source import ${team.source_import_job_id} · last imported ${team.last_imported_at || 'unknown'}.`)); host.append(advanced);
  const management = element('section', 'admin-section'); management.append(element('h2', '', 'Team management'));
  const edit = element('details'); edit.append(element('summary', '', 'Edit Team')); const editForm = element('form', 'manual-team-form'); const nameLabel = element('label', '', 'Team name'); const nameInput = document.createElement('input'); nameInput.name = 'name'; nameInput.required = true; nameInput.maxLength = 200; nameInput.value = detail.team.name; nameLabel.append(nameInput);
  const leaderLabel = element('label', '', 'Team Leader'); const leaderSelect = document.createElement('select'); leaderSelect.required = true; leaderLabel.append(leaderSelect);
  const updateLeaderChoices = (people) => { const previous = leaderSelect.value || leader?.user_id; leaderSelect.replaceChildren(new Option('Select one Writer…', '')); people.forEach((person) => leaderSelect.append(new Option(person.email, person.user_id))); if (people.some((person) => person.user_id === previous)) leaderSelect.value = previous; };
  const editWriters = peopleBuilder('writer', true, updateLeaderChoices); editWriters.selected.push(...writers.map(({ user_id, email }) => ({ user_id, email }))); editWriters.draw(); updateLeaderChoices(editWriters.selected);
  const editMentors = peopleBuilder('mentor', false, () => {}); editMentors.selected.push(...mentors.map(({ user_id, email }) => ({ user_id, email }))); editMentors.draw();
  const saveTeam = element('button', 'primary', 'Save Team'); saveTeam.type = 'submit'; editForm.append(nameLabel, editWriters.section, leaderLabel, editMentors.section, saveTeam); editForm.addEventListener('submit', async (event) => { event.preventDefault(); try { if (!editWriters.selected.length) throw new Error('Team must retain at least one Writer.'); if (!editWriters.selected.some((person) => person.user_id === leaderSelect.value)) throw new Error('Leader must be a selected Writer.'); await api(`/api/admin/v2/paper-teams/${team.id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: nameInput.value, writer_ids: editWriters.selected.map((person) => person.user_id), leader_writer_id: leaderSelect.value, mentor_ids: editMentors.selected.map((person) => person.user_id) }) }); announce(`Saved ${nameInput.value}.`); dialog.close(); await showSection('Paper Teams'); } catch (error) { showError(error); } }); edit.append(editForm); management.append(edit);
  [['Freeze', 'frozen'], ['Activate', 'active'], ['Archive', 'archived']].forEach(([label, value]) => management.append(buttonAction(label, async () => { await api(`/api/admin/v2/paper-teams/${team.id}/status`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ status: value }) }); dialog.close(); await showSection('Paper Teams'); }, value === 'archived' ? 'danger' : ''))); host.append(management);
  const section = element('section', 'admin-section'); section.append(element('h2', '', 'Change template'), element('p', '', `Current: ${detail.template_pin?.template_name || 'None'}`));
  const select = document.createElement('select'); select.setAttribute('aria-label', 'New template'); templates.filter((item) => item.id !== detail.template_pin?.template_id).forEach((item) => select.append(new Option(item.name, item.id)));
  const confirmation = element('div'); const change = buttonAction('Change template', async () => { if (!select.value) throw new Error('No alternative template is available.'); const preview = await api(`/api/admin/v2/paper-teams/${team.id}/template-change/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, confirm_main_file_change: false }) }); const fileConflicts = preview.writer_modified_conflicts.filter((item) => !String(item.reason).startsWith('Main file change')); if (fileConflicts.length) { const list = element('ul', 'compact-list'); fileConflicts.forEach((item) => list.append(element('li', '', item.path))); confirmation.replaceChildren(element('h3', '', 'Template cannot be changed yet.'), element('p', 'danger-box', 'Writer-edited files conflict:'), list, element('p', '', 'No files were changed.')); return; } const selectedTemplate = templates.find((item) => item.id === select.value); const mainChanged = preview.main_file_change.changed; const confirmBox = element('div', 'template-change-confirmation'); confirmBox.append(element('h3', '', 'Change template?'), element('p', '', `${detail.template_pin?.template_name || 'None'} → ${selectedTemplate?.name || 'Selected template'}`), element('p', '', `${preview.template_managed_files_to_update.length + preview.unchanged_old_template_files_to_update.length} files will update`), element('p', '', `${preview.files_to_add.length} files will be added`), element('p', '', 'Paper history will be preserved')); let mainConfirm = null; if (mainChanged) { const label = element('label', '', ` Confirm Main document change (${preview.main_file_change.from} → ${preview.main_file_change.to})`); mainConfirm = document.createElement('input'); mainConfirm.type = 'checkbox'; label.prepend(mainConfirm); confirmBox.append(label); } const apply = buttonAction('Change', async () => { const confirmedMain = Boolean(mainConfirm?.checked); if (mainChanged && !confirmedMain) return; const ready = mainChanged ? await api(`/api/admin/v2/paper-teams/${team.id}/template-change/preview`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, confirm_main_file_change: true }) }) : preview; await api(`/api/admin/v2/paper-teams/${team.id}/template-change/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_template_id: select.value, preview_token: ready.preview_token, confirm_main_file_change: confirmedMain }) }); announce('Template changed. Paper history was preserved.'); dialog.close(); await showSection('Paper Teams'); }, 'primary'); if (mainConfirm) { apply.disabled = true; mainConfirm.addEventListener('change', () => { apply.disabled = !mainConfirm.checked; }); } confirmBox.append(buttonAction('Cancel', () => confirmation.replaceChildren()), apply); confirmation.replaceChildren(confirmBox); });
  section.append(select, change, confirmation); host.append(section); dialog.showModal();
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
  const apply = buttonAction('Apply validated import', async () => { if (!confirm(`Apply ${job.mode} import ${job.original_filename}? Imports never remove absent rows.`)) return; const result = await api(`/api/admin/v2/institution/imports/${job.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Import applied: ${result.materialized_teams} Teams created, ${result.merged_teams} merged, ${result.unresolved_teams} unresolved.`); await showSection('Imports', { jobId: job.id }); showCredentialHandoff(result.account_provisioning, job.id); }, 'primary'); apply.disabled = job.mode === 'VALIDATE_ONLY' || job.status !== 'VALIDATED' || job.error_rows > 0;
  const retry = buttonAction('Retry Team Materialization', async () => { const result = await api(`/api/admin/v2/institution/imports/${job.id}/retry-teams`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); announce(`Retry: ${result.succeeded} succeeded, ${result.failed} remain unresolved.`, result.failed > 0); await showSection('Imports', { jobId: job.id }); }); retry.disabled = job.status !== 'PARTIAL';
  actions.append(download, apply, retry); section.append(actions); content.append(section);
}

const institutionTargets = ['departments', 'admins', 'faculty', 'programmes', 'schools', 'students', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles', 'paper_teams', 'paper_team_writers', 'paper_team_mentors'];
const targetColumns = {
  departments: ['department_id'], admins: ['admin_id', 'email', 'name', 'pfp'], faculty: ['faculty_id', 'name', 'email', 'dept_id', 'honorific', 'designation', 'status'], programmes: ['programme_code', 'hod_id'], schools: ['school_id'], students: ['reg_no', 'name', 'email', 'programme_code'], student_course_registrations: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'], faculty_guide_capacity: ['capacity_id', 'faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'integrated_pg_max_projects', 'status'], department_roles: ['id', 'dept_id', 'role_type', 'faculty_id'], faculty_roles: ['role_id', 'faculty_id', 'role_type', 'school_id', 'department_id', 'programme_code', 'status'], paper_teams: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status'], paper_team_writers: ['external_team_key', 'student_reg_no', 'writer_order', 'is_leader'], paper_team_mentors: ['external_team_key', 'faculty_id'],
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
  body.append(datasetHeading, datasetWrap);

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
  ['schools', 'admins', 'student_course_registrations', 'faculty_guide_capacity', 'department_roles', 'faculty_roles'].forEach((dataset) => optionalList.append(element('li', '', dataset)));
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
  const form = element('form', 'batch-import-form'); form.enctype = 'multipart/form-data'; const operations = element('fieldset', 'operation-control'); operations.innerHTML = '<legend>Operation</legend><label><input type="radio" name="operation" value="ADD" checked>Add</label><label><input type="radio" name="operation" value="EDIT">Edit</label><label><input type="radio" name="operation" value="DELETE">Delete</label>';
  const drop = element('div', 'import-dropzone'); drop.tabIndex = 0; drop.setAttribute('role', 'button'); drop.setAttribute('aria-label', 'Browse for CSV or XLSX files'); drop.append(element('strong', '', 'Drop CSV or XLSX files here'), element('span', '', 'or browse · multiple files supported')); const fileInput = document.createElement('input'); fileInput.type = 'file'; fileInput.multiple = true; fileInput.accept = '.csv,.xlsx,text/csv,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'; fileInput.hidden = true; drop.append(fileInput); const fileHost = element('div', 'import-file-list'); const submit = element('button', 'primary', 'Review changes'); submit.type = 'submit'; submit.disabled = true; const selected = [];
  const drawFiles = () => { fileHost.replaceChildren(); selected.forEach((entry, index) => { const chip = element('div', 'import-file-chip'); chip.append(element('strong', '', entry.file.name)); if (entry.file.name.toLowerCase().endsWith('.xlsx')) chip.append(element('span', '', entry.status)); else if (entry.dataset) chip.append(element('span', '', datasetLabel(entry.dataset))); else { const select = document.createElement('select'); select.setAttribute('aria-label', `What data is ${entry.file.name}?`); select.append(new Option('What data is this?', '')); institutionTargets.forEach((target) => select.append(new Option(datasetLabel(target), target))); select.addEventListener('change', () => { entry.dataset = select.value; entry.status = entry.dataset ? 'Ready' : 'Choose dataset'; drawFiles(); }); chip.append(select); } chip.append(element('span', entry.status === 'Ready' ? 'ready' : 'muted-note', entry.status), buttonAction('Remove', async () => { selected.splice(index, 1); drawFiles(); }, 'danger')); fileHost.append(chip); }); submit.disabled = !selected.length || selected.some((entry) => !entry.file.name.toLowerCase().endsWith('.xlsx') && !entry.dataset); };
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
  { label: 'Faculty', dataset: 'faculty', keys: ['faculty_id'], fields: ['faculty_id', 'name', 'email', 'dept_id', 'honorific', 'designation', 'status'], columns: ['faculty_id', 'name', 'email', 'dept_id', 'designation', 'link_status'] },
  { label: 'Programmes', dataset: 'programmes', keys: ['programme_code'], fields: ['programme_code', 'hod_id'], columns: ['programme_code', 'hod_id', 'template_name'] },
  { label: 'Departments', dataset: 'departments', keys: ['department_id'], fields: ['department_id'], columns: ['department_id'] },
  { label: 'Schools', dataset: 'schools', keys: ['school_id'], fields: ['school_id'], columns: ['school_id'] },
  { label: 'Course Registrations', dataset: 'student_course_registrations', keys: ['student_reg_no', 'course_id', 'academic_year', 'semester'], fields: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'], columns: ['student_reg_no', 'course_id', 'academic_year', 'semester', 'registration_status'] },
  { label: 'Guide Capacity', dataset: 'faculty_guide_capacity', keys: ['capacity_id'], fields: ['capacity_id', 'faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'integrated_pg_max_projects', 'status'], columns: ['faculty_id', 'academic_year', 'ug_max_projects', 'pg_max_projects', 'status'] },
  { label: 'Department Roles', dataset: 'department_roles', keys: ['id'], fields: ['id', 'dept_id', 'role_type', 'faculty_id'], columns: ['id', 'dept_id', 'role_type', 'faculty_id'] },
  { label: 'Faculty Roles', dataset: 'faculty_roles', keys: ['role_id'], fields: ['role_id', 'faculty_id', 'role_type', 'school_id', 'department_id', 'programme_code', 'status'], columns: ['faculty_id', 'role_type', 'programme_code', 'status'] },
  { label: 'Paper Assignments', dataset: 'paper_teams', keys: ['external_team_key'], fields: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status'], columns: ['external_team_key', 'team_name', 'academic_year', 'semester', 'status', 'materialized_paper_team_id'] },
];

function canonicalRecord(config, item) { return Object.fromEntries(config.fields.filter((field) => Object.hasOwn(item, field)).map((field) => [field, item[field]])); }

async function manualInstitutionRecord(config, operation, item, reload) {
  const dialog = document.querySelector('#adminDialog'); const host = document.querySelector('#adminDialogBody'); const form = element('form', 'manual-record-form'); const title = `${operation[0]}${operation.slice(1).toLowerCase()} ${config.label.replace(/s$/, '')}`; host.replaceChildren(element('h2', '', title)); const source = canonicalRecord(config, item || {}); const shownFields = operation === 'DELETE' ? config.keys : config.fields;
  shownFields.forEach((field) => { const label = element('label', '', field.replaceAll('_', ' ')); let input; if (field === 'is_leader') { input = document.createElement('select'); input.append(new Option('False', 'false'), new Option('True', 'true')); } else { input = document.createElement('input'); input.type = field === 'email' ? 'email' : 'text'; } input.name = field; input.value = source[field] ?? ''; input.readOnly = operation !== 'ADD' && config.keys.includes(field); label.append(input); form.append(label); }); const result = element('div'); const submit = element('button', operation === 'DELETE' ? 'danger' : 'primary', operation === 'DELETE' ? 'Check dependencies' : `Review ${operation.toLowerCase()}`); submit.type = 'submit'; form.append(submit, result); host.append(form); dialog.showModal();
  form.addEventListener('submit', async (event) => { event.preventDefault(); try { const payload = {}; new FormData(form).forEach((value, key) => { payload[key] = value === '' ? null : value; }); const detail = await api(`/api/admin/v2/institution/data/${config.dataset}/validate`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ operation, payload }) }); if (detail.batch.error_rows) { result.replaceChildren(element('p', 'danger-box', detail.issues.map((issue) => issue.problem).join('; '))); submit.disabled = true; return; } const question = operation === 'DELETE' ? `Delete this ${config.label.replace(/s$/, '')}? LaTeX Core users and paper history are retained.` : `${title}?`; if (!confirm(question)) return; const applied = await api(`/api/admin/v2/institution/import-batches/${detail.batch.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); dialog.close(); announce(`${title} complete.`); await reload(); if (operation === 'ADD') showCredentialHandoff(applied.account_provisioning, detail.batch.id); } catch (error) { result.replaceChildren(element('p', 'danger-box', error.message)); } });
}

async function renderIdentityLinkManager(host) {
  const state = { page: 1, limit: 50, search: '', external_type: '' }; const filters = filterForm([{ name: 'search', label: 'External ID, name, or email' }, { name: 'external_type', label: 'Identity type', type: 'select', options: [['Student', 'STUDENT'], ['Faculty', 'FACULTY'], ['Admin', 'ADMIN']] }], (values) => { Object.assign(state, values, { page: 1 }); load(); }); const pageHost = element('div'); host.replaceChildren(filters, pageHost);
  const load = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/identity-links?${queryString(state)}`); const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Type</th><th>External ID</th><th>Name / email</th><th>V2 account</th><th>Status</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody'); data.items.forEach((item) => { const row = document.createElement('tr'); const actions = element('td'); if (item.status === 'LINKED') actions.append(buttonAction('Unlink', async () => { if (!confirm(`Unlink ${item.external_type} ${item.external_id}? The V2 account is retained.`)) return; await api(`/api/admin/v2/institution/identity-links/${encodeURIComponent(item.external_type)}/${encodeURIComponent(item.external_id)}`, { method: 'DELETE' }); await load(); }, 'danger')); else actions.append(buttonAction('Manual link', async () => { const q = prompt('Search an existing compatible V2 account by email:'); if (!q) return; const role = item.external_type === 'STUDENT' ? 'writer' : item.external_type === 'FACULTY' ? 'mentor' : ''; const users = await searchPeople(role, q); if (!users.length) throw new Error('No compatible V2 account found.'); await api('/api/admin/v2/institution/identity-links', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ external_type: item.external_type, external_id: item.external_id, user_id: users[0].user_id }) }); await load(); })); row.append(element('td', '', item.external_type), element('td', '', item.external_id), element('td', '', `${item.name || '—'} · ${item.email || '—'}`), element('td', '', item.v2_account || '—'), element('td', '', `${item.status} · ${item.match_method || 'none'}`), actions); body.append(row); }); table.append(body); wrap.append(table); pageHost.replaceChildren(wrap, pager(data, load)); }; await load();
}

async function renderInstitutionData(options = {}) {
  let active = institutionDatasets.find((config) => config.label === options.tab) || institutionDatasets[0]; const tabsHost = element('div', 'admin-tabs'); const host = element('div'); institutionDatasets.forEach((config) => tabsHost.append(buttonAction(config.label, () => load(config)))); const identities = element('details', 'admin-section'); identities.append(element('summary', '', 'Identity Links')); const identityHost = element('div'); identities.append(identityHost); let identitiesLoaded = false; identities.addEventListener('toggle', () => { if (identities.open && !identitiesLoaded) { identitiesLoaded = true; renderIdentityLinkManager(identityHost).catch(showError); } }); content.append(element('p', 'muted-note', 'Search stays server-side. Add, Edit, and Delete use the same validation and dependency rules as file imports.'), tabsHost, host, identities);
  const load = async (config = active) => { active = config; [...tabsHost.children].forEach((button) => button.setAttribute('aria-current', button.textContent === config.label ? 'page' : 'false')); const state = { page: 1, limit: 50, search: '', programme_code: '', department_id: '', link_status: '' }; const filterFields = [{ name: 'search', label: `Search ${config.label}` }, ...(config.dataset === 'students' ? [{ name: 'programme_code', label: 'Programme' }] : []), ...(config.dataset === 'faculty' ? [{ name: 'department_id', label: 'Department UUID' }] : []), ...(['students', 'faculty'].includes(config.dataset) ? [{ name: 'link_status', label: 'Link status', type: 'select', options: ['LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE'].map((value) => [value]) }] : [])]; const filters = filterForm(filterFields, (values) => { Object.assign(state, values, { page: 1 }); pageLoad(); }); const add = buttonAction(`+ Add ${config.label.replace(/s$/, '')}`, () => manualInstitutionRecord(config, 'ADD', {}, pageLoad), 'primary'); const pageHost = element('div'); host.replaceChildren(element('div', 'admin-toolbar'), filters, pageHost); host.firstChild.append(add);
    const pageLoad = async (page = state.page) => { state.page = page; const data = await api(`/api/admin/v2/institution/data/${config.dataset}?${queryString(state)}`); if (!data.items.length) { pageHost.replaceChildren(element('p', 'empty-copy', `No ${config.label.toLowerCase()} records.`), pager(data, pageLoad)); return; } const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = `<thead><tr>${config.columns.map((field) => `<th>${field.replaceAll('_', ' ')}</th>`).join('')}<th>Actions</th></tr></thead>`; const body = document.createElement('tbody'); data.items.forEach((item) => { const row = document.createElement('tr'); config.columns.forEach((field) => { let value = item[field]; if (field === 'materialized_paper_team_id') value = value ? `Materialized → ${value}` : 'Not materialized'; row.append(element('td', '', value ?? '—')); }); const actions = element('td'); actions.append(buttonAction('Edit', () => manualInstitutionRecord(config, 'EDIT', item, pageLoad)), buttonAction('Delete', () => manualInstitutionRecord(config, 'DELETE', item, pageLoad), 'danger')); if (config.dataset === 'paper_teams') { const writerConfig = { label: 'Paper Team Writers', dataset: 'paper_team_writers', keys: ['external_team_key', 'student_reg_no'], fields: ['external_team_key', 'student_reg_no', 'writer_order', 'is_leader'] }; const mentorConfig = { label: 'Paper Team Mentors', dataset: 'paper_team_mentors', keys: ['external_team_key', 'faculty_id'], fields: ['external_team_key', 'faculty_id'] }; const assignment = element('details'); const writerList = element('div'); (item.writers || []).forEach((writer) => { const entry = element('p', '', `${writer.writer_order}. ${writer.student_reg_no}${writer.is_leader ? ' (Leader)' : ''} `); const values = { external_team_key: item.external_team_key, ...writer }; entry.append(buttonAction('Edit', () => manualInstitutionRecord(writerConfig, 'EDIT', values, pageLoad)), buttonAction('Remove', () => manualInstitutionRecord(writerConfig, 'DELETE', values, pageLoad), 'danger')); writerList.append(entry); }); const mentorList = element('div'); (item.mentors || []).forEach((mentor) => { const entry = element('p', '', `${mentor.faculty_id} `); const values = { external_team_key: item.external_team_key, ...mentor }; entry.append(buttonAction('Remove', () => manualInstitutionRecord(mentorConfig, 'DELETE', values, pageLoad), 'danger')); mentorList.append(entry); }); assignment.append(element('summary', '', 'Writers and Mentors'), writerList, mentorList, buttonAction('Add Writer', () => manualInstitutionRecord(writerConfig, 'ADD', { external_team_key: item.external_team_key }, pageLoad)), buttonAction('Add Mentor', () => manualInstitutionRecord(mentorConfig, 'ADD', { external_team_key: item.external_team_key }, pageLoad))); actions.prepend(assignment); } row.append(actions); body.append(row); }); table.append(body); wrap.append(table); pageHost.replaceChildren(wrap, pager(data, pageLoad)); };
    await pageLoad(); };
  await load(active);
}

async function renderAutomaticDefaults(templates) {
  const [mappings, fallback] = await Promise.all([api('/api/admin/v2/institution/template-defaults/programmes'), api('/api/admin/v2/institution/template-defaults/global-fallback')]); content.append(element('h2', '', 'Automatic Defaults'), element('p', 'muted-note', 'These mappings affect future Teams only. Existing template pins remain unchanged.'));
  const fallbackSection = element('section', 'admin-section'); fallbackSection.append(element('h3', '', 'Global fallback')); const fallbackSelect = document.createElement('select'); templates.forEach((item) => fallbackSelect.append(new Option(item.name, item.id, false, item.id === fallback.template_id))); fallbackSection.append(fallbackSelect, buttonAction('Save', async () => { if (!fallbackSelect.value) throw new Error('Select a template.'); await api('/api/admin/v2/institution/template-defaults/global-fallback', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: fallbackSelect.value }) }); announce('Global fallback saved for future Teams.'); await showSection('Templates'); }, 'primary')); content.append(fallbackSection);
  const wrap = element('div', 'admin-table-wrap'); const table = element('table', 'admin-table'); table.innerHTML = '<thead><tr><th>Programme</th><th>Default template</th><th>Actions</th></tr></thead>'; const body = document.createElement('tbody'); mappings.forEach((mapping) => { const row = document.createElement('tr'); const select = document.createElement('select'); select.append(new Option('No programme default', '')); templates.forEach((item) => select.append(new Option(item.name, item.id, false, item.id === mapping.template_id))); const actions = element('td'); actions.append(buttonAction('Save', async () => { if (!select.value) throw new Error('Select a template.'); await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ template_id: select.value }) }); announce(`${mapping.programme_code} default saved for future Teams.`); await showSection('Templates'); }), buttonAction('Remove mapping', async () => { if (!mapping.template_id || !confirm(`Remove ${mapping.programme_code} default?`)) return; await api(`/api/admin/v2/institution/template-defaults/programmes/${encodeURIComponent(mapping.programme_code)}`, { method: 'DELETE' }); await showSection('Templates'); }, 'danger')); const templateCell = element('td'); templateCell.append(select); row.append(element('td', '', mapping.programme_code), templateCell, actions); body.append(row); }); table.append(body); wrap.append(table); content.append(wrap);
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
      content.replaceChildren(element('h1', '', 'DATA IMPORT'));
      await renderImports(options);
      return;
    }
    if (section === 'Institution Data') {
      content.replaceChildren(element('h1', '', 'INSTITUTION DATA'));
      await renderInstitutionData(options);
      return;
    }
    if (section === 'Templates') {
      const templates = await api('/api/admin/templates');
      content.replaceChildren(element('h1', '', 'TEMPLATES'));
      await renderTemplates(templates);
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
