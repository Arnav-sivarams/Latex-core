import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { pdfPointFromClient, pdfPreviewState } from './writer-pdf.mjs';

test('Writer PDF empty and viewer states are mutually exclusive', () => {
  assert.deepEqual(pdfPreviewState({ current_build_id: null, active_build_id: null, latest_status: null }), {
    empty: true, viewer: false, rebuildingWithLastGood: false, failedWithLastGood: false,
  });
  assert.deepEqual(pdfPreviewState({ current_build_id: 'current', active_build_id: null, latest_status: 'succeeded' }), {
    empty: false, viewer: true, rebuildingWithLastGood: false, failedWithLastGood: false,
  });
  assert.deepEqual(pdfPreviewState({ current_build_id: 'last-good', active_build_id: 'next', latest_status: 'running' }), {
    empty: false, viewer: true, rebuildingWithLastGood: true, failedWithLastGood: false,
  });
  assert.deepEqual(pdfPreviewState({ current_build_id: 'last-good', active_build_id: null, latest_status: 'failed' }), {
    empty: false, viewer: true, rebuildingWithLastGood: false, failedWithLastGood: true,
  });
});

test('inverse SyncTeX coordinates account for rendered zoom and page position', () => {
  assert.deepEqual(pdfPointFromClient(241, 334, { left: 100, top: 200 }, 100), { x: 141, y: 134 });
  assert.deepEqual(pdfPointFromClient(350, 500, { left: 100, top: 200 }, 125), { x: 200, y: 240 });
});

test('inverse navigation keeps the displayed build, nested mapping, stale gate, and feedback-loop guard connected', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.match(writer, /!model\.pdfCurrent \|\| model\.pdfDisplayBuildId !== model\.currentBuildId/);
  assert.match(writer, /const buildId = model\.pdfDisplayBuildId/);
  assert.match(writer, /mapping\.build_id !== buildId/);
  assert.match(writer, /mapping\.mapped_file_id[\s\S]*mapping\.mapped_line/);
  assert.match(writer, /No source position is mapped at that PDF location\./);
  assert.match(writer, /invalidateForwardSync\(\);\s*model\.inverseNavigating = true;[\s\S]*await openLocation/);
  assert.match(writer, /else if \(update\.selectionSet && !model\.inverseNavigating\) scheduleForwardSync\(\)/);
});

test('Writer DOM and CSS collapse the hidden PDF state', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
  assert.match(writer, /ui\.pdfScroll\.hidden = !preview\.viewer/);
  assert.match(writer, /ui\.pdfEmpty\.hidden = !preview\.empty/);
  assert.match(css, /\.pdf-foundation > \[hidden\] \{ display: none !important; \}/);
  assert.match(css, /\.writer-pdf-scroll \{[^}]*overflow: auto;/);
  assert.match(writer, /capturePdfView/);
  assert.match(writer, /model\.pdfRestoring/);
  assert.match(writer, /model\.pdfLoadingTask\?\.destroy\?\.\(\)/);
  assert.match(writer, /model\.paper\?\.id !== paperId/);
});

test('automatic forward SyncTeX follows the caret only against the current exact PDF', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.match(writer, /EditorView\.updateListener[\s\S]*update\.selectionSet[\s\S]*scheduleForwardSync/);
  assert.match(writer, /const head = selection\.head/);
  assert.match(writer, /model\.pdfDisplayBuildId !== model\.currentBuildId/);
  assert.match(writer, /model\.view\?\.state\.selection\.main\.head !== head/);
  assert.match(writer, /if \(update\.docChanged\) markPdfStaleFromEditor\(\)/);
  assert.match(writer, /Compile current source to synchronize PDF\./);
});
