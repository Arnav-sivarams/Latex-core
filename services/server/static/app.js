import { api, apiText, ApiError } from '/static/api.js';
import { state, savePreferences, cacheBuffer, clearBufferCache } from '/static/state.js';

const $ = (selector) => document.querySelector(selector);
const textExtensions = new Set(['tex', 'bib', 'cls', 'sty', 'bst', 'cfg', 'def', 'txt', 'csv', 'md', 'log', 'aux', 'toc']);
const conflictCopy = { changed_since_edit: 'This file changed after you started editing.', destination_exists: 'The rename destination already exists.', missing_source: 'The original file no longer exists.', main_target_missing: 'The selected Main file no longer exists.', main_locked: "This project's Main file is locked.", policy_changed: 'This file is now protected.', permission_changed: 'Your project permissions changed.' };
const structuralPrivateProjects = new Set();

function isText(path) { return textExtensions.has((path.split('.').pop() || '').toLowerCase()); }
function currentRecord() { return state.project?.files.find((file) => file.path === state.currentFile); }
function buffer(path = state.currentFile) { return state.tabs.get(path); }
function isTeam() { return !!state.project?.collaboration; }
function canEdit(file = currentRecord()) { return !!file && (!isTeam() || (state.project.collaboration.can_write && file.policy === 'editable')); }
function hasKnownPrivateChanges() { return !!state.project?.files.some((file) => file.has_draft) || structuralPrivateProjects.has(state.project?.id); }
function projectCacheKey() { return state.project ? `latex-core.recent.${state.project.id}` : ''; }
function recentFiles() { try { return JSON.parse(localStorage.getItem(projectCacheKey()) || '[]'); } catch (_) { return []; } }
function remember(path) { const values = [path, ...recentFiles().filter((item) => item !== path)].slice(0, 8); localStorage.setItem(projectCacheKey(), JSON.stringify(values)); }

function toast(message, error = false) { const item = document.createElement('div'); item.className = `toast${error ? ' error' : ''}`; item.textContent = message; $('#toastArea').append(item); setTimeout(() => item.remove(), 4300); }
function readableError(error) {
  if (error.status === 401) return 'Session expired — your local changes are still retained.';
  if (error.status === 0) return 'Offline — changes not saved.';
  if (error.message === 'stale workspace version') return 'Project changed since it was opened. Reload it and try again.';
  if (error.message === 'protected by project policy') return 'This file is protected by project policy.';
  if (error.message === 'draft changed in another session') return 'Your private draft changed in another session.';
  return error.message || 'Something went wrong.';
}
function setSaveState(value) {
  state.saveState = value;
  const labels = { CLEAN: 'Saved', DIRTY: 'Unsaved changes', SAVING: 'Saving…', SAVED: 'Saved', SAVED_PRIVATE: 'Private changes saved', PUBLISHING: 'Publishing…', CONFLICT: 'Save conflict', OFFLINE_UNSAVED: 'Offline — changes not saved' };
  const el = $('#saveState'); el.textContent = labels[value] || value; el.dataset.state = value === 'SAVED_PRIVATE' ? 'private' : value === 'OFFLINE_UNSAVED' || value === 'CONFLICT' || value === 'DIRTY' ? 'offline' : '';
  $('#saveButton').disabled = !state.currentFile || !buffer()?.dirty || !canEdit() || state.saveState === 'SAVING';
  document.title = `${buffer()?.dirty ? '• ' : ''}${state.project?.name || 'LaTeX Core'} — LaTeX Core`;
}
function updateTop() {
  $('#projectTitle').textContent = state.project ? state.project.name : 'No project selected';
  const publish = $('#publishButton');
  publish.classList.toggle('hidden', !isTeam() || !state.project.collaboration.can_write);
  publish.textContent = 'Publish Changes';
  $('#membersButton').classList.toggle('hidden', !isTeam() || !state.project.collaboration.can_manage);
  $('#newFileButton').disabled = !state.project || (isTeam() && !state.project.collaboration.can_write);
}

