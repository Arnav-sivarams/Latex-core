import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { showStatusNotice, NOTICE_TIMEOUT_MS } from './status-notice.mjs';

class Node {
  constructor(tag, doc) { this.tag = tag; this.ownerDocument = doc; this.children = []; this.attributes = {}; this.listeners = {}; this.className = ''; this.textContent = ''; this.classes = new Set(); this.classList = { toggle: (name, on) => (on ? this.classes.add(name) : this.classes.delete(name)) }; }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(event, callback) { this.listeners[event] = callback; }
  click() { this.listeners.click?.(); }
}
const doc = { createElement: (tag) => new Node(tag, doc) };
const fakeTimers = () => { const timers = []; return { timers, schedule: (callback, delay) => { timers.push({ callback, delay }); return timers.length; }, cancel: () => {} }; };

test('success notices show a close button and fade after the timeout', () => {
  const region = new Node('div', doc); const clock = fakeTimers();
  showStatusNotice(region, 'Saved main.tex', clock);
  assert.equal(region.children[0].textContent, 'Saved main.tex');
  assert.equal(region.children.at(-1).attributes['aria-label'], 'Dismiss notification');
  assert.equal(clock.timers[0].delay, NOTICE_TIMEOUT_MS);
  clock.timers[0].callback(); assert.equal(region.children.length, 0);
});

test('failures persist until closed and may offer one action', () => {
  const region = new Node('div', doc); const clock = fakeTimers(); let ran = 0;
  showStatusNotice(region, 'Compile failed', { failed: true, action: { label: 'Open Problems', run: () => { ran += 1; } }, ...clock });
  assert.equal(clock.timers.length, 0); assert.ok(region.classes.has('danger')); assert.equal(region.attributes.role, 'alert');
  assert.equal(region.children[1].textContent, 'Open Problems');
  region.children.at(-1).click(); assert.equal(region.children.length, 0); assert.ok(!region.classes.has('danger'));
});

test('notices are placed prominently under the toolbar instead of the bottom-right corner', () => {
  const css = readFileSync(new URL('../static/shells.css', import.meta.url), 'utf8');
  const rule = css.match(/\.status-region \{[^}]*\}/)?.[0] || '';
  assert.match(rule, /top:/); assert.doesNotMatch(rule, /bottom: 18px/);
  assert.match(css, /\.status-close/);
});
