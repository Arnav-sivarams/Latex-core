import { basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { StreamLanguage } from '@codemirror/language';
import { stex } from '@codemirror/legacy-modes/mode/stex';
import { yCollab } from 'y-codemirror.next';
import * as Y from 'yjs';
import * as pdfjsLib from '/static/pdf.min.mjs';
import { denormalizeRectangle, normalizeRectangle, resolveSuggestionRange } from './review-helpers.mjs';

pdfjsLib.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';
const INITIAL_STATE = 0x10;
const REMOTE_FRAME = 0x11;
const REMOTE_ORIGIN = Symbol('mentor-remote');

class ReviewApi {
  async request(path, options = {}) {
    const response = await fetch(path, { credentials: 'same-origin', ...options });
    if (!response.ok) {
      const payload = await response.json().catch(() => ({}));
      const failure = new Error(payload.error || `Request failed (${response.status})`);
      failure.status = response.status;
      throw failure;
    }
    return response.status === 204 ? null : response.json();
  }
  json(path, method, body = {}) { return this.request(path, { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); }
  papers() { return this.request('/api/v2/mentor/papers'); }
  paper(id) { return this.request(`/api/v2/reviews/papers/${id}`); }
  files(id) { return this.request(`/api/v2/reviews/papers/${id}/files`); }
  file(id, fileId) { return this.request(`/api/v2/reviews/papers/${id}/files/${fileId}`); }
  writers(id) { return this.request(`/api/v2/reviews/papers/${id}/writers`); }
  rounds(id) { return this.request(`/api/v2/reviews/papers/${id}/rounds`); }
  threads(id) { return this.request(`/api/v2/reviews/papers/${id}/threads`); }
  createThread(id, body) { return this.json(`/api/v2/reviews/papers/${id}/threads`, 'POST', body); }
  message(id, threadId, body) { return this.json(`/api/v2/reviews/papers/${id}/threads/${threadId}/messages`, 'POST', { body }); }
  transition(id, threadId, state) { return this.json(`/api/v2/reviews/papers/${id}/threads/${threadId}/state`, 'POST', { state }); }
  map(id, body) { return this.json(`/api/v2/reviews/papers/${id}/synctex`, 'POST', body); }
  build(id) { return this.json(`/api/v2/papers/${id}/builds`, 'POST', { trigger_type: 'manual' }); }
  buildStatus(id) { return this.request(`/api/v2/papers/${id}/builds`); }
}

const api = new ReviewApi();
const ui = Object.fromEntries([
  'reviewContext', 'liveStatus', 'assignedPapers', 'reviewFiles', 'csvExport', 'printExport',
  'reviewTitle', 'reviewSummary', 'buildStatus', 'compileReview',
  'reviewEditor', 'sourceSelection', 'reviewSelection', 'previousPage', 'pageLabel', 'nextPage', 'zoomOut',
  'zoomLabel', 'zoomIn', 'pdfState', 'pdfViewport', 'pdfCanvas', 'pdfOverlay',
  'reviewPopover', 'anchorSummary', 'threadType', 'threadMessage', 'cancelAnnotation',
  'createAnnotation', 'createComment', 'createSuggestion', 'threadFilters', 'threadList',
  'roundList', 'reviewNotice', 'reviewGateBadge', 'mentorCommentsToggle', 'mentorReviewDrawer',
  'mentorDrawerClose',
].map((id) => [id, document.getElementById(id)]));

const model = {
  papers: [], paper: null, detail: null, files: [], file: null, rounds: [], threads: [],
  reviewOpen: false, currentReviewRound: null,
  filter: 'active', collaboration: null, view: null, pendingAnchor: null, pendingAnchorSummary: null,
  pdf: null, pdfBuildId: null, page: 1, scale: 1, viewport: null, renderTask: null, dragStart: null,
  suppressSelection: false,
};

function notice(message, failed = false) { ui.reviewNotice.textContent = message; ui.reviewNotice.classList.toggle('danger', failed); }
function button(label, handler, active = false) { const node = document.createElement('button'); node.type = 'button'; node.textContent = label; if (active) node.setAttribute('aria-current', 'page'); node.addEventListener('click', handler); return node; }
function clearNode(node, message) { node.replaceChildren(Object.assign(document.createElement('p'), { className: 'empty-copy', textContent: message })); }

class ReadOnlySession {
  constructor(paper, file) {
    this.paper = paper; this.file = file; this.socket = null; this.doc = null; this.text = null;
    this.metadata = null; this.initial = null; this.destroyed = false; this.buffer = [];
    this.ready = new Promise((resolve) => { this.resolveReady = resolve; });
  }
  start() {
    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${scheme}//${location.host}/api/v2/collab/${this.paper.id}/files/${this.file.file_id}`);
    socket.binaryType = 'arraybuffer'; this.socket = socket; ui.liveStatus.textContent = 'Connecting…';
    socket.addEventListener('message', (event) => this.receive(event));
    socket.addEventListener('close', () => { if (!this.destroyed) { ui.liveStatus.textContent = 'Disconnected'; ui.liveStatus.dataset.state = 'error'; } });
    socket.addEventListener('error', () => { ui.liveStatus.textContent = 'Read-only connection failed'; ui.liveStatus.dataset.state = 'error'; });
  }
  receive(event) {
    if (typeof event.data === 'string') {
      const value = JSON.parse(event.data);
      if (value.type === 'JOIN_ACCEPTED') { this.metadata = value; this.initialize(); }
      else if (value.type === 'REMOTE_DURABLE') { ui.liveStatus.textContent = 'Live · read only'; refreshPaperDetail(); }
      else if (value.type === 'RELOAD_REQUIRED') notice('The selected file was removed.', true);
      else if (value.type === 'PAPER_EPOCH_CHANGED') { notice('The Team Leader changed the paper version; reloading.', true); openPaper(model.paper); }
      else if (value.type === 'POLICY_CHANGED') { notice(value.message || 'File policy changed; reload.', true); openPaper(model.paper); }
      else if (value.type === 'ERROR') notice(value.message || 'Read-only collaboration failed.', true);
      return;
    }
    const bytes = new Uint8Array(event.data);
    if (bytes[0] === INITIAL_STATE) { this.initial = bytes.slice(1); this.initialize(); }
    else if (bytes[0] === REMOTE_FRAME) { if (this.doc) Y.applyUpdate(this.doc, bytes.slice(1), REMOTE_ORIGIN); else this.buffer.push(bytes.slice(1)); }
  }
  initialize() {
    if (!this.metadata || !this.initial || this.doc) return;
    if (this.metadata.access !== 'read_only') { notice('Mentor transport did not enforce read-only access.', true); this.destroy(); return; }
    this.doc = new Y.Doc(); this.text = this.doc.getText('source');
    Y.applyUpdate(this.doc, this.initial, REMOTE_ORIGIN); this.buffer.forEach((update) => Y.applyUpdate(this.doc, update, REMOTE_ORIGIN)); this.buffer = [];
    mountReadOnlyEditor(this); ui.liveStatus.textContent = 'Live · read only'; ui.liveStatus.dataset.state = 'synced'; this.resolveReady();
  }
  destroy() { this.destroyed = true; if (this.socket) this.socket.close(); if (this.doc) this.doc.destroy(); this.socket = null; this.doc = null; }
}

function mountReadOnlyEditor(session) {
  if (model.view) model.view.destroy(); ui.reviewEditor.replaceChildren();
  const extensions = [basicSetup, EditorState.readOnly.of(true), EditorView.editable.of(false), yCollab(session.text),
    EditorView.updateListener.of((update) => { if (update.selectionSet && !model.suppressSelection) sourceSelected(update.state); }),
    EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' }, '.cm-content': { caretColor: 'transparent' } }),
  ];
  if (session.file.path.endsWith('.tex')) extensions.push(StreamLanguage.define(stex));
  model.view = new EditorView({ state: EditorState.create({ doc: session.text.toString(), extensions }), parent: ui.reviewEditor });
}

async function sourceSelected(state) {
  const range = state.selection.main;
  if (range.empty || !model.collaboration?.text) { ui.sourceSelection.textContent = 'Select text, then right-click to review'; ui.reviewSelection.disabled = true; return; }
  if (!reviewOpen()) { ui.sourceSelection.textContent = 'Waiting for Team Review'; ui.reviewSelection.disabled = true; return; }
  const quoted = state.sliceDoc(range.from, range.to);
  const relativeStart = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(model.collaboration.text, range.from));
  const relativeEnd = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(model.collaboration.text, range.to));
  const context = state.sliceDoc(Math.max(0, range.from - 80), Math.min(state.doc.length, range.to + 80));
  const sourceAnchor = {
    file_id: model.file.file_id, encoded_relative_start: [...relativeStart], encoded_relative_end: [...relativeEnd],
    quoted_text: quoted, context_hash: await sha256(context), source_sequence: model.detail.version,
    source_version_id: model.paper.current_version_id, document_epoch: model.collaboration.metadata.document_epoch,
  };
  const pending = { source_anchor: sourceAnchor, pdf_anchor: null };
  model.pendingAnchor = pending;
  ui.sourceSelection.textContent = `${quoted.length} characters selected`;
  model.pendingAnchorSummary = `Source: ${model.file.path} · “${quoted.slice(0, 80)}”`;
  ui.reviewSelection.disabled = false;
  const line = state.doc.lineAt(range.from);
  try {
    const mapping = await api.map(model.paper.id, { direction: 'FORWARD', file_id: model.file.file_id, line: line.number, column: range.from - line.from });
    const pdfAnchor = await pdfAnchorFromMapping(mapping);
    if (model.pendingAnchor === pending) { pending.pdf_anchor = pdfAnchor; renderOverlays(); }
  } catch { /* Source annotation remains valid without a projection. */ }
}

