// Visible, dismissible status notices shared by the Writer and Mentor shells.
// Success notices fade after a few seconds; failures stay until the user closes them.
export const NOTICE_TIMEOUT_MS = 8000;

export function showStatusNotice(region, message, { failed = false, action = null, timeoutMs = failed ? 0 : NOTICE_TIMEOUT_MS, schedule = (callback, delay) => setTimeout(callback, delay), cancel = (timer) => clearTimeout(timer) } = {}) {
  if (!region) return;
  if (region.noticeTimer) { cancel(region.noticeTimer); region.noticeTimer = null; }
  region.classList.toggle('danger', Boolean(failed));
  const text = String(message ?? '');
  if (!text) { region.replaceChildren(); return; }
  const documentRef = region.ownerDocument;
  const body = documentRef.createElement('span'); body.className = 'status-message'; body.textContent = text;
  const parts = [body];
  if (action?.label && typeof action.run === 'function') {
    const button = documentRef.createElement('button'); button.type = 'button'; button.className = 'status-action'; button.textContent = action.label;
    button.addEventListener('click', () => { Promise.resolve().then(action.run).catch(() => {}); });
    parts.push(button);
  }
  const close = documentRef.createElement('button'); close.type = 'button'; close.className = 'status-close'; close.textContent = '×';
  close.setAttribute('aria-label', 'Dismiss notification'); close.title = 'Dismiss';
  close.addEventListener('click', () => showStatusNotice(region, '', { cancel }));
  parts.push(close);
  region.replaceChildren(...parts);
  region.setAttribute('role', failed ? 'alert' : 'status');
  if (timeoutMs > 0) region.noticeTimer = schedule(() => { region.noticeTimer = null; region.replaceChildren(); }, timeoutMs);
}
