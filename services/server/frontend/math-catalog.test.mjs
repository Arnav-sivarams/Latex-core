import assert from 'node:assert/strict';
import test from 'node:test';
import { MATH_CATALOG, searchMathCatalog } from './math-catalog.mjs';

test('math catalog is broad, categorized, and searched by symbol, command, or description', () => {
  assert.ok(MATH_CATALOG.length >= 100);
  const categories = new Set(MATH_CATALOG.map((entry) => entry.category));
  for (const category of ['Greek lowercase', 'Greek uppercase', 'Relations', 'Calculus', 'Templates', 'Components']) assert.ok(categories.has(category));
  assert.equal(searchMathCatalog('alpha')[0].latex, '\\alpha');
  assert.equal(searchMathCatalog('less equal')[0].latex, '\\leq');
  assert.ok(searchMathCatalog('integral').some((entry) => entry.latex === '\\int'));
  assert.ok(searchMathCatalog('pmatrix').some((entry) => entry.latex.includes('begin{pmatrix}')));
});

test('math catalog entries carry insertion-ready LaTeX and searchable metadata', () => {
  for (const entry of MATH_CATALOG) {
    assert.ok(entry.category && entry.symbol && entry.latex && entry.description);
  }
});
