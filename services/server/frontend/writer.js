import { basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { StreamLanguage } from '@codemirror/language';
import { stex } from '@codemirror/legacy-modes/mode/stex';
import * as Y from 'yjs';
import { yCollab } from 'y-codemirror.next';
import { IndexeddbPersistence } from 'y-indexeddb';

const SOURCE_UPDATE = 0x01;
const FLUSH = 0x02;
const INITIAL_STATE = 0x10;
const REMOTE_SOURCE_UPDATE = 0x11;
const REMOTE_ORIGIN = Symbol('server-remote');

class PaperApi {
  async request(path, options = {}) {
    const response = await fetch(path, { credentials: 'same-origin', ...options });
    if (!response.ok) {
      const payload = await response.json().catch(() => ({}));
      const error = new Error(payload.error || `Request failed (${response.status})`);
      error.status = response.status;
      throw error;
    }
    return response.status === 204 ? null : response.json();
  }

  me() { return this.request('/api/v2/me'); }
  papers() { return this.request('/api/v2/writer/papers'); }
  createPaper(name) { return this.json('/api/v2/writer/personal-papers', 'POST', { name }); }
  paper(id) { return this.request(`/api/v2/papers/${id}`); }
  files(id) { return this.request(`/api/v2/papers/${id}/files`); }
  file(paperId, fileId) { return this.request(`/api/v2/papers/${paperId}/files/${fileId}`); }
  createFile(paperId, body) { return this.json(`/api/v2/papers/${paperId}/files`, 'POST', body); }
  renameFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}/path`, 'PATCH', body); }
  deleteFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}`, 'DELETE', body); }
  setMain(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/main/${fileId}`, 'POST', body); }
  versions(paperId) { return this.request(`/api/v2/papers/${paperId}/versions`); }
  checkpoint(paperId, name) { return this.json(`/api/v2/papers/${paperId}/versions`, 'POST', { name }); }
  compare(paperId, from, to) { return this.request(`/api/v2/papers/${paperId}/versions/compare?from=${from}&to=${to}`); }
  build(paperId, triggerType) { return this.json(`/api/v2/papers/${paperId}/builds`, 'POST', { trigger_type: triggerType }); }
  buildStatus(paperId) { return this.request(`/api/v2/papers/${paperId}/builds`); }

  json(path, method, body) {
    return this.request(path, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  }
}

const api = new PaperApi();
const ui = Object.fromEntries([
  'myPapers', 'teamPapers', 'fileTree', 'newPaper', 'newFile', 'renameFile', 'deleteFile',
  'setMain', 'saveFile', 'currentPaper', 'currentFile', 'mainBadge', 'saveStatus',
  'editorMount', 'writerNotice', 'compilePaper', 'buildStatus', 'pdfRelation', 'pdfEmpty',
  'pdfFrame', 'createCheckpoint', 'versionHistory', 'versionDiff',
].map((id) => [id, document.getElementById(id)]));

const model = {
  identity: null,
  papers: [],
  paper: null,
  paperDetail: null,
  files: [],
  file: null,
  version: 0,
  view: null,
  collaboration: null,
  conflict: false,
  autoBuildTimer: null,
  currentBuildId: null,
  versions: [],
};

function notice(message, failed = false) {
  ui.writerNotice.textContent = message;
  ui.writerNotice.classList.toggle('danger', failed);
}

function saveState(state) {
  const labels = {
    local: 'Local',
    syncing: 'Syncing…',
    synced: 'Synced',
    offline: 'Offline — stored locally',
    reconnecting: 'Reconnecting…',
    conflict: 'Conflict/Error',
  };
  ui.saveStatus.textContent = labels[state];
  ui.saveStatus.dataset.state = state;
}

function button(text, onClick, active = false) {
  const node = document.createElement('button');
  node.type = 'button';
  node.textContent = text;
  if (active) node.setAttribute('aria-current', 'page');
  node.addEventListener('click', onClick);
  return node;
}

function renderPapers() {
  renderPaperGroup(ui.myPapers, model.papers.filter((paper) => paper.kind === 'personal'), 'No personal papers yet.');
  renderPaperGroup(ui.teamPapers, model.papers.filter((paper) => paper.kind === 'team'), 'No assigned team papers.');
}

function renderPaperGroup(parent, papers, empty) {
  parent.replaceChildren();
  if (!papers.length) {
    const copy = document.createElement('p');
    copy.className = 'empty-copy';
    copy.textContent = empty;
    parent.append(copy);
    return;
  }
  const list = document.createElement('div');
  list.className = 'paper-list';
  papers.forEach((paper) => {
    const item = button(paper.name, () => openPaper(paper), model.paper?.id === paper.id);
    item.title = paper.status === 'active' ? paper.name : `${paper.name} (${paper.status}, read-only)`;
    if (paper.status !== 'active') item.append(Object.assign(document.createElement('small'), { textContent: paper.status }));
    list.append(item);
  });
  parent.append(list);
}

function renderFiles() {
  ui.fileTree.replaceChildren();
  if (!model.paper) {
    ui.fileTree.innerHTML = '<p class="empty-copy">Choose a paper to view its files.</p>';
    return;
  }
  if (!model.files.length) {
    ui.fileTree.innerHTML = '<p class="empty-copy">No files.</p>';
    return;
  }
  const root = { directories: new Map(), files: [] };
  model.files.forEach((file) => {
    const parts = file.path.split('/');
    let node = root;
    parts.slice(0, -1).forEach((part) => {
      if (!node.directories.has(part)) node.directories.set(part, { directories: new Map(), files: [] });
      node = node.directories.get(part);
    });
    node.files.push({ ...file, label: parts.at(-1) });
  });
  ui.fileTree.append(renderTree(root));
}

function renderTree(node) {
  const list = document.createElement('ul');
  list.className = 'file-tree';
  [...node.directories.entries()].sort().forEach(([name, child]) => {
    const item = document.createElement('li');
    const label = document.createElement('span');
    label.className = 'directory';
    label.textContent = name;
    item.append(label, renderTree(child));
    list.append(item);
  });
  node.files.sort((a, b) => a.label.localeCompare(b.label)).forEach((file) => {
    const item = document.createElement('li');
    const leaf = button(file.label, () => openFile(file), model.file?.file_id === file.file_id);
    if (model.paperDetail?.main_file === file.path) leaf.append(Object.assign(document.createElement('small'), { textContent: 'Main' }));
    item.append(leaf);
    list.append(item);
  });
  return list;
}

class CollaborationSession {
  constructor(paper, file, requestedEditable, latex) {
    this.paper = paper;
    this.file = file;
    this.requestedEditable = requestedEditable;
    this.latex = latex;
    this.socket = null;
    this.doc = null;
    this.text = null;
    this.persistence = null;
    this.destroyed = false;
    this.connected = false;
    this.access = 'read_only';
    this.clientSequence = 0;
    this.pending = new Set();
    this.flushWaiters = [];
    this.reconnectTimer = null;
    this.backoff = 250;
    this.metadata = null;
    this.initialState = null;
    this.initializing = false;
    this.bufferedRemote = [];
  }

  start() {
    saveState('local');
    this.connect();
  }

  connect() {
    if (this.destroyed) return;
    saveState(this.doc ? 'reconnecting' : 'local');
    const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const path = `/api/v2/collab/${this.paper.id}/files/${this.file.file_id}`;
    const socket = new WebSocket(`${scheme}//${window.location.host}${path}`);
    socket.binaryType = 'arraybuffer';
    this.socket = socket;
    this.metadata = null;
    this.initialState = null;
    socket.addEventListener('open', () => {
      this.connected = true;
      this.backoff = 250;
      saveState('syncing');
    });
    socket.addEventListener('message', (event) => this.receive(event));
    socket.addEventListener('close', () => {
      if (socket !== this.socket) return;
      this.connected = false;
      this.pending.clear();
      this.rejectFlushes(new Error('connection closed before durable flush'));
      if (!this.destroyed) {
        saveState('reconnecting');
        this.reconnectTimer = window.setTimeout(() => this.connect(), this.backoff);
        this.backoff = Math.min(this.backoff * 2, 5000);
      }
    });
    socket.addEventListener('error', () => {
      if (!this.connected && this.doc) saveState('offline');
    });
  }

  receive(event) {
    if (typeof event.data === 'string') {
      const control = JSON.parse(event.data);
      if (control.type === 'JOIN_ACCEPTED') {
        this.metadata = control;
        this.access = control.access;
        this.initializeOrMerge();
      } else if (control.type === 'DURABLE_ACK') {
        this.pending.delete(control.client_seq);
        if (this.pending.size === 0) {
          saveState('synced');
          scheduleAutoBuild();
        }
      } else if (control.type === 'FLUSHED') {
        this.resolveFlushes(control.durable_seq);
        if (this.pending.size === 0) saveState('synced');
      } else if (control.type === 'REMOTE_DURABLE') {
        if (this.pending.size === 0) saveState('synced');
        scheduleAutoBuild();
      } else if (control.type === 'RELOAD_REQUIRED') {
        model.conflict = true;
        saveState('conflict');
        notice('The collaborative file was deleted. Reload the paper.', true);
        this.destroy();
      } else if (control.type === 'ERROR') {
        model.conflict = true;
        saveState('conflict');
        notice(control.message || 'Collaboration failed.', true);
      }
      return;
    }
    const bytes = new Uint8Array(event.data);
    if (!bytes.length) return;
    if (bytes[0] === INITIAL_STATE) {
      this.initialState = bytes.slice(1);
      this.initializeOrMerge();
    } else if (bytes[0] === REMOTE_SOURCE_UPDATE) {
      saveState('syncing');
      if (this.doc) Y.applyUpdate(this.doc, bytes.slice(1), REMOTE_ORIGIN);
      else this.bufferedRemote.push(bytes.slice(1));
    }
  }

  async initializeOrMerge() {
    if (!this.metadata || !this.initialState || this.initializing) return;
    this.initializing = true;
    try {
      if (!this.doc) {
        this.doc = new Y.Doc();
        this.text = this.doc.getText('source');
        const key = `latex-core:${model.identity.user_id}:${this.paper.workspace_id}:${this.file.file_id}:${this.metadata.document_epoch}`;
        this.persistence = new IndexeddbPersistence(key, this.doc);
        await new Promise((resolve) => this.persistence.once('synced', resolve));
        Y.applyUpdate(this.doc, this.initialState, REMOTE_ORIGIN);
        this.bufferedRemote.forEach((update) => Y.applyUpdate(this.doc, update, REMOTE_ORIGIN));
        this.bufferedRemote = [];
        this.doc.on('update', (update, origin) => {
          if (origin === REMOTE_ORIGIN) return;
          if (this.connected && this.access === 'read_write') this.sendUpdate(update);
          else saveState('offline');
        });
        mountEditor(this.text, this.requestedEditable && this.access === 'read_write', this.latex);
      } else {
        Y.applyUpdate(this.doc, this.initialState, REMOTE_ORIGIN);
      }
      if (this.access === 'read_write') this.sendUpdate(Y.encodeStateAsUpdate(this.doc));
      else saveState('synced');
    } finally {
      this.initializing = false;
    }
  }

  sendUpdate(update) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      saveState('offline');
      return;
    }
    this.clientSequence += 1;
    const sequence = this.clientSequence;
    const frame = new Uint8Array(9 + update.length);
    frame[0] = SOURCE_UPDATE;
    new DataView(frame.buffer).setBigUint64(1, BigInt(sequence));
    frame.set(update, 9);
    this.pending.add(sequence);
    saveState('syncing');
    this.socket.send(frame);
  }

  flush() {
    if (this.access !== 'read_write') return Promise.resolve(0);
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      saveState('offline');
      return Promise.reject(new Error('Offline edits are safely stored locally; reconnect before this structural operation.'));
    }
    saveState('syncing');
    this.socket.send(Uint8Array.of(FLUSH));
    return new Promise((resolve, reject) => {
      const timeout = window.setTimeout(() => reject(new Error('Durable flush timed out.')), 10000);
      this.flushWaiters.push({
        resolve: (value) => { window.clearTimeout(timeout); resolve(value); },
        reject: (error) => { window.clearTimeout(timeout); reject(error); },
      });
    });
  }

  resolveFlushes(sequence) {
    this.flushWaiters.splice(0).forEach(({ resolve }) => resolve(sequence));
  }

  rejectFlushes(error) {
    this.flushWaiters.splice(0).forEach(({ reject }) => reject(error));
  }

  destroy() {
    this.destroyed = true;
    window.clearTimeout(this.reconnectTimer);
    this.rejectFlushes(new Error('collaboration session closed'));
    if (this.socket) this.socket.close();
    if (this.persistence) this.persistence.destroy();
    if (this.doc) this.doc.destroy();
    this.socket = null;
    this.doc = null;
  }
}

