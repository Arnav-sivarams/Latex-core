export function canSetMain(file, detail, editable = true) {
  return Boolean(editable && file?.file_id && /\.tex$/i.test(file.path)
    && !detail?.main_file_fixed && file.path !== detail?.main_file);
}

export function diagnosticIcon(severity = 'information') {
  const shape = severity === 'error'
    ? '<circle cx="12" cy="12" r="9"/><path d="m8 8 8 8m0-8-8 8"/>'
    : '<path d="M12 3 2 21h20Z"/><path d="M12 9v5m0 3v1"/>';
  return `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">${shape}</svg>`;
}
