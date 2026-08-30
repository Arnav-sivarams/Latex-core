import assert from 'node:assert/strict';
import test from 'node:test';
import * as Y from 'yjs';

function sync(from, to) {
  Y.applyUpdate(to, Y.encodeStateAsUpdate(from, Y.encodeStateVector(to)), 'remote');
}

test('writer undo preserves remote edits and redo converges', () => {
  const seed = new Y.Doc();
  seed.getText('source').insert(0, 'abc');

  const writerA = new Y.Doc();
  const writerB = new Y.Doc();
  sync(seed, writerA);
  sync(seed, writerB);

  const textA = writerA.getText('source');
  const textB = writerB.getText('source');
  const originA = Symbol('writer-a');
  const originB = Symbol('writer-b');
  const undoA = new Y.UndoManager(textA, { trackedOrigins: new Set([originA]) });

  writerA.transact(() => textA.insert(textA.length, 'A'), originA);
  sync(writerA, writerB);
  writerB.transact(() => textB.insert(textB.length, 'B'), originB);
  sync(writerB, writerA);

  assert.match(textA.toString(), /A/);
  assert.match(textA.toString(), /B/);
  undoA.undo();
  assert.doesNotMatch(textA.toString(), /A/);
  assert.match(textA.toString(), /B/);
  sync(writerA, writerB);
  assert.equal(textA.toString(), textB.toString());

  undoA.redo();
  assert.match(textA.toString(), /A/);
  assert.match(textA.toString(), /B/);
  sync(writerA, writerB);
  assert.equal(textA.toString(), textB.toString());
});
