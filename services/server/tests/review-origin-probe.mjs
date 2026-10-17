import assert from 'node:assert/strict';
import { networkInterfaces } from 'node:os';
import { chromium, firefox } from 'playwright';
const config = JSON.parse(process.env.LATEX_CORE_REVIEW_CONFIG);
const engine = process.env.REVIEW_BROWSER === 'firefox' ? firefox : chromium;
const browser = await engine.launch({headless:true, args: engine === chromium ? ['--no-sandbox'] : []});
const host = process.env.REVIEW_HOST === 'network' ? Object.values(networkInterfaces()).flat().find(a=>a.family==='IPv4'&&!a.internal).address : (process.env.REVIEW_HOST || 'localhost');
const base = `http://${host}:${config.port}`;
const contexts = [];
async function client(cookie) {
  const ctx = await browser.newContext({viewport:{width:1440,height:1000},acceptDownloads:true}); contexts.push(ctx);
  const [name,...value]=cookie.split('='); await ctx.addCookies([{name,value:value.join('='),url:base}]);
  const page=await ctx.newPage(); page.on('pageerror',e=>console.log('BROWSER_EXCEPTION:',e.stack));
  return {ctx,page};
}
async function json(ctx,path,method='GET',data) {
  const res=await ctx.request.fetch(base+path,{method,data,headers:{Origin:base}});
  assert.ok(res.ok(),`${method} ${path}: ${res.status()} ${await res.text()}`);
  return res.json();
}
const paper=`/api/v2/papers/${config.paperId}`, review=`/api/v2/reviews/papers/${config.paperId}`;
try {
  const w=await client(config.writerCookie), m=await client(config.mentorCookie); w.page.on('dialog',d=>d.accept());
  await w.page.goto(`${base}/write?paper=${config.paperId}`); await w.page.locator('.cm-content').waitFor();
  await w.page.waitForLoadState('networkidle');
  console.log('ORIGIN:',base,await w.page.evaluate(()=>({secure:isSecureContext,subtle:!!crypto.subtle,randomUUID:typeof crypto.randomUUID})));
  await w.page.locator('#documentDetails').click();
  const panel=w.page.locator('#documentDetailsBody');
  await panel.locator('[name="field:project_title"]').fill('Synthetic Review Regression');
  await panel.locator('[name="field:submission_date"]').fill('2026-10-17');
  const saved=w.page.waitForResponse(r=>r.request().method()==='PUT'&&r.url().endsWith('/document-details'));
  await panel.getByRole('button',{name:'Save document details',exact:true}).click(); assert.equal((await saved).status(),200);
  await w.page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('metadata saved'));
  await w.page.locator('#drawerClose').click();
  await w.page.locator('#compilePaper').click();
  await w.page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compiled',null,{timeout:120000});
  await w.page.locator('#sendReview').click();
  await w.page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='In Review');
  const round=(await json(w.ctx,review+'/rounds')).current_review_round;
  console.log('LEADER_SEND_PASS:',config.paperId,round.id,round.baseline_state_hash);
  await m.page.goto(`${base}/review?paper=${config.paperId}`); await m.page.locator('.cm-content').waitFor();
  await m.page.waitForFunction(()=>!document.querySelector('#pushReview').disabled);
  await m.page.locator('#reviewFiles').getByRole('button',{name:'Full_Report_template_v1.0/thesis.tex',exact:true}).click();
  await m.page.locator('.cm-content').waitFor();
  const lines=m.page.locator('.cm-line');
  const box=await lines.nth(0).boundingBox(), end=await lines.nth(1).boundingBox();
  await m.page.mouse.move(box.x+4,box.y+box.height/2); await m.page.mouse.down();
  await m.page.mouse.move(end.x+Math.min(end.width-4,150),end.y+end.height/2,{steps:12}); await m.page.mouse.up();
  console.log('SELECTION:',await m.page.locator('#sourceSelection').textContent());
  await m.page.mouse.click(box.x+18,box.y+box.height/2,{button:'right'});
  let source=false, published=false, requests=0;
  m.page.on('request',r=>{if(r.method()==='POST'&&r.url().endsWith('/submit')) requests++;});
  try {
    await m.page.locator('#reviewPopover').waitFor({state:'visible',timeout:2000});
    await m.page.locator('#threadMessage').fill('Synthetic origin probe'); await m.page.locator('#createComment').click();
    await m.page.waitForFunction(()=>document.querySelector('#draftStatus').textContent.includes('Draft saved'));
    source=true; console.log('SOURCE_POPUP: PASS');
  } catch { console.log('SOURCE_POPUP: FAIL; popup did not open; no source-comment request sent'); }
  await m.page.keyboard.press('Escape');
  m.page.on('dialog',d=>d.accept()); await m.page.locator('#pushReview').click();
  try {await m.page.waitForFunction(()=>document.querySelector('#draftStatus').textContent==='Review submitted',null,{timeout:3000});published=true;} catch {}
  console.log('PUSH_REVIEW:',published?'PASS':'FAIL', 'publication_requests=',requests);
  const rounds=await json(m.ctx,review+'/rounds');const status=rounds.current_review_round||rounds.rounds[0];
  console.log('PERSISTED_REVIEW:',status?.status,rounds.current_review_round?.mentor_participation?.status||'no active participation');
  if(published)assert.equal((await json(w.ctx,review+'/threads')).threads.length,1);
  assert.ok(source&&published,'origin probe found a review failure');
} finally { for(const ctx of contexts) await ctx.close(); await browser.close(); }
