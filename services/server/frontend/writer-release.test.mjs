import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');

test('Download is one accessible menu with the existing non-compiling PDF action', () => {
  assert.match(html, /id="downloadMenu"[^>]*class="[^"]*download-menu/);
  assert.match(html, /<summary aria-label="Download report">[\s\S]*?<span id="downloadLabel">Download<\/span>/);
  assert.match(html, /id="downloadPdf"[^>]*role="menuitem"[^>]*aria-label="Download PDF"[^>]+disabled/);
  assert.match(html, /id="downloadSource"[^>]*role="menuitem"[^>]*aria-label="Download source as ZIP"[^>]+disabled/);
  assert.doesNotMatch(html, />Download PDF<\/button>/);
  assert.match(html, /id="downloadPdf"[^>]+disabled/);
  assert.match(writer, /downloadPdfUrl\(buildId\).*download=true/);
  assert.match(writer, /ui\.downloadPdf\.disabled = !model\.currentBuildId/);
  assert.match(writer, /link\.click\(\)/);
  assert.doesNotMatch(writer.slice(writer.indexOf("ui.downloadPdf.addEventListener")), /api\.build\(/);
});

test('source download flushes durable collaboration state and prevents duplicate requests', () => {
  const handler = writer.slice(
    writer.indexOf("ui.downloadSource.addEventListener"),
    writer.indexOf("ui.sendReview.addEventListener"),
  );
  assert.match(handler, /model\.sourceDownloadInFlight/);
  assert.match(handler, /ui\.downloadSource\.disabled = true/);
  assert.match(handler, /ui\.downloadLabel\.textContent = 'Preparing…'/);
  assert.ok(handler.indexOf('requireDurableFlush()') < handler.indexOf('/source.zip'));
  assert.match(handler, /fetch\(`\/api\/v2\/papers\/\$\{model\.paper\.id\}\/source\.zip`\)/);
  assert.match(handler, /response\.blob\(\)/);
  assert.match(handler, /-source\.zip/);
  assert.match(handler, /finally[\s\S]*?sourceDownloadInFlight = false/);
});

test('Download menu closes with Escape and click-away while retaining native keyboard activation', () => {
  assert.match(html, /<details id="downloadMenu"/);
  assert.match(writer, /event\.key === 'Escape'[\s\S]*?closeTransientMenus\(\)/);
  assert.match(writer, /if \(!ui\.downloadMenu\.contains\(event\.target\)\) ui\.downloadMenu\.open = false/);
  assert.doesNotMatch(writer, /downloadMenu\.addEventListener\(['"]keydown/);
});

test('standard editor selection shortcuts are not application-handled', () => {
  const globalHandler = writer.slice(writer.indexOf("document.addEventListener('keydown'"));
  assert.doesNotMatch(globalHandler, /event\.shiftKey.*(ArrowLeft|ArrowRight|ArrowUp|ArrowDown|Home|End)/);
  assert.doesNotMatch(globalHandler, /event\.preventDefault\(\).*event\.shiftKey/);
  assert.match(writer, /EditorState\.readOnly\.of\(!editable\)/);
  assert.match(writer, /EditorView\.editable\.of\(editable\)/);
});

test('writer selection remains visible over the active line without intercepting native selection', () => {
  assert.match(writer, /&\.cm-focused > \.cm-scroller > \.cm-selectionLayer \.cm-selectionBackground/);
  assert.match(writer, /&:not\(\.cm-focused\) > \.cm-scroller > \.cm-selectionLayer \.cm-selectionBackground/);
  assert.match(writer, /'\.cm-activeLine': \{ backgroundColor: dark \? 'rgba\([^']+,0\.42\)' : 'rgba\([^']+,0\.52\)' \}/);
  assert.match(writer, /\.cm-content ::selection/);
  assert.doesNotMatch(writer, /addEventListener\(['"](?:mousedown|mousemove|mouseup)['"]/);
});