ui.reviewEditor.addEventListener('contextmenu', (event) => {
  if (!reviewOpen() || !model.pendingAnchor?.source_anchor || !model.view) return;
  const selection = model.view.state.selection.main;
  const position = model.view.posAtCoords({ x: event.clientX, y: event.clientY });
  if (selection.empty || position == null || position < selection.from || position > selection.to) return;
  event.preventDefault();
  showReviewPopover(event.clientX, event.clientY, model.pendingAnchorSummary);
});

async function sha256(value) { const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value)); return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, '0')).join(''); }

async function refreshPapers() {
  const payload = await api.papers(); model.papers = payload.papers; if (model.paper) model.paper = model.papers.find((paper) => paper.id === model.paper.id) || model.paper; ui.assignedPapers.replaceChildren();
  if (!model.papers.length) return clearNode(ui.assignedPapers, 'No assigned Team Papers.');
  const list = document.createElement('div'); list.className = 'paper-list';
  model.papers.forEach((paper) => { const item = button(paper.name, () => openPaper(paper), model.paper?.id === paper.id); const count = document.createElement('small'); count.textContent = `${paper.open_review_count} open`; item.append(count); list.append(item); });
  ui.assignedPapers.append(list);
}

async function openPaper(paper) {
  disposeSource(); disposePdf(); model.paper = paper; model.file = null; model.pendingAnchor = null;
  ui.reviewContext.textContent = paper.name; ui.reviewTitle.textContent = paper.name; ui.reviewSummary.textContent = `${paper.status} · ${paper.open_review_count} open · ${paper.blocking_review_count} blocking`;
  ui.compileReview.disabled = paper.status !== 'active'; ui.csvExport.href = `/api/v2/reviews/papers/${paper.id}/report.csv`; ui.printExport.href = `/api/v2/reviews/papers/${paper.id}/report.html`; ui.csvExport.removeAttribute('aria-disabled'); ui.printExport.removeAttribute('aria-disabled');
  const [detail, files] = await Promise.all([api.paper(paper.id), api.files(paper.id)]);
  model.detail = detail; model.files = files.files; renderPapers(); renderFiles();
  const initial = model.files.find((file) => file.path === detail.main_file) || model.files[0]; if (initial) await openFile(initial);
  await Promise.all([refreshThreads(), refreshRounds(), refreshBuild()]);
}

