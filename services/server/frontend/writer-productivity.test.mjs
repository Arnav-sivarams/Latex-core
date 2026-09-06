import assert from 'node:assert/strict';
import test from 'node:test';
import { buildAlgorithm, buildBibtexEntry, buildCodeListing, buildEquation, buildFigure, buildLongTable, buildOutlineTree, buildPlot, buildTable, buildTheorem, fuzzyRankFiles, packageRequirement } from './writer-productivity.mjs';

test('outline hierarchy follows section levels', () => {
  const tree = buildOutlineTree([{ level: 'section', title: 'A' }, { level: 'subsection', title: 'B' }, { level: 'section', title: 'C' }]);
  assert.equal(tree.length, 2);
  assert.equal(tree[0].children[0].title, 'B');
});

test('quick open favors exact basename and supports subsequences', () => {
  const files = [{ path: 'sections/methods.tex' }, { path: 'main.tex' }, { path: 'notes.tex' }];
  assert.equal(fuzzyRankFiles(files, 'main.tex')[0].path, 'main.tex');
  assert.equal(fuzzyRankFiles(files, 'mthd')[0].path, 'sections/methods.tex');
});

test('table and figure builders generate bounded ordinary LaTeX', () => {
  const table = buildTable({ rows: 2, columns: 2, header: true, booktabs: true, caption: 'Results', label: 'tab:results' });
  assert.match(table, /\\begin\{tabular\}\{ll\}/);
  assert.match(table, /\\toprule/);
  assert.match(buildFigure({ asset: 'images/result.png', width: '0.5\\linewidth', caption: 'Result' }), /\\includegraphics\[width=0.5\\linewidth\]\{images\/result.png\}/);
});

test('equation builder covers matrix and cases', () => {
  assert.match(buildEquation({ type: 'matrix', rows: 2, columns: 3, delimiter: '[]' }), /\\begin\{bmatrix\}/);
  assert.match(buildEquation({ type: 'cases', rows: 2 }), /\\begin\{cases\}/);
});

test('plot, BibTeX, algorithm, listing, and theorem output stays editable', () => {
  assert.match(buildPlot({ asset: 'data.csv', x: 'time', y: 'value', type: 'scatter' }), /only marks/);
  assert.match(buildBibtexEntry({ type: 'article', key: 'doe2026', title: 'Paper', author: 'Doe', year: '2026' }), /@article\{doe2026/);
  assert.match(buildAlgorithm({ caption: 'Method' }), /\\begin\{algorithmic\}/);
  assert.match(buildAlgorithm({ family: 'algpseudocode' }), /\\State Describe/);
  assert.match(buildAlgorithm({ family: 'algorithmic' }), /\\STATE Describe/);
  const longtable = buildLongTable({ rows: 40, columns: 2, header: true, caption: 'Results' });
  assert.match(longtable, /^\\begin\{longtable\}/);
  assert.match(longtable, /\\endfirsthead/);
  assert.match(longtable, /Cell 40\.2/);
  assert.doesNotMatch(longtable, /\\begin\{table\}|\\begin\{minipage\}|\\resizebox/);
  assert.match(buildCodeListing({ language: 'Rust', code: 'fn main() {}' }), /lstlisting/);
  assert.match(buildTheorem({ environment: 'lemma', label: 'lem:x' }), /\\begin\{lemma\}/);
});

test('package awareness is explicit', () => {
  assert.equal(packageRequirement(['graphicx'], 'graphicx').available, true);
  assert.equal(packageRequirement([], 'pgfplots').message, 'Requires package: pgfplots');
});
