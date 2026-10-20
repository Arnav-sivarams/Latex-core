import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';

export async function verifyServedAssets(context, base) {
  for (const name of ['writer.js', 'review.js', 'shells.css', 'admin.js', 'pdf.min.mjs', 'pdf.worker.min.mjs']) {
    const response = await context.request.get(`${base}/static/${name}`);
    assert.equal(response.status(), 200);
    assert.deepEqual(await response.body(), await readFile(new URL(`../static/${name}`, import.meta.url)), `Served ${name} must match the tested source`);
  }
  console.log('SERVED_ASSETS_PASS: exact source bytes');
}

export async function verifyProblems(page, base, paperId, files) {
  const target = files.find(file => file.path.endsWith('/chapters/chapter1.tex'));
  assert.ok(target);
  const one = {
    severity: 'error', code: 'synthetic-error', message: 'Undefined control sequence',
    file_id: target.file_id, path: target.path, range: { start_line: 2, start_column: 0 },
  };
  const longMessage = 'Overfull \\hbox (12pt) ' + 'long diagnostic with Unicode 研究 and <source> text '.repeat(20);
  const many = [one, { severity: 'warning', message: longMessage, path: 'main.tex', range: { start_line: 87 } },
    { severity: 'info', message: 'No file or line location' },
    { severity: 'warning', message: 'File without line', path: 'chapter2.tex' },
    ...Array.from({ length: 36 }, (_, i) => ({ ...one, message: `Diagnostic ${i + 5}` }))];
  let diagnostics = [];
  const route = async route => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({ response, json: { ...body, diagnostics } });
  };
  await page.route(`**/api/v2/papers/${paperId}/intelligence`, route);
  const reload = async () => {
    await page.goto(`${base}/write?paper=${paperId}`);
    await page.locator('.cm-content').waitFor();
    await page.waitForFunction(count => document.querySelector('#buildFooterCount').textContent === String(count), diagnostics.length);
    await page.waitForLoadState('networkidle');
    if (await page.locator('#workspaceDrawer').isVisible()) await page.locator('#drawerClose').click();
    await page.locator('#buildProblemsTab').click();
  };
  try {
    await reload();
    assert.equal(await page.locator('#buildProblemsList .problem-row').count(), 0);
    await page.locator('#buildProblemsList').getByText('No current problems.', { exact: true }).waitFor();
    diagnostics = [one];
    await reload();
    assert.equal(await page.locator('#buildProblemsList .problem-row').count(), 1);
    diagnostics = many;
    await reload();
    const rows = page.locator('#buildProblemsList .problem-row');
    assert.deepEqual(await rows.locator('.problem-message').allTextContents(), many.map(d => d.message));
    assert.equal(await page.locator('#problemsToggle').getAttribute('aria-label'), 'Problems: 37 errors, 2 warnings');
    assert.deepEqual(await rows.locator('.problem-location').allTextContents(),
      many.map(d => `${d.path?.split('/').pop() || 'Build'}${d.range?.start_line ? `:${d.range.start_line}` : ''}`));
    assert.equal(await rows.nth(1).getAttribute('title'), `warning: ${longMessage}\nmain.tex:87`);
    assert.equal(await rows.nth(1).getAttribute('aria-label'), `warning: ${longMessage}\nmain.tex:87`);
    for (const theme of ['LIGHT', 'DARK']) {
      await page.locator('#editorSettings summary').click();
      const saved = page.waitForResponse(r => r.request().method() === 'PUT' && r.url().endsWith('/preferences/editor'));
      await page.locator('#editorTheme').selectOption(theme);
      assert.equal((await saved).status(), 200);
      await page.locator('#editorSettings summary').click();
      if (await page.locator('#workspaceDrawer').isVisible()) await page.locator('#drawerClose').click();
      for (const width of [375, 768, 1440]) {
        await page.setViewportSize({ width, height: 1000 });
        const geometry = await rows.evaluateAll(nodes => nodes.map(row => {
          const message = row.querySelector('.problem-message');
          const reference = row.querySelector('.problem-location');
          const box = row.getBoundingClientRect();
          return { height: box.height, right: box.right, left: box.left,
            referenceLeft: reference.getBoundingClientRect().left,
            messageStyle: getComputedStyle(message).whiteSpace,
            ellipsis: getComputedStyle(message).textOverflow,
            messageHeight: message.getBoundingClientRect().height,
            clipped: message.scrollWidth > message.clientWidth };
        }));
        for (const row of geometry) {
          assert.ok(row.height <= 30 && row.height >= 24, JSON.stringify({ width, theme, row }));
          assert.ok(row.left >= 0 && row.right <= width, JSON.stringify({ width, row }));
          assert.equal(row.messageStyle, 'nowrap');
          assert.equal(row.ellipsis, 'ellipsis');
          assert.equal(row.messageHeight, 20);
          assert.equal(row.referenceLeft, geometry[0].referenceLeft);
        }
        assert.equal(geometry[1].clipped, true);
        assert.equal(await rows.first().locator('.problem-line').innerText(), ':2');
        assert.ok(await rows.first().locator('.problem-line').evaluate(el => el.scrollWidth <= el.clientWidth));
        await page.keyboard.press('Tab');
        await rows.first().focus();
        assert.equal(await rows.first().evaluate(el => el.matches(':focus-visible')), true);
        assert.equal(await rows.first().evaluate(el => getComputedStyle(el).outlineStyle), 'solid');
        const evidence = process.env.LATEX_CORE_ORIGIN_EVIDENCE;
        if (evidence) {
          await mkdir(evidence, { recursive: true });
          await page.screenshot({ path: `${evidence}/problems-${new URL(base).hostname}-${theme}-${width}.png` });
        }
      }
      await page.setViewportSize({ width: 1440, height: 1000 });
    }
    // Real Writer navigation, including keyboard activation and a missing location.
    await rows.nth(2).click();
    await rows.first().focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(path => document.querySelector('#currentFile').textContent === path, target.path);
    await page.waitForFunction(() => {
      const view = document.querySelector('.cm-content')?.cmTile?.root?.view;
      return view && view.state.doc.lineAt(view.state.selection.main.head).number === 2;
    });
    await rows.first().click();
    assert.equal(await page.locator('.cm-content').evaluate(el => {
      const view = el.cmTile.root.view;
      return view.state.doc.lineAt(view.state.selection.main.head).number;
    }), 2);
    console.log('PROBLEMS_PASS: zero/one/many, full data, ellipsis, ordering/counts, 375/768/1440, Light/Dark, keyboard and mouse navigation');
  } finally {
    await page.unroute(`**/api/v2/papers/${paperId}/intelligence`, route);
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`${base}/write?paper=${paperId}`);
    await page.locator('.cm-content').waitFor();
  }
}

