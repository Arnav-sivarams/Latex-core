import assert from 'node:assert/strict';
import test from 'node:test';
import { COMPILER_PACKAGES, insertionCapability, symbolCapability, mathInsertionSource } from './writer-capabilities.mjs';
import { buildEquation } from './writer-productivity.mjs';

test('Insert checks compiler capabilities and report package loading independently', () => {
  for (const [kind, packages] of [['figure',['graphicx']], ['wrapfigure',['graphicx','wrapfig']], ['plot',['pgfplots']], ['longtable',['longtable']], ['code',['listings']], ['algorithm',['algorithm','algpseudocode']]]) {
    assert.equal(insertionCapability(kind, {}, []).available, false, kind);
    assert.equal(insertionCapability(kind, {}, packages).available, true, kind);
    assert.ok(packages.every(name => COMPILER_PACKAGES.includes(name)));
  }
  assert.equal(insertionCapability('theorem', {environment:'proof'}, ['amsthm']).available, true);
  assert.equal(insertionCapability('theorem', {environment:'theorem'}, [], []).available, false);
  assert.equal(insertionCapability('theorem', {environment:'theorem'}, [], ['theorem']).available, true);
});

test('complete math templates retain their environment and cannot nest inside inline math', () => {
  assert.equal(mathInsertionSource('\\[x\\]'),'\\[x\\]');
  assert.equal(mathInsertionSource('\\[\n\n\\]'),'\\[\n\\]');
  assert.equal(mathInsertionSource('\\begin{align}a&=b\\end{align}'),'\\begin{align}a&=b\\end{align}');
  assert.match(mathInsertionSource('\\begin{split}a&=b\\end{split}'),/^\\begin\{equation\}/);
  assert.throws(()=>mathInsertionSource('\\[x\\]',true),/outside/);
  assert.equal(mathInsertionSource('\\alpha',true),'\\alpha');
});

test('matrix and cases produce complete display math, symbols require their packages', () => {
  for (const type of ['matrix','cases']) {
    const source=buildEquation({type});
    assert.ok(source.startsWith('\\[\n'));
    assert.ok(source.endsWith('\n\\]'));
    assert.equal(insertionCapability('equation',{type},[]).available,false);
    assert.equal(insertionCapability('equation',{type},['amsmath']).available,true);
  }
  assert.equal(symbolCapability('\\mathbb{R}',[]).available,false);
  assert.equal(symbolCapability('\\mathbb{R}',['amssymb']).available,true);
  assert.equal(symbolCapability('\\alpha',[]).available,true);
});