function applyTheme() {
  const theme = state.preferences.theme;
  document.documentElement.dataset.theme = theme === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : theme;
  $('#themeButton').textContent = theme[0].toUpperCase() + theme.slice(1);
}
function applyLayout() {
  const workspace = $('#workspace'); const { left, right, bottom, sidebar, pdf } = state.preferences;
  $('#sidebar').classList.toggle('hidden', sidebar); $('#leftResizer').classList.toggle('hidden', sidebar);
  $('#pdfPane').classList.toggle('hidden', pdf); $('#splitResizer').classList.toggle('hidden', pdf);
  if (innerWidth > 1050) workspace.style.gridTemplateColumns = `${sidebar ? 0 : left}px ${sidebar ? 0 : 5}px minmax(360px,1fr) ${pdf ? 0 : 5}px ${pdf ? 0 : right}px`;
  else workspace.style.gridTemplateColumns = '';
  $('#bottomPanel').style.height = `${bottom}px`;
}
function resetLayout() { state.preferences = { ...state.preferences, left: 242, right: 520, bottom: 190, sidebar: false, pdf: false }; savePreferences(); applyLayout(); toast('Layout reset'); }

function node(tag, className, content) { const el = document.createElement(tag); if (className) el.className = className; if (content !== undefined) el.textContent = content; return el; }
function renderProjects(items) { const root = $('#projects'); root.replaceChildren(); for (const item of items) { const button = node('button', `nav-item${state.project?.id === item.id ? ' active' : ''}`); button.append(node('span', '', item.name)); button.onclick = () => openProject(item.id); root.append(button); } if (!items.length) root.append(node('p', 'muted', 'Create your first project.')); }
async function loadNavigation() {
  const [projects, teams] = await Promise.all([api('/api/projects'), api('/api/teams')]); renderProjects(projects);
  const root = $('#teams'); const mentorRoot = $('#mentorGroups'); root.replaceChildren(); mentorRoot.replaceChildren();
  $('#mentorGroupsSection').classList.toggle('hidden', !teams.some((team) => team.group_type === 'mentor_group'));
  for (const team of teams) {
    const title = node('button', `team-name button-reset${state.activeTeam?.id === team.id ? ' active' : ''}`, team.name); title.onclick = () => { state.activeTeam = team; updateTop(); toast(`New projects will be created in ${team.name}.`); };
    const target = team.group_type === 'mentor_group' ? mentorRoot : root; target.append(title);
    const projectsForTeam = await api(`/api/teams/${team.id}/projects`);
    for (const project of projectsForTeam) { const button = node('button', `nav-item${state.project?.id === project.workspace_id ? ' active' : ''}`); button.append(node('span', '', project.name)); button.onclick = () => { state.activeTeam = team; openProject(project.workspace_id); }; target.append(button); }
  }
  if (!teams.length) root.append(node('p', 'muted', 'No teams available.'));
}
function fileIcon(path) { const extension = (path.split('.').pop() || '').toLowerCase(); return extension === 'tex' ? 'T' : extension === 'bib' ? 'B' : ['png','jpg','jpeg','pdf'].includes(extension) ? '◫' : '·'; }
function renderTree() {
  const root = $('#fileTree'); root.replaceChildren(); if (!state.project) { root.append(node('p', 'muted', 'Open a project to browse files.')); return; }
  const folders = new Map([['', { children: [], files: [] }]);
  for (const file of state.project.files.slice().sort((a,b) => a.path.localeCompare(b.path))) { let parent = ''; const parts = file.path.split('/'); for (let index = 0; index < parts.length - 1; index += 1) { const name = parts[index]; const path = parent ? `${parent}/${name}` : name; if (!folders.has(path)) { folders.set(path, { children: [], files: [] }); folders.get(parent).children.push(path); } parent = path; } folders.get(parent).files.push(file); }
  const visit = (path, depth) => { const group = folders.get(path); for (const child of group.children.sort()) { const expanded = !state.expanded.has(`closed:${child}`); const row = node('button', 'tree-item tree-folder'); row.dataset.depth = String(Math.min(depth, 4)); row.textContent = `${expanded ? '⌄' : '›'}  ${child.split('/').pop()}`; row.onclick = () => { const key = `closed:${child}`; state.expanded.has(key) ? state.expanded.delete(key) : state.expanded.add(key); renderTree(); }; root.append(row); if (expanded) visit(child, depth + 1); }
    for (const file of group.files) { const row = node('button', `tree-item tree-file${state.currentFile === file.path ? ' active' : ''}`); row.dataset.depth = String(Math.min(depth, 4)); row.append(node('span', '', fileIcon(file.path)), node('span', 'file-label', file.path.split('/').pop())); if (state.project.main_file === file.path) row.append(node('span', 'badge', 'Main')); if (file.policy && file.policy !== 'editable') row.append(node('span', 'badge locked', 'Locked')); if (file.has_draft) row.append(node('span', 'badge private', 'Private')); row.onclick = () => openFile(file.path); root.append(row); }
  }; visit('', 0);
}
function renderTabs() { const root = $('#tabs'); root.replaceChildren(); for (const [path, tab] of state.tabs) { const tabButton = node('button', `tab${path === state.currentFile ? ' active' : ''}`); tabButton.setAttribute('role', 'tab'); tabButton.setAttribute('aria-selected', String(path === state.currentFile)); tabButton.append(node('span', 'tab-name', path.split('/').pop())); if (tab.dirty) tabButton.append(node('span', 'dirty-dot', '•')); const close = node('button', 'tab-close', '×'); close.title = 'Close tab'; close.onclick = async (event) => { event.stopPropagation(); if (path === state.currentFile) { const saved = await flushDirty(); if (!saved) return; } state.tabs.delete(path); if (path === state.currentFile) { state.currentFile = state.tabs.keys().next().value || null; if (state.currentFile) showFile(); else clearEditor(); } renderTabs(); renderTree(); }; tabButton.onclick = () => switchTab(path); tabButton.append(close); root.append(tabButton); } }
function showFile() {
  const tab = buffer(); const record = currentRecord(); renderTabs(); renderTree();
  $('#breadcrumb').textContent = state.currentFile ? state.currentFile.split('/').join(' / ') : 'Choose a file';
  $('#fileMeta').textContent = record?.has_draft ? 'Private working copy' : record?.policy && record.policy !== 'editable' ? 'Protected by project policy' : '';
  $('#setMainButton').classList.toggle('hidden', !state.currentFile || !state.currentFile.endsWith('.tex') || state.project.main_file === state.currentFile || (isTeam() && !state.project.collaboration.can_manage));
  const editor = $('#editor'); editor.value = tab?.text || ''; const editable = !!tab && isText(state.currentFile) && canEdit(record); editor.disabled = !editable;
  $('#readOnly').classList.toggle('hidden', !tab || isText(state.currentFile) && editable); if (tab && !isText(state.currentFile)) $('#readOnly').querySelector('strong').textContent = 'Binary asset';
  setSaveState(tab?.dirty ? 'DIRTY' : record?.has_draft ? 'SAVED_PRIVATE' : 'SAVED');
}
function clearEditor() { state.currentFile = null; $('#editor').value = ''; $('#editor').disabled = true; $('#breadcrumb').textContent = 'Choose a file'; $('#fileMeta').textContent = ''; $('#readOnly').classList.add('hidden'); renderTabs(); renderTree(); setSaveState('CLEAN'); }

async function openProject(id) {
  if (!(await flushDirty())) return;
  try {
    const project = await api(`/api/projects/${id}`); state.project = project; state.currentFile = null; state.tabs.clear(); updateTop(); await loadNavigation(); renderTree();
    const preferred = recentFiles().find((path) => project.files.some((file) => file.path === path && canEdit(file))) || project.files.find((file) => canEdit(file))?.path || project.main_file || project.files[0]?.path;
    if (preferred) await openFile(preferred); else clearEditor();
  } catch (error) { toast(readableError(error), true); }
}
async function refreshProject() { if (!state.project) return; const project = await api(`/api/projects/${state.project.id}`); state.project = project; updateTop(); renderTree(); }
async function openFile(path) {
  if (path === state.currentFile) return;
  if (!(await flushDirty())) return;
  try {
    if (!state.tabs.has(path)) { const text = isText(path) ? await api(`/api/projects/${state.project.id}/files/${encodeURI(path)}`) : ''; state.tabs.set(path, { text, dirty: false, saving: false }); }
    state.currentFile = path; remember(path); showFile(); $('#editor').focus();
  } catch (error) { toast(readableError(error), true); }
}
async function switchTab(path) { if (path === state.currentFile) return; if (!(await flushDirty())) return; state.currentFile = path; showFile(); }
function onEditorInput() { const tab = buffer(); if (!tab || !canEdit()) return; tab.text = $('#editor').value; tab.dirty = true; cacheBuffer(tab); setSaveState('DIRTY'); renderTabs(); clearTimeout(state.saveTimer); state.saveTimer = setTimeout(() => save(), 1500); }
async function save() {
  const tab = buffer(); const record = currentRecord(); if (!tab || !tab.dirty || tab.saving || !canEdit(record)) return true;
  const payload = tab.text; tab.saving = true; setSaveState('SAVING');
  try {
    const headers = { 'If-Match': `"${state.project.version}"` };
    if (isTeam()) { headers['X-File-Revision'] = String(record?.revision || 0); headers['If-Draft-Match'] = String(record?.draft_revision || 0); }
    const result = await api(`/api/projects/${state.project.id}/files/${encodeURI(state.currentFile)}`, { method: 'PUT', headers, body: payload });
    state.project.version = result.version; if (tab.text === payload) { tab.dirty = false; clearBufferCache(); }
    await refreshProject(); setSaveState(isTeam() ? 'SAVED_PRIVATE' : 'SAVED'); if (isTeam()) toast('Private changes saved'); else toast('Changes saved'); renderTabs(); return true;
  } catch (error) {
    cacheBuffer(tab); setSaveState(error.status === 0 ? 'OFFLINE_UNSAVED' : error.status === 409 ? 'CONFLICT' : 'DIRTY'); toast(readableError(error), true); return false;
  } finally { tab.saving = false; if (tab.dirty) setSaveState('DIRTY'); }
}
async function flushDirty() { clearTimeout(state.saveTimer); return save(); }

async function createFile() { const path = $('#filePathInput').value.trim(); if (!path || !state.project) return; const record = currentRecord(); try { const headers = { 'If-Match': `"${state.project.version}"` }; if (isTeam()) { headers['X-File-Revision'] = '0'; headers['If-Draft-Match'] = '0'; } const result = await api(`/api/projects/${state.project.id}/files/${encodeURI(path)}`, { method: 'PUT', headers, body: '' }); state.project.version = result.version; $('#fileDialog').close(); await refreshProject(); await openFile(path); toast(isTeam() ? 'Private file created' : 'File created'); } catch (error) { $('#fileDialogError').textContent = readableError(error); } }
async function setMain() { if (!state.currentFile || !(await flushDirty())) return; try { const result = await api(`/api/projects/${state.project.id}/main`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ path: state.currentFile, version: state.project.version }) }); state.project.version = result.version; if (isTeam()) structuralPrivateProjects.add(state.project.id); await refreshProject(); showFile(); toast(isTeam() ? 'Main file staged privately' : 'Main file updated'); } catch (error) { toast(readableError(error), true); } }