export async function verifyCollaboration(writer, other, base, paperId, files, json) {
  const chapter = files.find(file => file.path.endsWith('/chapters/chapter1.tex'));
  const second = files.find(file => file.path.endsWith('/chapters/chapter2.tex'));
  assert.ok(chapter && second);
  const open = async (page, file) => {
    await page.getByRole('button', { name: `File actions for ${file.path}`, exact: true }).locator('..').locator('button').first().click();
    await page.waitForFunction(path => document.querySelector('#currentFile').textContent === path, file.path);
    await page.locator('.cm-content[contenteditable=true]').waitFor();
    await page.waitForFunction(() => document.querySelector('#saveStatus').dataset.state === 'synced');
  };
  const append = async (page, text) => {
    await page.locator('.cm-content').click();
    await page.keyboard.press('Control+End');
    await page.keyboard.insertText(text);
    await page.keyboard.press('Control+s');
    await page.waitForFunction(() => document.querySelector('#saveStatus').dataset.state === 'synced');
  };
  const contains = (page, text, expected = true) => page.waitForFunction(({ text, expected }) =>
    document.querySelector('.cm-content')?.textContent.includes(text) === expected, { text, expected });
  await other.page.goto(`${base}/write?paper=${paperId}`);
  await other.page.locator('.cm-content').waitFor();
  await open(writer.page, chapter);
  await open(other.page, chapter);
  await append(writer.page, '\n% origin-local-author\n');
  await contains(other.page, 'origin-local-author');
  await append(other.page, '\n% origin-remote-author\n');
  await contains(writer.page, 'origin-remote-author');
  await open(writer.page, second);
  await append(writer.page, '\n% origin-second-file\n');
  await open(writer.page, chapter);
  await writer.page.locator('#undoText').click();
  await contains(writer.page, 'origin-local-author', false);
  await contains(other.page, 'origin-local-author', false);
  await contains(writer.page, 'origin-remote-author');
  await writer.page.locator('#redoText').click();
  await contains(writer.page, 'origin-local-author');
  await open(writer.page, second);
  await writer.page.locator('#undoText').click();
  await contains(writer.page, 'origin-second-file', false);
  await writer.page.locator('#redoText').click();
  await contains(writer.page, 'origin-second-file');
  await writer.page.locator('.cm-content').press('Control+s');
  await writer.page.waitForFunction(() => document.querySelector('#saveStatus').dataset.state === 'synced');
  for (const [file, marker] of [[chapter, 'origin-local-author'], [second, 'origin-second-file']]) {
    const saved = await json(writer.ctx, `/api/v2/papers/${paperId}/files/${file.file_id}`);
    assert.ok(saved.content.includes(marker), 'Saved source must be durable');
  }
  await writer.page.reload();
  await writer.page.locator('.cm-content').waitFor();
  await open(writer.page, chapter);
  await contains(writer.page, 'origin-local-author');
  await contains(writer.page, 'origin-remote-author');
  assert.ok(writer.sockets.length && other.sockets.length);
  for (const socket of [...writer.sockets, ...other.sockets]) {
    const url = new URL(socket.url());
    assert.equal(url.host, new URL(base).host);
    assert.equal(url.protocol, base.startsWith('https:') ? 'wss:' : 'ws:');
  }
  console.log('COLLABORATION_PASS: same-origin sockets, durable save/reload, author-scoped per-file undo/redo');
}
