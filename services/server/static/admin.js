import { api } from '/static/api.js?v=control-plane-groups-1';

const nav = document.querySelector('#adminNav');
const content = document.querySelector('#adminContent');
const status = document.querySelector('#adminStatus');

const endpoints = {
  Overview: '/api/admin/overview',
  Templates: '/api/admin/templates',
  Versions: '/api/admin/v2/versions',
  'Restoration Requests': '/api/admin/v2/restoration-requests',
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
  const intro = element('p', 'empty-copy', 'Import a bounded local ZIP, inspect its safe file tree, select Main when detection is ambiguous, then save an immutable template. Existing-Team template update is unavailable in this RC.');
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

function memberSelect(users, role, label) {
  const field = element('label', '', label);
  const select = document.createElement('select');
  select.name = `${role}_ids`;
  select.multiple = true;
  users.filter((user) => user.v2_role === role).forEach((user) => {
    const option = element('option', '', user.email);
    option.value = user.user_id;
    select.append(option);
  });
  field.append(select);
  return field;
}

async function renderPaperTeams(teams, users, templates) {
  const form = element('form', 'admin-team-form');
  const name = element('label', '', 'Paper Team name');
  name.innerHTML = 'Paper Team name<input name="name" required maxlength="200">';
  const templateField = element('label', '', 'Initial template');
  const templateSelect = document.createElement('select'); templateSelect.name = 'template_id';
  templateSelect.append(new Option('Default blank paper', ''));
  templates.forEach((template) => templateSelect.append(new Option(`${template.name} · immutable pin`, template.id)));
  templateField.append(templateSelect);
  form.append(name, memberSelect(users, 'writer', 'Writers'), memberSelect(users, 'mentor', 'Mentors'), templateField);
  const create = element('button', 'primary', 'Create Paper Team');
  create.type = 'submit';
  form.append(create);
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const data = new FormData(form);
    try {
      await api('/api/admin/v2/paper-teams', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: data.get('name'), writer_ids: data.getAll('writer_ids'), mentor_ids: data.getAll('mentor_ids'), template_id: data.get('template_id') || null }),
      });
      announce('Paper Team and initialized main.tex workspace created.');
      await showSection('Paper Teams');
    } catch (error) { showError(error); }
  });
  content.append(element('p', 'empty-copy', 'One Paper Team maps to exactly one paper workspace. Membership grants access; capability comes from the exclusive global role.'), form);
  if (!teams.length) {
    content.append(element('p', 'empty-copy', 'No Paper Teams yet.'));
    return;
  }
  const details = await Promise.all(teams.map((team) => api(`/api/admin/v2/paper-teams/${team.id}`)));
  details.forEach(({ team, members, template_pin: templatePin }) => {
    const card = element('section', 'team-card');
    card.append(element('h2', '', team.name), element('p', 'empty-copy', `${team.status} · workspace ${team.workspace_id}`));
    card.append(element('p', 'empty-copy', templatePin ? `Template: ${templatePin.template_name} · pinned ${templatePin.source_identity}` : 'Template: Default blank paper'));
    const lifecycle = element('div', 'admin-actions');
    const transitions = team.status === 'active' ? [['Freeze', 'frozen'], ['Submit', 'submitted'], ['Archive', 'archived']]
      : team.status === 'frozen' ? [['Activate', 'active'], ['Archive', 'archived']]
        : team.status === 'submitted' ? [['Archive', 'archived']] : [];
    transitions.forEach(([label, value]) => lifecycle.append(buttonAction(label, async () => {
      await api(`/api/admin/v2/paper-teams/${team.id}/status`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ status: value }) });
      announce(`${team.name} is now ${value}.`); await showSection('Paper Teams');
    })));
    card.append(lifecycle);
    const list = element('div', 'members-list');
    members.forEach((member) => {
      const row = element('div', 'member-row');
      row.append(element('span', '', `${member.email} — ${member.role}`));
      const remove = element('button', 'danger', 'Remove');
      remove.type = 'button';
      remove.addEventListener('click', async () => {
        try {
          await api(`/api/admin/v2/paper-teams/${team.id}/members/${member.user_id}`, { method: 'DELETE' });
          await showSection('Paper Teams');
        } catch (error) { showError(error); }
      });
      row.append(remove);
      list.append(row);
    });
    const assigned = new Set(members.map((member) => member.user_id));
    const available = users.filter((user) => user.v2_role && user.v2_role !== 'admin' && !assigned.has(user.user_id));
    const add = element('form', 'member-add-form');
    const select = document.createElement('select');
    select.required = true;
    select.innerHTML = '<option value="">Assign Writer or Mentor…</option>';
    available.forEach((user) => {
      const option = element('option', '', `${user.email} — ${user.v2_role}`);
      option.value = user.user_id;
      select.append(option);
    });
    const submit = element('button', '', 'Assign');
    submit.type = 'submit';
    add.append(select, submit);
    add.addEventListener('submit', async (event) => {
      event.preventDefault();
      try {
        await api(`/api/admin/v2/paper-teams/${team.id}/members`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ user_id: select.value }),
        });
        await showSection('Paper Teams');
      } catch (error) { showError(error); }
    });
    card.append(list, add);
    content.append(card);
  });
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

function renderRestorationRequests(requests) {
  if (!requests.length) return content.append(element('p', 'empty-copy', 'No restoration requests.'));
  requests.forEach((request) => {
    const card = element('section', 'team-card');
    card.append(element('h2', '', request.paper_name), element('p', '', `Version #${request.target_version_number} · ${request.writer_email} · ${request.state.replaceAll('_', ' ')}`), element('p', 'empty-copy', request.reason || 'No reason supplied.'));
    if (request.state === 'AWAITING_ADMIN_REVIEW') {
      const actions = element('div', 'admin-actions');
      actions.append(buttonAction('Apply governed restoration', async () => { if (!confirm('Apply this endorsed restoration? The current exact state will be retained as PRE_RESTORE_SAFETY.')) return; const note = prompt('Optional Admin note', ''); if (note === null) return; await api(`/api/admin/v2/restoration-requests/${request.id}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ note: note || null }) }); announce('Restoration applied as a new workspace head.'); await showSection('Restoration Requests'); }, 'primary'));
      actions.append(buttonAction('Reject', async () => { const note = prompt('Optional rejection note', ''); if (note === null) return; await api(`/api/admin/v2/restoration-requests/${request.id}/reject`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ note: note || null }) }); await showSection('Restoration Requests'); }, 'danger'));
      card.append(actions);
    }
    content.append(card);
  });
}

async function showSection(section) {
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
      const [teams, users, templates] = await Promise.all([api('/api/admin/v2/paper-teams'), api('/api/admin/v2/users'), api('/api/admin/templates')]);
      content.replaceChildren(element('h1', '', 'PAPER TEAMS'));
      await renderPaperTeams(teams, users, templates);
      return;
    }
    if (section === 'Templates') {
      const templates = await api('/api/admin/templates');
      content.replaceChildren(element('h1', '', 'TEMPLATES'));
      renderTemplates(templates);
      return;
    }
    if (section === 'File Policies') {
      const teams = await api('/api/admin/v2/paper-teams'); content.replaceChildren(element('h1', '', 'FILE POLICIES')); await renderFilePolicies(teams);
      return;
    }
    const data = await api(endpoints[section]);
    content.replaceChildren(element('h1', '', section.toUpperCase()));
    if (section === 'Overview') renderOverview(data);
    else if (section === 'Restoration Requests') renderRestorationRequests(data);
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