async function refreshPapers() {
  model.papers = await api.papers();
  renderPapers();
}

async function openPaper(paper) {
  closeEditor();
  window.clearTimeout(model.autoBuildTimer);
  model.paper = paper;
  model.file = null;
  model.paperDetail = await api.paper(paper.id);
  model.version = model.paperDetail.version;
  model.files = await api.files(paper.id);
  ui.currentPaper.textContent = `${paper.name}${paper.status === 'active' ? '' : ` — ${paper.status} (read-only)`}`;
  ui.newFile.disabled = !model.paperDetail.editable;
  ui.compilePaper.disabled = !model.paperDetail.editable;
  ui.createCheckpoint.disabled = !model.paperDetail.editable;
  renderPapers();
  renderFiles();
  const initial = model.files.find((file) => file.path === model.paperDetail.main_file) || model.files[0];
  if (initial) await openFile(initial, true);
  await Promise.all([refreshBuildStatus(), refreshHistory()]);
}

async function openFile(file, force = false) {
  if (!force) closeEditor();
  try {
    const payload = await api.file(model.paper.id, file.file_id);
    model.file = payload.file;
    model.version = payload.version;
    model.conflict = false;
    ui.currentFile.textContent = payload.file.path;
    ui.mainBadge.hidden = !payload.main;
    ui.editorMount.innerHTML = '<div class="foundation-empty"><strong>Local</strong><p>Opening collaborative document…</p></div>';
    model.collaboration = new CollaborationSession(
      model.paperDetail.paper,
      payload.file,
      payload.editable,
      payload.file.path.endsWith('.tex'),
    );
    model.collaboration.start();
    updateFileActions(payload.editable);
    renderFiles();
  } catch (error) {
    if (error.status === 415) {
      closeEditor();
      ui.editorMount.innerHTML = '<div class="foundation-empty"><strong>Binary file</strong><p>This asset is visible in the tree but is not editable in S3.</p></div>';
      model.file = file;
      updateFileActions(false);
    }
    notice(error.message, true);
  }
}

