import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const start = source.indexOf('  preserveOldEpoch(');
const method = source.slice(start, source.indexOf('\n  reloadPaperAfterControl()', start));

function fixture(metadataSaving) {
  const model = { metadataSaving, recoveryText: null, conflict: false };
  let destroyed = 0, reloaded = 0;
  const session = runInNewContext(`({${method}})`, { model, saveState() {}, notice() {} });
  Object.assign(session, {
    text: { toString: () => 'old generation source' },
    destroy: () => destroyed++,
    reloadPaperAfterControl: () => reloaded++,
  });
  model.collaboration = session;
  return { model, session, counts: () => ({ destroyed, reloaded }) };
}

test('a retained file rejoining after a metadata save recovers from its old epoch', () => {
  const f = fixture(true);
  f.session.preserveOldEpoch(2, true);
  assert.deepEqual(f.counts(), { destroyed: 1, reloaded: 1 });
  assert.equal(f.model.recoveryText, 'old generation source');
  assert.equal(f.model.conflict, true);
  const handshake = source.slice(source.indexOf("control.type === 'JOIN_ACCEPTED'"), source.indexOf("control.type === 'DURABLE_ACK'"));
  assert.match(handshake, /preserveOldEpoch\(control\.document_epoch, true\)/);
});

test('metadata epoch notifications avoid competing reloads; ordinary restoration still reloads', () => {
  for (const metadataSaving of [true, false]) {
    const f = fixture(metadataSaving);
    f.session.preserveOldEpoch(2);
    assert.deepEqual(f.counts(), { destroyed: 1, reloaded: metadataSaving ? 0 : 1 });
    assert.equal(f.model.recoveryText, 'old generation source');
  }
});

test('inactive sessions cannot replace the current editor or its recovery state', () => {
  const f = fixture(true);
  f.model.collaboration = {};
  f.session.preserveOldEpoch(2, true);
  assert.deepEqual(f.counts(), { destroyed: 0, reloaded: 0 });
  assert.equal(f.model.recoveryText, null);
});