function waitForCompileChoice() { const dialog = $('#compileDialog'); dialog.showModal(); return new Promise((resolve) => dialog.addEventListener('close', () => resolve(dialog.returnValue || 'cancel'), { once: true })); }
async function compile() {
  if (!state.project || state.job) return; if (!(await flushDirty())) return;
  if (!state.project.main_file) { toast('Choose a Main .tex file before compiling.', true); return; }
  if (isTeam() && hasKnownPrivateChanges()) { const choice = await waitForCompileChoice(); if (choice === 'cancel') return; if (choice === 'publish') { const okay = await publish(); if (!okay) return; } }
  try { state.job = { state: 'queued' }; $('#compileButton').disabled = true; $('#compileButton').textContent = 'Queued'; $('#compileStatus').textContent = 'Queued'; $('#bottomPanel').classList.remove('collapsed'); const job = await api(`/api/projects/${state.project.id}/compile`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }); state.job.id = job.id; poll(job.id); toast('Compile queued'); } catch (error) { state.job = null; $('#compileButton').disabled = false; $('#compileButton').textContent = 'Compile'; toast(readableError(error), true); } }
function jobLabel(job) { if (job.state === 'queued') return `${job.jobs_ahead || 0} queued jobs ahead`; if (job.state === 'claimed' || job.state === 'running') return 'Compiling'; return job.state === 'succeeded' ? 'Success' : 'Failed'; }
async function poll(id) { try { const job = await api(`/api/jobs/${id}`); const label = jobLabel(job); $('#compileStatus').textContent = label; $('#compileButton').textContent = ['queued','claimed','running'].includes(job.state) ? (job.state === 'queued' ? 'Queued' : 'Compiling') : 'Compile'; if (['queued','claimed','running'].includes(job.state)) return setTimeout(() => poll(id), 900); state.job = null; $('#compileButton').disabled = false; const artifacts = await api(`/api/jobs/${id}/artifacts`); const log = artifacts.find((item) => item.name.endsWith('.log')); const pdf = artifacts.find((item) => item.name.endsWith('.pdf')); if (log) { const content = await apiText(`/api/jobs/${id}/artifacts/${log.id}`); $('#buildLog').textContent = content || 'Build completed without compiler output.'; const errors = (content.match(/^! |^.*(?:Error|Undefined control sequence).*$/gm) || []).length; $('#problemCount').textContent = errors ? `(${errors})` : ''; } if (job.state === 'succeeded') { if (pdf) showPdf(`/api/jobs/${id}/artifacts/${pdf.id}`); toast('Compilation completed'); } else { $('#buildLog').textContent ||= 'Compilation failed. Open the Build Log for details.'; toast('Compilation failed', true); } } catch (error) { state.job = null; $('#compileButton').disabled = false; $('#compileButton').textContent = 'Compile'; toast(readableError(error), true); } }
function showPdf(url) { $('#pdfFrame').src = `${url}${url.includes('?') ? '&' : '?'}v=${Date.now()}`; $('#pdfFrame').classList.remove('hidden'); $('#pdfEmpty').classList.add('hidden'); $('#downloadPdf').href = url; $('#downloadPdf').classList.remove('hidden'); }

