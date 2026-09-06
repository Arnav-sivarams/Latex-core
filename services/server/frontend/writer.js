import { basicSetup } from 'codemirror';
import { Compartment, EditorState, StateEffect, StateField } from '@codemirror/state';
import { Decoration, EditorView, keymap } from '@codemirror/view';
import { StreamLanguage } from '@codemirror/language';
import { autocompletion, snippetCompletion } from '@codemirror/autocomplete';
import { stex } from '@codemirror/legacy-modes/mode/stex';
import * as Y from 'yjs';
import { yCollab } from 'y-codemirror.next';
import { IndexeddbPersistence } from 'y-indexeddb';
import { resolveSuggestionRange } from './review-helpers.mjs';
import { createIdleBuildScheduler } from './auto-build.mjs';
import { pdfPreviewState } from './writer-pdf.mjs';
import { MATH_CATALOG } from './math-catalog.mjs';
import {
  buildAlgorithm, buildBibtexEntry, buildCodeListing, buildEquation, buildFigure,
  buildPlot, buildTable, buildTheorem, commonSnippets,
  fuzzyRankFiles, packageRequirement, symbols,
} from './writer-productivity.mjs';

const SOURCE_UPDATE = 0x01;
const FLUSH = 0x02;
const INITIAL_STATE = 0x10;
const REMOTE_SOURCE_UPDATE = 0x11;
const REMOTE_ORIGIN = Symbol('server-remote');
const setReviewMarks = StateEffect.define();
const reviewMarkField = StateField.define({
  create: () => Decoration.none,
  update(marks, transaction) {
    let next = marks.map(transaction.changes);
    transaction.effects.forEach((effect) => { if (effect.is(setReviewMarks)) next = Decoration.set(effect.value, true); });
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

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
  preferences() { return this.request('/api/v2/preferences/editor'); }
  savePreferences(body) { return this.json('/api/v2/preferences/editor', 'PUT', body); }
  papers() { return this.request('/api/v2/writer/papers'); }
  createPaper(name) { return this.json('/api/v2/writer/personal-papers', 'POST', { name }); }
  paper(id) { return this.request(`/api/v2/papers/${id}`); }
  files(id) { return this.request(`/api/v2/papers/${id}/files`); }
  file(paperId, fileId) { return this.request(`/api/v2/papers/${paperId}/files/${fileId}`); }
  createFile(paperId, body) { return this.json(`/api/v2/papers/${paperId}/files`, 'POST', body); }
  renameFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}/path`, 'PATCH', body); }
  deleteFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}`, 'DELETE', body); }
  setMain(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/main/${fileId}`, 'POST', body); }
  structuralUndo(paperId) { return this.json(`/api/v2/papers/${paperId}/structural-undo`, 'POST', {}); }
  structuralRedo(paperId) { return this.json(`/api/v2/papers/${paperId}/structural-redo`, 'POST', {}); }
  intelligence(paperId) { return this.request(`/api/v2/papers/${paperId}/intelligence`); }
  search(paperId, query, caseSensitive) { return this.request(`/api/v2/papers/${paperId}/search?q=${encodeURIComponent(query)}&case_sensitive=${caseSensitive}`); }
  uploadAsset(paperId, path, version, file) { return this.request(`/api/v2/papers/${paperId}/assets?path=${encodeURIComponent(path)}&version=${version}`, { method: 'POST', headers: { 'Content-Type': file.type }, body: file }); }
  map(paperId, body) { return this.json(`/api/v2/reviews/papers/${paperId}/synctex`, 'POST', body); }
  versions(paperId) { return this.request(`/api/v2/papers/${paperId}/versions`); }
  checkpoint(paperId, name) { return this.json(`/api/v2/papers/${paperId}/versions`, 'POST', { name }); }
  compare(paperId, from, to) { return this.request(`/api/v2/papers/${paperId}/versions/compare?from=${from}&to=${to}`); }
  restorationRequests(paperId) { return this.request(`/api/v2/papers/${paperId}/restoration-requests`); }
  requestRestoration(paperId, targetVersionId, reason) { return this.json(`/api/v2/papers/${paperId}/restoration-requests`, 'POST', { target_version_id: targetVersionId, reason: reason || null }); }
  rejectRestoration(requestId, note) { return this.json(`/api/v2/restoration-requests/${requestId}/reject`, 'POST', { note: note || null }); }
  applyRestoration(requestId, note) { return this.json(`/api/v2/restoration-requests/${requestId}/apply`, 'POST', { note: note || null }); }
  revertTeam(paperId, versionId) { return this.json(`/api/v2/papers/${paperId}/versions/${versionId}/revert`, 'POST', { confirmed: true }); }
  restorePersonal(paperId, versionId) { return this.json(`/api/v2/papers/${paperId}/versions/${versionId}/restore`, 'POST', {}); }
  build(paperId, triggerType) { return this.json(`/api/v2/papers/${paperId}/builds`, 'POST', { trigger_type: triggerType }); }
  buildStatus(paperId) { return this.request(`/api/v2/papers/${paperId}/builds`); }
  reviews(paperId) { return this.request(`/api/v2/reviews/papers/${paperId}/threads`); }
  reviewRounds(paperId) { return this.request(`/api/v2/reviews/papers/${paperId}/rounds`); }
  sendForReview(paperId) { return this.json(`/api/v2/reviews/papers/${paperId}/rounds`, 'POST', {}); }
  endReview(paperId, roundId) { return this.json(`/api/v2/reviews/papers/${paperId}/rounds/${roundId}/close`, 'POST', {}); }
  reviewMessage(paperId, threadId, body) { return this.json(`/api/v2/reviews/papers/${paperId}/threads/${threadId}/messages`, 'POST', { body }); }
  reviewState(paperId, threadId, state) { return this.json(`/api/v2/reviews/papers/${paperId}/threads/${threadId}/state`, 'POST', { state }); }
  acceptSuggestion(paperId, threadId, durableSequence) { return this.json(`/api/v2/reviews/papers/${paperId}/threads/${threadId}/suggestion/accept`, 'POST', { durable_sequence: durableSequence }); }
  rejectSuggestion(paperId, threadId, rejectionReason) { return this.json(`/api/v2/reviews/papers/${paperId}/threads/${threadId}/suggestion/reject`, 'POST', { rejection_reason: rejectionReason || null }); }
  documentDetails(paperId) { return this.request(`/api/v2/papers/${paperId}/document-details`); }
  saveDocumentDetails(paperId, body) { return this.json(`/api/v2/papers/${paperId}/document-details`, 'PUT', body); }

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
  'editorMount', 'writerNotice', 'compilePaper', 'sendReview', 'endReview', 'buildStatus', 'pdfRelation', 'pdfEmpty',
  'pdfFrame', 'createCheckpoint', 'versionHistory', 'versionDiff',
  'reviewCounts', 'writerReviewFilters', 'writerReviewList',
    'quickOpen', 'commandPalette', 'uploadImage', 'assetInput',
  'projectSearch', 'caseSensitive', 'searchResults', 'insertMenu', 'symbolPalette',
  'structuralUndo', 'structuralRedo', 'problemsCount', 'problemsList', 'productivityDialog',
  'dialogTitle', 'dialogSearch', 'dialogBody', 'dialogPreview', 'dialogActions',
  'copyRecoveryText',
  'reviewStateBadge', 'undoText', 'redoText', 'mathPalette', 'tableBuilder', 'figureBuilder',
  'plotBuilder', 'problemsToggle', 'historyToggle', 'commentsToggle', 'fileActionsToggle',
  'fileActionsMenu', 'workspaceDrawer', 'drawerTitle', 'drawerClose', 'problems', 'reviews', 'history',
  'moreActions', 'documentDetails', 'documentDetailsPanel', 'documentDetailsBody',
  'toolbarReviewCount', 'accountName', 'roleIdentity', 'editorSettings', 'editorFontSize',
  'editorTheme', 'resetEditorSettings',
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
  currentBuildId: null,
  versions: [],
  restorationRequests: [],
  reviewRounds: [],
  reviewOpen: false,
  currentReviewRound: null,
  reviews: [],
  reviewFilter: 'active',
  intelligence: { labels: [], bibliography: [], diagnostics: [], environments: [], packages: [] },
  intelligenceTimer: null,
  searchTimer: null,
  compileDiagnostic: null,
  recoveryText: null,
  undoManager: null,
  preferences: { font_size_px: 14, theme: 'LIGHT' },
  editorAppearance: new Compartment(),
};

const autoBuild = createIdleBuildScheduler({
  requestBuild: (triggerType) => requestBuild(triggerType),
  setTimer: window.setTimeout.bind(window),
  clearTimer: window.clearTimeout.bind(window),
});

function notice(message, failed = false) {
  ui.writerNotice.textContent = message;
  ui.writerNotice.classList.toggle('danger', failed);
}

function editorAppearance(preference) {
  const dark = preference.theme === 'DARK';
  return EditorView.theme({
    '&': { fontSize: `${preference.font_size_px}px`, backgroundColor: dark ? '#1f2329' : '#ffffff', color: dark ? '#e6edf3' : '#20242a' },
    '.cm-gutters': { backgroundColor: dark ? '#181b20' : '#f5f6f7', color: dark ? '#9da7b3' : '#626b75', borderColor: dark ? '#39414b' : '#d9dde2' },
    '.cm-content': { caretColor: dark ? '#f0f6fc' : '#111827' },
    '.cm-activeLine,.cm-activeLineGutter': { backgroundColor: dark ? '#2a313a' : '#eef4fb' },
    '.cm-selectionBackground,&.cm-focused .cm-selectionBackground': { backgroundColor: dark ? '#315b7d' : '#bfdcff' },
    '.review-source-highlight': { color: dark ? '#fff2b2' : '#241a00' },
  }, { dark });
}

function applyIdentity(identity) {
  model.identity = identity;
  ui.accountName.textContent = identity.display_name || identity.email;
  document.querySelectorAll('.branded-wordmark').forEach((brand) => {
    const image = brand.querySelector('.header-logo'); const fallback = brand.querySelector('span');
    if (identity.branding_logo_url) {
      image.src = identity.branding_logo_url; image.hidden = false; fallback.hidden = true;
      image.onerror = () => { image.hidden = true; fallback.hidden = false; };
    }
  });
}

async function loadPreferences() {
  model.preferences = await api.preferences();
  ui.editorFontSize.value = model.preferences.font_size_px;
  ui.editorTheme.value = model.preferences.theme;
}

async function persistPreferences(preferences) {
  model.preferences = await api.savePreferences(preferences);
  if (model.view) model.view.dispatch({ effects: model.editorAppearance.reconfigure(editorAppearance(model.preferences)) });
}

function saveState(state) {
  const labels = {
    local: 'Saved',
    syncing: 'Saving…',
    synced: 'Saved',
    offline: 'Offline',
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
  renderPaperGroup(ui.teamPapers, model.papers.filter((paper) => paper.kind === 'team'), 'No assigned team reports.');
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
    leaf.addEventListener('contextmenu', async (event) => {
      event.preventDefault();
      if (model.file?.file_id !== file.file_id) await openFile(file);
      ui.fileActionsMenu.hidden = false;
      Object.assign(ui.fileActionsMenu.style, { left: `${Math.min(event.clientX, window.innerWidth - 188)}px`, top: `${Math.min(event.clientY, window.innerHeight - 210)}px` });
    });
    if (model.paperDetail?.main_file === file.path) leaf.append(Object.assign(document.createElement('small'), { textContent: 'Main' }));
    item.append(leaf);
    list.append(item);
  });
  return list;
}

function renderProblems() {
  const diagnostics = [...(model.intelligence.diagnostics || [])];
  if (model.compileDiagnostic) diagnostics.push(model.compileDiagnostic);
  ui.problemsCount.textContent = diagnostics.length;
  ui.problemsList.replaceChildren();
  if (!diagnostics.length) {
    ui.problemsList.innerHTML = '<p class="empty-copy">No current problems.</p>';
    return;
  }
  diagnostics.forEach((diagnostic) => {
    const row = button(diagnostic.message, () => diagnostic.file_id && openLocation(diagnostic));
    row.className = `problem-row ${diagnostic.severity || 'error'}`;
    const line = diagnostic.range?.start_line;
    row.append(Object.assign(document.createElement('small'), { textContent: `${diagnostic.path || 'Build'}${line ? `:${line}` : ''} · ${diagnostic.code || diagnostic.severity}` }));
    ui.problemsList.append(row);
  });
}

async function openLocation(location) {
  const file = model.files.find((candidate) => candidate.file_id === location.file_id);
  if (!file) return;
  if (model.file?.file_id !== file.file_id) await openFile(file);
  if (!model.collaboration) return;
  await model.collaboration.ready;
  const lineNumber = Math.min(Math.max(location.range?.start_line || location.line || 1, 1), model.view.state.doc.lines);
  const line = model.view.state.doc.line(lineNumber);
  const from = Math.min(line.to, line.from + (location.range?.start_column || location.column || 0));
  const to = location.range?.end_line === lineNumber ? Math.min(line.to, line.from + (location.range?.end_column || 0)) : from;
  model.view.dispatch({ selection: { anchor: from, head: Math.max(from, to) }, scrollIntoView: true });
  model.view.focus();
}

async function refreshIntelligence() {
  if (!model.paper) return;
  try {
    model.intelligence = await api.intelligence(model.paper.id);
    renderProblems();
  } catch (error) { notice(error.message, true); }
}

function scheduleIntelligence() {
  window.clearTimeout(model.intelligenceTimer);
  model.intelligenceTimer = window.setTimeout(refreshIntelligence, 650);
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
    this.documentEpoch = null;
    this.initialState = null;
    this.initializing = false;
    this.bufferedRemote = [];
    this.ready = new Promise((resolve) => { this.resolveReady = resolve; });
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
        if (this.documentEpoch !== null && this.documentEpoch !== control.document_epoch) {
          this.preserveOldEpoch(control.document_epoch);
          return;
        }
        this.documentEpoch = control.document_epoch;
        this.metadata = control;
        this.access = control.access;
        this.initializeOrMerge();
      } else if (control.type === 'DURABLE_ACK') {
        this.pending.delete(control.client_seq);
        if (this.pending.size === 0) {
          saveState('synced');
          scheduleIntelligence();
          if (model.paperDetail?.editable) autoBuild.durableUpdate();
        }
      } else if (control.type === 'FLUSHED') {
        this.resolveFlushes(control.durable_seq);
        if (this.pending.size === 0) saveState('synced');
      } else if (control.type === 'REMOTE_DURABLE') {
        if (this.pending.size === 0) saveState('synced');
        scheduleIntelligence();
        if (model.paperDetail?.editable) autoBuild.durableUpdate();
      } else if (control.type === 'REVIEW_PUBLISHED') {
        refreshReviews();
        refreshReviewRounds();
        notice(`${control.published_count} new review item${control.published_count === 1 ? '' : 's'} published.`);
      } else if (control.type === 'RELOAD_REQUIRED') {
        model.conflict = true;
        saveState('conflict');
        notice('The collaborative file was deleted. Reload the paper.', true);
        this.destroy();
      } else if (control.type === 'PAPER_EPOCH_CHANGED') {
        this.preserveOldEpoch(control.document_epoch);
      } else if (control.type === 'POLICY_CHANGED') {
        model.conflict = true;
        saveState('conflict');
        notice(control.message || 'File policy changed; reload the paper.', true);
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

  preserveOldEpoch(newEpoch) {
    model.recoveryText = this.text?.toString() || '';
    ui.copyRecoveryText.hidden = !model.recoveryText;
    model.conflict = true;
    saveState('conflict');
    notice('Offline changes from the previous paper version were preserved locally and were not merged after restoration.', true);
    this.destroy();
    if (model.paper && Number.isInteger(newEpoch)) window.setTimeout(() => openPaper(model.paper), 0);
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
        this.resolveReady();
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
  autoBuild.cancel();
  closeEditor();
  model.currentBuildId = null;
  ui.pdfFrame.removeAttribute('src');
  ui.pdfFrame.hidden = true;
  ui.pdfEmpty.hidden = false;
  model.paper = paper;
  model.file = null;
  model.paperDetail = await api.paper(paper.id);
  model.version = model.paperDetail.version;
  model.files = await api.files(paper.id);
  ui.currentPaper.textContent = `${paper.name}${paper.status === 'active' ? '' : ` — ${paper.status} (read-only)`}`;
  ui.roleIdentity.textContent = paper.is_team_leader ? 'Writer · Team leader' : 'Writer';
  ui.newFile.disabled = !model.paperDetail.editable;
  ui.uploadImage.disabled = !model.paperDetail.editable;
  ui.projectSearch.disabled = false;
  ui.structuralUndo.disabled = !model.paperDetail.editable;
  ui.structuralRedo.disabled = !model.paperDetail.editable;
  ui.compilePaper.disabled = !model.paperDetail.editable;
  ui.fileActionsToggle.disabled = false;
  ui.documentDetails.disabled = paper.kind !== 'team';
  const teamLeader = paper.kind === 'team' && paper.is_team_leader;
  ui.createCheckpoint.hidden = paper.kind === 'team' && !teamLeader;
  ui.sendReview.hidden = !teamLeader;
  ui.endReview.hidden = !teamLeader;
  ui.createCheckpoint.disabled = !model.paperDetail.editable || (paper.kind === 'team' && !paper.is_team_leader);
  ui.sendReview.disabled = paper.kind !== 'team' || !paper.is_team_leader || !model.paperDetail.editable;
  ui.endReview.disabled = true;
  ui.reviewStateBadge.textContent = 'Draft';
  renderPapers();
  renderFiles();
  const initial = model.files.find((file) => file.path === model.paperDetail.main_file) || model.files[0];
  if (initial) await openFile(initial, true);
  await Promise.all([refreshBuildStatus(), refreshHistory(), refreshReviews(), refreshReviewRounds(), refreshIntelligence()]);
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
      const raster = /\.(png|jpe?g)$/i.test(file.path);
      ui.editorMount.replaceChildren();
      const preview = document.createElement('div');
      preview.className = raster ? 'binary-preview' : 'foundation-empty';
      if (raster) {
        const image = document.createElement('img');
        image.alt = `Preview of ${file.path}`;
        image.src = `/api/v2/papers/${model.paper.id}/files/${file.file_id}/raw`;
        const insert = button('Insert Figure', async () => {
          const target = model.files.find((candidate) => candidate.path === model.paperDetail.main_file && candidate.file_id !== file.file_id)
            || model.files.find((candidate) => candidate.path.endsWith('.tex'));
          if (!target) return notice('Open a LaTeX source file before inserting a figure.', true);
          await openFile(target);
          openBuilder('figure', { asset: file.path });
        });
        preview.append(image, insert);
      } else {
        const heading = document.createElement('strong');
        heading.textContent = 'Binary asset';
        const copy = document.createElement('p');
        copy.textContent = 'This file is stored in the paper and is not source-editable.';
        preview.append(heading, copy);
      }
      ui.editorMount.append(preview);
      model.file = file;
      updateFileActions(false);
    }
    notice(error.message, true);
  }
}

const latexCommands = [
  ['\\section', '\\section{${title}}'], ['\\subsection', '\\subsection{${title}}'],
  ['\\textbf', '\\textbf{${text}}'], ['\\textit', '\\textit{${text}}'], ['\\emph', '\\emph{${text}}'],
  ['\\label', '\\label{${label}}'], ['\\ref', '\\ref{${label}}'], ['\\eqref', '\\eqref{${label}}'],
  ['\\cite', '\\cite{${key}}'], ['\\includegraphics', '\\includegraphics[width=${\\linewidth}]{${asset}}'],
  ['\\frac', '\\frac{${numerator}}{${denominator}}'], ['\\sqrt', '\\sqrt{${value}}'],
];
const latexEnvironments = ['itemize', 'enumerate', 'equation', 'align', 'figure', 'table', 'tabular', 'theorem', 'proof', 'algorithm', 'lstlisting'];

function latexCompletionSource(context) {
  const line = context.state.doc.lineAt(context.pos);
  const before = context.state.sliceDoc(line.from, context.pos);
  const citation = before.match(/\\cite\w*\{([^}]*)$/);
  if (citation) {
    const fragment = citation[1].split(',').at(-1).trimStart();
    return { from: context.pos - fragment.length, options: (model.intelligence.bibliography || []).map((entry) => ({ label: entry.key, detail: [entry.author, entry.title].filter(Boolean).join(' · '), type: 'variable' })) };
  }
  const reference = before.match(/\\(?:ref|eqref|autoref)\{([^}]*)$/);
  if (reference) {
    return { from: context.pos - reference[1].length, options: (model.intelligence.labels || []).map((label) => ({ label: label.key, detail: label.path, type: 'variable' })) };
  }
  const word = context.matchBefore(/\\[A-Za-z]*/);
  if (!word || (word.from === word.to && !context.explicit)) return null;
  const options = latexCommands.map(([label, template]) => snippetCompletion(template, { label, type: 'keyword' }));
  options.push(...latexEnvironments.map((environment) => snippetCompletion(`\\begin{${environment}}\n  \${}\n\\end{${environment}}`, { label: `\\begin{${environment}}`, type: 'keyword' })));
  return { from: word.from, options };
}

function mountEditor(ytext, editable, latex) {
  destroyEditorView();
  ui.editorMount.replaceChildren();
  const undoManager = new Y.UndoManager(ytext, { captureTimeout: 500 });
  model.undoManager = undoManager;
  const extensions = [
    basicSetup,
    keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => { syncCurrent(); return true; } }]),
    keymap.of([{ key: 'Mod-Enter', preventDefault: true, run: () => { manualCompile(); return true; } }]),
    EditorState.readOnly.of(!editable),
    EditorView.editable.of(editable),
    yCollab(ytext, null, { undoManager }), reviewMarkField,
    EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' } }),
    model.editorAppearance.of(editorAppearance(model.preferences)),
  ];
  if (latex) extensions.push(StreamLanguage.define(stex), autocompletion({ override: [latexCompletionSource] }));
  model.view = new EditorView({
    state: EditorState.create({ doc: ytext.toString(), extensions }),
    parent: ui.editorMount,
  });
  ui.undoText.disabled = !editable;
  ui.redoText.disabled = !editable;
  applyReviewHighlights();
}

function destroyEditorView() {
  if (model.view) model.view.destroy();
  model.view = null;
  model.undoManager = null;
  ui.undoText.disabled = true;
  ui.redoText.disabled = true;
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
  ui.insertMenu.disabled = !selected || !editable;
  ui.symbolPalette.disabled = !selected || !editable;
  ui.mathPalette.disabled = !selected || !editable;
  ui.tableBuilder.disabled = !selected || !editable;
  ui.figureBuilder.disabled = !selected || !editable;
  ui.plotBuilder.disabled = !selected || !editable;
  ui.fileActionsToggle.disabled = !selected;
}

async function syncCurrent(showNotice = true) {
  if (!model.collaboration || model.conflict) return false;
  try {
    await model.collaboration.flush();
    scheduleIntelligence();
    if (showNotice) notice(`Saved ${model.file.path}`);
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
  autoBuild.cancel();
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
    const preview = pdfPreviewState(build);
    const rebuilding = Boolean(build.active_build_id) || (source != null && pdf != null && source !== pdf);
    model.compileDiagnostic = build.latest_status === 'failed' && build.latest_error?.message
      ? { severity: 'error', code: 'compile', message: build.latest_error.message, path: null, file_id: null }
      : null;
    if (build.current_build_id && build.current_build_id !== model.currentBuildId) {
      model.currentBuildId = build.current_build_id;
      ui.pdfFrame.src = `${payload.pdf_url}?build=${build.current_build_id}`;
    }
    ui.pdfFrame.hidden = !preview.viewer;
    ui.pdfEmpty.hidden = !preview.empty;
    if (source == null) ui.pdfRelation.textContent = 'No exact source state submitted yet';
    else if (pdf == null) ui.pdfRelation.textContent = `Source version ${source} · No PDF yet`;
    else ui.pdfRelation.textContent = `Source version ${source} · PDF version ${pdf}${rebuilding ? ' · Rebuilding…' : ''}`;
    if (build.active_build_id) ui.buildStatus.textContent = build.current_build_id ? 'Rebuilding…' : 'Building…';
    else if (build.latest_status === 'failed' && source !== pdf) {
      ui.buildStatus.textContent = build.current_build_id ? 'Build failed — showing last successful PDF' : 'Build failed';
      if (build.latest_error?.message) ui.pdfRelation.textContent += ` · ${build.latest_error.message}`;
    } else if (build.current_build_id) ui.buildStatus.textContent = 'Current';
    else ui.buildStatus.textContent = 'No PDF yet';
    renderProblems();
  } catch (error) {
    notice(error.message, true);
  }
}

async function refreshHistory() {
  if (!model.paper) return;
  try {
    [model.versions, model.restorationRequests] = await Promise.all([
      api.versions(model.paper.id), api.restorationRequests(model.paper.id),
    ]);
    renderHistory();
  } catch (error) {
    notice(error.message, true);
  }
}

async function refreshReviewRounds() {
  if (!model.paper || model.paper.kind !== 'team') {
    model.reviewRounds = [];
    model.reviewOpen = false;
    model.currentReviewRound = null;
    ui.sendReview.disabled = true;
    ui.endReview.disabled = true;
    return;
  }
  try {
    const payload = await api.reviewRounds(model.paper.id);
    model.reviewRounds = payload.rounds;
    model.reviewOpen = payload.review_open;
    model.currentReviewRound = payload.current_review_round;
    const leader = Boolean(model.paper.is_team_leader && model.paperDetail?.editable);
    ui.sendReview.disabled = !leader || model.reviewOpen;
    ui.endReview.disabled = !leader || !model.reviewOpen;
    ui.reviewStateBadge.textContent = model.reviewOpen ? 'In Review' : 'Draft';
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
    const requests = model.restorationRequests.filter((item) => item.target_version_id === version.id);
    if (requests.length) {
      const status = document.createElement('small');
      status.textContent = `Revert: ${requests.map((request) => request.state.replaceAll('_', ' ')).join(', ')}`;
      detail.append(status);
    }
    if (model.paper.kind === 'team') {
      if (model.paper.is_team_leader) {
        requests.filter((request) => request.state === 'REQUESTED').forEach((request) => {
          label.append(
            button(`Reject request from ${request.writer_email}`, async () => {
              const note = window.prompt('Optional rejection note', ''); if (note === null) return;
              try { await api.rejectRestoration(request.id, note); await refreshHistory(); notice('Revert request rejected.'); }
              catch (error) { notice(error.message, true); }
            }),
            button(`Revert for ${request.writer_email}`, async () => {
              if (!window.confirm('Revert this Team report as a new head? The current state will be retained as PRE_RESTORE_SAFETY.')) return;
              const note = window.prompt('Optional decision note', ''); if (note === null) return;
              try { await api.applyRestoration(request.id, note); await openPaper(model.paper); notice('Team report reverted as a new current version.'); }
              catch (error) { notice(error.message, true); }
            }),
          );
        });
        label.append(button('Revert', async () => {
          if (!window.confirm('Revert this Team report directly as a new head? The current state will be retained as PRE_RESTORE_SAFETY.')) return;
          try { await api.revertTeam(model.paper.id, version.id); await openPaper(model.paper); notice('Team report reverted as a new current version.'); }
          catch (error) { notice(error.message, true); }
        }));
      } else {
        const action = button('Request Revert', async () => {
          const reason = window.prompt('Optional reason for reverting to this version', '') ?? null;
          if (reason === null) return;
          try {
            await api.requestRestoration(model.paper.id, version.id, reason);
            await refreshHistory();
            notice('Revert request sent to the Team Leader.');
          } catch (error) { notice(error.message, true); }
        });
        label.append(action);
      }
    } else {
      const action = button('Restore this personal version', async () => {
        if (!window.confirm('Restore this personal paper as a new current head? The current state will be saved permanently as a safety version.')) return;
        try {
          await api.restorePersonal(model.paper.id, version.id);
          await openPaper(model.paper);
          notice('Personal paper restored as a new current version.');
        } catch (error) { notice(error.message, true); }
      });
      label.append(action);
    }
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

async function refreshReviews() {
  if (!model.paper || model.paper.kind !== 'team') {
    model.reviews = [];
    ui.reviewCounts.textContent = '';
    ui.toolbarReviewCount.textContent = '0';
    ui.writerReviewList.innerHTML = '<p class="empty-copy">Mentor review applies to assigned Team reports.</p>';
    return;
  }
  try {
    const payload = await api.reviews(model.paper.id);
    model.reviews = payload.threads;
    renderReviews();
  } catch (error) {
    ui.writerReviewList.innerHTML = `<p class="empty-copy">${error.message}</p>`;
  }
}

function renderReviews() {
  const active = model.reviews.filter((thread) => ['OPEN', 'REOPENED', 'ADDRESSED'].includes(thread.state)).length;
  const addressed = model.reviews.filter((thread) => thread.state === 'ADDRESSED').length;
  const resolved = model.reviews.filter((thread) => thread.state === 'RESOLVED').length;
  const blocking = model.reviews.filter((thread) => thread.severity === 'BLOCKING' && thread.state !== 'RESOLVED').length;
  ui.reviewCounts.textContent = `${active} open · ${addressed} addressed · ${resolved} resolved · ${blocking} blocking`;
  ui.toolbarReviewCount.textContent = String(active);
  applyReviewHighlights();
  const visible = model.reviews.filter((thread) => model.reviewFilter === 'all'
    || (model.reviewFilter === 'active' && ['OPEN', 'REOPENED', 'ADDRESSED'].includes(thread.state))
    || (model.reviewFilter === 'blocking' && thread.severity === 'BLOCKING' && thread.state !== 'RESOLVED')
    || thread.state === model.reviewFilter);
  ui.writerReviewList.replaceChildren();
  if (!visible.length) {
    ui.writerReviewList.innerHTML = '<p class="empty-copy">No matching review threads.</p>';
    return;
  }
  visible.forEach((thread) => {
    const card = document.createElement('article');
    card.className = `thread-card severity-${thread.severity.toLowerCase()}`;
    const heading = button(`${thread.thread_type.replaceAll('_', ' ')} · ${thread.severity} · ${thread.state}`, () => focusWriterReview(thread));
    heading.className = 'thread-title';
    const metadata = document.createElement('p');
    const fileName = thread.source_anchor?.path?.split('/').at(-1) || `PDF page ${thread.pdf_anchor?.page || '?'}`;
    const excerpt = thread.source_anchor?.quoted_text ? `“${thread.source_anchor.quoted_text.slice(0, 100)}”` : 'PDF-only annotation';
    metadata.textContent = `${fileName} · ${excerpt} · ${thread.category} · ${thread.mentor_email}${thread.assigned_writer_email ? ` · assigned to ${thread.assigned_writer_email}` : ''}${thread.due_at ? ` · due ${new Date(thread.due_at).toLocaleDateString()}` : ''} · ${writerAnchorStatus(thread)}`;
    const discussion = document.createElement('div');
    discussion.className = 'discussion';
    thread.messages.forEach((message) => {
      const row = document.createElement('p');
      const author = document.createElement('strong');
      author.textContent = `${message.author_email}: `;
      row.append(author, document.createTextNode(message.body));
      discussion.append(row);
    });
    const actions = document.createElement('div');
    actions.className = 'thread-actions';
    actions.append(button('Reply', () => writerReply(thread)));
    if (['OPEN', 'REOPENED', 'ADDRESSED'].includes(thread.state)) actions.append(button('Done', () => writerDone(thread)));
    if (thread.suggestion?.status === 'PENDING') actions.append(button('Accept', () => writerAcceptSuggestion(thread)), button('Reject', () => writerRejectSuggestion(thread)));
    card.append(heading, metadata, discussion, actions);
    ui.writerReviewList.append(card);
  });
}

function applyReviewHighlights() {
  if (!model.view || !model.collaboration?.doc || !model.file) return;
  const marks = [];
  model.reviews.filter((thread) => ['OPEN', 'REOPENED', 'ADDRESSED'].includes(thread.state)
    && thread.source_anchor?.file_id === model.file.file_id && !thread.source_anchor.file_deleted).forEach((thread) => {
    const anchor = thread.source_anchor;
    const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, decodeBase64(anchor.encoded_relative_start), decodeBase64(anchor.encoded_relative_end));
    if (!range || range.to <= range.from || model.collaboration.text.toString().slice(range.from, range.to) !== anchor.quoted_text) return;
    const body = thread.messages?.[0]?.body || 'Review annotation';
    marks.push(Decoration.mark({
      class: `review-source-highlight review-${thread.thread_type.toLowerCase()}`,
      attributes: { 'data-review-thread': thread.id, 'data-review-kind': thread.thread_type.toLowerCase(), title: `${thread.mentor_email}: ${body} · Done` },
    }).range(range.from, range.to));
  });
  model.view.dispatch({ effects: setReviewMarks.of(marks) });
}

let reviewPopoverTimer = null;
function hideReviewPopover() {
  window.clearTimeout(reviewPopoverTimer);
  reviewPopoverTimer = window.setTimeout(() => document.querySelector('#writerReviewPopover')?.remove(), 140);
}
ui.editorMount.addEventListener('mouseover', (event) => {
  const mark = event.target.closest?.('[data-review-thread]');
  if (!mark) return;
  const thread = model.reviews.find((candidate) => candidate.id === mark.dataset.reviewThread);
  if (!thread) return;
  showWriterReviewPopover(thread, mark.getBoundingClientRect());
});

function showWriterReviewPopover(thread, bounds) {
  window.clearTimeout(reviewPopoverTimer);
  document.querySelector('#writerReviewPopover')?.remove();
  const popover = document.createElement('aside'); popover.id = 'writerReviewPopover'; popover.className = 'review-highlight-popover';
  const body = thread.messages?.[0]?.body || 'Review annotation';
  popover.append(
    Object.assign(document.createElement('strong'), { textContent: thread.mentor_email }),
    Object.assign(document.createElement('span'), { className: 'review-kind', textContent: thread.thread_type.toLowerCase() }),
    Object.assign(document.createElement('p'), { textContent: body }),
  );
  popover.append(button('Done', async () => {
    try { await api.reviewState(model.paper.id, thread.id, 'RESOLVED'); popover.remove(); await refreshReviews(); }
    catch (error) { notice(error.message, true); }
  }));
  if (thread.suggestion?.status === 'PENDING') popover.append(button('Apply', () => writerAcceptSuggestion(thread)));
  document.body.append(popover);
  const width = popover.offsetWidth; const height = popover.offsetHeight;
  const left = Math.max(8, Math.min(bounds.left, window.innerWidth - width - 8));
  const top = bounds.bottom + height + 8 < window.innerHeight ? bounds.bottom + 6 : Math.max(8, bounds.top - height - 6);
  Object.assign(popover.style, { left: `${left}px`, top: `${top}px` });
  popover.addEventListener('mouseenter', () => window.clearTimeout(reviewPopoverTimer)); popover.addEventListener('mouseleave', hideReviewPopover);
}
ui.editorMount.addEventListener('mouseout', (event) => { if (event.target.closest?.('[data-review-thread]')) hideReviewPopover(); });

function writerAnchorStatus(thread) {
  if (thread.source_anchor?.file_deleted) return 'SOURCE_DELETED';
  if (thread.source_anchor && model.file?.file_id === thread.source_anchor.file_id && model.collaboration?.doc) {
    const anchor = thread.source_anchor;
    const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, decodeBase64(anchor.encoded_relative_start), decodeBase64(anchor.encoded_relative_end));
    if (!range || model.collaboration.text.toString().slice(range.from, range.to) !== anchor.quoted_text) return 'Source changed';
  }
  return thread.pdf_anchor?.mapping_status || (thread.source_anchor ? 'Source linked' : 'PDF_ONLY');
}

async function writerReply(thread) {
  const body = window.prompt('Reply to Mentor');
  if (!body) return;
  try { await api.reviewMessage(model.paper.id, thread.id, body); await refreshReviews(); }
  catch (error) { notice(error.message, true); }
}

async function writerDone(thread) {
  try { await api.reviewState(model.paper.id, thread.id, 'RESOLVED'); await refreshReviews(); }
  catch (error) { notice(error.message, true); }
}

async function focusWriterReview(thread) {
  const anchor = thread.source_anchor;
  if (anchor && !anchor.file_deleted) {
    const file = model.files.find((candidate) => candidate.file_id === anchor.file_id);
    if (file) {
      if (model.file?.file_id !== file.file_id) {
        if (model.collaboration?.access === 'read_write' && !await syncCurrent(false)) return;
        await openFile(file);
      }
      await model.collaboration.ready;
      const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, decodeBase64(anchor.encoded_relative_start), decodeBase64(anchor.encoded_relative_end));
      const current = range && model.collaboration.text.toString().slice(range.from, range.to);
      if (range && current === anchor.quoted_text) {
        model.view.dispatch({ selection: { anchor: range.from, head: range.to }, scrollIntoView: true });
        model.view.focus();
        requestAnimationFrame(() => {
          const mark = model.view?.dom.querySelector(`[data-review-thread="${thread.id}"]`);
          if (mark) { mark.classList.add('review-focus-emphasis'); window.setTimeout(() => mark.classList.remove('review-focus-emphasis'), 1800); showWriterReviewPopover(thread, mark.getBoundingClientRect()); }
        });
        return;
      }
      showReviewedPdf(thread);
      return notice(`Source changed. Original reviewed excerpt: “${anchor.quoted_text.slice(0, 160)}”. Showing the reviewed version/PDF location when available.`, true);
    }
  }
  showReviewedPdf(thread);
  if (thread.pdf_anchor) notice(`PDF-only annotation on reviewed PDF page ${thread.pdf_anchor.page}; exact source mapping is unavailable.`);
  else notice(`Source changed. Original reviewed excerpt: “${anchor?.quoted_text?.slice(0, 160) || 'unavailable'}”.`, true);
}

function showReviewedPdf(thread) {
  if (!thread.pdf_anchor) return;
  const build = thread.pdf_anchor.build_id || model.currentBuildId;
  ui.pdfFrame.src = `/api/v2/papers/${model.paper.id}/artifacts/pdf?build=${build}#page=${thread.pdf_anchor.page}`;
  ui.pdfFrame.hidden = false; ui.pdfEmpty.hidden = true;
}

