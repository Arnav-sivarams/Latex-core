import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as Y from 'yjs';
import { denormalizeRectangle, normalizeRectangle, resolveSuggestionRange, showsReplacementInput } from './review-helpers.mjs';

function sync(from, to) {
  Y.applyUpdate(to, Y.encodeStateAsUpdate(from, Y.encodeStateVector(to)), 'remote');
}

test('Yjs relative source anchor survives concurrent insertion before range', () => {
  const mentor = new Y.Doc();
  const source = mentor.getText('source');
  source.insert(0, 'alpha target omega');
  const writer = new Y.Doc();
  sync(mentor, writer);
  const start = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(source, 6));
  const end = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(source, 12));
  writer.getText('source').insert(0, 'prefix ');
  sync(writer, mentor);
  const range = resolveSuggestionRange(mentor, source, start, end);
  assert.deepEqual(range, { from: 13, to: 19 });
  assert.equal(source.toString().slice(range.from, range.to), 'target');
});

test('PDF rectangle normalization survives zoom and rerender', () => {
  const normalized = normalizeRectangle({ x1: 100, y1: 80, x2: 300, y2: 180 }, 600, 800);
  assert.deepEqual(denormalizeRectangle(normalized, 1200, 1600), { x: 200, y: 160, width: 400, height: 200 });
});

test('review round and replacement controls require their exact prerequisites', () => {
  for (const type of ['COMMENT', 'QUESTION', 'CHANGE_REQUEST', 'SECTION_APPROVAL']) {
    assert.equal(showsReplacementInput(type), false);
  }
  assert.equal(showsReplacementInput('SUGGESTION'), true);
  assert.equal(showsReplacementInput('SUGGESTED_REPLACEMENT'), true);
  const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
  assert.match(css, /\.annotation-composer \[hidden\] \{ display: none !important; \}/);
});

test('source selection preserves viewport until Review selection is clicked', () => {
  const review = readFileSync(new URL('./review.js', import.meta.url), 'utf8');
  const handler = review.slice(review.indexOf('async function sourceSelected'), review.indexOf('async function sha256'));
  assert.doesNotMatch(handler, /scrollIntoView|\.focus\(|showComposer/);
  assert.match(review, /ui\.reviewSelection\.addEventListener\('click'/);
  const html = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
  assert.match(html, /id="reviewSelection"[^>]*disabled>Review selection/);
});

test('suggestion helper refuses an anchor unresolved in the current document', () => {
  const oldDoc = new Y.Doc();
  const oldText = oldDoc.getText('source');
  oldText.insert(0, 'replace me');
  const start = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(oldText, 0));
  const end = Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(oldText, 7));
  const current = new Y.Doc();
  const currentText = current.getText('source');
  currentText.insert(0, 'different document');
  assert.equal(resolveSuggestionRange(current, currentText, start, end), null);
});

test('Mentor bundle is read-only and PDF.js assets are same-origin', () => {
  const review = readFileSync(new URL('./review.js', import.meta.url), 'utf8');
  const html = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
  const packageJson = JSON.parse(readFileSync(new URL('../../../package.json', import.meta.url), 'utf8'));
  assert.match(review, /EditorView\.editable\.of\(false\)/);
  assert.doesNotMatch(review, /sendUpdate|0x01|contenteditable\s*=\s*["']?true/i);
  assert.match(review, /\/static\/pdf\.min\.mjs/);
  assert.match(review, /\/static\/pdf\.worker\.min\.mjs/);
  assert.doesNotMatch(html, /iframe|cdn|Set Main|New File|Publish/i);
  assert.equal(packageJson.dependencies['pdfjs-dist'], '6.3.289');
});

test('V2.1 review UI exposes only gated comments and suggestions', () => {
  const review = readFileSync(new URL('./review.js', import.meta.url), 'utf8');
  const html = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
  assert.match(html, /value="COMMENT">Comment/);
  assert.match(html, /value="SUGGESTION">Suggestion/);
  assert.doesNotMatch(html, /id="severity"|id="category"|id="assignedWriter"|id="dueDate"/);
  assert.doesNotMatch(html, /RESTORATION REQUESTS|CHANGES SINCE LAST REVIEW|>ACTIVITY</);
  assert.match(review, /This paper has not been sent for review\./);
  assert.match(review, /status === 'OPEN_FOR_REVIEW'/);
});

test('Writer surface uses Save semantics and inline historical review highlights', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
  assert.match(html, /id="saveFile"[^>]*>Save</);
  assert.doesNotMatch(html, /Sync now/);
  assert.match(writer, /Saving…/);
  assert.match(writer, /Saved\/Synced/);
  assert.match(writer, /review-source-highlight/);
  assert.match(writer, /button\('Done'/);
  assert.match(writer, /button\('Apply'/);
  assert.match(html, />Send for Review</);
});

test('Admin Team UI requires and can reassign an assigned Writer Leader', () => {
  const admin = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
  const html = readFileSync(new URL('../src/admin.html', import.meta.url), 'utf8');
  assert.match(admin, /leader_writer_id/);
  assert.match(admin, /\/paper-teams\/\$\{team\.id\}\/leader/);
  assert.match(admin, /Leader: \$\{leader\?\.email/);
  assert.doesNotMatch(html, /data-section="Restoration Requests"/);
});

test('Writer suggestion acceptance orders Yjs edit, durable flush, then acceptance record', () => {
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  const edit = writer.indexOf("}, 'writer-suggestion-accept')");
  const flush = writer.indexOf('model.collaboration.flush()', edit);
  const accepted = writer.indexOf('api.acceptSuggestion', flush);
  assert.ok(edit > 0 && flush > edit && accepted > flush);
  assert.match(writer, /if \(!range\) return notice\('Suggestion anchor is stale or unresolved; nothing was changed\.'/);
});
