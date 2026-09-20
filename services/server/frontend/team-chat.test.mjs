import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const chat = readFileSync(new URL('../static/team-chat.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/team_chat.html', import.meta.url), 'utf8');

test('Team Chat uses scoped endpoints, text rendering, and lightweight polling', () => {
  assert.match(chat, /\/api\/v2\/team-chats/);
  assert.match(chat, /textContent = message\.body/);
  assert.match(chat, /setInterval\(loadMessages, 4000\)/);
  assert.match(html, /Message your team/);
});
