const MIN_FILES = 150;
const MIN_SOURCE = 280;
const MIN_PDF = 280;

export function clampPaneWidths(total, files, source) {
  const available = Math.max(MIN_FILES + MIN_SOURCE + MIN_PDF, Number(total) || 0);
  const nextFiles = Math.min(Math.max(MIN_FILES, Number(files) || MIN_FILES), available - MIN_SOURCE - MIN_PDF);
  const nextSource = Math.min(Math.max(MIN_SOURCE, Number(source) || MIN_SOURCE), available - nextFiles - MIN_PDF);
  return { files: nextFiles, source: nextSource };
}

export function installWorkspaceSplitters(workspace, storageKey) {
  if (!workspace) return;
  const splitters = [...workspace.querySelectorAll('.pane-splitter')];
  if (splitters.length !== 2) return;
  const defaults = {
    files: workspace.querySelector('.files-pane')?.getBoundingClientRect().width || 230,
    source: workspace.querySelector('.source-pane')?.getBoundingClientRect().width || 520,
  };
  let stored = {};
  try { stored = JSON.parse(sessionStorage.getItem(storageKey) || '{}'); } catch { stored = {}; }
  const apply = (candidate) => {
    const widths = clampPaneWidths(workspace.getBoundingClientRect().width - 12, candidate.files, candidate.source);
    workspace.style.setProperty('--files-pane-width', `${widths.files}px`);
    workspace.style.setProperty('--source-pane-width', `${widths.source}px`);
    try { sessionStorage.setItem(storageKey, JSON.stringify(widths)); } catch { /* Layout persistence is optional. */ }
    return widths;
  };
  let widths = apply({ ...defaults, ...stored });
  splitters.forEach((splitter, index) => {
    splitter.addEventListener('pointerdown', (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      const startX = event.clientX;
      const start = { ...widths };
      splitter.setPointerCapture(event.pointerId);
      workspace.classList.add('is-resizing');
      const move = (moveEvent) => {
        const delta = moveEvent.clientX - startX;
        widths = apply(index === 0 ? { files: start.files + delta, source: start.source } : { files: start.files, source: start.source + delta });
      };
      const stop = () => {
        splitter.removeEventListener('pointermove', move);
        splitter.removeEventListener('pointerup', stop);
        splitter.removeEventListener('pointercancel', stop);
        splitter.removeEventListener('lostpointercapture', stop);
        workspace.classList.remove('is-resizing');
      };
      splitter.addEventListener('pointermove', move);
      splitter.addEventListener('pointerup', stop);
      splitter.addEventListener('pointercancel', stop);
      splitter.addEventListener('lostpointercapture', stop);
    });
  });
  window.addEventListener('resize', () => { widths = apply(widths); });
}
