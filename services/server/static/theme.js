(function initializeApplicationTheme() {
  const storageKey = 'latex-core-theme';
  const validThemes = new Set(['LIGHT', 'DARK']);
  function normalize(theme) {
    const value = String(theme || '').toUpperCase();
    return validThemes.has(value) ? value : 'LIGHT';
  }
  function apply(theme) {
    const normalized = normalize(theme);
    document.documentElement.dataset.theme = normalized.toLowerCase();
    document.documentElement.style.colorScheme = normalized === 'DARK' ? 'dark' : 'light';
    try { localStorage.setItem(storageKey, normalized); } catch (_) { /* storage is optional */ }
    window.dispatchEvent(new CustomEvent('latex-core-theme-change', { detail: normalized }));
    return normalized;
  }
  let initial = 'LIGHT';
  try { initial = localStorage.getItem(storageKey) || initial; } catch (_) { /* storage is optional */ }
  apply(initial);
  window.LatexCoreTheme = { apply, normalize, storageKey };
  fetch('/api/v2/preferences/editor', { credentials: 'same-origin' })
    .then((response) => response.ok ? response.json() : null)
    .then((preferences) => preferences?.theme && apply(preferences.theme))
    .catch(() => {});
}());
