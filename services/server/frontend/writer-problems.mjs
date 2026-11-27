import { diagnosticIcon } from './writer-policy.mjs';

// Presentation only: keep the original diagnostic intact for source navigation.
export function renderProblemList(host, diagnostics, openLocation) {
  host.replaceChildren();
  if (!diagnostics.length) {
    const empty = document.createElement('p');
    empty.className = 'empty-copy';
    empty.textContent = 'No current problems.';
    host.append(empty);
    return;
  }
  for (const diagnostic of diagnostics) {
    const severity = diagnostic.severity || 'info';
    const line = diagnostic.range?.start_line;
    const location = `${diagnostic.path || 'Build'}${line ? `:${line}` : ''}`;
    const description = `${severity}: ${diagnostic.message}${diagnostic.detail ? `\n${diagnostic.detail}` : ''}\n${location}${diagnostic.code ? ` · ${diagnostic.code}` : ''}`;
    const row = document.createElement('button');
    row.type = 'button';
    row.className = `problem-row build-problem-${severity}`;
    row.title = description;
    row.setAttribute('aria-label', description);
    row.addEventListener('click', () => diagnostic.file_id && openLocation(diagnostic));
    const icon = document.createElement('span');
    icon.className = 'problem-icon';
    icon.setAttribute('aria-hidden', 'true');
    icon.innerHTML = diagnosticIcon(severity);
    const message = document.createElement('span');
    message.className = 'problem-message';
    message.textContent = diagnostic.message;
    if (diagnostic.detail) {
      // Root cause: the source text TeX reported for this error.
      const detail = document.createElement('code');
      detail.className = 'problem-detail';
      detail.textContent = diagnostic.detail;
      message.append(' ', detail);
    }
    const reference = document.createElement('span');
    reference.className = 'problem-location';
    const path = document.createElement('span');
    path.className = 'problem-path';
    path.textContent = diagnostic.path?.split('/').pop() || 'Build';
    if (diagnostic.path) path.title = diagnostic.path;
    reference.append(path);
    if (line) {
      const number = document.createElement('span');
      number.className = 'problem-line';
      number.textContent = `:${line}`;
      reference.append(number);
    }
    row.append(icon, message, reference);
    host.append(row);
  }
}