async function publish() {
  if (!isTeam() || !state.project.collaboration.can_write) return false; if (!(await flushDirty())) return false; setSaveState('PUBLISHING');
  try { const result = await api(`/api/team-projects/${state.project.collaboration.team_project_id}/publish`, { method: 'POST' }); structuralPrivateProjects.delete(state.project.id); await refreshProject(); setSaveState('SAVED'); toast(`Changes published${result.change_count ? ` (${result.change_count})` : ''}`); return true; }
  catch (error) { if (error.status === 404 && error.message === 'no unpublished changes') { structuralPrivateProjects.delete(state.project.id); await refreshProject(); setSaveState('SAVED'); toast('No unpublished changes'); return true; } if (error.status === 409 && error.body?.conflicts) { showConflicts(error.body.conflicts); setSaveState('CONFLICT'); } else toast(readableError(error), true); return false; }
}
function showConflicts(conflicts) { const list = $('#conflictList'); list.replaceChildren(); for (const conflict of conflicts) { const row = node('div', 'conflict-row'); row.append(node('code', '', conflict.path), node('p', '', conflictCopy[conflict.reason] || 'This change cannot be published yet.')); if (conflict.destination) row.append(node('p', 'muted', `Destination: ${conflict.destination}`)); list.append(row); } $('#conflictDialog').showModal(); }