async function writerAcceptSuggestion(thread) {
  const anchor = thread.source_anchor;
  if (!anchor || anchor.file_deleted) return notice('Suggestion source is unavailable; re-anchor before accepting.', true);
  const file = model.files.find((candidate) => candidate.file_id === anchor.file_id);
  if (!file) return notice('Suggestion file no longer exists; nothing was changed.', true);
  try {
    if (model.file?.file_id !== file.file_id) {
      if (model.collaboration?.access === 'read_write' && !await syncCurrent(false)) return;
      await openFile(file);
    }
    await model.collaboration.ready;
    const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, decodeBase64(anchor.encoded_relative_start), decodeBase64(anchor.encoded_relative_end));
    if (!range) return notice('Suggestion anchor is stale or unresolved; nothing was changed.', true);
    if (model.collaboration.text.toString().slice(range.from, range.to) !== anchor.quoted_text) return notice(`Source changed. The reviewed excerpt was “${anchor.quoted_text.slice(0, 160)}”; nothing was changed.`, true);
    model.collaboration.doc.transact(() => {
      model.collaboration.text.delete(range.from, range.to - range.from);
      model.collaboration.text.insert(range.from, thread.suggestion.replacement_text);
    }, 'writer-suggestion-accept');
    const durableSequence = await model.collaboration.flush();
    await api.acceptSuggestion(model.paper.id, thread.id, durableSequence);
    await refreshReviews();
    notice('Suggestion accepted as your durable Writer edit.');
  } catch (error) { notice(error.message, true); }
}

