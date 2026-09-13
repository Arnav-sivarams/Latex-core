import assert from 'node:assert/strict';
import test from 'node:test';
import { frontMatterStatus, editableDetail, needsFirstUseDetails } from './document-details.mjs';

test('first use only prompts a Leader with missing input', () => {
  const detail = { pack_id: 'pack', can_edit: true, missing_required_fields: ['Course code'] };
  assert.equal(needsFirstUseDetails(detail), true);
  assert.equal(needsFirstUseDetails({ ...detail, can_edit: false }), false);
  assert.equal(needsFirstUseDetails({ ...detail, missing_required_fields: [] }), false);
  assert.equal(frontMatterStatus(detail), 'Front Matter needs 1 details');
});

test('AUTO values and nonleaders are read-only; manual fields and modern packs remain supported', () => {
  assert.equal(editableDetail({ can_edit: true }, { editable: false, allow_team_override: true }), false);
  assert.equal(editableDetail({ can_edit: false }, { editable: true }), false);
  assert.equal(editableDetail({ can_edit: true }, { editable: true }), true);
  assert.equal(editableDetail({ can_edit: true }, { source: 'team.name', allow_team_override: true }), true);
  assert.equal(frontMatterStatus({ status: 'READY', warnings: ['missing section'] }), 'Front Matter has 1 template warnings');
  assert.equal(frontMatterStatus({ status: 'READY' }), 'Front Matter ready');
});