function renderPapers() { refreshPapers().catch((failure) => notice(failure.message, true)); }
function renderFiles() {
  ui.reviewFiles.replaceChildren(); if (!model.files.length) return clearNode(ui.reviewFiles, 'No live files.');
  const list = document.createElement('ul'); list.className = 'file-tree'; model.files.forEach((file) => { const item = document.createElement('li'); item.append(button(file.path, () => openFile(file), model.file?.file_id === file.file_id)); list.append(item); }); ui.reviewFiles.append(list);
}
async function openFile(file) {
  disposeSource(); const payload = await api.file(model.paper.id, file.file_id); model.file = payload.file; renderFiles();
  ui.reviewEditor.innerHTML = '<div class="foundation-empty"><strong>Opening live read-only source…</strong></div>';
  model.collaboration = new ReadOnlySession(model.detail.paper, payload.file); model.collaboration.start(); await model.collaboration.ready;
}
function disposeSource() { if (model.view) model.view.destroy(); if (model.collaboration) model.collaboration.destroy(); model.view = null; model.collaboration = null; }

async function refreshBuild() {
  if (!model.paper) return; const payload = await api.buildStatus(model.paper.id); const build = payload.build;
  if (build.active_build_id) ui.buildStatus.textContent = build.current_build_id ? 'Rebuilding…' : 'Building…';
  else if (build.latest_status === 'failed') ui.buildStatus.textContent = build.current_build_id ? 'Failed · showing last good' : 'Failed';
  else ui.buildStatus.textContent = build.current_build_id ? 'Current' : 'No PDF yet';
  if (build.current_build_id && build.current_build_id !== model.pdfBuildId) await loadPdf(payload.pdf_url, build.current_build_id);
}
async function refreshPaperDetail() {
  if (!model.paper) return;
  try { model.detail = await api.paper(model.paper.id); renderThreads(); }
  catch { /* The active access check will surface on the next explicit action. */ }
}
async function loadPdf(url, buildId) {
  disposePdf(); model.pdfBuildId = buildId; ui.pdfState.textContent = 'Loading current PDF…';
  try { model.pdf = await pdfjsLib.getDocument({ url: `${url}?build=${buildId}`, withCredentials: true }).promise; model.page = 1; ui.pdfState.textContent = `Current exact build ${buildId}`; setPdfControls(true); await renderPage(); }
  catch (failure) { ui.pdfState.textContent = `PDF failed: ${failure.message}`; setPdfControls(false); }
}
function disposePdf() { if (model.renderTask) model.renderTask.cancel(); if (model.pdf) model.pdf.destroy(); model.pdf = null; model.renderTask = null; model.viewport = null; ui.pdfCanvas.width = 0; ui.pdfCanvas.height = 0; ui.pdfOverlay.replaceChildren(); }
function setPdfControls(enabled) { [ui.previousPage, ui.nextPage, ui.zoomOut, ui.zoomIn].forEach((node) => { node.disabled = !enabled; }); }
async function renderPage() {
  if (!model.pdf) return; if (model.renderTask) model.renderTask.cancel(); const page = await model.pdf.getPage(model.page); const viewport = page.getViewport({ scale: model.scale }); model.viewport = viewport;
  const context = ui.pdfCanvas.getContext('2d'); ui.pdfCanvas.width = Math.ceil(viewport.width); ui.pdfCanvas.height = Math.ceil(viewport.height); ui.pdfOverlay.style.width = `${viewport.width}px`; ui.pdfOverlay.style.height = `${viewport.height}px`;
  model.renderTask = page.render({ canvasContext: context, viewport }); try { await model.renderTask.promise; } catch (failure) { if (failure.name !== 'RenderingCancelledException') throw failure; } finally { model.renderTask = null; }
  ui.pageLabel.textContent = `${model.page} / ${model.pdf.numPages}`; ui.zoomLabel.textContent = `${Math.round(model.scale * 100)}%`; ui.previousPage.disabled = model.page <= 1; ui.nextPage.disabled = model.page >= model.pdf.numPages; renderOverlays();
}
function renderOverlays() {
  ui.pdfOverlay.replaceChildren(); if (!model.viewport) return;
  model.threads.filter((thread) => thread.pdf_anchor?.page === model.page).forEach((thread) => thread.pdf_anchor.normalized_rectangles.forEach((rectangle) => addOverlay(rectangle, `persisted ${thread.pdf_anchor.mapping_status}`, () => focusThread(thread))));
  if (model.pendingAnchor?.pdf_anchor?.page === model.page) model.pendingAnchor.pdf_anchor.normalized_rectangles.forEach((rectangle) => addOverlay(rectangle, 'pending'));
}
function addOverlay(rectangle, className, handler) { const pixels = denormalizeRectangle(rectangle, model.viewport.width, model.viewport.height); const node = document.createElement('button'); node.type = 'button'; node.className = `pdf-highlight ${className}`; Object.assign(node.style, { left: `${pixels.x}px`, top: `${pixels.y}px`, width: `${pixels.width}px`, height: `${pixels.height}px` }); if (handler) node.addEventListener('click', handler); ui.pdfOverlay.append(node); }

