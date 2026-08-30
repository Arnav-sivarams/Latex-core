import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');

test('Writer exposes exact-state PDF and History with manual compilation only', () => {
  assert.doesNotMatch(writer, /requestBuild\('auto'\)/);
  assert.match(writer, /requestBuild\('manual'\)/);
  assert.match(writer, /key: 'Mod-Enter'/);
  assert.match(writer, /key: 'Mod-s'/);
  assert.match(writer, /Build failed — showing last successful PDF/);
  assert.match(writer, /REMOTE_DURABLE/);
  assert.match(html, /id="compilePaper"/);
  assert.match(html, /id="pdfFrame"/);
  assert.match(html, /id="versionHistory"/);
  assert.match(html, /id="createCheckpoint"/);
  assert.doesNotMatch(html, /Restore/);
  assert.doesNotMatch(html, /cdn/i);
});
