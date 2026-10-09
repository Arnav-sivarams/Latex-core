import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const admin = readFileSync(new URL('../static/admin.js', import.meta.url), 'utf8');
const render = admin.slice(admin.indexOf('async function renderFilePolicies('), admin.indexOf('function templateTabs()'));

// Exercise the actual Admin renderer with only its DOM/API boundaries replaced.
class Node {
  constructor(tag, text = '') { this.tag = tag; this.textContent = text; this.children = []; this.listeners = {}; this.attributes = {}; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(event, callback) { this.listeners[event] = callback; }
  get value() { return this.selection ?? this.children.find((node) => node.selected)?.value ?? this.children[0]?.value; }
  set value(value) { this.selection = value; }
  async trigger(event) { return this.listeners[event]?.(); }
}
function descendants(node) { return [node, ...node.children.flatMap(descendants)]; }
async function fixture({ count = 2, fail = false, confirmed = true, excluded = false } = {}) {
  const content = new Node('main'); const calls = []; const notices = []; const confirmations = []; const errors = [];
  const overview = { team_count: count, files: count ? [
    { path: 'main.tex', policy: null, team_count: count },
    { path: 'chapter.tex', policy: 'EDITABLE', team_count: count },
  ] : [] };
  const context = {
    content, document: { createElement: (tag) => new Node(tag) },
    Option: class extends Node { constructor(text, value) { super('option', text); this.value = value; } },
    element: (tag, _class, text) => new Node(tag, text), humanLabel: (value) => value,
    confirm: (message) => { confirmations.push(message); return confirmed; },
    announce: (message) => notices.push(message), showError: (error) => errors.push(error.message),
    buttonAction: (label, handler) => { const node = new Node('button', label); node.addEventListener('click', () => Promise.resolve().then(handler).catch(context.showError)); return node; },
    api: async (path, options) => {
      calls.push({ path, options });
      if (options?.method === 'PATCH') {
        if (fail) throw new Error('No policies committed');
        return { team_count: count, affected_team_count: count, updated_files: [{ path: 'main.tex' }], excluded_files: excluded ? [{ paper_team_id: 'team-b', path: 'main.tex', reason: 'Logical path does not exist in this Team' }] : [] };
      }
      return path === '/api/admin/v2/file-policies' ? overview : [{ file_id: 'file-a', path: 'main.tex', policy: 'EDITABLE' }];
    },
  };
  await runInNewContext(`${render}; renderFilePolicies(${JSON.stringify(count ? [{ id: 'team-a', name: 'Team A' }, { id: 'team-b', name: 'Team B' }] : [])});`, context);
  const team = content.children.find((node) => node.tag === 'select');
  const controls = () => descendants(content).filter((node) => node.tag === 'select' && node !== team);
  const apply = () => descendants(content).find((node) => node.tag === 'button');
  return { content, calls, notices, confirmations, errors, team, controls, apply };
}

test('All Teams is first, shows mixed/count, and applies only explicit changes in one request', async () => {
  const view = await fixture();
  assert.deepEqual(view.team.children.map((option) => option.textContent), ['All Teams', 'Team A', 'Team B']);
  assert.ok(descendants(view.content).some((node) => node.textContent === 'Mixed / Varies'));
  assert.ok(descendants(view.content).some((node) => node.textContent.includes('2 Teams affected')));
  assert.equal(view.apply().disabled, true);
  view.controls()[0].value = 'CONTENT_READ_ONLY'; await view.controls()[0].trigger('change');
  assert.equal(view.apply().disabled, false);
  await view.apply().trigger('click');
  const writes = view.calls.filter((call) => call.options?.method === 'PATCH');
  assert.equal(writes.length, 1);
  assert.equal(writes[0].path, '/api/admin/v2/file-policies');
  assert.deepEqual(JSON.parse(writes[0].options.body), { expected_team_count: 2, confirmed: true, changes: [{ path: 'main.tex', policy: 'CONTENT_READ_ONLY' }] });
  assert.match(view.confirmations[0], /all 2 Teams/);
  assert.match(view.notices[0], /1 file policies in 2 of 2 Teams/);
  assert.equal(view.calls.filter((call) => !call.options).length, 2, 'success reloads persisted policies');
});

test('cancel and failure never announce a successful bulk update', async () => {
  for (const options of [{ confirmed: false }, { fail: true }]) {
    const view = await fixture(options);
    view.controls()[0].value = 'STRUCTURE_LOCKED'; await view.controls()[0].trigger('change');
    await view.apply().trigger('click');
    assert.equal(view.notices.length, 0);
    if (options.fail) assert.deepEqual(view.errors, ['No policies committed']);
    else assert.equal(view.calls.filter((call) => call.options).length, 0);
  }
});

test('individual Team selection retains the existing single-file operation', async () => {
  const view = await fixture(); view.team.value = 'team-a'; await view.team.trigger('change');
  view.controls()[0].value = 'STRUCTURE_LOCKED'; await view.controls()[0].trigger('change');
  const write = view.calls.find((call) => call.options);
  assert.equal(write.path, '/api/admin/v2/paper-teams/team-a/file-policies/file-a');
  assert.deepEqual(JSON.parse(write.options.body), { policy: 'STRUCTURE_LOCKED' });
});

test('empty Team collection retains All Teams and has no apply action', async () => {
  const view = await fixture({ count: 0 });
  assert.equal(view.team.children[0].textContent, 'All Teams');
  assert.equal(view.apply(), undefined);
  assert.ok(descendants(view.content).some((node) => node.textContent.includes('0 Teams affected')));
});

test('bulk exclusions identify the Team and exact missing path in the result', async () => {
  const view = await fixture({ excluded: true });
  view.controls()[0].value = 'CONTENT_READ_ONLY'; await view.controls()[0].trigger('change');
  await view.apply().trigger('click');
  assert.match(view.notices[0], /Excluded: Team B: main\.tex \(Logical path does not exist in this Team\)/);
});
