const LEVELS = { part: 0, chapter: 0, section: 0, subsection: 1, subsubsection: 2, paragraph: 3, subparagraph: 4 };

export function buildOutlineTree(entries) {
  const roots = [];
  const stack = [];
  for (const entry of entries) {
    const level = LEVELS[entry.level] ?? 0;
    const node = { ...entry, children: [] };
    while (stack.length && stack.at(-1).level >= level) stack.pop();
    if (stack.length) stack.at(-1).node.children.push(node);
    else roots.push(node);
    stack.push({ level, node });
  }
  return roots;
}

export function fuzzyRankFiles(files, query) {
  const needle = query.trim().toLowerCase();
  const score = (path) => {
    if (!needle) return 0;
    const value = path.toLowerCase();
    const basename = value.split('/').at(-1);
    if (basename === needle) return 1000;
    if (value === needle) return 900;
    if (basename.startsWith(needle)) return 800 - basename.length;
    if (value.includes(needle)) return 600 - value.indexOf(needle);
    let cursor = -1;
    let gaps = 0;
    for (const character of needle) {
      const next = value.indexOf(character, cursor + 1);
      if (next < 0) return -1;
      gaps += next - cursor - 1;
      cursor = next;
    }
    return 300 - gaps;
  };
  return files.map((file) => ({ file, score: score(file.path) }))
    .filter(({ score: value }) => value >= 0)
    .sort((a, b) => b.score - a.score || a.file.path.localeCompare(b.file.path))
    .map(({ file }) => file);
}