async function openProjectDialog() {
  $('#projectDialogError').textContent = ''; $('#projectForm').reset(); $('#projectKind').value = state.activeTeam ? `Team — ${state.activeTeam.name}` : 'Personal'; state.templates = await api('/api/templates').catch(() => []); const options = $('#templateOptions'); options.replaceChildren(); const blank = templateChoice('', 'Blank project', 'A simple editable LaTeX document'); blank.querySelector('input').checked = true; options.append(blank); for (const template of state.templates) options.append(templateChoice(template.id, template.name, `${template.description || 'Create from template'}${template.main_file ? ` · Main: ${template.main_file}` : ''}`)); $('#projectDialog').showModal(); }
function templateChoice(id, title, description) { const label = node('label', 'template-option'); const radio = document.createElement('input'); radio.type = 'radio'; radio.name = 'template'; radio.value = id; label.append(radio, node('span', '', title)); label.lastChild.append(node('span', '', description)); return label; }
async function createProject(event) { event.preventDefault(); const name = $('#projectNameInput').value.trim(); const template = $('#projectForm input[name=template]:checked')?.value; if (!name) return; try { let project; if (state.activeTeam) { const endpoint = template ? `/api/teams/${state.activeTeam.id}/templates/${template}/projects` : `/api/teams/${state.activeTeam.id}/projects`; project = await api(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name }) }); $('#projectDialog').close(); await openProject(project.workspace_id); } else { const endpoint = template ? `/api/templates/${template}/projects` : '/api/projects'; project = await api(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name }) }); $('#projectDialog').close(); await openProject(project.id); } toast('Project created'); } catch (error) { $('#projectDialogError').textContent = readableError(error); } }
async function importProject() { if (state.activeTeam) { $('#projectDialogError').textContent = 'Team ZIP import is not available in this release.'; return; } const picker = $('#projectZip'); picker.click(); picker.onchange = async () => { const file = picker.files[0]; const name = $('#projectNameInput').value.trim(); if (!file || !name) { $('#projectDialogError').textContent = 'Enter a project name, then choose a ZIP.'; return; } try { const project = await api(`/api/projects/import?name=${encodeURIComponent(name)}`, { method: 'POST', headers: { 'Content-Type': 'application/zip' }, body: file }); $('#projectDialog').close(); await openProject(project.id); toast('Project imported'); } catch (error) { $('#projectDialogError').textContent = readableError(error); } }; }

