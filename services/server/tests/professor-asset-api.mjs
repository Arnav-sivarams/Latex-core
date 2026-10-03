import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { execFile } from 'node:child_process';
import { copyFile, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { createCanvas } from '@napi-rs/canvas';
import * as pdfjs from '../../../node_modules/pdfjs-dist/legacy/build/pdf.mjs';
import {
  buildEquation, buildFigure, buildPlot, buildTable, buildWrapFigure,
} from '../frontend/writer-productivity.mjs';

const execute = promisify(execFile);
const base = process.env.PROFESSOR_ASSET_BASE_URL;
const adminEmail = process.env.PROFESSOR_ASSET_ADMIN_EMAIL;
const passwordFile = process.env.PROFESSOR_ASSET_ADMIN_PASSWORD_FILE;
const manifestFile = process.env.PROFESSOR_ASSET_MANIFEST;
const phase = process.env.PROFESSOR_ASSET_PHASE || 'full';
const continuation = phase === 'continuation';
if (!base || !adminEmail || !passwordFile || !manifestFile) {
  throw new Error('PROFESSOR_ASSET_BASE_URL, PROFESSOR_ASSET_ADMIN_EMAIL, PROFESSOR_ASSET_ADMIN_PASSWORD_FILE, and PROFESSOR_ASSET_MANIFEST are required');
}

const adminPassword = (await readFile(passwordFile, 'utf8')).trim();
const suffix = `${Date.now().toString(36)}-${randomBytes(3).toString('hex')}`;
const writerEmail = `prof-asset-writer-${suffix}@example.test`;
const mentorEmail = `prof-asset-mentor-${suffix}@example.test`;
const accountPassword = `A9!${randomBytes(18).toString('base64url')}`;
const temp = await mkdtemp(join(tmpdir(), 'latex-core-professor-assets-'));
const evidence = {
  schema_version: 1, suffix, admin_email: adminEmail, writer_email: writerEmail,
  mentor_email: mentorEmail, templates: [], teams: [], builds: [],
};
let adminCookie;
let writerCookie;
let writerId;
let mentorId;
let rootPaper;
let compileJobs = 0;
let successfulBuilds = 0;
let failedBuilds = 0;
let renderedPdfs = 0;

function sha256(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function normalizedPdfText(value) { return String(value).toLowerCase().replace(/[^a-z0-9]/g, ''); }
function sleep(milliseconds) { return new Promise((resolve) => setTimeout(resolve, milliseconds)); }
function cookieFrom(response) {
  const values = response.headers.getSetCookie?.() || [];
  const value = values.find((entry) => entry.startsWith('latex_core_session_v2='));
  assert.ok(value, 'login did not return a session cookie');
  return value.split(';', 1)[0];
}

async function login(email, password) {
  const response = await fetch(`${base}/api/auth/login`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }), redirect: 'manual',
  });
  assert.ok([200, 201].includes(response.status), `login failed for disposable ${email}: ${response.status} ${await response.text()}`);
  return cookieFrom(response);
}

async function request(cookie, path, options = {}) {
  const headers = new Headers(options.headers || {});
  if (cookie) headers.set('Cookie', cookie);
  if (options.json !== undefined) headers.set('Content-Type', 'application/json');
  const response = await fetch(`${base}${path}`, {
    method: options.method || 'GET', headers,
    body: options.json === undefined ? options.body : JSON.stringify(options.json),
    redirect: 'manual',
  });
  const bytes = Buffer.from(await response.arrayBuffer());
  let payload = null;
  if ((response.headers.get('content-type') || '').includes('json')) {
    try { payload = JSON.parse(bytes.toString('utf8')); } catch { payload = null; }
  }
  return { response, status: response.status, bytes, payload, text: bytes.toString('utf8') };
}

async function okJson(cookie, path, options = {}, expected = 200) {
  const result = await request(cookie, path, options);
  assert.equal(result.status, expected, `${options.method || 'GET'} ${path}: ${result.status} ${result.text.slice(0, 800)}`);
  return result.payload;
}

async function importTemplate(name, archivePath, main, arrangement) {
  const form = new FormData();
  form.set('name', name); form.set('description', `Disposable professor acceptance ${suffix}`);
  form.set('main', main); form.set('arrangement', arrangement);
  form.set('archive', new Blob([await readFile(archivePath)], { type: 'application/zip' }), 'template.zip');
  const imported = await okJson(adminCookie, '/api/admin/v2/templates/import', { method: 'POST', body: form }, 201);
  evidence.templates.push({ id: imported.id, name, main_file: imported.main_file, arrangement: imported.front_matter_arrangement });
  return imported;
}

