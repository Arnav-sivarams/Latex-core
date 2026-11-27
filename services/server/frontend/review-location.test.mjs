import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { reviewLocation, reviewStateLabel } from './review-location.mjs';

test('review comments name their file, line range and PDF page', () => {
  const files = [{ file_id: 'f2', path: 'report/chapters/results.tex' }];
  assert.equal(reviewLocation({ source_anchor: { path: 'report/chapters/intro.tex', start_line: 12, end_line: 14 }, pdf_anchor: { page: 30 } }).label, 'intro.tex · lines 12–14 · PDF page 30');
  assert.equal(reviewLocation({ source_anchor: { path: 'main.tex', start_line: 5, end_line: 5 } }).label, 'main.tex · line 5');
  assert.equal(reviewLocation({ pdf_anchor: { page: 19, mapped_file_id: 'f2', mapped_line: 40 } }, files).label, 'results.tex · line 40 · PDF page 19');
  assert.equal(reviewLocation({ pdf_anchor: { page: 3 } }).label, 'PDF page 3');
  assert.equal(reviewLocation({}).label, 'Location not recorded');
});

test('Writer Done goes back to the Mentor for confirmation instead of resolving', () => {
  assert.equal(reviewStateLabel('ADDRESSED'), 'Done — awaiting Mentor confirmation');
  assert.equal(reviewStateLabel('ADDRESSED', 'mentor'), 'Writer marked done — confirm or reopen');
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.doesNotMatch(writer, /reviewState\([^)]*'RESOLVED'\)/);
  assert.match(writer, /reviewState\(model\.paper\.id, thread\.id, 'ADDRESSED'\)/);
  const mentor = readFileSync(new URL('./review.js', import.meta.url), 'utf8');
  assert.match(mentor, /transition\(thread, 'RESOLVED'\)/);
});
