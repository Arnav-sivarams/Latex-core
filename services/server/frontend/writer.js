import { basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { StreamLanguage } from '@codemirror/language';
import { stex } from '@codemirror/legacy-modes/mode/stex';

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

  papers() { return this.request('/api/v2/writer/papers'); }
  createPaper(name) { return this.json('/api/v2/writer/personal-papers', 'POST', { name }); }
  paper(id) { return this.request(`/api/v2/papers/${id}`); }
  files(id) { return this.request(`/api/v2/papers/${id}/files`); }
  file(paperId, fileId) { return this.request(`/api/v2/papers/${paperId}/files/${fileId}`); }
  createFile(paperId, body) { return this.json(`/api/v2/papers/${paperId}/files`, 'POST', body); }
  saveFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}`, 'PUT', body); }
  renameFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}/path`, 'PATCH', body); }
  deleteFile(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/files/${fileId}`, 'DELETE', body); }
  setMain(paperId, fileId, body) { return this.json(`/api/v2/papers/${paperId}/main/${fileId}`, 'POST', body); }

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
  'editorMount', 'writerNotice',
].map((id) => [id, document.getElementById(id)]));

const model = {
  papers: [],
  paper: null,
  paperDetail: null,
  files: [],
  file: null,
  version: 0,
  view: null,
  dirty: false,
  conflict: false,
};

function notice(message, failed = false) {
  ui.writerNotice.textContent = message;
  ui.writerNotice.classList.toggle('danger', failed);
}

function saveState(state) {
  const labels = {
    saved: 'Saved', saving: 'Saving…', unsaved: 'Unsaved',
    conflict: 'Conflict — reload/resolve required', error: 'Error',
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

async function refreshPapers() {
  model.papers = await api.papers();
  renderPapers();
}

async function openPaper(paper) {
  if (!canLeaveEditor()) return;
  model.paper = paper;
  model.file = null;
  model.paperDetail = await api.paper(paper.id);
  model.version = model.paperDetail.version;
  model.files = await api.files(paper.id);
  ui.currentPaper.textContent = `${paper.name}${paper.status === 'active' ? '' : ` — ${paper.status} (read-only)`}`;
  ui.newFile.disabled = !model.paperDetail.editable;
  renderPapers();
  renderFiles();
  const initial = model.files.find((file) => file.path === model.paperDetail.main_file) || model.files[0];
  if (initial) await openFile(initial, true);
}

async function openFile(file, force = false) {
  if (!force && !canLeaveEditor()) return;
  try {
    const payload = await api.file(model.paper.id, file.file_id);
    model.file = payload.file;
    model.version = payload.version;
    model.dirty = false;
    model.conflict = false;
    ui.currentFile.textContent = payload.file.path;
    ui.mainBadge.hidden = !payload.main;
    mountEditor(payload.content, payload.editable, payload.file.path.endsWith('.tex'));
    updateFileActions(payload.editable);
    saveState('saved');
    renderFiles();
  } catch (error) {
    if (error.status === 415) {
      destroyEditor();
      ui.editorMount.innerHTML = '<div class="foundation-empty"><strong>Binary file</strong><p>This asset is visible in the tree but is not editable in S2.</p></div>';
      model.file = file;
      updateFileActions(false);
    }
    notice(error.message, true);
  }
}

function mountEditor(content, editable, latex) {
  destroyEditor();
  ui.editorMount.replaceChildren();
  const extensions = [
    basicSetup,
    keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => { saveCurrent(); return true; } }]),
    EditorState.readOnly.of(!editable),
    EditorView.editable.of(editable),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        model.dirty = true;
        model.conflict = false;
        saveState('unsaved');
      }
    }),
    EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' } }),
  ];
  if (latex) extensions.push(StreamLanguage.define(stex));
  model.view = new EditorView({
    state: EditorState.create({ doc: content, extensions }),
    parent: ui.editorMount,
  });
}

function destroyEditor() {
  if (model.view) model.view.destroy();
  model.view = null;
}

function canLeaveEditor() {
  return !model.dirty || window.confirm('Discard unsaved changes?');
}

function updateFileActions(editable) {
  const selected = Boolean(model.file);
  ui.renameFile.disabled = !selected || !editable;
  ui.deleteFile.disabled = !selected || !editable;
  ui.setMain.disabled = !selected || !editable || ui.mainBadge.hidden === false;
  ui.saveFile.disabled = !selected || !editable;
}

async function saveCurrent() {
  if (!model.view || !model.file || model.conflict || !model.paperDetail?.editable) return false;
  saveState('saving');
  try {
    const result = await api.saveFile(model.paper.id, model.file.file_id, {
      content: model.view.state.doc.toString(),
      version: model.version,
    });
    model.file = result.file;
    model.version = result.version;
    model.dirty = false;
    saveState('saved');
    notice(`Saved ${model.file.path}`);
    return true;
  } catch (error) {
    if (error.status === 409) {
      model.conflict = true;
      saveState('conflict');
      notice('A newer workspace version exists. Your local text is preserved; reopen the file to reload or copy it before resolving.', true);
    } else {
      saveState('error');
      notice(error.message, true);
    }
    return false;
  }
}

async function reloadPaperAndFile(fileId) {
  model.paperDetail = await api.paper(model.paper.id);
  model.version = model.paperDetail.version;
  model.files = await api.files(model.paper.id);
  renderFiles();
  const file = model.files.find((candidate) => candidate.file_id === fileId);
  if (file) await openFile(file, true);
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
  if (!canLeaveEditor()) return;
  const path = window.prompt('New file path', model.file.path);
  if (!path || path === model.file.path) return;
  try {
    const result = await api.renameFile(model.paper.id, model.file.file_id, { path, version: model.version });
    model.version = result.version;
    await reloadPaperAndFile(result.file.file_id);
  } catch (error) { notice(error.message, true); }
});

ui.deleteFile.addEventListener('click', async () => {
  if (!window.confirm(`Delete ${model.file.path}?`)) return;
  try {
    const deletedId = model.file.file_id;
    const result = await api.deleteFile(model.paper.id, deletedId, { version: model.version });
    model.version = result.version;
    model.file = null;
    model.dirty = false;
    model.conflict = false;
    destroyEditor();
    await openPaper(model.paper);
  } catch (error) { notice(error.message, true); }
});

ui.setMain.addEventListener('click', async () => {
  try {
    const result = await api.setMain(model.paper.id, model.file.file_id, { version: model.version });
    model.version = result.version;
    await reloadPaperAndFile(model.file.file_id);
  } catch (error) { notice(error.message, true); }
});

ui.saveFile.addEventListener('click', saveCurrent);

refreshPapers().catch((error) => notice(error.message, true));