async function pdfAnchorFromMapping(mapping) {
  if (!model.pdf || !mapping.page || !Number.isFinite(mapping.x)) return null; if (model.page !== mapping.page) { model.page = mapping.page; await renderPage(); }
  const baseWidth = model.viewport.width / model.scale; const baseHeight = model.viewport.height / model.scale;
  return { page: mapping.page, normalized_rectangles: [{ x: mapping.x / baseWidth, y: Math.max(0, mapping.y - mapping.height) / baseHeight, width: Math.max(mapping.width, 8) / baseWidth, height: Math.max(mapping.height, 8) / baseHeight }], mapping_status: mapping.mapping_status, mapped_file_id: model.file.file_id, mapped_line: mapping.mapped_line, mapped_column: mapping.mapped_column };
}

ui.pdfOverlay.addEventListener('pointerdown', (event) => { if (!model.viewport || event.target !== ui.pdfOverlay) return; const bounds = ui.pdfOverlay.getBoundingClientRect(); model.dragStart = { x: event.clientX - bounds.left, y: event.clientY - bounds.top }; ui.pdfOverlay.setPointerCapture(event.pointerId); });
ui.pdfOverlay.addEventListener('pointerup', async (event) => {
  if (!model.dragStart || !model.viewport) return; const bounds = ui.pdfOverlay.getBoundingClientRect(); const end = { x: event.clientX - bounds.left, y: event.clientY - bounds.top }; const rectangle = normalizeRectangle({ x1: model.dragStart.x, y1: model.dragStart.y, x2: end.x, y2: end.y }, model.viewport.width, model.viewport.height); model.dragStart = null; if (rectangle.width < 0.005 || rectangle.height < 0.005) return;
  if (!reviewOpen()) return;
  const point = { x: (rectangle.x + rectangle.width / 2) * model.viewport.width / model.scale, y: (rectangle.y + rectangle.height / 2) * model.viewport.height / model.scale };
  let mapping = { mapping_status: 'PDF_ONLY' }; try { mapping = await api.map(model.paper.id, { direction: 'INVERSE', page: model.page, ...point }); } catch { /* PDF-only is truthful. */ }
  let sourceAnchor = null; if (mapping.mapped_file_id) sourceAnchor = await anchorMappedLine(mapping);
  model.pendingAnchor = { source_anchor: sourceAnchor, pdf_anchor: { page: model.page, normalized_rectangles: [rectangle], mapping_status: mapping.mapping_status, mapped_file_id: mapping.mapped_file_id || null, mapped_line: mapping.mapped_line || null, mapped_column: mapping.mapped_column ?? null } };
  model.pendingAnchorSummary = `PDF page ${model.page} · ${mapping.mapping_status}`;
  renderOverlays();
});

