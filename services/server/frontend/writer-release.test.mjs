import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');

test('PDF download is a non-compiling last-good artifact action', () => {
  assert.match(html, /id="downloadPdf"[^>]+disabled/);
  assert.match(writer, /downloadPdfUrl\(buildId\).*download=true/);
  assert.match(writer, /ui\.downloadPdf\.disabled = !model\.currentBuildId/);
  assert.match(writer, /link\.click\(\)/);
  assert.doesNotMatch(writer.slice(writer.indexOf("ui.downloadPdf.addEventListener")), /api\.build\(/);
});

test('standard editor selection shortcuts are not application-handled', () => {
  const globalHandler = writer.slice(writer.indexOf("document.addEventListener('keydown'"));
  assert.doesNotMatch(globalHandler, /event\.shiftKey.*(ArrowLeft|ArrowRight|ArrowUp|ArrowDown|Home|End)/);
  assert.doesNotMatch(globalHandler, /event\.preventDefault\(\).*event\.shiftKey/);
  assert.match(writer, /EditorState\.readOnly\.of\(!editable\)/);
  assert.match(writer, /EditorView\.editable\.of\(editable\)/);
});

test('writer selection uses the accent without intercepting native selection', () => {
  assert.match(writer, /cm-selectionBackground/);
  assert.match(writer, /color-mix\(in srgb, var\(--accent\)/);
  assert.match(writer, /\.cm-content ::selection/);
  assert.doesNotMatch(writer, /addEventListener\(['"](?:mousedown|mousemove|mouseup)['"]/);
});
