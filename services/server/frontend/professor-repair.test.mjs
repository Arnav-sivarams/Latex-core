import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const review = readFileSync(new URL('./review.js', import.meta.url), 'utf8');
const writerHtml = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
const reviewHtml = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
const templateRoot = new URL('../../../artifacts/Full_Report_template_v1.1-source/Full_Report_template_v1.0/', import.meta.url);

function functionBody(source, name, nextName) {
  return source.slice(source.indexOf(`function ${name}`), source.indexOf(`function ${nextName}`));
}

test('Writer and Mentor keep visible selections and install bounded pane splitters', () => {
  assert.match(writer, /&:not\(\.cm-focused\) \.cm-selectionBackground/);
  assert.match(review, /&:not\(\.cm-focused\) \.cm-selectionBackground/);
  assert.equal((writerHtml.match(/class="pane-splitter"/g) || []).length, 2);
  assert.equal((reviewHtml.match(/class="pane-splitter"/g) || []).length, 2);
  assert.match(writer, /installWorkspaceSplitters\([^;]+latex-core-writer-pane-widths/);
  assert.match(review, /installWorkspaceSplitters\([^;]+latex-core-mentor-pane-widths/);
});

test('Writer presents the conventional images directory without a legacy warning', () => {
  assert.match(writer, /name === 'images' \? 'Images' : name/);
  assert.match(writer, /Project images: images\//);
  assert.doesNotMatch(writer, /Images? \(legacy\)|Legacy project-local asset path/);
  assert.doesNotMatch(review, /Images? \(legacy\)|Legacy project-local asset path/);
});

test('upload insertion, save sequencing, inverse SyncTeX, and Insert menu contracts remain connected', () => {
  assert.match(writerHtml, /id="newFile"[^>]+aria-label="New file"[^>]*>＋<\/button>/);
  assert.match(writerHtml, /id="uploadImage"[^>]+aria-label="Upload image"[^>]*>⇧<\/button>/);
  assert.match(writer, /captureInsertionSelection\(\);\s*model\.assetUploadTarget[\s\S]+ui\.assetInput\.click\(\)/);
  assert.match(writer, /openBuilder\('figure', \{ asset: result\.file\.path \}, \{ preserveSelection: true \}\)/);
  assert.match(writer, /session\.clientSequence === sequence && session\.pending\.size === 0/);
  assert.match(writer, /model\.saveInFlight \? 'Saving…' : 'Save'/);
  assert.match(writer, /addEventListener\('dblclick'[\s\S]+direction: 'INVERSE'[\s\S]+mapping\.build_id !== buildId/);
  const insertMenu = functionBody(writer, 'openInsertMenu', 'openSymbols');
  assert.match(insertMenu, /category: 'Structure'/);
  assert.match(insertMenu, /category: 'Media'/);
  assert.match(insertMenu, /label: 'Wrap figure'/);
  assert.doesNotMatch(insertMenu, /Comment selected lines|Uncomment selected lines/);
  assert.match(writer, /function toggleSourceComment/);
});

test('authoritative report source fixes acknowledgement wrapping and labels every starter heading', () => {
  const acknowledgement = readFileSync(new URL('acknowledgement.tex', templateRoot), 'utf8');
  const active = acknowledgement.split('\n').filter((line) => !line.trimStart().startsWith('%')).join('\n');
  assert.doesNotMatch(active, /\\hspace\{1cm\}/);
  assert.equal((active.match(/\\setlength\{\\parindent\}\{1cm\}\\indent/g) || []).length, 5);

  const labels = new Set();
  for (let chapter = 1; chapter <= 8; chapter += 1) {
    const source = readFileSync(new URL(`chapters/chapter${chapter}.tex`, templateRoot), 'utf8');
    const lines = source.split('\n');
    for (let index = 0; index < lines.length; index += 1) {
      if (!/^\\(?:chapter|section|subsection)\{/.test(lines[index].trim())) continue;
      const next = lines.slice(index + 1).find((line) => line.trim());
      assert.match(next || '', /^\\label\{[^}]+\}$/);
      assert.equal(labels.has(next.trim()), false, `${next} must be unique`);
      labels.add(next.trim());
    }
  }
  assert.ok(labels.size > 8);
});

test('all title-bearing report pages share the authoritative title macro', () => {
  for (const page of ['coverpage.tex', 'certificate.tex', 'declaration.tex']) {
    assert.match(readFileSync(new URL(page, templateRoot), 'utf8'), /\\thesistitle/);
  }
});