async function createTeam(name, templateId) {
  const created = await okJson(adminCookie, '/api/admin/v2/paper-teams', {
    method: 'POST', json: {
      name, template_id: templateId, front_matter_pack_id: null, use_front_matter_default: false,
      leader_writer_id: writerId, writer_ids: [writerId], mentor_ids: [mentorId],
    },
  }, 201);
  assert.equal(created.template_pin.template_id, templateId);
  evidence.teams.push({ id: created.team.id, name, workspace_id: created.team.workspace_id, template_id: templateId });
  return created.team.id;
}

async function paperState(paperId) {
  return okJson(writerCookie, `/api/v2/papers/${paperId}`);
}

async function files(paperId) { return okJson(writerCookie, `/api/v2/papers/${paperId}/files`); }

async function fileByPath(paperId, path) {
  const file = (await files(paperId)).find((entry) => entry.path === path);
  assert.ok(file, `missing file ${path}`); return file;
}

async function readText(paperId, path) {
  const file = await fileByPath(paperId, path);
  return okJson(writerCookie, `/api/v2/papers/${paperId}/files/${file.file_id}`);
}

async function saveText(paperId, path, content) {
  const current = await readText(paperId, path);
  const saved = await okJson(writerCookie, `/api/v2/papers/${paperId}/files/${current.file.file_id}`, {
    method: 'PUT', json: { content, version: current.version },
  });
  const reloaded = await readText(paperId, path);
  assert.equal(reloaded.content, content, `durable reload differed for ${path}`);
  assert.equal(reloaded.version, saved.version);
  return saved.version;
}

async function upload(paperId, path, bytes, contentType, expected = 201) {
  const version = (await paperState(paperId)).version;
  const result = await request(writerCookie, `/api/v2/papers/${paperId}/assets?path=${encodeURIComponent(path)}&version=${version}`, {
    method: 'POST', headers: { 'Content-Type': contentType }, body: bytes,
  });
  assert.equal(result.status, expected, `upload ${path}: ${result.status} ${result.text.slice(0, 500)}`);
  if (expected !== 201) return result;
  const raw = await request(writerCookie, `/api/v2/papers/${paperId}/files/${result.payload.file.file_id}/raw`);
  assert.equal(raw.status, 200); assert.equal(sha256(raw.bytes), sha256(bytes), `${path} bytes changed in storage`);
  return result.payload.file;
}

async function parsePdf(bytes) {
  assert.equal(bytes.subarray(0, 5).toString(), '%PDF-');
  const task = pdfjs.getDocument({ data: new Uint8Array(bytes), disableWorker: true });
  const document = await task.promise; const pageTexts = []; const pageMetrics = []; let imageOperators = 0;
  for (let number = 1; number <= document.numPages; number += 1) {
    const page = await document.getPage(number);
    const text = await page.getTextContent();
    pageTexts.push(text.items.map((item) => item.str).join(' ').replace(/\s+/g, ' ').trim());
    const viewport = page.getViewport({ scale: 1 });
    pageMetrics.push({
      width: viewport.width,
      maxTextRight: Math.max(0, ...text.items.map((item) => Number(item.transform?.[4] || 0) + Number(item.width || 0))),
      textLines: new Set(text.items.map((item) => Math.round(Number(item.transform?.[5] || 0)))).size,
    });
    const operators = await page.getOperatorList();
    imageOperators += operators.fnArray.filter((operation) => [pdfjs.OPS.paintImageXObject, pdfjs.OPS.paintInlineImageXObject, pdfjs.OPS.paintImageMaskXObject].includes(operation)).length;
  }
  const result = { pages: document.numPages, pageTexts, pageMetrics, text: pageTexts.join('\n'), imageOperators };
  await task.destroy(); renderedPdfs += 1; return result;
}

