import assert from 'node:assert/strict';
import test from 'node:test';
import { clampPaneWidths } from './workspace-split.mjs';

test('workspace splitters keep all three panes usable', () => {
  assert.deepEqual(clampPaneWidths(1200, 240, 500), { files: 240, source: 500 });
  assert.deepEqual(clampPaneWidths(900, 20, 900), { files: 150, source: 470 });
  assert.deepEqual(clampPaneWidths(700, 600, 10), { files: 150, source: 280 });
});
