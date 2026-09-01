const item = (category, symbol, latex, description, keywords = '') => ({
  category, symbol, latex, description, keywords,
});

const commands = (category, rows) => rows.map(([symbol, latex, description, keywords]) => item(category, symbol, latex, description, keywords));

export const MATH_CATALOG = [
  ...commands('Basic', [
    ['±', '\\pm', 'plus or minus'], ['∓', '\\mp', 'minus or plus'], ['∞', '\\infty', 'infinity'],
    ['⋅', '\\cdot', 'centered multiplication dot'], ['×', '\\times', 'multiplication times'], ['÷', '\\div', 'division'],
  ]),
  ...commands('Greek lowercase', [
    ['α', '\\alpha', 'alpha'], ['β', '\\beta', 'beta'], ['γ', '\\gamma', 'gamma'], ['δ', '\\delta', 'delta'],
    ['ε', '\\epsilon', 'epsilon'], ['ϵ', '\\varepsilon', 'variant epsilon'], ['ζ', '\\zeta', 'zeta'], ['η', '\\eta', 'eta'],
    ['θ', '\\theta', 'theta'], ['ϑ', '\\vartheta', 'variant theta'], ['ι', '\\iota', 'iota'], ['κ', '\\kappa', 'kappa'],
    ['λ', '\\lambda', 'lambda'], ['μ', '\\mu', 'mu'], ['ν', '\\nu', 'nu'], ['ξ', '\\xi', 'xi'], ['π', '\\pi', 'pi'],
    ['ρ', '\\rho', 'rho'], ['σ', '\\sigma', 'sigma'], ['τ', '\\tau', 'tau'], ['υ', '\\upsilon', 'upsilon'],
    ['φ', '\\phi', 'phi'], ['ϕ', '\\varphi', 'variant phi'], ['χ', '\\chi', 'chi'], ['ψ', '\\psi', 'psi'], ['ω', '\\omega', 'omega'],
  ]),
  ...commands('Greek uppercase', [
    ['Γ', '\\Gamma', 'capital gamma'], ['Δ', '\\Delta', 'capital delta'], ['Θ', '\\Theta', 'capital theta'],
    ['Λ', '\\Lambda', 'capital lambda'], ['Ξ', '\\Xi', 'capital xi'], ['Π', '\\Pi', 'capital pi'],
    ['Σ', '\\Sigma', 'capital sigma'], ['Υ', '\\Upsilon', 'capital upsilon'], ['Φ', '\\Phi', 'capital phi'],
    ['Ψ', '\\Psi', 'capital psi'], ['Ω', '\\Omega', 'capital omega'],
  ]),
  ...commands('Relations', [
    ['=', '=', 'equals'], ['≠', '\\neq', 'not equal'], ['<', '<', 'less than'], ['>', '>', 'greater than'],
    ['≤', '\\leq', 'less than or equal', 'less equal'], ['≥', '\\geq', 'greater than or equal', 'greater equal'],
    ['≈', '\\approx', 'approximately equal'], ['≡', '\\equiv', 'equivalent'], ['∼', '\\sim', 'similar'],
    ['≅', '\\cong', 'congruent'], ['∝', '\\propto', 'proportional to'], ['⊥', '\\perp', 'perpendicular'],
    ['∥', '\\parallel', 'parallel'], ['∣', '\\mid', 'divides'], ['∤', '\\nmid', 'does not divide'],
  ]),
  ...commands('Binary operators', [
    ['+', '+', 'plus'], ['−', '-', 'minus'], ['∗', '\\ast', 'asterisk product'], ['∘', '\\circ', 'composition'],
    ['⊕', '\\oplus', 'direct sum'], ['⊗', '\\otimes', 'tensor product'], ['∧', '\\wedge', 'wedge and'],
    ['∨', '\\vee', 'vee or'], ['∩', '\\cap', 'intersection'], ['∪', '\\cup', 'union'], ['∖', '\\setminus', 'set difference'],
  ]),
  ...commands('Arrows', [
    ['←', '\\leftarrow', 'left arrow'], ['→', '\\rightarrow', 'right arrow'], ['↔', '\\leftrightarrow', 'left right arrow'],
    ['⇐', '\\Leftarrow', 'left implication'], ['⇒', '\\Rightarrow', 'right implication implies'], ['⇔', '\\Leftrightarrow', 'if and only if'],
    ['↦', '\\mapsto', 'maps to'], ['↗', '\\nearrow', 'north east arrow'], ['↘', '\\searrow', 'south east arrow'],
    ['⟶', '\\longrightarrow', 'long right arrow'],
  ]),
  ...commands('Sets', [
    ['∈', '\\in', 'element of'], ['∉', '\\notin', 'not an element of'], ['∋', '\\ni', 'contains element'],
    ['⊂', '\\subset', 'proper subset'], ['⊆', '\\subseteq', 'subset or equal'], ['⊃', '\\supset', 'proper superset'],
    ['⊇', '\\supseteq', 'superset or equal'], ['∅', '\\emptyset', 'empty set'], ['ℕ', '\\mathbb{N}', 'natural numbers'],
    ['ℤ', '\\mathbb{Z}', 'integers'], ['ℚ', '\\mathbb{Q}', 'rational numbers'], ['ℝ', '\\mathbb{R}', 'real numbers'],
    ['ℂ', '\\mathbb{C}', 'complex numbers'],
  ]),
  ...commands('Logic', [
    ['¬', '\\neg', 'logical not'], ['∀', '\\forall', 'for all universal'], ['∃', '\\exists', 'there exists existential'],
    ['∄', '\\nexists', 'does not exist'], ['∴', '\\therefore', 'therefore'], ['∵', '\\because', 'because'],
    ['⊢', '\\vdash', 'proves entails'], ['⊨', '\\models', 'models satisfies'], ['⊤', '\\top', 'true top'], ['⊥', '\\bot', 'false bottom'],
  ]),
  ...commands('Calculus', [
    ['∫', '\\int', 'integral'], ['∬', '\\iint', 'double integral multiple integral'], ['∭', '\\iiint', 'triple integral multiple integral'],
    ['∮', '\\oint', 'contour integral'], ['∂', '\\partial', 'partial derivative'], ['∇', '\\nabla', 'nabla gradient'],
    ['∆', '\\Delta', 'Laplacian delta'], ['d', '\\mathrm{d}', 'differential'],
  ]),
  ...commands('Big operators', [
    ['∑', '\\sum', 'sum summation'], ['∏', '\\prod', 'product'], ['∐', '\\coprod', 'coproduct'],
    ['⋃', '\\bigcup', 'big union'], ['⋂', '\\bigcap', 'big intersection'], ['⨁', '\\bigoplus', 'big direct sum'],
    ['⨂', '\\bigotimes', 'big tensor product'],
  ]),
  ...commands('Delimiters', [
    ['( )', '\\left(  \\right)', 'scalable parentheses'], ['[ ]', '\\left[  \\right]', 'scalable brackets'],
    ['{ }', '\\left\\{  \\right\\}', 'scalable braces'], ['| |', '\\left|  \\right|', 'absolute value'],
    ['‖ ‖', '\\left\\|  \\right\\|', 'norm'], ['⌈ ⌉', '\\left\\lceil  \\right\\rceil', 'ceiling'],
    ['⌊ ⌋', '\\left\\lfloor  \\right\\rfloor', 'floor'], ['⟨ ⟩', '\\langle  \\rangle', 'angle brackets inner product'],
  ]),
  ...commands('Accents', [
    ['x̂', '\\hat{x}', 'hat accent'], ['x̃', '\\tilde{x}', 'tilde accent'], ['x̄', '\\bar{x}', 'bar accent'],
    ['x⃗', '\\vec{x}', 'vector arrow accent'], ['ẋ', '\\dot{x}', 'dot accent'], ['ẍ', '\\ddot{x}', 'double dot accent'],
    ['abc̅', '\\overline{abc}', 'overline'], ['abc̲', '\\underline{abc}', 'underline'],
  ]),
  ...commands('Functions', [
    ['sin', '\\sin', 'sine'], ['cos', '\\cos', 'cosine'], ['tan', '\\tan', 'tangent'], ['arcsin', '\\arcsin', 'inverse sine'],
    ['log', '\\log', 'logarithm'], ['ln', '\\ln', 'natural logarithm'], ['exp', '\\exp', 'exponential'],
    ['min', '\\min', 'minimum'], ['max', '\\max', 'maximum'], ['det', '\\det', 'determinant'], ['gcd', '\\gcd', 'greatest common divisor'],
  ]),
  ...commands('Blackboard / calligraphic helpers', [
    ['𝔸', '\\mathbb{A}', 'blackboard bold'], ['𝒜', '\\mathcal{A}', 'calligraphic'], ['𝐀', '\\mathbf{A}', 'bold math'],
    ['𝔄', '\\mathfrak{A}', 'Fraktur'], ['A', '\\mathrm{A}', 'upright roman math'], ['A', '\\mathsf{A}', 'sans serif math'],
  ]),
  ...commands('Templates', [
    ['inline', '\\(  \\)', 'inline equation'], ['display', '\\[\n\n\\]', 'display equation'],
    ['equation', '\\begin{equation}\n\n\\end{equation}', 'numbered equation environment'],
    ['align', '\\begin{align}\n  a &= b \\\\\n  c &= d\n\\end{align}', 'align equations environment'],
    ['aligned', '\\begin{aligned}\n  a &= b \\\\\n  c &= d\n\\end{aligned}', 'aligned equations component'],
    ['gather', '\\begin{gather}\n  a=b \\\\\n  c=d\n\\end{gather}', 'gather equations environment'],
    ['split', '\\begin{split}\n  a &= b \\\\\n    &\quad + c\n\\end{split}', 'split equation component'],
    ['cases', '\\begin{cases}\n  value, & condition \\\\\n  other, & otherwise\n\\end{cases}', 'piecewise cases'],
    ...['matrix', 'pmatrix', 'bmatrix', 'Bmatrix', 'vmatrix', 'Vmatrix'].map((name) => [name, `\\begin{${name}}\n  a & b \\\\\n  c & d\n\\end{${name}}`, `${name} matrix template`]),
  ]),
  ...commands('Components', [
    ['a/b', '\\frac{}{}', 'fraction'], ['√', '\\sqrt{}', 'square root'], ['ⁿ√', '\\sqrt[]{}', 'nth root'],
    ['xⁿ', '^{}', 'superscript power'], ['xₙ', '_{}', 'subscript index'], ['Σ', '\\sum_{}^{}', 'sum with limits'],
    ['Π', '\\prod_{}^{}', 'product with limits'], ['∫', '\\int_{}^{} \\mathrm{d}x', 'definite integral'],
    ['∬', '\\iint_{} \\mathrm{d}A', 'multiple integral'], ['lim', '\\lim_{}', 'limit'],
    ['dy/dx', '\\frac{\\mathrm{d}y}{\\mathrm{d}x}', 'derivative'], ['∂y/∂x', '\\frac{\\partial y}{\\partial x}', 'partial derivative'],
    ['x⃗', '\\vec{}', 'vector'], ['x̂', '\\hat{}', 'hat'], ['x̄', '\\bar{}', 'bar'], ['abc̅', '\\overline{}', 'overline'],
    ['⏟', '\\underbrace{}_{}', 'underbrace'], ['⏞', '\\overbrace{}^{}', 'overbrace'],
  ]),
];

export function searchMathCatalog(query, catalog = MATH_CATALOG) {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return catalog;
  return catalog.filter((entry) => {
    const haystack = `${entry.category} ${entry.symbol} ${entry.latex} ${entry.description} ${entry.keywords}`.toLocaleLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
}