async function compile(paperId, label, expected = 'success') {
  compileJobs += 1;
  const submitted = await okJson(writerCookie, `/api/v2/papers/${paperId}/builds`, { method: 'POST', json: { trigger_type: 'manual' } }, 202);
  const buildId = submitted.build_id;
  assert.ok(buildId, `${label} did not return build_id`);
  const deadline = Date.now() + 180_000; let status;
  while (Date.now() < deadline) {
    status = await okJson(writerCookie, `/api/v2/papers/${paperId}/builds`);
    const build = status.build;
    if (build.latest_build_id === buildId && !build.active_build_id && ['succeeded', 'failed'].includes(build.latest_status)) break;
    await sleep(500);
  }
  assert.equal(status?.build?.latest_build_id, buildId, `${label} did not become the latest exact build`);
  assert.equal(status.build.active_build_id, null, `${label} remained active`);
  const record = { label, build_id: buildId, state_hash: submitted.state_hash, source_sequence: submitted.source_sequence, status: status.build.latest_status };
  evidence.builds.push(record);
  if (status.build.latest_status === 'failed') {
    assert.notEqual(expected, 'success', `${label} unexpectedly failed: ${JSON.stringify(status.build.latest_error)}`);
    const log = await request(writerCookie, `/api/v2/papers/${paperId}/artifacts/log?build=${buildId}`);
    assert.equal(log.status, 200); assert.match(log.text, /(?:error|undefined|not found|Emergency stop|Fatal)/i, `${label} log was not actionable`);
    failedBuilds += 1; return { status, log: log.text, buildId };
  }
  assert.notEqual(expected, 'failed', `${label} unexpectedly succeeded`);
  assert.equal(status.build.latest_status, 'succeeded', `${label} failed: ${JSON.stringify(status.build.latest_error)}`);
  assert.equal(status.build.current_build_id, buildId, `${label} did not promote its exact PDF`);
  assert.equal(status.build.current_source_sequence, status.build.source_sequence, `${label} PDF is stale`);
  const artifact = await request(writerCookie, `/api/v2/papers/${paperId}/artifacts/pdf?build=${buildId}`);
  assert.equal(artifact.status, 200, `${label} exact PDF fetch failed`);
  successfulBuilds += 1;
  return { status, pdf: await parsePdf(artifact.bytes), bytes: artifact.bytes, buildId };
}

async function makeZip(directory, name, paths) {
  const output = join(directory, name);
  await execute('zip', ['-q', output, ...paths], { cwd: directory }); return output;
}

function replaceTitle(source, title) {
  return source.replace(/\\newcommand\{\\thesistitle\}\{[^\n]*\}/, `\\newcommand{\\thesistitle}{${title}}`);
}

