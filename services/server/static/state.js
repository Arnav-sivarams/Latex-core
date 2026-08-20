const key = 'latex-core.workspace-preferences.v1';
const defaults = { left: 242, right: 520, bottom: 190, sidebar: false, pdf: false };
function preferences() { try { return { ...defaults, ...JSON.parse(localStorage.getItem(key) || '{}') }; } catch (_) { return { ...defaults }; } }
export const state = { user: null, project: null, currentFile: null, tabs: new Map(), saveState: 'CLEAN', saveTimer: null, job: null, activeTeam: null, expanded: new Set(), preferences: preferences(), templates: [], commands: [], commandIndex: 0 };
export function savePreferences() { localStorage.setItem(key, JSON.stringify(state.preferences)); }
export function emergencyKey() { return state.project && state.currentFile ? `latex-core.emergency.${state.project.id}.${state.currentFile}` : ''; }
export function cacheBuffer(buffer) { const key = emergencyKey(); if (key) localStorage.setItem(key, JSON.stringify({ text: buffer.text, savedAt: Date.now() })); }
export function clearBufferCache() { const key = emergencyKey(); if (key) localStorage.removeItem(key); }
