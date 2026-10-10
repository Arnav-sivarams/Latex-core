import assert from 'node:assert/strict';
import test from 'node:test';
import { buildIsStale, compileIsActive, buildLogLines, classifyBuildLine, recognizedBuildProblems, shortBuildState } from './writer-build.mjs';

test('build lines use conservative error and warning classification', () => {
  assert.equal(classifyBuildLine('! Undefined control sequence.'), 'error');
  assert.equal(classifyBuildLine('! LaTeX Error: broken'), 'error');
  assert.equal(classifyBuildLine('! Emergency stop.'), 'error');
  assert.equal(classifyBuildLine('LaTeX Warning: Citation undefined.'), 'warning');
  assert.equal(classifyBuildLine("Package acro Warning: Unknown option `sort'"), 'warning');
  assert.equal(classifyBuildLine("Package natbib Warning: Citation `x' undefined"), 'warning');
  assert.equal(classifyBuildLine('Overfull \\hbox (1.0pt too wide)'), 'warning');
  assert.equal(classifyBuildLine('Underfull \\hbox (badness 10000)'), 'warning');
  assert.equal(classifyBuildLine('Fatal error'), 'error');
  assert.equal(classifyBuildLine('(natbib) Rerun to get citations correct.'), 'warning');
  assert.equal(classifyBuildLine('file:line:error style messages enabled.'), 'info');
  assert.equal(classifyBuildLine('(/usr/share/texlive/texmf-dist/tex/latex/base/article.cls)'), 'info');
  assert.deepEqual(recognizedBuildProblems('ok\n! LaTeX Error: broken\nUnderfull \\hbox'), [
    { severity: 'error', message: '! LaTeX Error: broken' },
    { severity: 'warning', message: 'Underfull \\hbox' },
  ]);
});

test('raw build log marks only classified warning and error lines', () => {
  assert.deepEqual(buildLogLines([
    'normal compiler output',
    "Package acro Warning: Unknown option `sort'",
    'Overfull \\hbox (1.0pt too wide)',
    '! LaTeX Error: broken',
    'Fatal error',
    'file:line:error style messages enabled.',
  ].join('\n')), [
    { text: 'normal compiler output', problem: false },
    { text: "Package acro Warning: Unknown option `sort'", problem: true },
    { text: 'Overfull \\hbox (1.0pt too wide)', problem: true },
    { text: '! LaTeX Error: broken', problem: true },
    { text: 'Fatal error', problem: true },
    { text: 'file:line:error style messages enabled.', problem: false },
  ]);
});

test('successful build state is independent of warning-bearing build log', () => {
  const warningLog = [
    'file:line:error style messages enabled.',
    "Package acro Warning: Unknown option `sort'",
    "Package natbib Warning: Citation `x' undefined",
    'Overfull \\hbox',
    'Underfull \\hbox',
    'LaTeX Warning: Label(s) may have changed.',
  ].join('\n');
  const problems = recognizedBuildProblems(warningLog);
  assert.equal(shortBuildState({ latest_status: 'succeeded', current_build_id: 'good' }), 'Compiled');
  assert.equal(problems.some((problem) => problem.severity === 'error'), false);
  assert.equal(problems.filter((problem) => problem.severity === 'warning').length, 5);
});

test('short build state does not contain compiler output', () => {
  assert.equal(shortBuildState(null), 'Ready');
  assert.equal(shortBuildState({ active_build_id: 'queued', active_status: 'queued' }), 'Queued');
  assert.equal(shortBuildState({ active_build_id: 'running' }), 'Compiling…');
  assert.equal(shortBuildState({ latest_status: 'failed' }), 'Compilation failed');
  assert.equal(shortBuildState({ current_build_id: 'good' }), 'Compiled');
});

test('metadata-only changes make the last successful PDF stale', () => {
  const current = {
    current_build_id: 'good', source_sequence: 7, current_source_sequence: 7,
    desired_state_hash: 'compiled-with-metadata-1', current_state_hash: 'compiled-with-metadata-1',
  };
  assert.equal(buildIsStale(current), false);
  assert.equal(buildIsStale({ ...current, desired_state_hash: null }), true);
  assert.equal(buildIsStale({ ...current, desired_state_hash: 'compiled-with-metadata-2' }), true);
  assert.equal(buildIsStale({ ...current, source_sequence: 8 }), true);
});

test('terminal jobs stop compiling even while their active build reference remains', () => {
  for (const status of ['queued','claimed','running']) assert.equal(compileIsActive({active_build_id:'job',active_status:status}),true);
  for (const status of ['cancelled','failed','succeeded']) assert.equal(compileIsActive({active_build_id:'job',active_status:status}),false);
  assert.equal(compileIsActive({active_build_id:null}),false);
  assert.equal(shortBuildState({active_build_id:'job',active_status:'cancelled',current_build_id:'last-good'}),'Compilation cancelled');
});
