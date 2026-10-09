import assert from 'node:assert/strict';
import test from 'node:test';
import { frontMatterStatus, editableDetail, needsFirstUseDetails } from './document-details.mjs';

test('first use only prompts a Leader with missing input', () => {
  const detail = { pack_id: 'pack', can_edit: true, missing_required_fields: ['Course code'], project_metadata: { setup_complete: true } };
  assert.equal(needsFirstUseDetails(detail), true);
  assert.equal(needsFirstUseDetails({ ...detail, can_edit: false }), false);
  assert.equal(needsFirstUseDetails({ ...detail, missing_required_fields: [] }), false);
  assert.equal(needsFirstUseDetails({ ...detail, pack_id: null, project_metadata: { setup_complete: false } }), true);
  assert.equal(needsFirstUseDetails({ ...detail, pack_id: null, single_source: true }), true);
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

test('Complete Report exposes only three editable fields, never registration overrides', () => {
  for (const source of ['course_code', 'team.semester', 'team.academic_year', 'department_name', 'guide.name']) {
    assert.equal(editableDetail({ single_source: true, can_edit: true }, { source, editable: true, allow_team_override: true }), false);
  }
  for (const source of ['course_name', 'project.title', 'submission_date']) {
    assert.equal(editableDetail({ single_source: true, can_edit: true }, { source, allow_team_override: true }), true);
    assert.equal(editableDetail({ single_source: true, can_edit: false }, { source, allow_team_override: true }), false);
  }
});

test('details group document inputs separately from institutional and team fields', async () => {
  const { detailCategory } = await import('./document-details.mjs');
  for (const source of ['course_code', 'course_name', 'project.title', 'team.semester', 'team.academic_year']) assert.equal(detailCategory({ source }), 'Document details');
  assert.equal(detailCategory({ source: 'student.a.name' }), 'Team members');
  assert.equal(detailCategory({ source: 'department_name' }), 'Institutional details');
  assert.equal(detailCategory({ source: 'guide.name' }), 'Institutional details');
});
