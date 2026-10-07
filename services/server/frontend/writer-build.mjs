const ERROR_PATTERNS = [
  /^!\s+/,
  /^[^\r\n]*\.(?:tex|ltx|sty|cls):\d+:\s*(?!.*\bWarning\b).+/i,
  /^Fatal error\b/i,
];
const WARNING_PATTERNS = [
  /^LaTeX Warning:/i,
  /^Package .* Warning:/i,
  /^(?:Overfull|Underfull) \\[hv]box/i,
  /^(?:\([^)]+\)\s+)?Rerun\b/i,
];

export function classifyBuildLine(line) {
  const explicit = line.match(/^.+?\.(?:tex|ltx|sty|cls):\d+:\s*(.+)$/i);
  const message = explicit?.[1] || line;
  if (WARNING_PATTERNS.some((pattern) => pattern.test(message))) return 'warning';
  if (explicit || ERROR_PATTERNS.some((pattern) => pattern.test(message))) return 'error';
  return 'info';
}

export function buildLogLines(log) {
  return String(log || '').split(/\r?\n/).map((text) => ({
    text,
    problem: classifyBuildLine(text) !== 'info',
  }));
}

export function renderBuildLog(host, log, documentRef = document) {
  host.replaceChildren();
  buildLogLines(log).forEach(({ text, problem }, index, lines) => {
    const line = documentRef.createElement('span');
    line.className = `build-log-line${problem ? ' build-log-problem' : ''}`;
    line.textContent = text;
    host.append(line);
    if (index < lines.length - 1) host.append(documentRef.createTextNode('\n'));
  });
}

export function recognizedBuildProblems(log, files = [], mainFile = '') {
  const mainDirectory = mainFile.includes('/') ? mainFile.slice(0, mainFile.lastIndexOf('/') + 1) : '';
  return String(log || '').split(/\r?\n/).filter((line) => classifyBuildLine(line) !== 'info').map((message) => {
    const problem = { severity: classifyBuildLine(message), message };
    const match = message.match(/^(.+?\.(?:tex|ltx|sty|cls)):(\d+):\s*(.+)$/i);
    if (!match) return problem;
    const rawPath = match[1].replace(/^\.\//, '').replace(/^\/work\//, '');
    if (rawPath.split('/').some((part) => part === '..')) return problem;
    const file = files.find((item) => item.path === rawPath)
      || files.find((item) => item.path === mainDirectory + rawPath);
    return { ...problem, message: match[3], path: file?.path || rawPath,
      file_id: file?.file_id, range: { start_line: Number(match[2]), start_column: 0 } };
  });
}

export function problemsState(diagnostics) {
  const errors = diagnostics.filter((item) => item.severity === 'error').length;
  const warnings = diagnostics.filter((item) => item.severity === 'warning').length;
  return { severity: errors ? 'error' : warnings ? 'warning' : 'neutral',
    label: `Problems: ${errors} ${errors === 1 ? 'error' : 'errors'}, ${warnings} ${warnings === 1 ? 'warning' : 'warnings'}` };
}

export function supportLink(email) {
  return typeof email === 'string' && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)
    ? `mailto:${encodeURIComponent(email).replace('%40', '@')}?subject=LaTeX%20Core%20Support` : null;
}

export function shortBuildState(build) {
  if (!build) return 'Ready';
  if (build.active_build_id && build.active_status === 'queued') return 'Queued';
  if (build.active_build_id) return 'Compiling…';
  if (build.latest_status === 'failed') return 'Compilation failed';
  if (build.current_build_id) return 'Compiled';
  return 'Ready';
}

export function buildIsStale(build) {
  if (!build?.current_build_id) return false;
  const sourceChanged = build.source_sequence != null
    && build.current_source_sequence != null
    && build.source_sequence !== build.current_source_sequence;
  const exactStateChanged = build.desired_state_hash !== build.current_state_hash;
  return sourceChanged || exactStateChanged;
}
