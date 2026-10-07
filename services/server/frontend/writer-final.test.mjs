import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { recognizedBuildProblems, problemsState, supportLink } from './writer-build.mjs';

test('explicit nested compiler locations open the exact project file and line', () => {
  const files = [{ file_id: 'nested', path: 'Full_Report_template_v1.0/chapters/chapter2.tex' }];
  const [problem] = recognizedBuildProblems('./chapters/chapter2.tex:37: Undefined control sequence.', files, 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
  assert.equal(problem.file_id, 'nested');
  assert.equal(problem.path, files[0].path);
  assert.equal(problem.range.start_line, 37);
  assert.equal(problem.severity, 'error');
  assert.equal(recognizedBuildProblems('chapters/chapter2.tex:37: LaTeX Warning: Citation undefined.')[0].severity, 'warning');
  assert.equal(recognizedBuildProblems('chapters/chapter2.tex:37: Overfull \\hbox')[0].severity, 'warning');
  assert.equal(recognizedBuildProblems('! Emergency stop.')[0].range, undefined);
  assert.equal(recognizedBuildProblems('../outside.tex:37: Error', files)[0].file_id, undefined);
});

test('Problems state gives errors precedence and accessible counts', () => {
  assert.deepEqual(problemsState([]), { severity: 'neutral', label: 'Problems: 0 errors, 0 warnings' });
  assert.deepEqual(problemsState([{ severity: 'warning' }]), { severity: 'warning', label: 'Problems: 0 errors, 1 warning' });
  assert.deepEqual(problemsState([{ severity: 'warning' }, { severity: 'error' }]), { severity: 'error', label: 'Problems: 1 error, 1 warning' });
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.match(writer, /problemsToggle\.dataset\.severity = state\.severity/);
  assert.match(writer, /setAttribute\('aria-label', state\.label\)/);
  const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
  assert.match(css, /#problemsToggle\[data-severity="warning"\] \{ color: var\(--warning\)/);
  assert.match(css, /#problemsToggle\[data-severity="error"\] \{ color: var\(--danger\)/);
});

test('Help uses institutional configuration and safe unconfigured state', () => {
  assert.equal(supportLink('help@institution.example'), 'mailto:help@institution.example?subject=LaTeX%20Core%20Support');
  for (const value of [undefined, null, '', 'null', 'bad']) assert.equal(supportLink(value), null);
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.match(writer, /api\.request\('\/api\/v2\/support'\)/);
  assert.match(writer, /help\.disabled = !href/);
  assert.match(writer, /window\.location\.href = href/);
});

test('Writer omits the recovery-copy control while retaining diagnostic navigation and recovery storage', () => {
  const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
  const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
  assert.doesNotMatch(html, /Copy recovery text|copyRecoveryText/);
  assert.doesNotMatch(writer, /copyRecoveryText/);
  assert.match(html, /id="buildProblemsList"/);
  assert.match(html, /id="buildLogTab"/);
  assert.match(writer, /openLocation\(diagnostic\)/);
  assert.match(writer, /model\.recoveryText = recovered/);
  assert.match(writer, /IndexeddbPersistence/);
});