const clamp = (value, minimum = 1, maximum = 20) => Math.min(maximum, Math.max(minimum, Number.parseInt(value, 10) || minimum));
const safeLabel = (value = '') => value.replace(/[^A-Za-z0-9:_.-]/g, '-');
const safeText = (value = '') => value.replace(/[\\{}%&#]/g, (character) => ({ '\\': '\\textbackslash{}', '{': '\\{', '}': '\\}', '%': '\\%', '&': '\\&', '#': '\\#' })[character]);
const safePath = (value = '') => value.replace(/[{}\\\r\n%#]/g, '');

export function packageRequirement(packages, required) {
  return packages.includes(required) ? { available: true, message: 'Available' } : { available: false, message: `Requires package: ${required}` };
}

export function buildTable(options = {}) {
  const rows = clamp(options.rows, 1, 30);
  const columns = clamp(options.columns, 1, 12);
  const alignments = Array.from({ length: columns }, (_, index) => ({ left: 'l', center: 'c', right: 'r' }[options.alignments?.[index]] || 'l')).join('');
  const booktabs = Boolean(options.booktabs);
  const lines = [`\\begin{table}[${options.placement || 'htbp'}]`, '\\centering', `\\begin{tabular}{${alignments}}`];
  if (booktabs) lines.push('\\toprule');
  for (let row = 0; row < rows; row += 1) {
    lines.push(Array.from({ length: columns }, (_, column) => options.header && row === 0 ? `Header ${column + 1}` : `Cell ${row + 1}.${column + 1}`).join(' & ') + ' \\\\');
    if (booktabs && options.header && row === 0) lines.push('\\midrule');
  }
  if (booktabs) lines.push('\\bottomrule');
  lines.push('\\end{tabular}');
  if (options.caption) lines.push(`\\caption{${safeText(options.caption)}}`);
  if (options.label) lines.push(`\\label{${safeLabel(options.label)}}`);
  lines.push('\\end{table}');
  return lines.join('\n');
}

export function buildFigure(options = {}) {
  const width = options.width === 'custom' ? (options.customWidth || '\\linewidth') : (options.width || '\\linewidth');
  const lines = [`\\begin{figure}[${options.placement || 'htbp'}]`, '\\centering', `\\includegraphics[width=${width}]{${safePath(options.asset || 'path/to/image')}}`];
  if (options.caption) lines.push(`\\caption{${safeText(options.caption)}}`);
  if (options.label) lines.push(`\\label{${safeLabel(options.label)}}`);
  lines.push('\\end{figure}');
  return lines.join('\n');
}

export function buildEquation(options = {}) {
  const body = options.body || 'x = y';
  if (options.type === 'inline') return `\\(${body}\\)`;
  if (options.type === 'matrix') {
    const rows = clamp(options.rows, 1, 10);
    const columns = clamp(options.columns, 1, 10);
    const delimiter = { '()': 'pmatrix', '[]': 'bmatrix', '||': 'vmatrix', none: 'matrix' }[options.delimiter] || 'pmatrix';
    const matrix = Array.from({ length: rows }, (_, row) => Array.from({ length: columns }, (_, column) => `a_{${row + 1}${column + 1}}`).join(' & ')).join(' \\\\\n');
    return `\\begin{${delimiter}}\n${matrix}\n\\end{${delimiter}}`;
  }
  if (options.type === 'cases') {
    const rows = clamp(options.rows, 1, 10);
    const cases = Array.from({ length: rows }, (_, row) => `${row ? '0' : 'f(x)'} & \\text{if } ${row ? 'x < 0' : 'x \\geq 0'}`).join(' \\\\\n');
    return `\\begin{cases}\n${cases}\n\\end{cases}`;
  }
  if (options.type === 'aligned') return `\\begin{align}\n${body}\n\\end{align}`;
  return `\\begin{equation}\n${body}${options.label ? `\n\\label{${safeLabel(options.label)}}` : ''}\n\\end{equation}`;
}

export function buildPlot(options = {}) {
  const type = options.type || 'line';
  const plotOptions = type === 'scatter' ? 'only marks' : type === 'bar' ? 'ybar' : '';
  const lines = ['\\begin{figure}[htbp]', '\\centering', '\\begin{tikzpicture}', `\\begin{axis}[width=${options.width || '\\linewidth'},title={${safeText(options.title || '')}},xlabel={${safeText(options.xLabel || '')}},ylabel={${safeText(options.yLabel || '')}}${options.legend ? `,legend entries={${safeText(options.legend)}}` : ''}]`, `\\addplot[${plotOptions}] table[x=${safeText(options.x || 'x')},y=${safeText(options.y || 'y')},col sep=comma] {${safePath(options.asset || 'data.csv')}};`, '\\end{axis}', '\\end{tikzpicture}'];
  if (options.caption) lines.push(`\\caption{${safeText(options.caption)}}`);
  if (options.label) lines.push(`\\label{${safeLabel(options.label)}}`);
  lines.push('\\end{figure}');
  return lines.join('\n');
}

export function buildBibtexEntry(options = {}) {
  const type = ['article', 'book', 'inproceedings', 'misc'].includes(options.type) ? options.type : 'article';
  const fields = [['title', options.title], ['author', options.author], ['year', options.year], ['journal', options.journal], ['booktitle', options.booktitle], ['doi', options.doi], ['url', options.url]].filter(([, value]) => value);
  return `@${type}{${safeLabel(options.key || 'key')},\n${fields.map(([name, value]) => `  ${name} = {${safeText(String(value))}}`).join(',\n')}\n}`;
}

export function buildAlgorithm(options = {}) {
  const body = (options.body || '\\State Describe the method').split('\n').join('\n');
  return `\\begin{algorithm}\n\\caption{${safeText(options.caption || 'Algorithm')}}${options.label ? `\n\\label{${safeLabel(options.label)}}` : ''}\n\\begin{algorithmic}[1]\n${body}\n\\end{algorithmic}\n\\end{algorithm}`;
}

export function buildCodeListing(options = {}) {
  if (options.file) return `\\lstinputlisting[language=${safeText(options.language || '')},caption={${safeText(options.caption || '')}}${options.label ? `,label={${safeLabel(options.label)}}` : ''}]{${safePath(options.file)}}`;
  return `\\begin{lstlisting}[language=${safeText(options.language || '')},caption={${safeText(options.caption || '')}}${options.label ? `,label={${safeLabel(options.label)}}` : ''}]\n${options.code || ''}\n\\end{lstlisting}`;
}

export function buildTheorem(options = {}) {
  const environment = safeLabel(options.environment || 'theorem');
  return `\\begin{${environment}}${options.title ? `[${safeText(options.title)}]` : ''}${options.label ? `\n\\label{${safeLabel(options.label)}}` : ''}\n${options.body || 'Statement.'}\n\\end{${environment}}`;
}

export const symbols = {
  Greek: ['\\alpha', '\\beta', '\\gamma', '\\Delta'], Relations: ['\\leq', '\\geq', '\\neq'],
  Arrows: ['\\rightarrow', '\\Rightarrow'], Operators: ['\\sum', '\\prod'], Sets: ['\\in', '\\subseteq'],
  Calculus: ['\\int', '\\partial', '\\nabla', '\\infty'],
};

export const commonSnippets = {
  figure: '\\begin{figure}[htbp]\n\\centering\n\\includegraphics[width=\\linewidth]{image}\n\\caption{Caption}\n\\label{fig:label}\n\\end{figure}',
  table: buildTable({ rows: 2, columns: 2, header: true }), equation: buildEquation({ type: 'display' }),
  align: buildEquation({ type: 'aligned' }), itemize: '\\begin{itemize}\n  \\item Item\n\\end{itemize}',
  enumerate: '\\begin{enumerate}\n  \\item Item\n\\end{enumerate}', theorem: buildTheorem(),
  proof: '\\begin{proof}\nProof.\n\\end{proof}', algorithm: buildAlgorithm(), code: buildCodeListing(),
};
