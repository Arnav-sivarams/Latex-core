import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  boundedMenuPosition, fileActionError, renamePathError, resolveFileActionTarget,
} from './writer-file-actions.mjs';

const files = [
  { file_id: 'open', path: 'open.tex' },
  { file_id: 'selected', path: 'sections/selected.tex' },
  { file_id: 'explicit', path: 'images/explicit.png' },
  { file_id: 'collision', path: 'existing.tex' },
];

test('explicit row/context target wins over selected and open files', () => {
  assert.equal(resolveFileActionTarget({
    explicitTarget: files[2], selectedFileId: 'selected', currentFile: files[0], files,
  }), files[2]);
  assert.equal(resolveFileActionTarget({ selectedFileId: 'selected', currentFile: files[0], files }), files[1]);
  assert.equal(resolveFileActionTarget({ currentFile: files[0], files }), files[0]);
  assert.equal(resolveFileActionTarget({ explicitTarget: { file_id: 'gone' }, selectedFileId: 'selected', currentFile: files[0], files }), null);
});

test('rename validation rejects empty, traversal, absolute, and colliding paths', () => {
  assert.match(renamePathError('', files[1], files), /non-empty/);
  assert.match(renamePathError('../escape.tex', files[1], files), /without/);
  assert.match(renamePathError('/absolute.tex', files[1], files), /project-relative/);
  assert.match(renamePathError('C:\\absolute.tex', files[1], files), /project-relative/);
  assert.match(renamePathError('existing.tex', files[1], files), /already exists/);
  assert.equal(renamePathError('sections/introduction.tex', files[1], files), null);
});

test('file action errors are useful and never expose backend identifiers', () => {
  const target = { file_id: 'secret-id', path: 'other.tex' };
  const collision = fileActionError('rename', target, { status: 409, message: 'a live file already uses path existing.tex in workspace secret-workspace' });
  assert.match(collision, /Could not rename other\.tex/);
  assert.match(collision, /already exists/);
  assert.doesNotMatch(collision, /secret/);
  assert.match(fileActionError('delete', target, { status: 404 }), /no longer exists/);
  assert.match(fileActionError('rename', target, { status: 500 }), /server could not complete/);
});

test('context menu coordinates remain within the viewport', () => {
  assert.deepEqual(boundedMenuPosition(990, 790, 1000, 800), { left: 812, top: 582 });
  assert.deepEqual(boundedMenuPosition(-10, -20, 1000, 800), { left: 8, top: 8 });
});

test('Writer API refetches authoritative file state after collaboration actions', () => {
  const source = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.match(source, /fetch\(path, \{ credentials: 'same-origin', cache: 'no-store', \.\.\.options \}\)/);
  assert.match(source, /control\.type === 'FILES_CHANGED'/);
  assert.match(source, /refreshReportFiles\(control\.file_id, control\.revision\)/);
});