function mountEditor(ytext, editable, latex) {
  destroyEditorView();
  ui.editorMount.replaceChildren();
  const undoManager = new Y.UndoManager(ytext, { captureTimeout: 500 });
  const extensions = [
    basicSetup,
    keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => { syncCurrent(); return true; } }]),
    keymap.of([{ key: 'Mod-Enter', preventDefault: true, run: () => { manualCompile(); return true; } }]),
    EditorState.readOnly.of(!editable),
    EditorView.editable.of(editable),
    yCollab(ytext, null, { undoManager }),
    EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' } }),
  ];
  if (latex) extensions.push(StreamLanguage.define(stex));
  model.view = new EditorView({
    state: EditorState.create({ doc: ytext.toString(), extensions }),
    parent: ui.editorMount,
  });
}

function destroyEditorView() {
  if (model.view) model.view.destroy();
  model.view = null;
}

function closeEditor() {
  destroyEditorView();
  if (model.collaboration) model.collaboration.destroy();
  model.collaboration = null;
}

function updateFileActions(editable) {
  const selected = Boolean(model.file);
  ui.renameFile.disabled = !selected || !editable;
  ui.deleteFile.disabled = !selected || !editable;
  ui.setMain.disabled = !selected || !editable || ui.mainBadge.hidden === false;
  ui.saveFile.disabled = !selected || !editable;
}

