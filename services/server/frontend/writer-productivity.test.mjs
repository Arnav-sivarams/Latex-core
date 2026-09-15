import assert from 'node:assert/strict';
import test from 'node:test';
import { buildAlgorithm, buildBibtexEntry, buildCodeListing, buildEquation, buildFigure, buildLongTable, buildOutlineTree, buildPlot, buildPublicationBibitems, buildTable, buildTheorem, commentLatexLines, compilationRelativePath, fuzzyRankFiles, inlineMathInsertion, insertionDirectories, isInsideInlineMath, latexDimension, packageRequirement, suggestedInsertionPath } from './writer-productivity.mjs';

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
  const table = buildTable({ rows: 2, columns: 2, header: true, booktabs: true, caption: 'Results', label: 'tab:results', columnWidths: ['3cm', ''], minimumRowHeight: '8mm' });
  assert.match(table, /\\begin\{tabular\}\{p\{3cm\}l\}/);
  assert.match(table, /\\toprule/);
  assert.ok(table.indexOf('\\caption{Results}') < table.indexOf('\\begin{tabular}'));
  assert.match(table, /\\rule\{0pt\}\{8mm\}Header 1/);
  assert.match(buildFigure({ asset: 'images/result.png', width: '0.5\\linewidth', caption: 'Result' }), /\\includegraphics\[width=0.5\\linewidth\]\{images\/result.png\}/);
});

test('table dimensions are explicit and bounded', () => {
  assert.equal(latexDimension('2.5 cm'), '2.5cm');
  assert.throws(() => latexDimension('-1cm'), /positive number/);
  assert.throws(() => buildTable({ columnWidths: ['2\\linewidth'] }), /positive number/);
  const selected = buildTable({ rows: 3, columns: 2, selectedCell: '2,1', selectedCellContent: 'First line\nSecond & line', selectedColumnWidth: '4cm', selectedRowHeight: '11mm' });
  assert.match(selected, /\\begin\{tabular\}\{p\{4cm\}l\}/);
  assert.match(selected, /\\rule\{0pt\}\{11mm\}\\shortstack/);
  assert.match(selected, /\\shortstack\{First line\\\\Second \\& line\}/);
  assert.doesNotMatch(selected.split('Cell 1.1')[0], /11mm/);
  assert.throws(() => buildTable({ rows: 2, columns: 2, selectedCell: '3,1', selectedColumnWidth: '2cm' }), /within the 2 by 2 table/);
  assert.throws(() => buildTable({ selectedColumnWidth: '2cm' }), /Choose a selected cell/);
});

test('source line comments, inline math, and template-aware destinations are deterministic', () => {
  assert.equal(commentLatexLines('  alpha\n\nbeta'), '  % alpha\n% \n% beta');
  assert.equal(commentLatexLines('  % alpha\n% \n% beta', true), '  alpha\n\nbeta');
  assert.deepEqual(inlineMathInsertion('x+y'), { source: '\\(x+y\\)', cursorOffset: null });
  assert.equal(isInsideInlineMath('before \\(x+y', 12), true);
  assert.equal(isInsideInlineMath('before $x', 9), true);
  const files = [{ path: 'Thesis/main.tex' }, { path: 'Thesis/chapters/chapter1.tex' }, { path: 'Thesis/images/logo.png' }];
  assert.deepEqual(insertionDirectories(files, 'Thesis/main.tex', 'chapter'), ['Thesis/chapters']);
  assert.equal(suggestedInsertionPath(files, 'Thesis/main.tex', 'chapter', 'chapter9.tex').path, 'Thesis/chapters/chapter9.tex');
  assert.equal(suggestedInsertionPath(files, 'Thesis/main.tex', 'asset', 'plot.png').path, 'Thesis/images/plot.png');
  assert.equal(compilationRelativePath('Thesis/images/plot.png', 'Thesis/main.tex'), 'images/plot.png');
  assert.equal(compilationRelativePath('assets/plot.png', 'Thesis/main.tex'), '../assets/plot.png');
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

test('categorized publication bibitems preserve all three statuses', () => {
  const source = buildPublicationBibitems([
    { citation_key: 'a', authors: 'A', title: 'Draft', venue: 'V', year: 2026, status: 'communicated' },
    { citation_key: 'b', authors: 'B', title: 'Accepted', venue: 'V', year: 2026, status: 'accepted' },
    { citation_key: 'c', authors: 'C', title: 'Published', venue: 'V', year: 2026, status: 'published', doi: '10.1/x' },
  ]);
  assert.match(source, /\\item\[\]\\textbf\{Communicated\}[\s\S]*\\bibitem\{a\}/);
  assert.match(source, /\\textbf\{Accepted\}[\s\S]*\\bibitem\{b\}/);
  assert.match(source, /\\textbf\{Published\}[\s\S]*\\bibitem\{c\}/);
  assert.throws(() => buildPublicationBibitems([{ citation_key: 'a', status: 'published' }, { citation_key: 'a', status: 'accepted' }]), /unique/);
  assert.throws(() => buildPublicationBibitems([
    { citation_key: 'existing', authors: 'A', title: 'T', venue: 'V', year: 2026, status: 'published' },
  ], new Set(['existing'])), /unique across this report/);
});

test('package awareness is explicit', () => {
  assert.equal(packageRequirement(['graphicx'], 'graphicx').available, true);
  assert.equal(packageRequirement([], 'pgfplots').message, 'Requires package: pgfplots');
});