async function openMembers() { if (!isTeam() || !state.project.collaboration.can_manage) return; $('#memberError').textContent = ''; $('#capabilitySummary').textContent = 'Writers can edit private copies and publish. Mentors can inspect and compile. Project Managers can manage membership.'; try { const members = await api(`/api/team-projects/${state.project.collaboration.team_project_id}/members`); const list = $('#membersList'); list.replaceChildren(); for (const member of members) { const roles = [member.writer && 'Writer', member.mentor && 'Mentor', member.project_manager && 'Project Manager'].filter(Boolean).join(' · '); const row = node('div', 'member-row'); row.append(node('span', '', member.email), node('span', 'role', roles || 'Read only')); list.append(row); } $('#membersDialog').showModal(); } catch (error) { toast(readableError(error), true); } }
async function addMember(event) { event.preventDefault(); const email = $('#memberEmail').value.trim(); const role = $('#memberRole').value; const roles = { writer: { writer: true, mentor: false, project_manager: false }, mentor: { writer: false, mentor: true, project_manager: false }, manager: { writer: true, mentor: false, project_manager: true } }; try { await api(`/api/team-projects/${state.project.collaboration.team_project_id}/members`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ email, ...roles[role] }) }); toast('Member updated'); await openMembers(); } catch (error) { $('#memberError').textContent = readableError(error); } }