ui.pdfOverlay.addEventListener('contextmenu', (event) => {
  if (!event.target.closest?.('.pdf-highlight.pending') || !reviewOpen()) return;
  event.preventDefault();
  showReviewPopover(event.clientX, event.clientY, model.pendingAnchorSummary || `PDF page ${model.page}`);
});

async function anchorMappedLine(mapping) {
  const target = model.files.find((file) => file.file_id === mapping.mapped_file_id); if (!target) return null; if (model.file?.file_id !== target.file_id) await openFile(target);
  const lineNumber = Math.min(Math.max(mapping.mapped_line, 1), model.view.state.doc.lines); const line = model.view.state.doc.line(lineNumber); const from = Math.min(line.to, line.from + (mapping.mapped_column || 0)); const to = line.to;
  const context = model.view.state.sliceDoc(Math.max(0, from - 80), Math.min(model.view.state.doc.length, to + 80));
  return { file_id: target.file_id, encoded_relative_start: [...Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(model.collaboration.text, from))], encoded_relative_end: [...Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(model.collaboration.text, to))], quoted_text: model.view.state.sliceDoc(from, to), context_hash: await sha256(context), source_sequence: model.detail.version, source_version_id: model.paper.current_version_id, document_epoch: model.collaboration.metadata.document_epoch };
}

