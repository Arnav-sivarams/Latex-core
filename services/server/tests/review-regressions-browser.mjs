import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { networkInterfaces } from 'node:os';
import { credentialHandoff } from './credential-handoff-browser.mjs';
import { verifyServedAssets, verifyProblems, verifyCollaboration } from './origin-workflows-browser.mjs';
import { chromium, firefox } from 'playwright';
const config = JSON.parse(process.env.LATEX_CORE_REVIEW_CONFIG);
const engine = process.env.REVIEW_BROWSER === 'firefox' ? firefox : chromium;
const browser = await engine.launch({headless:true, ...(process.env.REVIEW_BROWSER_EXECUTABLE ? {executablePath:process.env.REVIEW_BROWSER_EXECUTABLE} : {}), args: engine === chromium ? ['--no-sandbox'] : []});
const host = process.env.REVIEW_HOST === 'network' ? Object.values(networkInterfaces()).flat().find(a=>a.family==='IPv4'&&!a.internal).address : (process.env.REVIEW_HOST || 'localhost');
const base = `http://${host}:${config.port}`;
const contexts = [];
const browserFailures = [];
const serverFailures = [];
const blockedRequests = [];
const assetChecks = [];
async function client(cookie) {
  const ctx = await browser.newContext({viewport:{width:1440,height:1000},acceptDownloads:true}); contexts.push(ctx);
  ctx.setDefaultTimeout(30000);
  const [name,...value]=cookie.split('='); await ctx.addCookies([{name,value:value.join('='),url:base}]);
  const sockets=[]; const page=await ctx.newPage(); page.on('websocket',socket=>sockets.push(socket)); page.on('pageerror',e=>{browserFailures.push(e.message);console.log('BROWSER_EXCEPTION:',e.stack);});page.on('response',r=>{if(r.status()>=500)serverFailures.push(`${r.status()} ${new URL(r.url()).pathname}`);});
  page.on('response',response=>{
    const name=new URL(response.url()).pathname.split('/').pop();
    if(!['writer.js','review.js'].includes(name))return;
    assetChecks.push((async()=>{
      const actual=await response.body(), expected=await readFile(new URL(`../static/${name}`,import.meta.url));
      assert.deepEqual(actual,expected,`Browser-loaded ${name}, including cached bytes, must match checkout`);
      console.log('BROWSER_ASSET_SHA256:',name,createHash('sha256').update(actual).digest('hex'));
    })().catch(error=>browserFailures.push(error.message)));
  });
  page.on('requestfailed', request => {
    // Navigations intentionally cancel superseded reads; security/network failures do not.
    if (!['net::ERR_ABORTED', 'NS_BINDING_ABORTED'].includes(request.failure()?.errorText)) blockedRequests.push(`${request.failure()?.errorText} ${request.url()}`);
  });
  return {ctx,page,sockets};
}
async function json(ctx,path,method='GET',data) {
  const res=await ctx.request.fetch(base+path,{method,data,headers:{Origin:base}});
  assert.ok(res.ok(),`${method} ${path}: ${res.status()} ${await res.text()}`);
  return res.json();
}
async function login(client,email) {
  await client.page.bringToFront();
  await client.ctx.clearCookies();await client.page.goto(base);
  await client.page.locator('#email').fill(email);await client.page.locator('#password').fill(config.password);
  await client.page.getByRole('button',{name:'Sign in',exact:true}).click();await client.page.waitForURL(/\/(home|admin|review|write)$/);
}
const paper=`/api/v2/papers/${config.paperId}`, review=`/api/v2/reviews/papers/${config.paperId}`;
try {
  const w=await client(config.writerCookie), m=await client(config.mentorCookie); w.page.on('dialog',d=>{console.log('WRITER_DIALOG:',d.type());return d.type()==='prompt'?d.accept('Synthetic Writer reply'):d.accept();});
  await login(w,config.writerEmail);await login(m,config.mentorEmail);
  await w.page.bringToFront();await w.page.goto(`${base}/write?paper=${config.paperId}`); await w.page.locator('.cm-content').waitFor();
  console.log('ORIGIN:',base,await w.page.evaluate(()=>({secure:isSecureContext,subtle:!!crypto.subtle,randomUUID:typeof crypto.randomUUID})));
  assert.equal(await w.page.evaluate(() => isSecureContext), host === 'localhost' || host === '127.0.0.1', 'LAN HTTP must run without a browser security bypass');
  await verifyServedAssets(w.ctx, base);
  if (process.env.LATEX_CORE_TEST_API_EXECUTABLE) {
    const build=await json(w.ctx,'/api/build-info');
    assert.equal(build.schema_version,1);assert.equal(build.source_revision,process.env.LATEX_CORE_TEST_SOURCE_REVISION);
    for(const name of ['review','writer','admin']) {
      const expected=createHash('sha256').update(await readFile(new URL(`../static/${name}.js`,import.meta.url))).digest('hex');
      assert.equal(build[`${name}_js_sha256`],expected);
    }
    console.log('DEPLOYED_API_BUILD_PASS:',build.source_revision,'embedded and HTTP-served assets match checkout');
  }

  if (!process.env.REVIEW_AUDIT_ONLY) await verifyProblems(w.page, base, config.paperId, await json(w.ctx, paper+'/files'));
  const other=await client(config.otherCookie);await login(other,config.otherEmail);
  if (!process.env.REVIEW_AUDIT_ONLY) await verifyCollaboration(w, other, base, config.paperId, await json(w.ctx,paper+'/files'), json);
  if (process.env.REVIEW_AUDIT_ONLY) {
    await w.page.locator('.cm-content').click();await w.page.keyboard.press('Control+End');
    await w.page.keyboard.insertText('\n% Synthetic audit acceptance source save\n');await w.page.keyboard.press('Control+s');
    await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
  }
  await w.page.locator('#documentDetails').click();
  const panel=w.page.locator('#documentDetailsBody');
  await panel.getByText('Course code: BCSE4973',{exact:true}).waitFor();assert.equal(await panel.locator('[name="field:course_code"]').count(),0);assert.equal(await panel.locator('[name="field:course_name"]').inputValue(),'Project-I');
  await panel.locator('[name="field:project_title"]').fill('Synthetic Review Regression');
  await panel.locator('[name="field:submission_date"]').fill('2026-10-17');
  const saved=w.page.waitForResponse(r=>r.request().method()==='PUT'&&r.url().endsWith('/document-details')).catch(async error=>{console.log('SAVE_BLOCKED:',await w.page.locator('#writerNotice').textContent(),await panel.locator('input:invalid').evaluateAll(nodes=>nodes.map(n=>n.name)));throw error;});
  console.log('DETAILS_READY:',await panel.locator('input:invalid').evaluateAll(nodes=>nodes.map(n=>n.name)),await w.page.locator('#saveStatus').textContent());
  await panel.getByRole('button',{name:'Save document details',exact:true}).click(); assert.equal((await saved).status(),200);
  await w.page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('metadata saved'));
  await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
  await w.page.locator('#drawerClose').click();
  await w.page.locator('#compilePaper').click();
  await w.page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compiled',null,{timeout:120000});
  await w.page.waitForFunction(()=>!document.querySelector('#downloadPdf').disabled);
  console.log('DOWNLOAD_READY:',await w.page.locator('#workspaceDrawer').isVisible(),await w.page.locator('#buildStatus').textContent());
  const [pdfDownload]=await Promise.all([w.page.waitForEvent('download'),w.page.locator('#downloadPdf').click({timeout:10000})]);assert.equal(pdfDownload.suggestedFilename(),'Review regression fixture.pdf');console.log('TEAM_PDF_DOWNLOAD_PASS');
  assert.equal(await other.page.locator('#sendReview').isVisible(),false);
  await w.page.bringToFront();await w.page.locator('#sendReview').click();
  await w.page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='In Review');
  const round=(await json(w.ctx,review+'/rounds')).current_review_round;
  console.log('LEADER_SEND_PASS:',config.paperId,round.id,round.baseline_state_hash);
  assert.equal(round.status,'OPEN_FOR_REVIEW');
  let auditAdmin;
  async function verifyAdminAudit(published) {
    if (!process.env.REVIEW_AUDIT_ONLY) return;
    if (!auditAdmin) { auditAdmin=await client(config.adminCookie);await login(auditAdmin,config.adminEmail);await auditAdmin.page.goto(base+'/admin'); }
    await auditAdmin.page.bringToFront();
    const response=auditAdmin.page.waitForResponse(r=>new URL(r.url()).pathname==='/api/admin/audit');
    await auditAdmin.page.getByRole('button',{name:'Audit',exact:true}).click();
    const received=await response;assert.equal(received.status(),200);
    const events=(await received.json()).filter(event=>event.round_id===round.id);
    assert.equal(events.length,published?2:1,'only authoritative successes appear in Audit');
    for (const [type,email,label] of [
      ['review.round.opened',config.writerEmail,'Send for Review'],
      ...(published?[['review.feedback.published',config.mentorEmail,'Review feedback published']]:[]),
    ]) {
      const matches=events.filter(event=>event.event===type);assert.equal(matches.length,1);
      const event=matches[0];assert.equal(event.paper_id,config.paperId);assert.equal(event.resource_id,round.id);
      assert.equal(event.actor,email);assert.ok(event.actor_user_id);assert.equal(event.outcome,'SUCCESS');
      assert.ok(Number.isFinite(Date.parse(event.created_at)));
      const row=auditAdmin.page.locator('#adminContent tbody tr').filter({hasText:label}).filter({hasText:round.id});
      assert.equal(await row.count(),1);await row.locator('summary').click();
      await row.getByText(`Paper/Team ID: ${config.paperId}`,{exact:true}).waitFor();
      await row.getByText(`Round ID: ${round.id}`,{exact:true}).waitFor();
      await row.getByText('Outcome: SUCCESS',{exact:true}).waitFor();
      await row.getByText(email,{exact:true}).waitFor();
    }
    console.log('ADMIN_AUDIT_UI_PASS:',published?'Leader and Mentor success, one event each':'Leader success only; no publication event');
  }
  await verifyAdminAudit(false);

  await m.page.bringToFront();await m.page.goto(`${base}/review?paper=${config.paperId}`); await m.page.locator('.cm-content').waitFor();
  await m.page.waitForFunction(()=>!document.querySelector('#pushReview').disabled);
  await m.page.locator('#reviewFiles').getByRole('button',{name:'Full_Report_template_v1.0/chapters/chapter1.tex',exact:true}).click();
  await m.page.locator('.cm-content').waitFor();
  assert.equal(await m.page.locator('.cm-content').getAttribute('contenteditable'),'false');
  const originalText=await m.page.locator('.cm-content').evaluate(el=>el.cmTile.root.view.state.doc.toString());
  const first=await m.page.locator('.cm-line').first().boundingBox();
  await m.page.mouse.move(first.x+4,first.y+first.height/2);await m.page.mouse.down();await m.page.mouse.move(first.x+120,first.y+first.height/2,{steps:8});await m.page.mouse.up();
  await m.page.mouse.click(first.x+20,first.y+first.height/2,{button:'right'});
  await m.page.locator('#reviewPopover').waitFor({state:'visible'});await m.page.locator('#cancelAnnotation').click();
  await m.page.locator('#reviewFiles').getByRole('button',{name:'Full_Report_template_v1.0/certificate.tex',exact:true}).click();await m.page.locator('.cm-content').waitFor();
  await m.page.locator('#reviewFiles').getByRole('button',{name:'Full_Report_template_v1.0/chapters/chapter1.tex',exact:true}).click();await m.page.locator('.cm-content').waitFor();
  assert.equal(await m.page.locator('#reviewPopover').isVisible(),false);
  await m.page.locator('#editorSettings summary').click();const preferences=m.page.waitForResponse(r=>r.request().method()==='PUT'&&r.url().endsWith('/preferences/editor'));await m.page.locator('#editorTheme').selectOption('DARK');assert.equal((await preferences).status(),200);await m.page.locator('#editorSettings summary').click();
  await m.page.locator('.cm-content').click();await m.page.keyboard.type('MUST_NOT_CHANGE_SOURCE');
  assert.equal(await m.page.locator('.cm-content').evaluate(el=>el.cmTile.root.view.state.doc.toString()),originalText);
  await m.page.keyboard.press('Escape');await m.page.locator('.cm-scroller').hover({position:{x:100,y:100}});
  for(let attempt=0;attempt<12;attempt++) {
    await m.page.mouse.wheel(-20000,0);await m.page.mouse.wheel(0,-20000);await m.page.waitForTimeout(100);
    if(await m.page.locator('.cm-scroller').evaluate(el=>el.scrollLeft<=1&&el.scrollTop<=1))break;
  }
  assert.ok(await m.page.locator('.cm-scroller').evaluate(el=>el.scrollLeft<=1&&el.scrollTop<=1),'Source scroll must return to visible line starts');
  const lines=m.page.locator('.cm-line');
  const box=await lines.nth(0).boundingBox(), end=await lines.nth(1).boundingBox();
  const gutter=await m.page.locator('.cm-gutters').boundingBox();const sourceLeft=Math.max(box.x+4,gutter.x+gutter.width+4);
  await m.page.mouse.move(sourceLeft,box.y+box.height/2); await m.page.mouse.down();
  await m.page.mouse.move(Math.max(end.x+Math.min(end.width-4,150),sourceLeft+30),end.y+end.height/2,{steps:12}); await m.page.mouse.up();
  const selectedQuote=await m.page.evaluate(()=>window.getSelection().toString());
  assert.ok(selectedQuote.length>0);
  console.log('SELECTION:',await m.page.locator('#sourceSelection').textContent(),await m.page.locator('.cm-content').evaluate(el=>({nativeLength:window.getSelection()?.toString().length,nativeInEditor:el.contains(window.getSelection()?.anchorNode)&&el.contains(window.getSelection()?.focusNode),editor:el.cmTile?.root?.view?.state.selection.main.toJSON()})),{box,end});
  await m.page.evaluate(()=>document.addEventListener('contextmenu',event=>{window.reviewMenuPrevented=event.defaultPrevented;}));
  // Anchor failures must suppress the native menu and report a safe code, without sending source text.
  await m.page.evaluate(()=>{
    const state=document.querySelector('.cm-content').cmTile.root.view.state;
    window.restoreReviewSlice=()=>{delete state.sliceDoc;};
    state.sliceDoc=()=>{throw new Error('Synthetic anchor resolution failure');};
  });
  const anchorReport=m.page.waitForResponse(r=>r.url().endsWith('/client-errors')&&r.request().method()==='POST');
  await m.page.mouse.click(sourceLeft+12,box.y+box.height/2,{button:'right'});
  await m.page.waitForFunction(()=>document.querySelector('#reviewNotice').textContent.includes('Cannot prepare source comment: Synthetic anchor resolution failure'));
  assert.equal(await m.page.evaluate(()=>window.reviewMenuPrevented),true);
  assert.equal(await m.page.locator('#reviewPopover').isVisible(),false);
  const anchorReported=await anchorReport;
  assert.equal(anchorReported.status(),204);
  assert.deepEqual(anchorReported.request().postDataJSON(),{schema_version:1,code:'SOURCE_ANCHOR_FAILED',round_id:round.id});
  await m.page.evaluate(()=>window.restoreReviewSlice());
  console.log('SOURCE_ANCHOR_FAILURE_PASS: real right-click, visible actionable error, native menu prevented, code-only report');
  await m.page.mouse.click(sourceLeft+12,box.y+box.height/2,{button:'right'});
  assert.equal(await m.page.evaluate(()=>window.reviewMenuPrevented),true,'actual contextmenu must be synchronously cancelled');
  await m.page.locator('#reviewPopover').waitFor({state:'visible',timeout:5000});
  const popup=await m.page.locator('#reviewPopover').boundingBox();
  assert.ok(Math.abs(popup.x-(sourceLeft+12))<40 && Math.abs(popup.y-(box.y+box.height/2))<40, 'Source popup opens at the actual selection');
  console.log('SOURCE_POPUP_PASS');
  await m.page.locator('#threadMessage').fill('Synthetic source feedback'); await m.page.locator('#createComment').click(); console.log('DRAFT_CLICKED');
  await m.page.waitForFunction(()=>document.querySelector('#draftStatus').textContent.includes('Draft saved'));
  await m.page.locator('#reviewTitle').click({button:'right'});
  assert.equal(await m.page.evaluate(()=>window.reviewMenuPrevented),false,'normal browser menu outside source selections is preserved');
  await m.page.keyboard.press('Escape');
  await m.page.reload(); await m.page.locator('.cm-content').waitFor();
  const privateSource=(await json(m.ctx,review+'/threads')).threads;
  assert.equal(privateSource.length,1);
  assert.equal(privateSource[0].source_anchor.quoted_text,selectedQuote,'Mouse selection must be stored at the exact source range');
  assert.equal((await json(w.ctx,review+'/threads')).threads.length,0);
  const overlay=m.page.locator('#pdfOverlay'); await overlay.waitFor();
  const pdf=await overlay.boundingBox();
  await m.page.mouse.move(pdf.x+60,pdf.y+60); await m.page.mouse.down(); await m.page.mouse.move(pdf.x+160,pdf.y+90,{steps:10}); await m.page.mouse.up();
  await m.page.locator('.pdf-highlight.pending').waitFor(); await m.page.locator('.pdf-highlight.pending').click({button:'right'});
  await m.page.locator('#reviewPopover').waitFor({state:'visible'});
  await m.page.locator('#threadMessage').fill('Synthetic PDF feedback'); await m.page.locator('#createComment').click(); console.log('DRAFT_CLICKED');
  await m.page.waitForFunction(()=>document.querySelector('#draftStatus').textContent.includes('Draft saved'));
  assert.equal((await json(w.ctx,review+'/threads')).threads.length,0,'Both source and PDF drafts remain private until Push Review');
  await m.page.bringToFront();m.page.on('dialog',d=>d.accept());
  const submitUrl = `${base}${review}/rounds/${round.id}/submit`;
  let rejectedSubmission;
  await m.page.route(submitUrl,async route=>{
    const body=route.request().postDataJSON();rejectedSubmission=body.submission_id;
    const response=await route.fetch({postData:JSON.stringify({...body,expected_revision:body.expected_revision+1})});
    assert.equal(response.status(),409,'actual server rejects a stale publication revision');
    await route.fulfill({response});
  },{times:1});
  const rejectionReport=m.page.waitForResponse(r=>r.url().endsWith('/client-errors')&&r.request().method()==='POST');
  const rejected=m.page.waitForResponse(r=>r.url()===submitUrl&&r.status()===409);
  await m.page.locator('#pushReview').click();await rejected;
  assert.equal((await rejectionReport).status(),204);
  await m.page.waitForFunction(()=>document.querySelector('#reviewNotice').textContent.includes('Drafts changed while submitting'));
  assert.equal((await json(w.ctx,review+'/threads')).threads.length,0,'rejected publication must leave feedback private');
  assert.equal((await json(m.ctx,review+'/rounds')).current_review_round.mentor_participation.status,'PENDING');
  await m.page.waitForFunction(()=>!document.querySelector('#pushReview').disabled);
  await verifyAdminAudit(false);await m.page.bringToFront();
  console.log('PUBLICATION_REJECTION_PASS: HTTP 409, visible error, round pending, two private drafts, zero Writer feedback');
  const requests = [];
  let committed;
  await m.page.route(submitUrl, async route => {
    requests.push(route.request().postDataJSON());
    const response = await route.fetch();
    assert.equal(response.status(), 200);
    committed = await response.json();
    // The server committed; a proxy lost the response. The next click must replay it.
    await route.fulfill({status:503, contentType:'application/json', body:JSON.stringify({error:'Synthetic lost publication response'})});
  }, {times:1});
  const lost = m.page.waitForResponse(r=>r.url()===submitUrl&&r.status()===503);
  await m.page.locator('#pushReview').click(); await lost;
  await m.page.waitForFunction(()=>document.querySelector('#reviewNotice').textContent.includes('Synthetic lost publication response'));
  const roundAfter = await json(m.ctx,review+'/rounds');
  assert.equal(roundAfter.current_review_round,null,'server committed and closed the round');
  assert.equal((await json(w.ctx,review+'/threads')).threads.length,2,'server publication is authoritative despite a lost response');
  assert.ok(!['Review published','Review submitted'].includes(await m.page.locator('#draftStatus').textContent()),'lost response must remain unconfirmed');
  await m.page.waitForFunction(()=>!document.querySelector('#pushReview').disabled);
  const retryResponse=m.page.waitForResponse(r=>r.request().method()==='POST'&&r.url()===submitUrl);
  await m.page.locator('#pushReview').click();
  const sent=await retryResponse; assert.equal(sent.status(),200);
  assert.notEqual(requests[0].submission_id,rejectedSubmission,'definite rejection permits a corrected publication identity');
  assert.deepEqual(sent.request().postDataJSON(),requests[0],'actual Push Review retry retains both revision and secure submission identity');
  const replay=await sent.json();assert.equal(replay.already_submitted,true);assert.equal(replay.submission_id,committed.submission_id);
  await m.page.waitForFunction(()=>document.querySelector('#draftStatus').textContent==='Review published');
  const expectedFailure=serverFailures.indexOf(`503 ${new URL(submitUrl).pathname}`);
  assert.ok(expectedFailure>=0);serverFailures.splice(expectedFailure,1);
  console.log('PUSH_REVIEW_RETRY_PASS:',submitUrl,'HTTP 200; same submission/revision; already_submitted=true');
  assert.equal((await json(w.ctx,review+'/threads')).threads.length,2);
  for(let count=0;count<5;count++) {
    const report=await m.ctx.request.post(base+review+'/client-errors',{headers:{Origin:base},data:{schema_version:1,code:'SOURCE_ANCHOR_FAILED',round_id:round.id}});
    assert.equal(report.status(),count<3?204:429,'anchor and publication failures consumed two of five bounded reports');
  }
  assert.equal((await w.ctx.request.post(base+review+'/client-errors',{headers:{Origin:base},data:{schema_version:1,code:'SOURCE_ANCHOR_FAILED'}})).status(),403);
  assert.equal((await m.ctx.request.post(base+review+'/client-errors',{headers:{Origin:'http://foreign.invalid'},data:{schema_version:1,code:'SOURCE_ANCHOR_FAILED'}})).status(),403);
  console.log('CLIENT_DIAGNOSTICS_PASS: authenticated Mentor, exact-origin CSRF, five reports/minute');
  await verifyAdminAudit(true);await w.page.bringToFront();
  console.log('CORE_REVIEW_PASS');
  await w.page.bringToFront();await w.page.reload(); await w.page.locator('.cm-content').waitFor();
  await w.page.locator('#commentsToggle').click();
  const sourceCard=w.page.locator('.thread-card').filter({hasText:'Synthetic source feedback'});
  await sourceCard.locator('.thread-title').click();
  await w.page.waitForFunction(()=>document.querySelector('#currentFile').textContent==='Full_Report_template_v1.0/chapters/chapter1.tex');
  await w.page.locator('.cm-content').waitFor();await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
  await w.page.waitForFunction(quote=>{const view=document.querySelector('.cm-content')?.cmTile?.root?.view;return view&&view.state.sliceDoc(view.state.selection.main.from,view.state.selection.main.to)===quote;},selectedQuote);
  await w.page.locator('.review-source-highlight').first().waitFor();
  const [reply]=await Promise.all([w.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/messages'),{timeout:15000}),sourceCard.getByRole('button',{name:'Reply',exact:true}).click({timeout:10000})]).catch(async error=>{console.log('WRITER_FEEDBACK_UI:',await w.page.locator('#writerNotice').textContent());await w.page.screenshot({path:'/tmp/latex-review-feedback.png',mask:[w.page.locator('.discussion')]});throw error;});assert.ok(reply.ok());await sourceCard.locator('.discussion').getByText('Synthetic Writer reply',{exact:false}).waitFor();
  const [done]=await Promise.all([w.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/state'),{timeout:15000}),sourceCard.getByRole('button',{name:'Done',exact:true}).click({timeout:10000})]);assert.ok(done.ok());
  assert.ok((await json(w.ctx,review+'/threads')).threads.some(t=>t.state==='RESOLVED'));
  await m.page.reload();await m.page.locator('.cm-content').waitFor();assert.equal(await m.page.locator('#pushReview').isDisabled(),true);
  const outsider=await client(config.outsiderCookie);assert.ok([403,404].includes((await outsider.ctx.request.get(base+review+'/files')).status()));
  console.log('WRITER_FEEDBACK_PASS: navigation, reply, Done, reload, idempotent publication and denied unassigned Mentor');
  if (process.env.REVIEW_AUDIT_ONLY) {
    await w.page.reload();await w.page.locator('.cm-content').waitFor();
    const persisted=(await json(w.ctx,review+'/threads')).threads;assert.equal(persisted.length,2);
    await w.page.locator('#commentsToggle').click();
    await w.page.locator('#writerReviewFilters [data-filter=all]').click();
    await w.page.locator('.thread-card').filter({hasText:'Synthetic source feedback'}).waitFor();
    await w.page.locator('.thread-card').filter({hasText:'Synthetic PDF feedback'}).waitFor();
    await verifyAdminAudit(true);
    const response=auditAdmin.page.waitForResponse(r=>new URL(r.url()).pathname==='/api/admin/v2/runtime-logs');
    await auditAdmin.page.getByRole('button',{name:'Runtime Logs',exact:true}).click();
    const runtime=await response;
    if (runtime.status()===200) {
      const logs=await runtime.json();assert.ok(Array.isArray(logs.records));assert.ok(Array.isArray(logs.services));
      for(const record of logs.records)assert.ok(logs.services.includes(record.service));
      if(logs.records.length)await auditAdmin.page.locator('.admin-log-message').first().waitFor();
      else await auditAdmin.page.getByText('No retained log records matched.',{exact:true}).waitFor();
      console.log('ADMIN_RUNTIME_LOGS_CHECK: HTTP 200; actual configured container source displayed; checked separately from durable review Audit');
    } else {
      assert.equal(runtime.status(),503);await auditAdmin.page.getByRole('alert').waitFor();
      const expectedRuntimeFailure=serverFailures.indexOf('503 /api/admin/v2/runtime-logs');
      assert.ok(expectedRuntimeFailure>=0);serverFailures.splice(expectedRuntimeFailure,1);
      console.log('ADMIN_RUNTIME_LOGS_CHECK: HTTP 503; unavailable source visibly reported, separately from durable Audit');
    }
  }

  if (process.env.REVIEW_SKIP_POLICY) process.exitCode=0;
  else {
    const admin=await client(config.adminCookie);await login(admin,config.adminEmail); await admin.page.goto(base+'/admin');
    await admin.page.getByRole('button',{name:'Paper Teams',exact:true}).click();
    const filters=admin.page.locator('form.admin-filters');
    for(const [field,value] of [['search',config.mentorEmail],['search',config.writerEmail.toUpperCase()],['mentor_email',`  ${config.mentorEmail.split('@')[0]}  `],['leader_email',config.writerEmail]]) {
      for(const name of ['search','mentor_email','leader_email'])await filters.locator(`[name="${name}"]`).fill(name===field?value:'');
      const response=admin.page.waitForResponse(r=>r.url().includes('/paper-teams/query?'));
      await filters.getByRole('button',{name:'Apply filters',exact:true}).click();
      const result=await (await response).json();assert.equal(result.total,3);assert.equal(result.items.length,3);
      await admin.page.getByRole('cell',{name:'Review regression fixture',exact:true}).waitFor();
    }
    assert.equal(await filters.getByText('Mentor user ID',{exact:true}).count(),0);
    assert.equal(await filters.getByText('Leader user ID',{exact:true}).count(),0);
    console.log('TEAM_EMAIL_SEARCH_BROWSER_PASS: generic and role-specific email filters, case and whitespace');
    await admin.page.getByRole('button',{name:'File Policies',exact:true}).click();
    const chapter='Full_Report_template_v1.0/chapters/chapter1.tex';
    const policyTeam=admin.page.getByRole('combobox',{name:'Paper Team',exact:true});
    await policyTeam.selectOption(config.paperId);
    const policy=admin.page.getByRole('combobox',{name:`Change policy for ${chapter}`,exact:true});
    const before=await json(w.ctx,paper+'/files');
    await policy.selectOption('CONTENT_READ_ONLY');
    await w.page.bringToFront();if (!await w.page.locator('#endReview').isDisabled()) await w.page.locator('#endReview').click();
    await w.page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='Draft');
    await w.page.reload(); await w.page.locator('.cm-content').waitFor();
    await w.page.locator('#documentDetailsPanel').waitFor({state:'hidden'});
    const openChapter=async()=>{await w.page.getByRole('button',{name:`File actions for ${chapter}`,exact:true}).locator('..').locator('button').first().click();await w.page.waitForFunction(path=>document.querySelector('#currentFile').textContent===path,chapter);};
    await openChapter();
    assert.equal(await w.page.locator('.cm-content').getAttribute('contenteditable'),'false');
    const compile=async(id=config.paperId)=>{await w.page.bringToFront();console.log('POLICY_COMPILE_ATTEMPT',await w.page.locator('#saveStatus').textContent(),await w.page.locator('#writerNotice').textContent());const response=w.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/builds'),{timeout:15000});await w.page.locator('#compilePaper').click();const res=await response;assert.equal(res.status(),202,await res.text());const expected=await res.json(); const deadline=Date.now()+120000;let build;do{build=(await json(w.ctx,`/api/v2/papers/${id}/builds`)).build;assert.notEqual(build.latest_status,'failed',JSON.stringify(build.latest_error));if(build.current_build_id===expected.build_id&&!build.active_build_id)break;await w.page.waitForTimeout(150);}while(Date.now()<deadline);assert.equal(build.current_build_id,expected.build_id);assert.equal(build.current_source_sequence,build.source_sequence);return build;};
    const stable=await compile();assert.equal(stable.current_state_hash,round.baseline_state_hash);
    const changed=w.sockets.at(-1).waitForEvent('framereceived',{predicate:e=>typeof e.payload==='string'&&e.payload.includes('POLICY_CHANGED')});
    await policy.selectOption('EDITABLE'); await changed;
    await w.page.locator('.cm-content[contenteditable=true]').waitFor();
    await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
    // Exercise the existing open file/session, without a page reload.
    await compile();
    const after=await json(w.ctx,paper+'/files');
    assert.deepEqual(after.map(f=>[f.file_id,f.path,f.revision]),before.map(f=>[f.file_id,f.path,f.revision]));
    await admin.page.bringToFront();await policyTeam.selectOption('ALL_TEAMS');
    admin.page.on('dialog',d=>d.accept());
    await admin.page.getByRole('combobox',{name:`Change policy for ${chapter} across All Teams`,exact:true}).selectOption('CONTENT_READ_ONLY');
    const bulk=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().endsWith('/api/admin/v2/file-policies'));
    await admin.page.getByRole('button',{name:'Apply policies to All Teams',exact:true}).click();assert.equal((await bulk).status(),200);
    await w.page.locator('.cm-content[contenteditable=false]').waitFor();
    await compile();
    for(const id of config.teamIds) assert.equal((await json(admin.ctx,`/api/admin/v2/paper-teams/${id}/file-policies`)).find(f=>f.path===chapter).policy,'CONTENT_READ_ONLY');
    for(const id of config.teamIds.slice(1)) {
      await w.page.goto(`${base}/write?paper=${id}`);await w.page.locator('.cm-content').waitFor();await w.page.waitForLoadState('networkidle');
      if(!await w.page.locator('#documentDetailsPanel').isVisible())await w.page.locator('#documentDetails').click();
      const details=w.page.locator('#documentDetailsBody');await details.locator('[name="field:project_title"]').fill('Synthetic policy Team report');await details.locator('[name="field:submission_date"]').fill('2026-10-17');
      const saved=w.page.waitForResponse(r=>r.request().method()==='PUT'&&r.url().endsWith('/document-details'));await details.getByRole('button',{name:'Save document details',exact:true}).click();assert.equal((await saved).status(),200);await w.page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('metadata saved'));if(await w.page.locator('#documentDetailsPanel').isVisible())await w.page.locator('#drawerClose').click();
      await compile(id);
    }
    await admin.page.getByRole('combobox',{name:`Change policy for ${chapter} across All Teams`,exact:true}).selectOption('EDITABLE');
    const restored=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().endsWith('/api/admin/v2/file-policies'));await admin.page.getByRole('button',{name:'Apply policies to All Teams',exact:true}).click();assert.equal((await restored).status(),200);
    for(const id of config.teamIds){await w.page.goto(`${base}/write?paper=${id}`);await w.page.locator('.cm-content').waitFor();await compile(id);}
    console.log('POLICY_COMPILE_PASS: individual read-only/restore; All Teams read-only/restore; three real report builds');
    await w.page.goto(`${base}/write?paper=${config.paperId}`);await w.page.locator('.cm-content').waitFor();await openChapter();await w.page.locator('.cm-content[contenteditable=true]').waitFor();
    await policyTeam.selectOption(config.paperId);
    const racingFile=(await json(w.ctx,paper+'/files')).find(file=>file.path===chapter);
    const overlap='\n\\par Policy save overlap content.\\par\n';
    await w.page.bringToFront();await w.page.locator('.cm-content').click();await w.page.keyboard.press('Control+End');await w.page.keyboard.insertText(overlap);
    const racedPolicy=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().includes('/file-policies/'));
    await Promise.all([w.page.keyboard.press('Control+s'),admin.page.getByRole('combobox',{name:`Change policy for ${chapter}`,exact:true}).selectOption('CONTENT_READ_ONLY')]);assert.equal((await racedPolicy).status(),200);
    await w.page.locator('.cm-content[contenteditable=false]').waitFor();
    const durableDuringPolicy=(await json(w.ctx,`${paper}/files/${racingFile.file_id}`)).content;
    assert.equal(await w.page.locator('.cm-content').evaluate(el=>el.cmTile.root.view.state.doc.toString()),durableDuringPolicy,'Read-only view must show acknowledged server source');
    const allowAgain=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().includes('/file-policies/'));await admin.page.getByRole('combobox',{name:`Change policy for ${chapter}`,exact:true}).selectOption('EDITABLE');assert.equal((await allowAgain).status(),200);await w.page.locator('.cm-content[contenteditable=true]').waitFor();
    await w.page.locator('.cm-content').click();await w.page.keyboard.press('Control+s');await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
    assert.ok((await json(w.ctx,`${paper}/files/${racingFile.file_id}`)).content.includes('Policy save overlap content.'),'Concurrent save must be durable or recover after access is restored');
    console.log('POLICY_SAVE_OVERLAP_PASS');
    await w.page.locator('.cm-content').click();await w.page.keyboard.press('Control+End');await w.page.keyboard.insertText('\n\\par Synthetic acceptance revision.\\par\n');await w.page.keyboard.press('Control+s');await w.page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
    const stale=w.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/rounds'));await w.page.locator('#sendReview').click();assert.equal((await stale).status(),409);await w.page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('Compile the latest changes'));
    const submitted=w.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/builds'));await w.page.locator('#compilePaper').click();const buildRequest=await submitted;assert.equal(buildRequest.status(),202);const inFlight=await buildRequest.json();
    await policyTeam.selectOption(config.paperId);const concurrent=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().includes('/file-policies/'));await admin.page.getByRole('combobox',{name:`Change policy for ${chapter}`,exact:true}).selectOption('CONTENT_READ_ONLY');assert.equal((await concurrent).status(),200);
    const deadline=Date.now()+120000;let completed;do{completed=(await json(w.ctx,paper+'/builds')).build;if(completed.current_build_id===inFlight.build_id)break;assert.notEqual(completed.latest_status,'failed');await w.page.waitForTimeout(200);}while(Date.now()<deadline);assert.equal(completed.current_build_id,inFlight.build_id);assert.equal(completed.current_source_sequence,completed.source_sequence);
    await w.page.locator('#sendReview').click();await w.page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='In Review');await w.page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('Current report sent for Mentor review.'));await w.page.bringToFront();await w.page.locator('.cm-content[contenteditable=false]').waitFor();const next=(await json(w.ctx,review+'/rounds')).current_review_round;assert.equal(next.round_number,2);assert.notEqual(next.baseline_state_hash,round.baseline_state_hash);
    const restoredDuringReview=admin.page.waitForResponse(r=>r.request().method()==='PATCH'&&r.url().includes('/file-policies/'));await admin.page.getByRole('combobox',{name:`Change policy for ${chapter}`,exact:true}).selectOption('EDITABLE');assert.equal((await restoredDuringReview).status(),200);await w.page.bringToFront();await w.page.locator('.cm-content[contenteditable=false]').waitFor();assert.equal((await json(w.ctx,review+'/rounds')).current_review_round.baseline_state_hash,next.baseline_state_hash);
    await w.page.bringToFront();await w.page.locator('#endReview').click();await w.page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='Draft');
    console.log('SUBSEQUENT_REVIEW_PASS: stale build denied, real edited report compiled during policy change, reviewed snapshot preserved');
    await admin.page.bringToFront();await credentialHandoff(admin,base);
  }
  await Promise.all(assetChecks);
  assert.deepEqual(browserFailures,[],'No browser runtime exceptions are permitted in the accepted flow');
  assert.deepEqual(serverFailures,[],'No server errors are permitted in the accepted flow');
  assert.deepEqual(blockedRequests,[],'No blocked network requests are permitted in the accepted flow');
} catch (error) {
  for (const ctx of contexts) for (const page of ctx.pages()) {
    console.error('ACCEPTANCE_FAILURE_STATE:', page.url(), await page.locator('#writerNotice, #reviewNotice, #saveStatus, #buildStatus, #draftStatus').allTextContents());
  }
  throw error;
} finally { for(const ctx of contexts) await ctx.close(); await browser.close(); }