function commands() { return [ ['Compile', compile], ['Save', save], ['Publish Changes', publish], ['Publish & Compile', async () => { if (await publish()) compile(); }], ['Open Problems', () => $('#bottomPanel').classList.remove('collapsed')], ['Toggle PDF', () => { state.preferences.pdf = !state.preferences.pdf; savePreferences(); applyLayout(); }], ['Toggle Sidebar', () => { state.preferences.sidebar = !state.preferences.sidebar; savePreferences(); applyLayout(); }], ['Toggle Bottom Panel', toggleBottom], ['Theme', cycleTheme], ['Reset Layout', resetLayout], ['Quick Open', openQuickOpen] ]; }
function openPalette() { state.commands = commands(); state.commandIndex = 0; $('#commandSearch').value = ''; renderCommands(); $('#commandDialog').showModal(); $('#commandSearch').focus(); }
function renderCommands() { const query = $('#commandSearch').value.toLowerCase(); const items = state.commands.filter(([label]) => label.toLowerCase().includes(query)); const list = $('#commandList'); list.replaceChildren(); items.forEach(([label, run], index) => { const button = node('button', `command-item${index === state.commandIndex ? ' active' : ''}`, label); button.onclick = () => { $('#commandDialog').close(); run(); }; list.append(button); }); $('#commandDialog').dataset.count = String(items.length); }
function openQuickOpen() { if (!state.project) return; state.commands = state.project.files.filter((file) => isText(file.path)).map((file) => [file.path, () => openFile(file.path)]); state.commandIndex = 0; $('#commandSearch').value = ''; renderCommands(); $('#commandDialog').showModal(); $('#commandSearch').focus(); }
function toggleBottom() { const panel = $('#bottomPanel'); panel.classList.toggle('collapsed'); document.querySelector('[data-action="toggle-bottom"]').setAttribute('aria-expanded', String(!panel.classList.contains('collapsed'))); }
function cycleTheme() { const values = ['light','dark','system']; state.preferences.theme = values[(values.indexOf(state.preferences.theme) + 1) % values.length]; savePreferences(); applyTheme(); toast(`${state.preferences.theme[0].toUpperCase() + state.preferences.theme.slice(1)} theme`); }
function openHelp() { toast('Shortcuts: Ctrl/Cmd+S save · Ctrl/Cmd+Enter compile · Ctrl/Cmd+P quick open · Ctrl/Cmd+K commands'); }
function logout() { api('/api/auth/logout', { method: 'POST' }).catch(() => {}).finally(() => { state.project = null; state.tabs.clear(); $('#appView').classList.add('hidden'); $('#loginView').classList.remove('hidden'); }); }
function showAdmin() { $('#workspace').replaceChildren(); const panel = node('main', 'admin-shell'); panel.innerHTML = '<section class="login-card"><div class="wordmark">LaTeX Core / Admin</div><h1>Control plane</h1><p>Global administration is intentionally separate from membership-scoped projects.</p><div class="template-options"><div class="conflict-row"><strong>Overview</strong><p class="muted">Not available in this release.</p></div><div class="conflict-row"><strong>Users</strong><p class="muted">Not available in this release.</p></div><div class="conflict-row"><strong>Teams · Projects · Templates · Build Queue</strong><p class="muted">These operational views require a dedicated global-admin API and are not fabricated here.</p></div><div class="conflict-row"><strong>Mentor Groups · Audit · System</strong><p class="muted">Not available in this release.</p></div></div></section>'; $('#workspace').append(panel); }

