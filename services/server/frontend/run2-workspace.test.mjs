import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const writerHtml = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
const mentorHtml = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const mentor = readFileSync(new URL('./review.js', import.meta.url), 'utf8');

test('Writer and Mentor use viewport-locked three-pane workspaces with independent scroll owners', () => {
  assert.match(css, /\.workspace-shell\s*\{[^}]*height:\s*100vh[^}]*overflow:\s*hidden/s);
  assert.match(css, /\.three-pane-workspace\s*\{[^}]*min-height:\s*0[^}]*grid-template-columns:[^}]*overflow:\s*hidden/s);
  assert.match(css, /\.files-pane[^}]*overflow-y:\s*auto/s);
  assert.match(css, /\.source-pane \.editor-mount \.cm-scroller[^}]*overflow:\s*auto/s);
  assert.match(css, /\.pdf-scroll-region[^}]*overflow:\s*auto/s);
  for (const html of [writerHtml, mentorHtml]) assert.match(html, /three-pane-workspace/);
});

test('toolbar icon actions are named and permanent detail cards are absent', () => {
  for (const html of [writerHtml, mentorHtml]) {
    const icons = [...html.matchAll(/<(?:button|a)[^>]*class="[^"]*(?:icon-button|icon-link|mini-icon)[^"]*"[^>]*>/g)].map((match) => match[0]);
    assert.ok(icons.length >= 8);
    icons.forEach((element) => { assert.match(element, /aria-label="[^"]+"/); assert.match(element, /title="[^"]+"/); });
  }
  assert.doesNotMatch(mentorHtml, /NEW ANNOTATION|RESTORATION REQUESTS|CHANGES SINCE|>ACTIVITY</);
  assert.doesNotMatch(writerHtml, /detail-tabs/);
  assert.match(writerHtml, /id="workspaceDrawer"[^>]*hidden/);
});

test('Mentor selection opens one compact gated input only from contextual actions', () => {
  const selection = mentor.slice(mentor.indexOf('async function sourceSelected'), mentor.indexOf("ui.reviewEditor.addEventListener('contextmenu'"));
  assert.doesNotMatch(selection, /scrollIntoView|\.focus\(|showReviewPopover/);
  assert.match(mentor, /addEventListener\('contextmenu'/);
  assert.match(mentor, /if \(!reviewOpen\(\)/);
  assert.match(mentorHtml, /id="reviewPopover"[^>]*hidden/);
  assert.equal((mentorHtml.match(/id="threadMessage"/g) || []).length, 1);
  assert.doesNotMatch(mentorHtml, /id="severity"|id="category"|id="assignedWriter"|id="dueDate"/);
  assert.match(mentor, /type === 'SUGGESTION'/);
  assert.match(mentor, /createSuggestion\.hidden = !model\.pendingAnchor\.source_anchor/);
});

test('Writer review decorations, hover grace, resolution, and leader-only review action remain wired', () => {
  assert.match(writer, /Decoration\.mark/);
  assert.match(writer, /data-review-thread/);
  assert.match(writer, /setTimeout\([^,]+, 140\)/);
  assert.match(writer, /reviewState\(model\.paper\.id, thread\.id, 'RESOLVED'\)/);
  assert.match(writer, /popover\.addEventListener\('mouseenter'/);
  assert.match(writer, /ui\.sendReview\.hidden = !teamLeader/);
  assert.match(writerHtml, /id="sendReview"[^>]*hidden/);
});

test('Writer exposes data-driven Math and dedicated high-flexibility builders', () => {
  assert.match(writer, /MATH_CATALOG\.map/);
  assert.match(writer, /writer-math-palette/);
  for (const id of ['mathPalette', 'tableBuilder', 'figureBuilder', 'plotBuilder']) assert.match(writerHtml, new RegExp(`id="${id}"`));
});