async function syncCurrent(showNotice = true) {
  if (!model.collaboration || model.conflict) return false;
  try {
    await model.collaboration.flush();
    if (showNotice) notice(`Synced ${model.file.path}`);
    return true;
  } catch (error) {
    saveState('offline');
    notice(error.message, true);
    return false;
  }
}

async function requireDurableFlush() {
  if (!await syncCurrent(false)) return false;
  try {
    model.paperDetail = await api.paper(model.paper.id);
    model.version = model.paperDetail.version;
    return true;
  } catch (error) {
    notice(error.message, true);
    return false;
  }
}

async function reloadPaperAndFile(fileId) {
  closeEditor();
  model.paperDetail = await api.paper(model.paper.id);
  model.version = model.paperDetail.version;
  model.files = await api.files(model.paper.id);
  renderFiles();
  const file = model.files.find((candidate) => candidate.file_id === fileId);
  if (file) await openFile(file, true);
}

function scheduleAutoBuild() {
  if (!model.paper || !model.paperDetail?.editable) return;
  window.clearTimeout(model.autoBuildTimer);
  ui.buildStatus.textContent = 'Waiting for edits to settle…';
  model.autoBuildTimer = window.setTimeout(() => requestBuild('auto'), 2000);
}

async function requestBuild(triggerType) {
  if (!model.paper) return;
  try {
    ui.buildStatus.textContent = model.currentBuildId ? 'Rebuilding…' : 'Building…';
    await api.build(model.paper.id, triggerType);
    await Promise.all([refreshBuildStatus(), refreshHistory()]);
  } catch (error) {
    ui.buildStatus.textContent = model.currentBuildId ? 'Build failed — showing last successful PDF' : 'Build failed';
    notice(error.message, true);
  }
}