async function writerRejectSuggestion(thread) {
  const reason = window.prompt('Optional rejection reason') || null;
  try { await api.rejectSuggestion(model.paper.id, thread.id, reason); await refreshReviews(); notice('Suggestion rejected.'); }
  catch (error) { notice(error.message, true); }
}

function decodeBase64(value) {
  if (!value) return null;
  const binary = atob(value.replaceAll('\n', ''));
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function closeDialog() {
  if (ui.productivityDialog.open) ui.productivityDialog.close();
}

function showPalette(title, items) {
  ui.dialogTitle.textContent = title;
  ui.dialogBody.replaceChildren();
  ui.dialogActions.replaceChildren();
  ui.dialogPreview.hidden = true;
  ui.dialogSearch.hidden = false;
  ui.dialogSearch.value = '';
  let selected = 0;
  const render = () => {
    const query = ui.dialogSearch.value.toLowerCase();
    const visible = items.filter((item) => `${item.label} ${item.detail || ''} ${item.search || ''}`.toLowerCase().includes(query));
    selected = Math.min(selected, Math.max(visible.length - 1, 0));
    const list = document.createElement('div');
    list.className = 'palette-list';
    visible.forEach((item, index) => {
      const node = button(item.label, async () => { closeDialog(); await item.run(); }, index === selected);
      if (item.detail) node.title = item.detail;
      list.append(node);
    });
    ui.dialogBody.replaceChildren(list);
  };
  ui.dialogSearch.oninput = () => { selected = 0; render(); };
  ui.dialogSearch.onkeydown = (event) => {
    const count = ui.dialogBody.querySelectorAll('button').length;
    if (event.key === 'ArrowDown') { event.preventDefault(); selected = Math.min(selected + 1, count - 1); render(); }
    if (event.key === 'ArrowUp') { event.preventDefault(); selected = Math.max(selected - 1, 0); render(); }
    if (event.key === 'Enter') { event.preventDefault(); ui.dialogBody.querySelectorAll('button')[selected]?.click(); }
    if (event.key === 'Escape') closeDialog();
  };
  render();
  ui.productivityDialog.showModal();
  ui.dialogSearch.focus();
}

function quickOpen() {
  showPalette('Quick Open', fuzzyRankFiles(model.files, '').map((file) => ({ label: file.path, run: () => openFile(file) })));
  ui.dialogSearch.oninput = () => {
    const ranked = fuzzyRankFiles(model.files, ui.dialogSearch.value);
    const list = document.createElement('div');
    list.className = 'palette-list';
    ranked.forEach((file) => list.append(button(file.path, () => { closeDialog(); openFile(file); })));
    ui.dialogBody.replaceChildren(list);
  };
}

function insertLatex(source, origin = 'writer-builder') {
  if (!model.view || !model.collaboration || model.collaboration.access !== 'read_write') {
    notice('Open an editable text file before inserting LaTeX.', true);
    return false;
  }
  const selection = model.view.state.selection.main;
  model.collaboration.doc.transact(() => {
    if (selection.to > selection.from) model.collaboration.text.delete(selection.from, selection.to - selection.from);
    model.collaboration.text.insert(selection.from, source);
  }, origin);
  model.view.focus();
  scheduleIntelligence();
  return true;
}

const builderSchemas = {
  table: [['rows', 'Rows', 'number', 3], ['columns', 'Columns', 'number', 3], ['header', 'Header row', 'checkbox', true], ['alignments', 'Column alignments', 'text', 'left,center,left'], ['booktabs', 'Booktabs', 'checkbox', false], ['caption', 'Caption', 'text', ''], ['label', 'Label', 'text', 'tab:'], ['placement', 'Placement', 'text', 'htbp']],
  figure: [['asset', 'Asset', 'asset', ''], ['width', 'Width', 'select', ['\\linewidth', '0.75\\linewidth', '0.5\\linewidth', 'custom']], ['customWidth', 'Custom width', 'text', ''], ['placement', 'Placement', 'text', 'htbp'], ['caption', 'Caption', 'text', ''], ['label', 'Label', 'text', 'fig:']],
  equation: [['type', 'Type', 'select', ['inline', 'display', 'aligned', 'matrix', 'cases']], ['body', 'Expression / body', 'textarea', 'x = y'], ['rows', 'Rows', 'number', 2], ['columns', 'Columns', 'number', 2], ['delimiter', 'Matrix delimiter', 'select', ['()', '[]', '||', 'none']], ['label', 'Label', 'text', 'eq:']],
  plot: [['asset', 'CSV asset', 'csv', ''], ['x', 'X column', 'text', 'x'], ['y', 'Y column', 'text', 'y'], ['type', 'Plot type', 'select', ['line', 'scatter', 'bar']], ['title', 'Title', 'text', ''], ['xLabel', 'X label', 'text', ''], ['yLabel', 'Y label', 'text', ''], ['legend', 'Legend', 'text', ''], ['caption', 'Caption', 'text', ''], ['label', 'Label', 'text', 'fig:plot'], ['width', 'Width', 'text', '\\linewidth']],
  algorithm: [['caption', 'Caption', 'text', 'Algorithm'], ['label', 'Label', 'text', 'alg:'], ['body', 'Algorithm lines', 'textarea', '\\State Describe the method']],
  code: [['language', 'Language', 'text', ''], ['caption', 'Caption', 'text', ''], ['label', 'Label', 'text', 'lst:'], ['file', 'File reference (optional)', 'text', ''], ['code', 'Inline code', 'textarea', '']],
  bibliography: [['target', 'Target .bib file', 'bib', ''], ['type', 'Entry type', 'select', ['article', 'book', 'inproceedings', 'misc']], ['key', 'Citation key', 'text', 'key2026'], ['title', 'Title', 'text', ''], ['author', 'Author', 'text', ''], ['year', 'Year', 'text', '2026'], ['journal', 'Journal', 'text', ''], ['booktitle', 'Book title', 'text', ''], ['doi', 'DOI', 'text', ''], ['url', 'URL', 'text', '']],
  theorem: [['environment', 'Environment', 'select', ['theorem', 'lemma', 'proposition', 'corollary', 'definition', 'remark', 'proof']], ['title', 'Optional title', 'text', ''], ['label', 'Label', 'text', 'thm:'], ['body', 'Body', 'textarea', 'Statement.']],
};

function builderRequirement(kind, values) {
  if (kind === 'theorem' && values.environment !== 'proof' && !(model.intelligence.environments || []).includes(values.environment)) {
    return { available: false, message: `Environment not detected: ${values.environment}` };
  }
  const required = {
    table: values.booktabs ? ['booktabs'] : [], figure: ['graphicx'], plot: ['pgfplots'],
    algorithm: ['algorithm', 'algpseudocode'], code: ['listings'],
    equation: ['aligned', 'matrix', 'cases'].includes(values.type) ? ['amsmath'] : [],
  }[kind] || [];
  const missing = required.filter((name) => !packageRequirement(model.intelligence.packages || [], name).available);
  return missing.length ? { available: false, message: `Requires package${missing.length > 1 ? 's' : ''}: ${missing.join(', ')}` } : { available: true, message: 'Available' };
}

function builderSource(kind, values) {
  if (kind === 'table') return buildTable({ ...values, alignments: values.alignments.split(',').map((value) => value.trim()) });
  if (kind === 'figure') return buildFigure(values);
  if (kind === 'equation') return buildEquation(values);
  if (kind === 'plot') return buildPlot(values);
  if (kind === 'algorithm') return buildAlgorithm(values);
  if (kind === 'code') return buildCodeListing(values);
  if (kind === 'bibliography') return buildBibtexEntry(values);
  return buildTheorem(values);
}

function openBuilder(kind, defaults = {}) {
  const schema = builderSchemas[kind];
  ui.dialogTitle.textContent = `${kind[0].toUpperCase()}${kind.slice(1)} Builder`;
  ui.dialogSearch.hidden = true;
  ui.dialogBody.replaceChildren();
  ui.dialogActions.replaceChildren();
  ui.dialogPreview.hidden = false;
  const controls = {};
  schema.forEach(([name, label, type, initial]) => {
    const wrapper = document.createElement('label');
    wrapper.append(document.createTextNode(label));
    let control;
    if (type === 'select' || ['asset', 'csv', 'bib'].includes(type)) {
      control = document.createElement('select');
      const choices = type === 'select' ? initial : model.files.filter((file) => type === 'asset' ? /\.(png|jpe?g|pdf)$/i.test(file.path) : type === 'csv' ? file.path.endsWith('.csv') : file.path.endsWith('.bib')).map((file) => file.path);
      choices.forEach((choice) => control.add(new Option(choice, choice)));
    } else if (type === 'textarea') control = document.createElement('textarea');
    else { control = document.createElement('input'); control.type = type; }
    if (type === 'checkbox') control.checked = defaults[name] ?? initial;
    else if (defaults[name] != null) control.value = defaults[name];
    else if (!['select', 'asset', 'csv', 'bib'].includes(type)) control.value = initial;
    controls[name] = control;
    wrapper.append(control);
    ui.dialogBody.append(wrapper);
  });
  if (kind === 'equation') {
    const helpers = document.createElement('div');
    helpers.className = 'dialog-actions';
    [['Fraction', '\\frac{}{}'], ['Root', '\\sqrt{}'], ['Superscript', '^{}'], ['Subscript', '_{}'], ['Sum', '\\sum_{}^{}'], ['Integral', '\\int_{}^{}'], ['Greek', '\\alpha']].forEach(([label, source]) => {
      helpers.append(button(label, () => {
        controls.body.value += source;
        controls.body.dispatchEvent(new Event('input'));
        controls.body.focus();
      }));
    });
    ui.dialogBody.append(helpers);
  }
  const note = document.createElement('div');
  const values = () => Object.fromEntries(Object.entries(controls).map(([name, control]) => [name, control.type === 'checkbox' ? control.checked : control.value]));
  const refresh = () => {
    const current = values();
    const requirement = builderRequirement(kind, current);
    note.className = requirement.available ? '' : 'package-note';
    note.textContent = requirement.message;
    ui.dialogPreview.textContent = builderSource(kind, current);
  };
  ui.dialogBody.append(note);
  Object.values(controls).forEach((control) => control.addEventListener('input', refresh));
  const insert = button('Insert', async () => {
    const current = values();
    if (kind === 'bibliography' && current.target) {
      const target = model.files.find((file) => file.path === current.target);
      if (target && model.file?.file_id !== target.file_id) await openFile(target);
      await model.collaboration?.ready;
      if (model.view) model.view.dispatch({ selection: { anchor: model.view.state.doc.length } });
    }
    if (insertLatex(`${builderSource(kind, current)}\n`, `writer-${kind}-builder`)) closeDialog();
  });
  ui.dialogActions.append(insert);
  refresh();
  ui.productivityDialog.showModal();
}

function openInsertMenu() {
  showPalette('Insert LaTeX', [
    ...['table', 'figure', 'plot', 'algorithm', 'code', 'bibliography', 'theorem'].map((kind) => ({ label: `${kind[0].toUpperCase()}${kind.slice(1)} Builder`, run: () => openBuilder(kind) })),
    { label: 'Insert Citation', run: openCitationPalette }, { label: 'Insert Reference', run: openReferencePalette },
    ...Object.entries(commonSnippets).map(([name, source]) => ({ label: `${name} snippet`, run: () => insertLatex(`${source}\n`, 'writer-snippet') })),
  ]);
}

function openMathPalette() {
  showPalette('Math', MATH_CATALOG.map((entry) => ({
    label: `${entry.symbol}  ${entry.latex}`,
    detail: `${entry.category} · ${entry.description}`,
    search: `${entry.category} ${entry.description} ${entry.keywords}`,
    run: () => insertLatex(entry.latex, 'writer-math-palette'),
  })));
}

function openCitationPalette() {
  showPalette('Insert Citation', (model.intelligence.bibliography || []).map((entry) => ({ label: entry.key, detail: [entry.author, entry.title].filter(Boolean).join(' · '), run: () => insertLatex(`\\cite{${entry.key}}`, 'writer-citation') })));
}

function openReferencePalette() {
  showPalette('Insert Reference', (model.intelligence.labels || []).map((entry) => ({ label: entry.key, detail: entry.path, run: () => insertLatex(`\\ref{${entry.key}}`, 'writer-reference') })));
}

function openSymbols() {
  const items = Object.entries(symbols).flatMap(([category, values]) => values.map((value) => ({ label: `${category} · ${value}`, run: () => insertLatex(value, 'writer-symbol') })));
  showPalette('Symbols', items);
}

function closeDrawer() {
  ui.workspaceDrawer.hidden = true;
  [ui.problems, ui.reviews, ui.history, ui.documentDetailsPanel].forEach((panel) => { panel.hidden = true; });
}

function openDrawer(kind) {
  const panels = { problems: ui.problems, reviews: ui.reviews, history: ui.history, documentDetails: ui.documentDetailsPanel };
  const titles = { problems: 'Problems', reviews: 'Comments', history: 'History', documentDetails: 'Document details' };
  Object.values(panels).forEach((panel) => { panel.hidden = panel !== panels[kind]; });
  ui.drawerTitle.textContent = titles[kind];
  ui.workspaceDrawer.hidden = false;
}

async function openDocumentDetails() {
  if (!model.paper || model.paper.kind !== 'team') return;
  try {
    const detail = await api.documentDetails(model.paper.id);
    ui.documentDetailsBody.replaceChildren();
    ui.documentDetailsBody.append(Object.assign(document.createElement('p'), { textContent: `Front Matter Pack: ${detail.pack_name || 'None'}` }));
    if (!detail.pack_id) {
      ui.documentDetailsBody.append(Object.assign(document.createElement('p'), { className: 'empty-copy', textContent: 'No Front Matter is assigned to this Team.' }));
      openDrawer('documentDetails'); return;
    }
    const status = Object.assign(document.createElement('p'), { textContent: `Status: ${detail.status === 'READY' ? 'Ready' : detail.status.replaceAll('_', ' ')}` }); ui.documentDetailsBody.append(status);
    if ((detail.missing_required_fields || []).length) ui.documentDetailsBody.append(Object.assign(document.createElement('p'), { className: 'danger', textContent: `Needs information: ${detail.missing_required_fields.join(', ')}` }));
    const currentValues = Object.fromEntries((detail.values || []).map((item) => [item.field_key, item.value]));
    const currentSections = Object.fromEntries((detail.sections || []).map((item) => [item.section_key, item.enabled]));
    const form = document.createElement('form'); form.className = 'document-details-form';
    const sectionsHeading = document.createElement('h4'); sectionsHeading.textContent = 'Sections'; form.append(sectionsHeading);
    (detail.manifest.sections || []).forEach((section) => { const label = document.createElement('label'); const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.name = `section:${section.key}`; checkbox.checked = section.required || (currentSections[section.key] ?? section.default_enabled); checkbox.disabled = section.required || !detail.can_edit; label.append(checkbox, document.createTextNode(` ${section.label}${section.required ? ' · Required' : ' · Optional'}`)); form.append(label); });
    const fieldsHeading = document.createElement('h4'); fieldsHeading.textContent = 'Metadata'; form.append(fieldsHeading);
    (detail.manifest.fields || []).forEach((field) => { const label = document.createElement('label'); label.textContent = field.label; const multiline = field.type === 'MULTILINE' || Array.isArray(currentValues[field.key]); const input = document.createElement(multiline ? 'textarea' : 'input'); input.name = `field:${field.key}`; if (field.type === 'DATE') input.type = 'date'; if (field.type === 'BOOLEAN') input.type = 'checkbox'; const value = currentValues[field.key]; if (field.type === 'BOOLEAN') input.checked = Boolean(value); else input.value = Array.isArray(value) ? value.join('\n') : value ?? ''; const editable = detail.can_edit && (!field.source || field.allow_team_override); input.disabled = !editable; if (field.required) input.required = true; label.append(input); if (field.source) label.append(Object.assign(document.createElement('small'), { textContent: ` Automatic: ${field.source}${field.allow_team_override ? ' · override permitted' : ''}` })); form.append(label); });
    if (detail.can_edit) { const save = document.createElement('button'); save.type = 'submit'; save.className = 'primary'; save.textContent = 'Save document details'; form.append(save); }
    form.addEventListener('submit', async (event) => { event.preventDefault(); try { if (model.collaboration && !await syncCurrent(false)) return; const values = {}; const sections = {}; (detail.manifest.fields || []).forEach((field) => { const input = form.elements[`field:${field.key}`]; if (!input || input.disabled) return; values[field.key] = field.type === 'BOOLEAN' ? input.checked : input.value; }); (detail.manifest.sections || []).forEach((section) => { const input = form.elements[`section:${section.key}`]; sections[section.key] = section.required || input.checked; }); const result = await api.saveDocumentDetails(model.paper.id, { values, sections }); model.version = result.workspace_version; await openPaper(model.paper); if (result.auto_build_required) await requestBuild('auto'); notice('Document details saved. Front Matter was rebuilt.'); } catch (error) { notice(error.message, true); } });
    ui.documentDetailsBody.append(form); openDrawer('documentDetails');
  } catch (error) { notice(error.message, true); }
}

function closeTransientMenus() {
  ui.fileActionsMenu.hidden = true;
}

async function runStructural(redo) {
  if (!model.paper || (model.collaboration && !await syncCurrent(false))) return;
  try {
    const result = redo ? await api.structuralRedo(model.paper.id) : await api.structuralUndo(model.paper.id);
    model.version = result.version;
    await openPaper(model.paper);
    notice(`${redo ? 'Redid' : 'Undid'} ${result.operation_type.toLowerCase().replaceAll('_', ' ')}.`);
  } catch (error) { notice(error.message, true); }
}

function commandItems() {
  const items = [
    ['New File', () => ui.newFile.click()], ['Rename File', () => ui.renameFile.click()], ['Delete File', () => ui.deleteFile.click()], ['Set Main', () => ui.setMain.click()],
    ['Save', () => ui.saveFile.click()], ['Compile', manualCompile], ['Structural Undo', () => runStructural(false)], ['Structural Redo', () => runStructural(true)],
    ['Open Table Builder', () => openBuilder('table')], ['Open Figure Builder', () => openBuilder('figure')], ['Open Math Palette', openMathPalette], ['Open Equation Builder', () => openBuilder('equation')], ['Open Plot Builder', () => openBuilder('plot')],
    ['Open Problems', () => openDrawer('problems')], ['Open History', () => openDrawer('history')], ['Open Comments', () => openDrawer('reviews')],
    ['Document details', openDocumentDetails],
    ['Insert Citation', openCitationPalette], ['Insert Reference', openReferencePalette],
  ];
  if (model.paper?.kind !== 'team' || model.paper?.is_team_leader) items.push(['Create Checkpoint', () => ui.createCheckpoint.click()]);
  if (model.paper?.is_team_leader && !ui.sendReview.disabled) items.push(['Send for Review', () => ui.sendReview.click()]);
  return items.map(([label, run]) => ({ label, run }));
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
ui.sendReview.addEventListener('click', async () => {
  if (!model.paper?.is_team_leader || !await syncCurrent(false)) return;
  try {
    await api.sendForReview(model.paper.id);
    await refreshReviewRounds();
    notice('Current report sent for Mentor review.');
  } catch (error) { notice(error.message, true); }
});
ui.endReview.addEventListener('click', async () => {
  const open = model.currentReviewRound;
  if (!open || !window.confirm('Withdraw the current review request? Unpublished Mentor drafts will stay private and published feedback will remain available.')) return;
  try {
    await api.endReview(model.paper.id, open.id);
    await refreshReviewRounds();
    notice('Review request withdrawn. Published feedback remains available.');
  } catch (error) { notice(error.message, true); }
});
ui.quickOpen.addEventListener('click', quickOpen);
ui.commandPalette.addEventListener('click', () => showPalette('Command Palette', commandItems()));
ui.moreActions.addEventListener('click', () => showPalette('More actions', commandItems()));
ui.insertMenu.addEventListener('click', openInsertMenu);
ui.symbolPalette.addEventListener('click', openSymbols);
ui.mathPalette.addEventListener('click', openMathPalette);
ui.tableBuilder.addEventListener('click', () => openBuilder('table'));
ui.figureBuilder.addEventListener('click', () => openBuilder('figure'));
ui.plotBuilder.addEventListener('click', () => openBuilder('plot'));
ui.undoText.addEventListener('click', () => model.undoManager?.undo());
ui.redoText.addEventListener('click', () => model.undoManager?.redo());
ui.problemsToggle.addEventListener('click', () => openDrawer('problems'));
ui.historyToggle.addEventListener('click', () => openDrawer('history'));
ui.commentsToggle.addEventListener('click', () => openDrawer('reviews'));
ui.documentDetails.addEventListener('click', openDocumentDetails);
ui.editorFontSize.addEventListener('change', () => persistPreferences({ font_size_px: Number(ui.editorFontSize.value), theme: ui.editorTheme.value }).catch((error) => notice(error.message, true)));
ui.editorTheme.addEventListener('change', () => persistPreferences({ font_size_px: Number(ui.editorFontSize.value), theme: ui.editorTheme.value }).catch((error) => notice(error.message, true)));
ui.resetEditorSettings.addEventListener('click', () => { ui.editorFontSize.value = 14; ui.editorTheme.value = 'LIGHT'; persistPreferences({ font_size_px: 14, theme: 'LIGHT' }).catch((error) => notice(error.message, true)); });
ui.drawerClose.addEventListener('click', closeDrawer);
ui.fileActionsToggle.addEventListener('click', (event) => {
  event.stopPropagation();
  const bounds = ui.fileActionsToggle.getBoundingClientRect();
  ui.fileActionsMenu.hidden = !ui.fileActionsMenu.hidden;
  Object.assign(ui.fileActionsMenu.style, { left: `${Math.max(8, bounds.right - 180)}px`, top: `${bounds.bottom + 4}px` });
});
ui.structuralUndo.addEventListener('click', () => runStructural(false));
ui.structuralRedo.addEventListener('click', () => runStructural(true));
ui.uploadImage.addEventListener('click', () => ui.assetInput.click());
ui.assetInput.addEventListener('change', async () => {
  const file = ui.assetInput.files[0];
  if (!file) return;
  if (file.size > 1024 * 1024) return notice('Assets are limited to 1 MiB.', true);
  const path = window.prompt('Asset path', `Assets/${file.name}`);
  if (!path) return;
  try {
    if (model.collaboration && !await requireDurableFlush()) return;
    const result = await api.uploadAsset(model.paper.id, path, model.version, file);
    model.version = result.version;
    await reloadPaperAndFile(result.file.file_id);
    notice(`Uploaded ${result.file.path}.`);
  } catch (error) { notice(error.message, true); }
  finally { ui.assetInput.value = ''; }
});
ui.projectSearch.addEventListener('input', () => {
  window.clearTimeout(model.searchTimer);
  const query = ui.projectSearch.value.trim();
  if (!query) { ui.searchResults.replaceChildren(); return; }
  model.searchTimer = window.setTimeout(async () => {
    try {
      const payload = await api.search(model.paper.id, query, ui.caseSensitive.checked);
      ui.searchResults.replaceChildren();
      let currentPath = null;
      payload.results.forEach((result) => {
        if (result.path !== currentPath) {
          currentPath = result.path;
          const heading = document.createElement('strong');
          heading.className = 'directory';
          heading.textContent = currentPath;
          ui.searchResults.append(heading);
        }
        const row = button(result.preview || result.match, () => openLocation(result));
        row.className = 'search-result';
        row.append(Object.assign(document.createElement('small'), { textContent: `${result.path}:${result.line}` }));
        ui.searchResults.append(row);
      });
      if (!payload.results.length) ui.searchResults.innerHTML = '<p class="empty-copy">No matches.</p>';
    } catch (error) { notice(error.message, true); }
  }, 350);
});
ui.caseSensitive.addEventListener('change', () => ui.projectSearch.dispatchEvent(new Event('input')));
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') { closeDrawer(); closeTransientMenus(); return; }
  if (!(event.ctrlKey || event.metaKey)) return;
  if (event.key.toLowerCase() === 'p') { event.preventDefault(); quickOpen(); }
  if (event.key.toLowerCase() === 'k') { event.preventDefault(); showPalette('Command Palette', commandItems()); }
  if (event.shiftKey && event.key.toLowerCase() === 'r' && model.paper?.is_team_leader && !ui.sendReview.disabled) { event.preventDefault(); ui.sendReview.click(); }
});
document.addEventListener('click', (event) => { if (!ui.fileActionsMenu.contains(event.target) && event.target !== ui.fileActionsToggle) closeTransientMenus(); });
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
ui.copyRecoveryText.addEventListener('click', async () => {
  if (!model.recoveryText) return;
  try {
    await navigator.clipboard.writeText(model.recoveryText);
    notice('Previous-version recovery text copied.');
  } catch {
    window.prompt('Copy previous-version recovery text', model.recoveryText);
  }
});
ui.writerReviewFilters.addEventListener('click', (event) => {
  const filter = event.target.dataset.filter;
  if (!filter) return;
  model.reviewFilter = filter;
  [...ui.writerReviewFilters.children].forEach((node) => node.toggleAttribute('aria-current', node === event.target));
  renderReviews();
});

window.setInterval(() => {
  if (model.paper) {
    refreshBuildStatus();
    if (model.paper.kind === 'team') refreshReviews();
  }
}, 1500);

Promise.all([api.me(), loadPreferences()])
  .then(([identity]) => {
    applyIdentity(identity);
    return refreshPapers();
  })
  .catch((error) => notice(error.message, true));