function reviewOpen() { return model.reviewOpen; }
function showReviewPopover(x, y, summary) {
  if (!reviewOpen() || !model.pendingAnchor) return;
  ui.anchorSummary.textContent = summary;
  ui.createSuggestion.hidden = !model.pendingAnchor.source_anchor;
  ui.reviewPopover.hidden = false;
  const width = ui.reviewPopover.offsetWidth; const height = ui.reviewPopover.offsetHeight;
  Object.assign(ui.reviewPopover.style, {
    left: `${Math.max(8, Math.min(x, window.innerWidth - width - 8))}px`,
    top: `${Math.max(8, Math.min(y, window.innerHeight - height - 8))}px`,
  });
  ui.threadMessage.focus();
}
function hideComposer({ clearAnchor = true } = {}) {
  ui.reviewPopover.hidden = true;
  ui.threadMessage.value = '';
  if (clearAnchor) { model.pendingAnchor = null; model.pendingAnchorSummary = null; ui.reviewSelection.disabled = true; renderOverlays(); }
}

async function createAnnotation(type = 'COMMENT') {
  if (!reviewOpen()) throw new Error('This paper has not been sent for review.');
  const message = ui.threadMessage.value.trim();
  if (!message) throw new Error(type === 'SUGGESTION' ? 'Suggestion text is required.' : 'Comment text is required.');
  if (type === 'SUGGESTION' && !model.pendingAnchor?.source_anchor) throw new Error('Suggestions require a source selection.');
  const request = { thread_type: type, message, severity: 'NOTE', category: 'WRITING', assigned_writer_user_id: null, due_at: null, source_anchor: model.pendingAnchor?.source_anchor || null, pdf_anchor: model.pendingAnchor?.pdf_anchor || null, suggested_replacement: type === 'SUGGESTION' ? message : null, section_label: null };
  await api.createThread(model.paper.id, request); hideComposer(); ui.threadMessage.value = ''; await Promise.all([refreshThreads(), refreshRounds()]); notice('Review annotation created.');
}

