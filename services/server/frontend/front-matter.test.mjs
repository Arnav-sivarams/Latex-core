import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const adminHtml = readFileSync(new URL('../src/admin.html', import.meta.url), 'utf8');
const admin = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
const writerHtml = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const mentorHtml = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');
const mentor = readFileSync(new URL('./review.js', import.meta.url), 'utf8');

test('Front Matter shares the single Templates navigation destination', () => {
  assert.equal((adminHtml.match(/data-section="Templates"/g) || []).length, 1);
  for (const label of ['Main templates', 'Front Matter', 'Automatic defaults']) {
    assert.match(admin, new RegExp(label));
  }
  assert.match(admin, /Programme<\/th><th>Main Content Template<\/th><th>Front Matter Pack/);
  assert.match(admin, /front-matter-packs\/preview/);
  assert.match(admin, /front_matter_compatible/);
});

test('Document details is a temporary leader-governed Writer drawer', () => {
  assert.match(writerHtml, /id="documentDetails"[^>]*aria-label="Document details"[^>]*title="Document details"/);
  assert.match(writerHtml, /id="documentDetailsPanel"[^>]*hidden/);
  assert.match(writer, /detail\.can_edit/);
  assert.match(writer, /checkbox\.disabled = section\.required \|\| !detail\.can_edit/);
  assert.match(writer, /Save document details/);
  assert.match(writer, /section\.required \|\| input\.checked/);
});

test('Mentor workspace exposes no Front Matter metadata editor', () => {
  assert.doesNotMatch(mentorHtml, /Document details|front.?matter/i);
  assert.doesNotMatch(mentor, /documentDetails|frontMatter/i);
});
