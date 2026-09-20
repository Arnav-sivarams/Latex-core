const ERROR_PATTERNS = [
  /^!\s+/,
  /^[^\r\n]*:\d+:\s*(?:LaTeX\s+)?Error:/i,
  /^Fatal error\b/i,
];
const WARNING_PATTERNS = [
  /^LaTeX Warning:/i,
  /^Package .* Warning:/i,
  /^(?:Overfull|Underfull) \\[hv]box/i,
  /^(?:\([^)]+\)\s+)?Rerun\b/i,
];

export function classifyBuildLine(line) {
  if (ERROR_PATTERNS.some((pattern) => pattern.test(line))) return 'error';
  if (WARNING_PATTERNS.some((pattern) => pattern.test(line))) return 'warning';
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

export function recognizedBuildProblems(log) {
  return String(log || '').split(/\r?\n/).filter((line) => classifyBuildLine(line) !== 'info').map((message) => ({
    severity: classifyBuildLine(message), message,
  }));
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
