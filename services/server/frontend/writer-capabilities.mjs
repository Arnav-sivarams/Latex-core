// Probed with kpsewhich and compilation against frozen M7 image
// sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38.
// Availability in the compiler and loading in the report are separate checks.
export const COMPILER_PACKAGES = Object.freeze(['amsmath', 'amssymb', 'graphicx', 'booktabs', 'longtable', 'wrapfig', 'pgfplots', 'algorithm', 'algorithmic', 'algpseudocode', 'listings', 'amsthm']);

export function insertionCapability(kind, values = {}, packages = [], environments = []) {
  const required = {
    table: values.booktabs ? ['booktabs'] : [], longtable: ['longtable'],
    figure: ['graphicx'], wrapfigure: ['graphicx', 'wrapfig'], plot: ['pgfplots'],
    algorithm: ['algorithm', values.family === 'algorithmic' ? 'algorithmic' : 'algpseudocode'],
    code: ['listings'], equation: ['aligned', 'matrix', 'cases'].includes(values.type) ? ['amsmath'] : [],
    theorem: values.environment === 'proof' ? ['amsthm'] : [],
  }[kind] || [];
  const unavailable = required.filter(name => !COMPILER_PACKAGES.includes(name));
  if (unavailable.length) return { available: false, message: 'Unavailable in the current compiler' };
  const missing = required.filter(name => !packages.includes(name));
  if (missing.length) return { available: false, message: `Requires package${missing.length > 1 ? 's' : ''}: ${missing.join(', ')}` };
  if (kind === 'theorem' && values.environment !== 'proof' && !environments.includes(values.environment || 'theorem')) return { available: false, message: `Environment not detected: ${values.environment || 'theorem'}` };
  return { available: true, message: 'Available' };
}

export function symbolCapability(latex, packages) {
  const required = [];
  if (/\\(?:mathbb|mathfrak|nmid|nexists|varnothing|implies|impliedby|iff|therefore|because|lesssim|gtrsim|square|blacksquare|checkmark)\b/.test(latex)) required.push('amssymb');
  if (/\\(?:text|dfrac|tfrac|iint|iiint|operatorname|overset|underset|boxed|boldsymbol)\b|\\begin\{(?:align|aligned|gather|split|cases|[pbBvV]?matrix)\}/.test(latex)) required.push('amsmath');
  const missing = required.filter(name => !packages.includes(name));
  return { available: !missing.length, message: missing.length ? `Requires package: ${missing.join(', ')}` : 'Available' };
}

export function mathInsertionSource(latex, insideMath = false) {
  // Blank paragraphs are illegal inside TeX math environments.
  latex = latex.replace(/\n[ \t]*\n/g, '\n');
  if (/^\\(?:\(|\[)|^\\begin\{(?:equation|align|gather)\}/.test(latex)) {
    if (insideMath) throw new Error('Place the cursor outside the current math expression before inserting a complete equation.');
    return latex;
  }
  if (/^\\begin\{split\}/.test(latex)) {
    if (insideMath) throw new Error('Place the cursor outside the current math expression before inserting a split equation.');
    return `\\begin{equation}\n${latex}\n\\end{equation}`;
  }
  return insideMath ? latex : `\\(${latex}\\)`;
}