async function manualCompile() {
  if (!model.paper || !model.paperDetail?.editable) return;
  if (!await syncCurrent(false)) return;
  await requestBuild('manual');
}

async function refreshBuildStatus() {
  if (!model.paper) return;
  try {
    const payload = await api.buildStatus(model.paper.id);
    const build = payload.build;
    const source = build.source_sequence;
    const pdf = build.current_source_sequence;
    const rebuilding = Boolean(build.active_build_id) || (source != null && pdf != null && source !== pdf);
    if (build.current_build_id && build.current_build_id !== model.currentBuildId) {
      model.currentBuildId = build.current_build_id;
      ui.pdfFrame.src = `${payload.pdf_url}?build=${build.current_build_id}`;
    }
    ui.pdfFrame.hidden = !build.current_build_id;
    ui.pdfEmpty.hidden = Boolean(build.current_build_id);
    if (source == null) ui.pdfRelation.textContent = 'No exact source state submitted yet';
    else if (pdf == null) ui.pdfRelation.textContent = `Source version ${source} · No PDF yet`;
    else ui.pdfRelation.textContent = `Source version ${source} · PDF version ${pdf}${rebuilding ? ' · Rebuilding…' : ''}`;
    if (build.active_build_id) ui.buildStatus.textContent = build.current_build_id ? 'Rebuilding…' : 'Building…';
    else if (build.latest_status === 'failed' && source !== pdf) {
      ui.buildStatus.textContent = build.current_build_id ? 'Build failed — showing last successful PDF' : 'Build failed';
      if (build.latest_error?.message) ui.pdfRelation.textContent += ` · ${build.latest_error.message}`;
    } else if (build.current_build_id) ui.buildStatus.textContent = 'Current';
    else ui.buildStatus.textContent = 'No PDF yet';
  } catch (error) {
    notice(error.message, true);
  }
}

async function refreshHistory() {
  if (!model.paper) return;
  try {
    model.versions = await api.versions(model.paper.id);
    renderHistory();
  } catch (error) {
    notice(error.message, true);
  }
}

