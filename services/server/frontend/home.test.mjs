import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const home = readFileSync(new URL('../static/home.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');
const server = readFileSync(new URL('../src/main.rs', import.meta.url), 'utf8');

test('home uses role-scoped APIs and exact Writer/Review routes', () => {
  assert.match(home, /\/api\/v2\/writer\/papers/);
  assert.match(home, /\/api\/v2\/mentor\/papers/);
  assert.match(home, /paper\.id.*artifacts\/pdf\?build=/s);
  assert.match(home, /isMentor \? `\/review\?paper=\$\{paper\.id\}` : `\/write\?paper=\$\{paper\.id\}`/);
  assert.match(home, /Open Review/);
  assert.match(home, /assigned report/);
});

test('projects home keeps a placeholder when there is no successful PDF', () => {
  assert.match(home, /No successful PDF yet/);
  assert.match(home, /!build.*canvas\.remove/s);
  assert.doesNotMatch(html, /Coming next/);
});

test('Writer hides Team Chat presentation while Comments and the chat subsystem remain intact', () => {
  assert.match(html, /{{TEAM_CHAT_HEADER_LINK}}/);
  assert.match(html, /{{TEAM_CHAT_NAV_LINK}}/);
  assert.match(server, /GlobalRole::Writer[\s\S]*?"\/write",\s*"",\s*""/);
  assert.match(server, /GlobalRole::Mentor[\s\S]*?href=\\"\/team-chat\\"/);
  assert.match(server, /\.route\("\/team-chat", get\(team_chat_ui\)\)/);
  assert.match(server, /\.route\("\/api\/v2\/team-chats", get\(v2_team_chats\)\)/);
  assert.match(server, /\.route\([\s\S]*?\/api\/v2\/team-chats\/\{team_id\}\/messages/);
});

test('server home shell has distinct Mentor and Writer landing copy', () => {
  assert.match(html, /{{HOME_TITLE}}/);
  assert.match(html, /{{HOME_CTA_HREF}}/);
  assert.match(html, /{{HOME_CTA}}/);
  assert.match(home, /isMentor \? '\/review' : '\/write'/);
  assert.match(home, /isMentor \? api\('\/api\/v2\/mentor\/papers'\) : api\('\/api\/v2\/writer\/papers'\)/);
});