try {
  adminCookie = await login(adminEmail, adminPassword);
  const writer = await okJson(adminCookie, '/api/admin/v2/users', { method: 'POST', json: { email: writerEmail, password: accountPassword, role: 'writer', generate_temporary_password: false } }, 201);
  const mentor = await okJson(adminCookie, '/api/admin/v2/users', { method: 'POST', json: { email: mentorEmail, password: accountPassword, role: 'mentor', generate_temporary_password: false } }, 201);
  writerId = writer.user_id; mentorId = mentor.user_id;
  writerCookie = await login(writerEmail, accountPassword);

  let wrongCaseDefect = null;
  if (phase !== 'professor') {
  const actualPng = await readFile(join(process.cwd(), 'artifacts/Full_Report_template_v1.1-source/Full_Report_template_v1.0/images/vit_logo.png'));
  const samplePdf = await readFile(join(process.cwd(), 'artifacts/Full_Report_template_v1.1-source/Full_Report_template_v1.0/images/sample-graph.pdf'));
  const canvas = createCanvas(96, 64); const context = canvas.getContext('2d');
  context.fillStyle = '#2255aa'; context.fillRect(0, 0, 96, 64);
  context.fillStyle = '#ffffff'; context.font = '18px sans-serif'; context.fillText('JPEG', 18, 38);
  const jpeg = canvas.toBuffer('image/jpeg', 90);

  const rootDir = join(temp, 'root'); await mkdir(join(rootDir, 'images'), { recursive: true });
  await writeFile(join(rootDir, 'main.tex'), '\\documentclass{article}\\begin{document}Initial\\end{document}\n');
  await copyFile(join(process.cwd(), 'artifacts/Full_Report_template_v1.1-source/Full_Report_template_v1.0/images/vit_logo.png'), join(rootDir, 'images', 'imported.png'));
  const rootZip = await makeZip(rootDir, 'root.zip', ['main.tex', 'images/imported.png']);
  const rootTemplate = await importTemplate(`Professor assets root ${suffix}`, rootZip, 'main.tex', 'REPORT_CONTENT_ONLY');
  rootPaper = await createTeam(`Professor assets root ${suffix}`, rootTemplate.id);

  const uploadedPng = await upload(rootPaper, 'images/manual sample.v1.png', actualPng, 'image/png');
  await upload(rootPaper, 'images/synthetic.jpg', jpeg, 'image/jpeg');
  await upload(rootPaper, 'images/synthetic.jpeg', jpeg, 'image/jpeg');
  await upload(rootPaper, 'images/SYNTHETIC.JPG', jpeg, 'image/jpeg');
  await upload(rootPaper, 'data/plot.csv', Buffer.from('x,y,z\n0,0,0\n1,1,1\n2,4,2\n'), 'text/csv');
  await upload(rootPaper, 'images/sample-graph.pdf', samplePdf, 'application/pdf');
  assert.equal((await upload(rootPaper, 'images/rejected.svg', Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'), 'image/svg+xml', 415)).status, 415);
  assert.equal((await upload(rootPaper, 'images/rejected.eps', Buffer.from('%!PS-Adobe-3.0 EPSF-3.0'), 'application/postscript', 415)).status, 415);
  assert.equal((await upload(rootPaper, 'images/mismatch.png', jpeg, 'image/png', 415)).status, 415);
  assert.equal((await upload(rootPaper, '../outside.png', actualPng, 'image/png', 400)).status, 400);

  let lastGood;
  if (!continuation) {
  const manual = String.raw`\documentclass{article}
\usepackage{graphicx,amsmath}
\begin{document}
MANUAL SOURCE CONTROL
\begin{figure}[htbp]\centering\includegraphics[width=.35\linewidth]{\detokenize{images/manual sample.v1.png}}\caption{Manual image}\end{figure}
Imported existing image: \includegraphics[width=.20\linewidth]{images/imported.png}
Manual inline \(x^2+y^2=z^2\) and display \[\int_0^1 x^2\,dx=\frac13\].
\newpage SECOND PAGE FOR PDF ASSET
\end{document}
`;
  await saveText(rootPaper, 'main.tex', manual);
  const manualBuild = await compile(rootPaper, 'manual-image-equation');
  assert.match(manualBuild.pdf.text, /MANUAL SOURCE CONTROL/); assert.ok(manualBuild.pdf.imageOperators >= 2);
  assert.equal((await readText(rootPaper, 'main.tex')).content, manual);

  const toolbar = `\\documentclass{article}\n\\usepackage{graphicx,wrapfig,amsmath}\n\\begin{document}\nTOOLBAR EQUIVALENT\n${buildFigure({ asset: 'images/manual sample.v1.png', width: '0.35\\linewidth', caption: 'Builder figure', label: 'fig:builder' })}\n${buildWrapFigure({ asset: 'images/manual sample.v1.png', side: 'r', wrapWidth: '0.35\\textwidth', width: '\\linewidth', caption: 'Builder wrap' })}\n${buildEquation({ type: 'inline', body: 'x^2+y^2=z^2' })}\n\\end{document}\n`;
  await saveText(rootPaper, 'main.tex', toolbar);
  const toolbarBuild = await compile(rootPaper, 'toolbar-equivalent-image-equation');
  assert.match(toolbarBuild.pdf.text, /TOOLBAR EQUIVALENT/); assert.ok(toolbarBuild.pdf.imageOperators >= 2);

  const nestedDir = join(temp, 'nested'); await mkdir(join(nestedDir, 'Thesis', 'chapters'), { recursive: true });
  await writeFile(join(nestedDir, 'Thesis', 'main.tex'), '\\documentclass{article}\\begin{document}\\input{chapters/chapter.tex}\\end{document}\n');
  await writeFile(join(nestedDir, 'Thesis', 'chapters', 'chapter.tex'), 'Initial chapter.\n');
  const nestedZip = await makeZip(nestedDir, 'nested.zip', ['Thesis/main.tex', 'Thesis/chapters/chapter.tex']);
  const nestedTemplate = await importTemplate(`Professor assets nested ${suffix}`, nestedZip, 'Thesis/main.tex', 'REPORT_CONTENT_ONLY');
  const nestedPaper = await createTeam(`Professor assets nested ${suffix}`, nestedTemplate.id);
  await upload(nestedPaper, 'Thesis/images/same.png', actualPng, 'image/png');
  await upload(nestedPaper, 'assets/same.png', actualPng, 'image/png');
  await upload(nestedPaper, 'Thesis/figures/hyphen_under.multi.part.png', actualPng, 'image/png');
  const nestedMain = String.raw`\documentclass{article}
\usepackage{graphicx,wrapfig}
\graphicspath{{images/}{figures/}}
\newcommand{\plotpath}{../assets/same.png}
\begin{document}NESTED MAIN\input{chapters/chapter.tex}
\begin{minipage}{.45\linewidth}\includegraphics[width=\linewidth]{images/same.png}\end{minipage}
\begin{minipage}{.45\linewidth}\includegraphics[angle=2,trim=1 1 1 1,clip,width=\linewidth]{\plotpath}\end{minipage}
\end{document}
`;
  const nestedChapter = String.raw`INCLUDED CHAPTER uses the main directory.
\begin{figure}[htbp]\centering\includegraphics[height=2cm,keepaspectratio]{hyphen_under.multi.part}\caption{Extensionless graphicspath}\label{fig:nested}\end{figure}
\begin{wrapfigure}{r}{.30\textwidth}\includegraphics[scale=.05]{images/same.png}\caption{Handwritten wrap}\end{wrapfigure}
Figure~\ref{fig:nested}.
`;
  await saveText(nestedPaper, 'Thesis/main.tex', nestedMain); await saveText(nestedPaper, 'Thesis/chapters/chapter.tex', nestedChapter);
  const nestedBuild = await compile(nestedPaper, 'nested-main-included-chapter-paths');
  assert.match(nestedBuild.pdf.text, /INCLUDED CHAPTER/); assert.ok(nestedBuild.pdf.imageOperators >= 4);

  await upload(rootPaper, 'images/synthetic-multipage.pdf', manualBuild.bytes, 'application/pdf');
  const pdfAssets = String.raw`\documentclass{article}
\usepackage{graphicx,pdfpages}\begin{document}PDF FIGURE MATRIX
\includegraphics[page=1,width=.35\linewidth]{images/synthetic-multipage.pdf}
\includegraphics[page=2,angle=3,trim=2 2 2 2,clip,width=.35\linewidth]{images/synthetic-multipage.pdf}
\includepdf[pages=1-2]{images/synthetic-multipage.pdf}
\end{document}
`;
  await saveText(rootPaper, 'main.tex', pdfAssets);
  const pdfAssetBuild = await compile(rootPaper, 'pdf-figure-and-inserted-pages');
  assert.ok(pdfAssetBuild.pdf.pages >= 3); assert.match(pdfAssetBuild.pdf.text, /PDF FIGURE MATRIX/);

  const table = buildTable({ rows: 2, columns: 2, header: true, booktabs: true, caption: 'Builder table', label: 'tab:builder' });
  const mathTables = String.raw`\documentclass{article}
\usepackage{amsmath,booktabs,longtable}\begin{document}MATH TABLE MATRIX
Inline $a_1^2+\sqrt{x}=\alpha$ and \(\operatorname{rank}(A)\). Escaped \$5.
\[\left(\frac{\sum_{i=1}^n i}{\int_0^1 x\,dx}\right)\]
\begin{equation}\begin{split}a&=b+c\\&=d\end{split}\label{eq:split}\tag{A}\end{equation}
\begin{equation*}\begin{aligned}x&=1\\y&=2\end{aligned}\end{equation*}
\begin{align}p&=q\\r&=s\end{align}\begin{align*}u&=v\end{align*}
\begin{gather}a=b\\c=d\end{gather}\begin{multline}a+b+c+d\\=e\end{multline}
$\begin{matrix}1&2\\3&4\end{matrix}\quad\begin{pmatrix}1\\2\end{pmatrix}\quad\begin{bmatrix}1&0\end{bmatrix}\quad f(x)=\begin{cases}x&x>0\\0&x\le0\end{cases}$
Equation~\ref{eq:split}. $$E=mc^2$$
\begin{eqnarray}a&=&b\end{eqnarray}
% commented math $not rendered$
${table}
\begin{longtable}{ll}\caption{Long table}\\A&B\\C&D\\\end{longtable}
\end{document}
`;
  await saveText(rootPaper, 'main.tex', mathTables);
  const mathBuild = await compile(rootPaper, 'equation-table-syntax-matrix');
  assert.match(mathBuild.pdf.text, /MATH TABLE MATRIX/); assert.match(mathBuild.pdf.text, /Builder table/);

  const builderPlot = buildPlot({ type: 'line', asset: 'data/plot.csv', x: 'x', y: 'y', title: 'Builder plot', xLabel: '$x$', yLabel: '$y$', legend: 'series', caption: 'Builder CSV plot' });
  const plots = String.raw`\documentclass{article}
\usepackage{tikz,pgfplots}\pgfplotsset{compat=1.18}\begin{document}PLOT MATRIX
\begin{tikzpicture}\draw[->] (0,0)--(2,0);\draw[blue] (0,0) circle (4mm);\end{tikzpicture}
\begin{tikzpicture}\begin{axis}[legend entries={$x^2$,coordinates},xlabel={$x$},ylabel={$f(x)$}]\addplot[domain=0:2,samples=12]{x^2};\addplot coordinates{(0,0)(1,1)(2,3)};\end{axis}\end{tikzpicture}
\begin{tikzpicture}\begin{axis}\addplot[only marks] table[x=x,y=y,col sep=comma]{data/plot.csv};\addplot[ybar] coordinates{(1,2)(2,3)};\end{axis}\end{tikzpicture}
${builderPlot}
\begin{tikzpicture}\begin{axis}[view={25}{25}]\addplot3[surf,domain=0:1,samples=5] {x*y};\end{axis}\end{tikzpicture}
\end{document}
`;
  await saveText(rootPaper, 'main.tex', plots);
  const plotBuild = await compile(rootPaper, 'tikz-pgfplots-syntax-matrix');
  assert.match(plotBuild.pdf.text, /PLOT MATRIX/);

  lastGood = plotBuild.buildId;
  } else {
    await saveText(rootPaper, 'main.tex', '\\documentclass{article}\\begin{document}CONTINUATION LAST GOOD\\end{document}');
    lastGood = (await compile(rootPaper, 'continuation-last-good')).buildId;
  }
  await saveText(rootPaper, 'main.tex', '\\documentclass{article}\\usepackage{graphicx}\\begin{document}\\includegraphics{images/Manual sample.v1.png}\\end{document}');
  const wrongCase = await compile(rootPaper, 'negative-wrong-case-path', 'either');
  wrongCaseDefect = wrongCase.status.build.latest_status === 'succeeded';
  if (!wrongCaseDefect) assert.match(wrongCase.log, /Manual sample|not found|LaTeX Error/i);
  let state = await okJson(writerCookie, `/api/v2/papers/${rootPaper}/builds`);
  assert.equal(state.build.current_build_id, wrongCaseDefect ? wrongCase.buildId : lastGood);
  await saveText(rootPaper, 'main.tex', '\\documentclass{article}\\begin{document}$ unmatched\\end{document}');
  await compile(rootPaper, 'negative-unmatched-math', 'failed');
  await saveText(rootPaper, 'main.tex', '\\documentclass{article}\\begin{document}\\begin{notarealenvironment}x\\end{notarealenvironment}\\undefinedProfessorCommand\\end{document}');
  await compile(rootPaper, 'negative-invalid-environment-command', 'failed');
  }

  const professorArchive = join(process.cwd(), 'artifacts/Full_Report_template_v1.1.zip');
  const professorTemplate = await importTemplate(`Professor complete report ${suffix}`, professorArchive, 'Full_Report_template_v1.0/Full_Report_v1.0.tex', 'SINGLE_SOURCE');
  const professorPaper = await createTeam(`Disposable Team Name ${suffix}`, professorTemplate.id);
  const professorPath = 'Full_Report_template_v1.0/Full_Report_v1.0.tex';
  const professorOriginal = (await readText(professorPaper, professorPath)).content;
  const chapterOne = (await readText(professorPaper, 'Full_Report_template_v1.0/chapters/chapter1.tex')).content;
  assert.match(chapterOne, /\\chapter\{\\MakeUppercase\{Introduction\}\}\s*\\label\{chap:introduction\}/);
  assert.match(chapterOne, /\\section\{Background\}\s*\\label\{sec:introduction-background\}/);
  const titleA = `Professor Source Title A ${suffix}`; const titleB = `Professor Source Title B ${suffix}`; const override = `Professor Details Override ${suffix}`;
  await saveText(professorPaper, professorPath, replaceTitle(professorOriginal, titleA));
  const titleABuild = await compile(professorPaper, 'professor-source-title-a');
  assert.ok(titleABuild.pdf.pageTexts.slice(0, 3).every((text) => normalizedPdfText(text).includes(normalizedPdfText(titleA))), JSON.stringify(titleABuild.pdf.pageTexts.slice(0, 3)));
  await saveText(professorPaper, professorPath, replaceTitle(professorOriginal, titleB));
  const titleBBuild = await compile(professorPaper, 'professor-source-title-b');
  assert.ok(titleBBuild.pdf.pageTexts.slice(0, 3).every((text) => normalizedPdfText(text).includes(normalizedPdfText(titleB))), JSON.stringify(titleBBuild.pdf.pageTexts.slice(0, 3)));
  assert.ok(!titleBBuild.pdf.text.includes(`Disposable Team Name ${suffix}`));
  const details = await okJson(writerCookie, `/api/v2/papers/${professorPaper}/document-details`);
  assert.equal(details.single_source, true); assert.equal(details.pack_id, null);
  const titleField = details.values.find((entry) => entry.field_key === 'project_title');
  assert.ok(titleField); assert.notEqual(titleField.value, `Disposable Team Name ${suffix}`);
  await okJson(writerCookie, `/api/v2/papers/${professorPaper}/document-details`, { method: 'PUT', json: { values: { project_title: override }, sections: {} } });
  const overrideBuild = await compile(professorPaper, 'professor-document-details-title-override');
  assert.ok(overrideBuild.pdf.pageTexts.slice(0, 3).every((text) => normalizedPdfText(text).includes(normalizedPdfText(override))), JSON.stringify(overrideBuild.pdf.pageTexts.slice(0, 3)));
  assert.ok(!overrideBuild.pdf.text.includes(`Disposable Team Name ${suffix}`));
  const acknowledgementPage = overrideBuild.pdf.pageTexts.findIndex((text) => /ACKNOWLEDGEMENT/i.test(text));
  assert.ok(acknowledgementPage >= 0, 'fresh professor PDF omitted acknowledgement');
  const acknowledgementMetrics = overrideBuild.pdf.pageMetrics[acknowledgementPage];
  assert.ok(acknowledgementMetrics.textLines > 8, 'acknowledgement did not wrap into paragraph lines');
  assert.ok(acknowledgementMetrics.maxTextRight <= acknowledgementMetrics.width + 1, 'acknowledgement text exceeded the page width');

  const mentorCookie = await login(mentorEmail, accountPassword);
  const mentorPapers = await okJson(mentorCookie, '/api/v2/mentor/papers');
  assert.ok(Array.isArray(mentorPapers.papers));
  const mentorTargetPaper = rootPaper || professorPaper;
  const mentorTargetPath = rootPaper ? 'main.tex' : professorPath;
  const mentorTargetFile = await fileByPath(mentorTargetPaper, mentorTargetPath);
  const mentorMutation = await request(mentorCookie, `/api/v2/papers/${mentorTargetPaper}/files/${mentorTargetFile.file_id}`, { method: 'PUT', json: { content: 'forbidden', version: (await paperState(mentorTargetPaper)).version } });
  assert.equal(mentorMutation.status, 403);
  const logs = await okJson(adminCookie, '/api/admin/v2/runtime-logs?limit=100');
  assert.equal(logs.source_status, 'available'); assert.ok(logs.records.length > 0);
  assert.ok(logs.records.some((record) => ['api', 'worker'].includes(record.service)));

  await writeFile(manifestFile, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    compile_jobs: compileJobs, successful_builds: successfulBuilds, failed_builds: failedBuilds,
    rendered_pdfs: renderedPdfs, upload_store_positive: 9, upload_rejections: 4,
    templates: evidence.templates.length, teams: evidence.teams.length,
    runtime_log_records: logs.records.length, runtime_log_services: logs.services,
    professor_acknowledgement_page: acknowledgementPage + 1,
    wrong_case_reference: wrongCaseDefect === null ? 'NOT_RUN_IN_THIS_PHASE' : wrongCaseDefect ? 'UNEXPECTED_SUCCESS' : 'EXPECTED_FAILURE',
    tested_builds: evidence.builds,
  }));
} finally {
  if (adminCookie) {
    for (const email of [writerEmail, mentorEmail]) {
      await request(adminCookie, `/api/admin/users/${encodeURIComponent(email)}`, { method: 'PATCH', json: { enabled: false } }).catch(() => {});
    }
  }
  await rm(temp, { recursive: true, force: true });
}
