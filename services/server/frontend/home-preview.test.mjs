import assert from 'node:assert/strict';
import test from 'node:test';
import { successfulPreviewBuild } from './home-preview.mjs';

test('successful PDF is the first preview build for Writer projects', () => {
  assert.equal(
    successfulPreviewBuild('writer', { id: 'paper' }, { build: { current_build_id: 'good' } }),
    'good',
  );
});

test('a running build preserves the previous successful Writer preview', () => {
  assert.equal(
    successfulPreviewBuild('writer', { id: 'paper' }, {
      build: { current_build_id: 'last-good', active_build_id: 'running', latest_status: 'running' },
    }),
    'last-good',
  );
});

test('a failed build preserves the previous successful Writer preview', () => {
  assert.equal(
    successfulPreviewBuild('writer', { id: 'paper' }, {
      build: { current_build_id: 'last-good', latest_status: 'failed' },
    }),
    'last-good',
  );
});

test('never-built projects use the placeholder', () => {
  assert.equal(successfulPreviewBuild('writer', { id: 'paper' }, { build: {} }), null);
});

test('Mentor list payload requires an available successful PDF', () => {
  assert.equal(successfulPreviewBuild('mentor', { current_build_id: 'good', pdf_available: true }), 'good');
  assert.equal(successfulPreviewBuild('mentor', { current_build_id: 'last-good', pdf_available: false }), null);
});
