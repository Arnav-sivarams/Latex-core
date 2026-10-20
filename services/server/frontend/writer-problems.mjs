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
    const description = `${severity}: ${diagnostic.message}\n${location}${diagnostic.code ? ` · ${diagnostic.code}` : ''}`;
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
    const reference = document.createElement('span');
    reference.className = 'problem-location';
    const path = document.createElement('span');
    path.className = 'problem-path';
    path.textContent = diagnostic.path?.split('/').pop() || 'Build';
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