function renderHistory() {
  ui.versionHistory.replaceChildren();
  if (!model.versions.length) {
    ui.versionHistory.innerHTML = '<p class="empty-copy">No checkpoints yet.</p>';
    return;
  }
  model.versions.forEach((version) => {
    const label = document.createElement('label');
    label.className = 'version-row';
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.value = version.id;
    checkbox.addEventListener('change', compareSelectedVersions);
    const detail = document.createElement('span');
    const title = document.createElement('strong');
    title.textContent = `#${version.version_number} ${version.name || version.version_type.replaceAll('_', ' ')}`;
    const metadata = document.createElement('small');
    metadata.textContent = `${version.author_email} · ${new Date(version.created_at).toLocaleString()}`;
    detail.append(title, metadata);
    label.append(checkbox, detail);
    ui.versionHistory.append(label);
  });
}

async function compareSelectedVersions(event) {
  const selected = [...ui.versionHistory.querySelectorAll('input:checked')];
  if (selected.length > 2) {
    event.target.checked = false;
    return;
  }
  if (selected.length !== 2) {
    ui.versionDiff.textContent = 'Select two versions to compare.';
    return;
  }
  try {
    const comparison = await api.compare(model.paper.id, selected[1].value, selected[0].value);
    const lines = [
      `Added: ${comparison.files_added.join(', ') || 'none'}`,
      `Removed: ${comparison.files_removed.join(', ') || 'none'}`,
      `Changed: ${comparison.files_changed.join(', ') || 'none'}`,
      '',
      ...Object.values(comparison.text_diffs),
    ];
    ui.versionDiff.textContent = lines.join('\n');
  } catch (error) {
    ui.versionDiff.textContent = error.message;
  }
}

ui.newPaper.addEventListener('click', async () => {
  const name = window.prompt('Personal paper name');
  if (!name) return;
  try {
    const result = await api.createPaper(name);
    await refreshPapers();
    await openPaper(model.papers.find((paper) => paper.id === result.paper.id));
  } catch (error) { notice(error.message, true); }
});

ui.newFile.addEventListener('click', async () => {
  const path = window.prompt('New file path (nested paths are supported)');
  if (!path) return;
  try {
    const result = await api.createFile(model.paper.id, { path, content: '', version: model.version });
    model.version = result.version;
    await reloadPaperAndFile(result.file.file_id);
  } catch (error) { notice(error.message, true); }
});

ui.renameFile.addEventListener('click', async () => {
  if (!await requireDurableFlush()) return;
  const path = window.prompt('New file path', model.file.path);
  if (!path || path === model.file.path) return;
  try {
    const result = await api.renameFile(model.paper.id, model.file.file_id, { path, version: model.version });
    model.version = result.version;
    await reloadPaperAndFile(result.file.file_id);
  } catch (error) { notice(error.message, true); }
});

ui.deleteFile.addEventListener('click', async () => {
  if (!window.confirm(`Delete ${model.file.path}?`) || !await requireDurableFlush()) return;
  try {
    const deletedId = model.file.file_id;
    const result = await api.deleteFile(model.paper.id, deletedId, { version: model.version });
    model.version = result.version;
    model.file = null;
    model.conflict = false;
    closeEditor();
    await openPaper(model.paper);
  } catch (error) { notice(error.message, true); }
});

ui.setMain.addEventListener('click', async () => {
  if (!await requireDurableFlush()) return;
  try {
    const result = await api.setMain(model.paper.id, model.file.file_id, { version: model.version });
    model.version = result.version;
    await reloadPaperAndFile(model.file.file_id);
  } catch (error) { notice(error.message, true); }
});

ui.saveFile.addEventListener('click', syncCurrent);
ui.compilePaper.addEventListener('click', manualCompile);
ui.createCheckpoint.addEventListener('click', async () => {
  if (!model.paper || !await syncCurrent(false)) return;
  const name = window.prompt('Checkpoint name');
  if (!name) return;
  try {
    await api.checkpoint(model.paper.id, name);
    await refreshHistory();
    notice(`Checkpoint “${name}” created`);
  } catch (error) { notice(error.message, true); }
});

window.setInterval(() => {
  if (model.paper) refreshBuildStatus();
}, 1500);

api.me()
  .then((identity) => {
    model.identity = identity;
    return refreshPapers();
  })
  .catch((error) => notice(error.message, true));