async function refreshThreads() { const payload = await api.threads(model.paper.id); model.threads = payload.threads; renderThreads(); renderOverlays(); }
function renderThreads() {
  ui.threadList.replaceChildren(); const selected = model.threads.filter((thread) => model.filter === 'all' || (model.filter === 'active' && ['OPEN', 'REOPENED', 'ADDRESSED'].includes(thread.state)) || thread.state === model.filter);
  if (!selected.length) return clearNode(ui.threadList, 'No matching comments.');
  selected.forEach((thread) => { const card = document.createElement('article'); card.className = 'thread-card'; const heading = document.createElement('button'); heading.type = 'button'; heading.className = 'thread-title'; heading.textContent = `${thread.thread_type.replaceAll('_', ' ')} · ${thread.state}`; heading.addEventListener('click', () => focusThread(thread)); const meta = document.createElement('p'); meta.textContent = mappingStatus(thread); const discussion = document.createElement('div'); discussion.className = 'discussion'; thread.messages.forEach((message) => { const row = document.createElement('p'); const author = document.createElement('strong'); author.textContent = `${message.author_email}: `; row.append(author, document.createTextNode(message.body)); discussion.append(row); }); const actions = document.createElement('div'); actions.className = 'thread-actions'; if (reviewOpen()) actions.append(button('Reply', () => reply(thread))); if (thread.state === 'ADDRESSED') actions.append(button('Done', () => transition(thread, 'RESOLVED')), button('Reopen', () => transition(thread, 'REOPENED'))); else if (thread.state === 'RESOLVED' && reviewOpen()) actions.append(button('Reopen', () => transition(thread, 'REOPENED'))); card.append(heading, meta, discussion, actions); ui.threadList.append(card); });
}
function mappingStatus(thread) {
  if (thread.thread_type === 'PAPER_APPROVAL') return thread.approved_workspace_version === model.detail?.version ? 'EXACT · current version' : 'SOURCE_CHANGED · historical approval';
  if (thread.source_anchor?.file_deleted) return 'SOURCE_DELETED';
  if (thread.source_anchor && model.file?.file_id === thread.source_anchor.file_id && model.collaboration?.doc) {
    const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, base64(thread.source_anchor.encoded_relative_start), base64(thread.source_anchor.encoded_relative_end));
    if (!range || model.collaboration.text.toString().slice(range.from, range.to) !== thread.source_anchor.quoted_text) return 'SOURCE_CHANGED';
  }
  if (thread.pdf_anchor) return thread.pdf_anchor.mapping_status;
  return thread.source_anchor ? 'Source linked' : 'PDF_ONLY';
}
async function reply(thread) { const body = prompt('Reply'); if (!body) return; await api.message(model.paper.id, thread.id, body); await refreshThreads(); }
async function transition(thread, state) { await api.transition(model.paper.id, thread.id, state); await Promise.all([refreshThreads(), refreshRounds()]); }

async function focusThread(thread) {
  if (thread.pdf_anchor?.page) { model.page = thread.pdf_anchor.page; await renderPage(); }
  const anchor = thread.source_anchor; if (!anchor || anchor.file_deleted) return;
  const file = model.files.find((candidate) => candidate.file_id === anchor.file_id); if (!file) return; if (model.file?.file_id !== file.file_id) await openFile(file);
  const range = resolveSuggestionRange(model.collaboration.doc, model.collaboration.text, base64(anchor.encoded_relative_start), base64(anchor.encoded_relative_end));
  if (!range) return notice('Source anchor no longer resolves.', true); model.suppressSelection = true; model.view.dispatch({ selection: { anchor: range.from, head: range.to }, scrollIntoView: true }); model.suppressSelection = false;
  const current = model.view.state.sliceDoc(range.from, range.to); if (current !== anchor.quoted_text) notice('Source changed materially; historical context is retained.', true);
  const line = model.view.state.doc.lineAt(range.from); try { const mapping = await api.map(model.paper.id, { direction: 'FORWARD', file_id: file.file_id, line: line.number, column: range.from - line.from }); const projection = await pdfAnchorFromMapping(mapping); if (projection) { model.pendingAnchor = { pdf_anchor: projection }; renderOverlays(); } } catch { /* Historical anchor remains useful. */ }
}
function base64(value) { if (!value) return null; const binary = atob(value.replaceAll('\n', '')); return Uint8Array.from(binary, (character) => character.charCodeAt(0)); }

