import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { canSetMain, diagnosticIcon } from './writer-policy.mjs';
import { buildFigure, compilationRelativePath } from './writer-productivity.mjs';
const writer = readFileSync(new URL('./writer.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');

test('Writer main is fixed for Complete Report and generic TeX', () => {
  const main = 'Full_Report_template_v1.0/Full_Report_v1.0.tex';
  for (const path of [main, 'chapters/body.tex', 'images/demo.png', 'report.pdf', 'data.csv', 'references.bib']) {
    assert.equal(canSetMain({ file_id: 'id', path }, { main_file: main, main_file_fixed: true }), false);
  }
  assert.equal(canSetMain({ path: 'chapters' }, {}), false);
  assert.equal(canSetMain({ file_id: 'id', path: 'chapter.tex' }, { main_file: 'main.tex' }), false);
  assert.equal(canSetMain({ file_id: 'id', path: 'image.png' }, {}), false);
  assert.equal(canSetMain({ file_id: 'id', path: 'chapter.tex' }, {}, false), false);
  assert.match(writer, /textContent: 'Main'/);
});

test('Insert has compact categories and preserves existing actions and toolbar handlers', () => {
  const menu = writer.slice(writer.indexOf('function openInsertMenu'), writer.indexOf('async function openPublications'));
  for (const category of ['Structure', 'Math', 'Media', 'Data / visualization', 'Code / formal content', 'References']) assert.ok(menu.includes(`category: '${category}'`));
  for (const action of ['Table', 'Long table', 'Inline math', 'Figure', 'Plot', 'Code block', 'Algorithm', 'Theorem', 'Citation', 'Reference']) assert.ok(menu.includes(`label: '${action}'`));
  for (const id of ['insertMenu', 'figureBuilder', 'tableBuilder', 'plotBuilder']) {
    assert.ok(html.includes(`id="${id}"`));
    assert.ok(writer.includes(`ui.${id}.addEventListener('click'`));
  }
  assert.ok(diagnosticIcon('error').includes('<circle'));
  assert.ok(diagnosticIcon('warning').includes('M12 3 2 21h20Z'));
  assert.match(html, /id="problemsToggle"[^>]*aria-label="Problems"[^>]*>[\s\S]*?<svg aria-hidden="true"/);
});

test('Figure generates root-relative paths, escaped captions and valid dimensions', () => {
  const asset = compilationRelativePath('Full_Report_template_v1.0/images/demo-figure.png', 'Full_Report_template_v1.0/Full_Report_v1.0.tex');
  const source = buildFigure({ asset, width: '0.55\\linewidth', caption: 'Campus monitoring dashboard', label: 'fig:campus-dashboard' });
  assert.equal(source, '\\begin{figure}[htbp]\n\\centering\n\\includegraphics[width=0.55\\linewidth]{images/demo-figure.png}\n\\caption{Campus monitoring dashboard}\n\\label{fig:campus-dashboard}\n\\end{figure}');
  assert.match(buildFigure({ asset, caption: 'Cost $5_a & 50% ~ ^' }), /Cost \\\$5\\_a \\& 50\\% \\textasciitilde\{\} \\textasciicircum\{\}/);
  for (const width of ['0.55linewidth', '\\\\linewidth', '-2cm', 'oops']) assert.throws(() => buildFigure({ asset, width }));
  for (const asset of ['https://example.com/a.png', '/tmp/a.png', 'images/a.txt']) assert.throws(() => buildFigure({ asset }));
});


test('acknowledgement and certificate separate long student names from intact registration numbers', () => {
  for (const page of ['acknowledgement.tex', 'certificate.tex']) {
    const source = readFileSync(new URL(`../../../artifacts/Full_Report_template_v1.1-source/Full_Report_template_v1.0/${page}`, import.meta.url), 'utf8');
    const active = source.split('\n').filter((line) => !line.trimStart().startsWith('%')).join('\n');
    assert.ok(active.includes(String.raw`\studentAname\ (\studentAregno)`));
    assert.ok(active.includes(String.raw`\studentBname\ (\studentBregno)`));
    const expanded = active.replaceAll(String.raw`\studentAname`, 'Arnav Sivaram').replaceAll(String.raw`\studentAregno`, '22BCE2308').replaceAll(String.raw`\studentBname`, 'Meera Krishnan').replaceAll(String.raw`\studentBregno`, '22BCE2309');
    assert.ok(expanded.includes(String.raw`Arnav Sivaram\ (22BCE2308)`));
    assert.ok(expanded.includes(String.raw`Meera Krishnan\ (22BCE2309)`));
    assert.ok(!expanded.includes('22 BCE'));
    assert.ok(!active.includes(String.raw`\hspace{1cm}`));
  }
});