function bindResizer(selector, field, min, max) { const bar = $(selector); let start = 0; let value = 0; const move = (event) => { const delta = field === 'bottom' ? start - event.clientY : event.clientX - start; state.preferences[field] = Math.max(min, Math.min(max, value + delta)); applyLayout(); }; const end = () => { document.removeEventListener('pointermove', move); document.removeEventListener('pointerup', end); bar.classList.remove('dragging'); savePreferences(); }; bar.addEventListener('pointerdown', (event) => { start = field === 'bottom' ? event.clientY : event.clientX; value = state.preferences[field]; bar.classList.add('dragging'); document.addEventListener('pointermove', move); document.addEventListener('pointerup', end); }); }
function handleAction(action) { const all = { 'toggle-sidebar': () => { state.preferences.sidebar = !state.preferences.sidebar; savePreferences(); applyLayout(); }, 'new-project': openProjectDialog, 'new-file': () => { if (state.project) { $('#fileDialogError').textContent = ''; $('#fileForm').reset(); $('#fileDialog').showModal(); $('#filePathInput').focus(); } }, save, compile, publish, members: openMembers, help: openHelp, theme: cycleTheme, logout, 'set-main': setMain, 'reload-pdf': () => { const frame = $('#pdfFrame'); if (frame.src) frame.src = frame.src.replace(/([?&]v=)\d+/, `$1${Date.now()}`); }, 'toggle-pdf': () => { state.preferences.pdf = true; savePreferences(); applyLayout(); }, 'toggle-bottom': toggleBottom, 'close-dialog': () => document.querySelectorAll('dialog[open]').forEach((dialog) => dialog.close()), 'import-project': importProject }; all[action]?.(); }
function bindEvents() {
  $('#loginForm').addEventListener('submit', async (event) => { event.preventDefault(); try { const email = $('#email').value.trim().toLowerCase(); await api('/api/auth/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ email, password: $('#password').value }) }); const user = await api('/api/auth/me'); if (user.email !== email) throw new ApiError('authenticated session does not match the requested account', 401); await boot(user); } catch (error) { $('#loginError').textContent = readableError(error); } });
  document.addEventListener('click', (event) => { const target = event.target.closest('[data-action]'); if (target) handleAction(target.dataset.action); }); $('#editor').addEventListener('input', onEditorInput); $('#editor').addEventListener('keydown', (event) => { if (event.key === 'Tab') { event.preventDefault(); const start = event.target.selectionStart; event.target.setRangeText('  ', start, event.target.selectionEnd, 'end'); onEditorInput(); } });
  $('#fileForm').addEventListener('submit', (event) => { event.preventDefault(); createFile(); }); $('#projectForm').addEventListener('submit', createProject); $('#memberForm').addEventListener('submit', addMember); $('#commandSearch').addEventListener('input', () => { state.commandIndex = 0; renderCommands(); });
  document.addEventListener('keydown', (event) => { if (event.key === 'Escape') document.querySelectorAll('dialog[open]').forEach((dialog) => dialog.close()); if (!(event.ctrlKey || event.metaKey)) return; const key = event.key.toLowerCase(); if (key === 's') { event.preventDefault(); save(); } if (key === 'enter') { event.preventDefault(); compile(); } if (key === 'p') { event.preventDefault(); openQuickOpen(); } if (key === 'k') { event.preventDefault(); openPalette(); } });
  $('#commandDialog').addEventListener('keydown', (event) => { const count = Number($('#commandDialog').dataset.count || 0); if (event.key === 'ArrowDown') { event.preventDefault(); state.commandIndex = Math.min(count - 1, state.commandIndex + 1); renderCommands(); } else if (event.key === 'ArrowUp') { event.preventDefault(); state.commandIndex = Math.max(0, state.commandIndex - 1); renderCommands(); } else if (event.key === 'Enter') { event.preventDefault(); const query = $('#commandSearch').value.toLowerCase(); const item = state.commands.filter(([label]) => label.toLowerCase().includes(query))[state.commandIndex]; if (item) { $('#commandDialog').close(); item[1](); } } });
  bindResizer('#leftResizer', 'left', 180, 420); bindResizer('#splitResizer', 'right', 300, 900); bindResizer('#bottomResizer', 'bottom', 100, 480); addEventListener('resize', applyLayout);
}
async function boot(user) { state.user = user; $('#userEmail').textContent = user.email; $('#loginView').classList.add('hidden'); $('#appView').classList.remove('hidden'); applyTheme(); applyLayout(); if (location.pathname === '/admin') { showAdmin(); return; } await loadNavigation(); }
async function init() { bindEvents(); try { const user = await api('/api/auth/me'); await boot(user); } catch (_) { applyTheme(); } }
init();
