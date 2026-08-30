import assert from 'node:assert/strict';
import test from 'node:test';
import { AUTO_BUILD_IDLE_MS, createIdleBuildScheduler } from './auto-build.mjs';

function clock() {
  let now = 0;
  let next = 0;
  const timers = new Map();
  return {
    setTimer(fn, delay) { const id = ++next; timers.set(id, { at: now + delay, fn }); return id; },
    clearTimer(id) { timers.delete(id); },
    async advance(ms) {
      now += ms;
      const due = [...timers].filter(([, timer]) => timer.at <= now);
      for (const [id, timer] of due) { timers.delete(id); await timer.fn(); }
    },
  };
}

test('durable local or remote state waits two idle seconds and requests auto build', async () => {
  const fake = clock();
  const requests = [];
  const scheduler = createIdleBuildScheduler({
    requestBuild: async (trigger) => requests.push(trigger),
    setTimer: fake.setTimer,
    clearTimer: fake.clearTimer,
  });
  scheduler.durableUpdate();
  await fake.advance(AUTO_BUILD_IDLE_MS - 1);
  assert.deepEqual(requests, []);
  await fake.advance(1);
  assert.deepEqual(requests, ['auto']);
  scheduler.durableUpdate(); // the same entry point is used for REMOTE_DURABLE
  await fake.advance(AUTO_BUILD_IDLE_MS);
  assert.deepEqual(requests, ['auto', 'auto']);
});

test('newer durable state resets debounce and manual cancellation prevents duplicate auto build', async () => {
  const fake = clock();
  const requests = [];
  const scheduler = createIdleBuildScheduler({
    requestBuild: async (trigger) => requests.push(trigger),
    setTimer: fake.setTimer,
    clearTimer: fake.clearTimer,
  });
  scheduler.durableUpdate();
  await fake.advance(1500);
  scheduler.durableUpdate();
  await fake.advance(1999);
  assert.deepEqual(requests, []);
  scheduler.cancel();
  requests.push('manual');
  await fake.advance(1);
  assert.deepEqual(requests, ['manual']);
});
