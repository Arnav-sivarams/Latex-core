import { api } from '/static/api.js?v=control-plane-groups-1';

const nav = document.querySelector('#adminNav');
const content = document.querySelector('#adminContent');
const status = document.querySelector('#adminStatus');

const endpoints = {
  Overview: '/api/admin/overview',
  'Legacy Users': '/api/admin/users',
  'Legacy Teams': '/api/admin/teams',
  'Research Groups': '/api/admin/research-groups',
  Projects: '/api/admin/projects',
  Templates: '/api/admin/templates',
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
  table.innerHTML = '<thead><tr><th>Email</th><th>Exclusive role</th><th>Status</th><th>Created</th></tr></thead>';
  const body = document.createElement('tbody');
  users.forEach((user) => {
    const row = document.createElement('tr');
    const select = document.createElement('select');
    select.setAttribute('aria-label', `Exclusive V2 role for ${user.email}`);
    ['writer', 'mentor', 'admin'].forEach((role) => {
      const option = element('option', '', role[0].toUpperCase() + role.slice(1));
      option.value = role;
      option.selected = role === user.role;
      select.append(option);
    });
    select.addEventListener('change', () => patchV2Role(user, select.value).catch(showError));
    const roleCell = document.createElement('td');
    roleCell.append(select);
    row.append(element('td', '', user.email), roleCell, element('td', '', user.enabled ? 'enabled' : 'disabled'), element('td', '', user.created_at));
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
  users.filter((user) => user.role === role).forEach((user) => {
    const option = element('option', '', user.email);
    option.value = user.user_id;
    select.append(option);
  });
  field.append(select);
  return field;
}

async function renderPaperTeams(teams, users) {
  const form = element('form', 'admin-team-form');
  const name = element('label', '', 'Paper Team name');
  name.innerHTML = 'Paper Team name<input name="name" required maxlength="200">';
  form.append(name, memberSelect(users, 'writer', 'Writers'), memberSelect(users, 'mentor', 'Mentors'));
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
        body: JSON.stringify({ name: data.get('name'), writer_ids: data.getAll('writer_ids'), mentor_ids: data.getAll('mentor_ids') }),
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
  details.forEach(({ team, members }) => {
    const card = element('section', 'team-card');
    card.append(element('h2', '', team.name), element('p', 'empty-copy', `${team.status} · workspace ${team.workspace_id}`));
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
    const available = users.filter((user) => user.role !== 'admin' && !assigned.has(user.user_id));
    const add = element('form', 'member-add-form');
    const select = document.createElement('select');
    select.required = true;
    select.innerHTML = '<option value="">Assign Writer or Mentor…</option>';
    available.forEach((user) => {
      const option = element('option', '', `${user.email} — ${user.role}`);
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
      const [teams, users] = await Promise.all([api('/api/admin/v2/paper-teams'), api('/api/admin/v2/users')]);
      content.replaceChildren(element('h1', '', 'PAPER TEAMS'));
      await renderPaperTeams(teams, users);
      return;
    }
    const data = await api(endpoints[section]);
    content.replaceChildren(element('h1', '', section.toUpperCase()));
    if (section === 'Legacy Users') renderLegacyUsers(data);
    else if (section === 'Overview') renderOverview(data);
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
