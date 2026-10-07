import assert from 'node:assert/strict';

// Called by the disposable PostgreSQL integration fixture against a live Axum listener.
const { base, cookie, paperId } = JSON.parse(process.env.LATEX_CORE_HOTFIX_SMOKE_CONFIG);
const root = `/api/v2/papers/${paperId}`;
async function request(path, method = 'GET', body) {
  const response = await fetch(`${base}${path}`, {
    method, headers: { Cookie: cookie, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json();
  assert.equal(response.status, 200, JSON.stringify(result));
  return result;
}
const files = await request(`${root}/files`);
const main = files.find((file) => file.path === 'project.tex');
const nested = files.find((file) => file.path === 'chapters/body.tex');
assert.ok(files.some((file) => file.path === 'images/foo.png'));
const before = await request(`${root}/files/${main.file_id}`);
assert.ok(before.content.includes('Professor Durable Title One'));
const detail = await request(`${root}/document-details`);
assert.ok(detail.values.some((item) => item.field_key === 'student_a_name' && item.value === 'Institutional Alice'));
const saved = await request(`${root}/document-details`, 'PUT', {
  values: { project_title: 'Professor Durable Title Two' }, sections: {},
});
const after = await request(`${root}/files/${main.file_id}`);
assert.ok(after.content.includes('Professor Durable Title Two'));
assert.ok(!after.content.includes('Professor Durable Title One'));
assert.ok(after.content.includes('% unrelated manual preamble'));
assert.ok(after.version > before.version);
assert.ok(after.file.revision > before.file.revision);
assert.equal(saved.workspace_version, after.version);
const reopened = await request(`${root}/document-details`);
assert.ok(reopened.values.some((item) => item.field_key === 'project_title' && item.value === 'Professor Durable Title Two'));
const analysis = await request(`${root}/intelligence`);
assert.ok(!analysis.diagnostics.some((item) => item.code === 'MissingProjectDependency'), JSON.stringify(analysis.diagnostics));
const body = await request(`${root}/files/${nested.file_id}`);
await request(`${root}/files/${nested.file_id}`, 'PUT', {
  content: `${body.content}\n\\includegraphics{images/actually-missing.png}\n`, version: body.version,
});
const missing = await request(`${root}/intelligence`);
assert.ok(missing.diagnostics.some((item) => item.code === 'MissingProjectDependency' && item.message.includes('actually-missing.png')));
const current = await request(`${root}/files/${nested.file_id}`);
await request(`${root}/files/${nested.file_id}`, 'PUT', { content: body.content, version: current.version });
assert.ok(!(await request(`${root}/intelligence`)).diagnostics.some((item) => item.code === 'MissingProjectDependency'));
console.log('HTTP smoke: durable TeX, revision, reopen, institutional authority, existing/nested/graphicspath image, real missing image PASS');
