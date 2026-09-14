import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const server = readFileSync(new URL('../src/main.rs', import.meta.url), 'utf8');

test('durable local and remote edits never schedule compilation', () => {
  assert.doesNotMatch(writer, /createIdleBuildScheduler|autoBuild|requestBuild\('auto'\)/);
  assert.match(writer, /DURABLE_ACK/);
  assert.match(writer, /REMOTE_DURABLE/);
});

test('Front Matter and build submission expose manual compilation only', () => {
  assert.doesNotMatch(server, /schedule_front_matter_auto_build|trigger_type: "auto"/);
  assert.match(server, /trigger_type must be manual/);
});
