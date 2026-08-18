export class ApiError extends Error {
  constructor(message, status, body = null) { super(message); this.status = status; this.body = body; }
}

export async function api(path, options = {}) {
  let response;
  try { response = await fetch(path, options); }
  catch (_) { throw new ApiError('Offline — changes not saved', 0); }
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(body?.error || response.statusText || 'Request failed', response.status, body);
  }
  const kind = response.headers.get('content-type') || '';
  return kind.includes('application/json') ? response.json() : response.text();
}

export async function apiText(path) {
  let response;
  try { response = await fetch(path); }
  catch (_) { throw new ApiError('Offline — changes not saved', 0); }
  if (!response.ok) throw new ApiError(response.statusText, response.status);
  return response.text();
}
