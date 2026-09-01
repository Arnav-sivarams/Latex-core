import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { pdfPreviewState } from './writer-pdf.mjs';

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

test('Writer DOM and CSS collapse the hidden PDF state', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
  assert.match(writer, /ui\.pdfFrame\.hidden = !preview\.viewer/);
  assert.match(writer, /ui\.pdfEmpty\.hidden = !preview\.empty/);
  assert.match(css, /\.pdf-foundation > \[hidden\] \{ display: none !important; \}/);
  assert.match(css, /\.pdf-frame \{[^}]*display: block;/);
});
