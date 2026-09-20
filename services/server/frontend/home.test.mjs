import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const home = readFileSync(new URL('../static/home.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');

test('home uses role-scoped APIs and exact Writer/Review routes', () => {
  assert.match(home, /\/api\/v2\/writer\/papers/);
  assert.match(home, /\/api\/v2\/mentor\/papers/);
  assert.match(home, /paper\.id.*artifacts\/pdf\?build=/s);
  assert.match(home, /isMentor \? `\/review\?paper=\$\{paper\.id\}` : `\/write\?paper=\$\{paper\.id\}`/);
  assert.match(home, /Open Review/);
  assert.match(home, /assigned report/);
});

test('projects home keeps a placeholder when there is no successful PDF and links to Team Chat', () => {
  assert.match(home, /No successful PDF yet/);
  assert.match(home, /!build.*canvas\.remove/s);
  assert.match(html, /href="\/team-chat"/);
  assert.doesNotMatch(html, /Coming next/);
});

test('server home shell has distinct Mentor and Writer landing copy', () => {
  assert.match(html, /{{HOME_TITLE}}/);
  assert.match(html, /{{HOME_CTA_HREF}}/);
  assert.match(html, /{{HOME_CTA}}/);
  assert.match(home, /isMentor \? '\/review' : '\/write'/);
  assert.match(home, /isMentor \? api\('\/api\/v2\/mentor\/papers'\) : api\('\/api\/v2\/writer\/papers'\)/);
});