async function refreshRounds() {
  const payload = await api.rounds(model.paper.id);
  model.rounds = payload.rounds;
  model.reviewOpen = payload.review_open;
  model.currentReviewRound = payload.current_review_round;
  renderRounds();
}
function renderRounds() {
  ui.roundList.replaceChildren();
  const open = reviewOpen();
  ui.reviewGateBadge.textContent = open ? 'Review open' : 'Waiting for Team Review';
  if (!model.rounds.length) clearNode(ui.roundList, 'This paper has not been sent for review.');
  else model.rounds.forEach((round) => { const row = document.createElement('div'); row.className = 'round-row'; const text = document.createElement('span'); text.textContent = `Round ${round.round_number} · ${round.status.replaceAll('_', ' ')}`; row.append(text); ui.roundList.append(row); });
  if (!open) hideComposer();
}

ui.reviewSelection.addEventListener('click', () => {
  if (!model.pendingAnchor || !model.pendingAnchorSummary || !model.view) return;
  const coordinates = model.view.coordsAtPos(model.view.state.selection.main.to);
  showReviewPopover(coordinates?.left || window.innerWidth / 2, coordinates?.bottom || 100, model.pendingAnchorSummary);
});
ui.cancelAnnotation.addEventListener('click', () => hideComposer());
ui.createComment.addEventListener('click', () => createAnnotation('COMMENT').catch((failure) => notice(failure.message, true)));
ui.createSuggestion.addEventListener('click', () => createAnnotation('SUGGESTION').catch((failure) => notice(failure.message, true)));
ui.createAnnotation.addEventListener('click', () => createAnnotation(ui.threadType.value).catch((failure) => notice(failure.message, true)));
ui.mentorCommentsToggle.addEventListener('click', () => { ui.mentorReviewDrawer.hidden = false; });
ui.mentorDrawerClose.addEventListener('click', () => { ui.mentorReviewDrawer.hidden = true; });
document.addEventListener('keydown', (event) => {
  if (event.key !== 'Escape') return;
  hideComposer({ clearAnchor: false });
  ui.mentorReviewDrawer.hidden = true;
});
document.addEventListener('pointerdown', (event) => {
  if (!ui.reviewPopover.hidden && !ui.reviewPopover.contains(event.target)) hideComposer({ clearAnchor: false });
});
ui.compileReview.addEventListener('click', async () => { try { ui.buildStatus.textContent = 'Building…'; await api.build(model.paper.id); await refreshBuild(); } catch (failure) { notice(failure.message, true); } });
ui.previousPage.addEventListener('click', async () => { if (model.page > 1) { model.page -= 1; await renderPage(); } }); ui.nextPage.addEventListener('click', async () => { if (model.page < model.pdf.numPages) { model.page += 1; await renderPage(); } }); ui.zoomOut.addEventListener('click', async () => { model.scale = Math.max(0.5, model.scale - 0.25); await renderPage(); }); ui.zoomIn.addEventListener('click', async () => { model.scale = Math.min(3, model.scale + 0.25); await renderPage(); });
ui.threadFilters.addEventListener('click', (event) => { const filter = event.target.dataset.filter; if (!filter) return; model.filter = filter; [...ui.threadFilters.children].forEach((node) => node.toggleAttribute('aria-current', node === event.target)); renderThreads(); });
window.setInterval(() => { if (model.paper) { refreshBuild().catch(() => {}); refreshPapers().catch(() => {}); } }, 2000);
refreshPapers().catch((failure) => notice(failure.message, true));
