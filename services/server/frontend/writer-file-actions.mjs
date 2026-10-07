export function resolveFileActionTarget({ explicitTarget = null, selectedFileId = null, currentFile = null, files = [] }) {
  const byId = (fileId) => fileId ? files.find((file) => file.file_id === fileId) || null : null;
  if (explicitTarget) return byId(explicitTarget.file_id);
  return byId(selectedFileId) || byId(currentFile?.file_id);
}

export function renamePathError(path, target, files) {
  if (!path.trim()) return 'Enter a non-empty project-relative path.';
  if (path.startsWith('/') || path.startsWith('\\') || /^[A-Za-z]:[\\/]/.test(path)) {
    return 'Use a project-relative path, not an absolute path.';
  }
  const segments = path.split('/');
  if (segments.some((segment) => !segment || segment === '.' || segment === '..') || path.includes('\\')) {
    return 'Use a valid project-relative path without empty, “.”, or “..” segments.';
  }
  if (files.some((file) => file.file_id !== target.file_id && file.path === path)) {
    return `A file named ${path} already exists.`;
  }
  return null;
}

export function fileActionError(operation, target, error) {
  const verb = operation === 'rename' ? 'rename' : 'delete';
  const prefix = `Could not ${verb} ${target.path}:`;
  const message = String(error?.message || 'the request failed.');
  if (error?.status === 404) return `${prefix} the file no longer exists.`;
  if (error?.status === 403) return `${prefix} you are not authorized to change this file.`;
  if (error?.status === 409 && /already uses path|already exists/i.test(message)) {
    return `${prefix} a file with the requested path already exists.`;
  }
  if (error?.status === 409) return `${prefix} the project changed. Refresh the file tree and try again.`;
  if (error?.status === 400) return `${prefix} ${message}`;
  if (error?.status >= 500) return `${prefix} the server could not complete the operation. Try again.`;
  return `${prefix} ${message}`;
}

export function boundedMenuPosition(x, y, viewportWidth, viewportHeight, width = 180, height = 210) {
  return {
    left: Math.max(8, Math.min(x, viewportWidth - width - 8)),
    top: Math.max(8, Math.min(y, viewportHeight - height - 8)),
  };
}
