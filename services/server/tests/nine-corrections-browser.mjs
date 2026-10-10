import assert from 'node:assert/strict';
import { chromium } from 'playwright';

const config = JSON.parse(process.env.LATEX_CORE_NINE_CONFIG);
const browser = await chromium.launch({headless:true, args:['--no-sandbox'], executablePath:process.env.PLAYWRIGHT_CHROMIUM_PATH || chromium.executablePath()});
let passed = 0;
const startedAt = Date.now();
async function check(name, run) { await run(); passed++; console.log(`NINE_BROWSER_PASS: ${name}`); }
async function context(cookie) {
  const ctx = await browser.newContext({acceptDownloads:true}); ctx.setDefaultTimeout(30000);
  if (cookie) { const [name,...value]=cookie.split('='); await ctx.addCookies([{name,value:value.join('='),url:config.base}]); }
  return ctx;
}
async function json(ctx,path,method='GET',data) {
  const response = await ctx.request.fetch(config.base+path,{method,data,headers: method==='GET'?{}:{'Content-Type':'application/json','Origin':config.base}});
  assert.ok(response.ok(), `${method} ${path}: ${response.status()} ${await response.text()}`);
  return response.status()===204?null:response.json();
}
const paperRoot=`/api/v2/papers/${config.paperId}`;
const reviewRoot=`/api/v2/reviews/papers/${config.paperId}`;
try {
  await check('login: unknown account, wrong password, keyboard retry and password clearing', async()=>{
    const ctx=await context(); const page=await ctx.newPage(); await page.goto(config.base);
    for (const email of ['unknown-nine@example.invalid',config.writerEmail]) {
      await page.locator('#email').fill(email); await page.locator('#password').fill('incorrect-non-sensitive-password');
      const reply=page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/login');
      await page.locator('#password').press('Enter'); assert.equal((await reply).status(),401);
      await page.waitForLoadState('domcontentloaded');
      assert.equal(await page.locator('#email').inputValue(),email);
      assert.equal(await page.locator('#password').inputValue(),'');
      assert.equal(await page.locator('#loginError').getAttribute('role'),'alert');
      assert.equal(await page.locator('#loginError').innerText(),'Invalid email or password.');
      assert.equal(await page.locator('#loginError').evaluate(el=>getComputedStyle(el).color),'rgb(180, 35, 24)');
    }
    await page.locator('#password').fill(config.password); await page.locator('#password').press('Enter');
    await page.waitForURL(/\/home/); assert.equal(await page.locator('#loginError').count(),0); await ctx.close();
  });
  const writer=await context(config.writerCookie); const page=await writer.newPage();
  const files=await json(writer,paperRoot+'/files'); const main=files.find(f=>f.path==='main.tex');
  const detail=await json(writer,paperRoot);
  const added=await json(writer,paperRoot+'/files','POST',{path:'notes.tex',version:detail.version,content:'% File B\n'});
  let b=added.file;
  await page.goto(`${config.base}/write?paper=${config.paperId}`); await page.locator('.cm-content').waitFor();
  const text=()=>page.locator('.cm-content').innerText();
  const open=async file=>{ await page.getByRole('button',{name:`File actions for ${file.path}`,exact:true}).locator('..').locator('button').first().click(); await page.waitForFunction(path=>document.querySelector('#currentFile')?.textContent===path,file.path); await page.locator('.cm-content').waitFor(); };
  const append=async value=>{await page.locator('.cm-content').click(); await page.keyboard.press('Control+End'); await page.keyboard.insertText(value);};
  await check('per-file undo/redo survives navigation and durable saves', async()=>{
    await append('\n% history-A'); await page.keyboard.press('Control+s');
    await page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced');
    await open(b); await append('\n% history-B'); await open(main);
    assert.match(await text(),/history-A/); await page.locator('.cm-content').click(); await page.keyboard.press('Control+z');
    assert.doesNotMatch(await text(),/history-A/); await page.keyboard.press('Control+y');
    await page.waitForFunction(()=>document.querySelector('.cm-content').textContent.includes('history-A'));
    assert.match(await text(),/history-A/);
    await page.keyboard.press('Control+z'); await page.keyboard.press('Control+Shift+Z');
    await page.waitForFunction(()=>document.querySelector('.cm-content').textContent.includes('history-A'));
    await open(b); assert.match(await text(),/history-B/); await page.locator('#undoText').click(); assert.doesNotMatch(await text(),/history-B/);
    await page.locator('#redoText').click(); assert.match(await text(),/history-B/); await open(main);
  });
  await check('Insert disables package-dependent actions in a report without those packages',async()=>{
    await page.locator('#insertMenu').click();
    for (const name of ['Plot','Wrap figure','Algorithm','Code block','Theorem']) {
      const action=page.locator('#dialogBody button').filter({hasText:new RegExp(`^${name}`)});
      assert.equal(await action.isDisabled(),true,name); assert.match(await action.innerText(),/Requires package|Environment not detected/);
    }
    assert.equal(await page.getByRole('button',{name:'Itemized list',exact:true}).isEnabled(),true);
    await page.keyboard.press('Escape');
  });
  await check('collaboration undo is scoped to the local author after repeated file switches',async()=>{
    await append('\n% isolated-local-author');
    const other=await context(config.otherCookie); const otherPage=await other.newPage();
    await otherPage.goto(`${config.base}/write?paper=${config.paperId}`); await otherPage.locator('.cm-content').waitFor();
    await otherPage.locator('.cm-content').click(); await otherPage.keyboard.press('Control+End'); await otherPage.keyboard.insertText('\n% isolated-remote-author');
    await page.waitForFunction(()=>document.querySelector('.cm-content').textContent.includes('isolated-remote-author'));
    for (let i=0;i<3;i++) { await open(b); await open(main); }
    await page.locator('#undoText').click();
    assert.doesNotMatch(await text(),/isolated-local-author/); assert.match(await text(),/isolated-remote-author/);
    await otherPage.waitForFunction(()=>!document.querySelector('.cm-content').textContent.includes('isolated-local-author'));
    await otherPage.locator('#undoText').click();
    await page.waitForFunction(()=>!document.querySelector('.cm-content').textContent.includes('isolated-remote-author'));
    await other.close();
  });
  await check('deleted and recreated files receive independent fresh undo history',async()=>{
    await open(b); await append('\n% deleted-file-history'); await page.keyboard.press('Control+s');
    await page.waitForFunction(()=>document.querySelector('#saveStatus').dataset.state==='synced'); await open(main);
    const before=await json(writer,paperRoot);
    await json(writer,`${paperRoot}/files/${b.file_id}`,'DELETE',{version:before.version});
    await page.getByRole('button',{name:'File actions for notes.tex',exact:true}).waitFor({state:'detached'});
    const after=await json(writer,paperRoot);
    const replacement=await json(writer,paperRoot+'/files','POST',{path:'notes.tex',version:after.version,content:'% recreated notes\n'});
    assert.notEqual(replacement.file.file_id,b.file_id); b=replacement.file;
    await page.locator('#teamPapers').getByRole('button',{name:config.teamName,exact:true}).click();
    await open(b); await page.locator('#undoText').click(); assert.equal((await text()).trim(),'% recreated notes');
    await append('% new-file-history'); await page.locator('#undoText').click(); assert.doesNotMatch(await text(),/new-file-history|deleted-file-history/);
    await open(main); assert.match(await text(),/history-A/);
  });
  await check('dark syntax has distinct AA colors; light mode remains usable',async()=>{
    await page.locator('#editorSettings summary').click(); await page.locator('#editorTheme').selectOption('DARK');
    await page.locator('#editorSettings summary').click();
    await page.locator('.cm-content').click(); await page.keyboard.press('Control+Home'); await page.keyboard.insertText('% syntax sample\n\\newcommand{\\institution}{2026}\n\\input{notes}\n');
    await page.waitForFunction(()=>[...document.querySelectorAll('.cm-content span')].some(el=>el.textContent.includes('newcommand')));
    await page.locator('.cm-content').click(); await page.keyboard.press('Control+a');
    await page.locator('.cm-selectionBackground').first().waitFor();
    const colors=await page.locator('.cm-content').evaluate(el=>{
      function rgb(s){return [...s.matchAll(/[\d.]+/g)].map(m=>+m[0]).slice(0,3);}
      function lum(c){return c.map(x=>{x/=255;return x<=.04045?x/12.92:((x+.055)/1.055)**2.4;}).reduce((a,x,i)=>a+x*[.2126,.7152,.0722][i],0);}
      const editor=el.closest('.cm-editor');
      const bases=[getComputedStyle(editor).backgroundColor,getComputedStyle(editor.querySelector('.cm-selectionBackground')).backgroundColor].map(background=>lum(rgb(background)));
      return [...el.querySelectorAll('span')].filter(s=>s.textContent.trim()).map(s=>{const color=getComputedStyle(s).color;const l=lum(rgb(color));return {text:s.textContent,color,contrast:Math.min(...bases.map(base=>(Math.max(l,base)+.05)/(Math.min(l,base)+.05)))};});
    });
    assert.ok(colors.some(c=>c.text.includes('newcommand'))); assert.ok(new Set(colors.map(c=>c.color)).size>=3);
    for (const c of colors) assert.ok(c.contrast>=4.5,JSON.stringify(c));
    await page.locator('#editorSettings summary').click(); await page.locator('#editorTheme').selectOption('LIGHT'); await page.locator('#editorSettings summary').click();
    assert.equal(await page.locator('html').getAttribute('data-theme'),'light');
  });
  await check('compile spinner is compact, allows file/PDF interaction, and stops on success',async()=>{
    await page.locator('#compilePaper').click(); await page.locator('#compileSpinner').waitFor({state:'visible'});
    const box=await page.locator('#compileSpinner').evaluate(el=>({width:el.offsetWidth,height:el.offsetHeight})); assert.ok(box.width>=14&&box.width<=16&&box.height>=14&&box.height<=16);
    await page.locator('#pdfZoom').selectOption('125'); await open(b); await page.locator('.cm-content').click(); await open(main);
    await page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compiled',{},{timeout:120000});
    assert.equal(await page.locator('#compileSpinner').isVisible(),false);
    await page.emulateMedia({reducedMotion:'reduce'});
    assert.equal(await page.locator('#compileSpinner').evaluate(el=>getComputedStyle(el).animationName),'none');
  });
  let good=(await json(writer,paperRoot+'/builds')).build.current_build_id;
  await check('PDF download uses actual Team name and unchanged authorized PDF bytes',async()=>{
    const response=await writer.request.get(config.base+paperRoot+'/artifacts/pdf?download=true'); assert.equal(response.status(),200);
    assert.match(response.headers()['content-disposition'],/filename\*=UTF-8''Smart%20Energy%20Monitoring%20/);
    const downloadPromise=page.waitForEvent('download'); await page.locator('#downloadPdf').click(); const download=await downloadPromise;
    assert.equal(download.suggestedFilename(),config.teamName+'.pdf');
    const {readFile}=await import('node:fs/promises'); assert.deepEqual(await readFile(await download.path()),await response.body());
    assert.equal((await writer.request.get(config.base+paperRoot+'/source.zip')).status(),403);
  });
  await check('review failures are actionable; successful review is authoritative and authorized',async()=>{
    await append('\n% needs-new-review-build'); await page.locator('#sendReview').click();
    await page.waitForFunction(()=>document.querySelector('#writerNotice').textContent.includes('Compile the latest changes'));
    assert.equal((await json(writer,reviewRoot+'/rounds')).review_open,false);
    await page.locator('#compilePaper').click(); await page.locator('#compileSpinner').waitFor({state:'visible'}); await page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compiled'&&document.querySelector('#compileSpinner').hidden,{},{timeout:120000});
    await page.locator('#sendReview').click(); await page.waitForFunction(()=>document.querySelector('#reviewStateBadge').textContent==='In Review');
    const rounds=await json(writer,reviewRoot+'/rounds'); assert.equal(rounds.review_open,true); assert.equal(rounds.current_review_round.status,'OPEN_FOR_REVIEW');
    const mentor=await context(config.mentorCookie); assert.equal((await json(mentor,reviewRoot+'/rounds')).review_open,true); await mentor.close();
    const outsider=await context(config.outsiderCookie); assert.ok([403,404].includes((await outsider.request.get(config.base+reviewRoot+'/rounds')).status())); await outsider.close();
    const other=await context(config.otherCookie); assert.equal((await other.request.post(config.base+reviewRoot+'/rounds',{data:{},headers:{Origin:config.base}})).status(),403); await other.close();
  });
  // End review through the authorized API without an unattended confirmation dialog.
  const openRound=(await json(writer,reviewRoot+'/rounds')).current_review_round;
  if (openRound) await json(writer,`${reviewRoot}/rounds/${openRound.id}/close`,'POST',{});
  await page.reload(); await page.locator('.cm-content').waitFor();
  await check('compile failure stops animation and preserves last successful PDF',async()=>{
    good=(await json(writer,paperRoot+'/builds')).build.current_build_id;
    await page.locator('.cm-content').click(); await page.keyboard.press('Control+Home'); await page.keyboard.insertText('\\NonexistentNineCorrectionCommand\n'); await page.locator('#compilePaper').click();
    await page.locator('#compileSpinner').waitFor({state:'visible'});
    await page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compilation failed',{},{timeout:120000});
    assert.equal(await page.locator('#compileSpinner').isVisible(),false);
    assert.equal((await json(writer,paperRoot+'/builds')).build.current_build_id,good);
  });
  const admin=await context(config.adminCookie); const adminPage=await admin.newPage(); await adminPage.goto(config.base+'/admin');
  const openTeam=async()=>{await adminPage.getByLabel('Search Teams',{exact:true}).fill(config.teamName);await adminPage.getByLabel('Search Teams',{exact:true}).press('Enter');await adminPage.getByLabel(`Actions for ${config.teamName}`,{exact:true}).click();await adminPage.getByRole('button',{name:'View / Manage',exact:true}).click();};
  await check('Admin Edit Team: responsive controls, reordering, Leader, Mentors, save and reopen',async()=>{
    await adminPage.getByRole('button',{name:'Paper Teams',exact:true}).click();
    await openTeam();
    await adminPage.locator('details > summary').filter({hasText:/^Edit Team$/}).click();
    const form=adminPage.locator('#adminDialogBody .manual-team-form'); const rows=form.locator('.ordered-person');
    for (const width of [375,768,1440]) {
      await adminPage.setViewportSize({width,height:900});
      const failures=await rows.evaluateAll(rows=>rows.flatMap(row=>{
        const email=row.querySelector('span').getBoundingClientRect();return [...row.querySelectorAll('button')].filter(b=>{const r=b.getBoundingClientRect();return (Math.min(email.right,r.right)>Math.max(email.left,r.left)&&Math.min(email.bottom,r.bottom)>Math.max(email.top,r.top))||r.right>innerWidth||r.left<0;}).map(b=>b.textContent);
      })); assert.deepEqual(failures,[],`viewport ${width}`);
    }
    await adminPage.evaluate(()=>{document.documentElement.style.zoom='1.25';});
    const writers=form.locator('.people-builder').first();
    const writerEmail=await writers.locator('.ordered-person > span').last().innerText();
    await writers.locator('.ordered-person').last().getByRole('button',{name:/Remove /}).click();
    await writers.locator('input[type=search]').fill(writerEmail);
    await writers.locator('select option').filter({hasText:writerEmail}).waitFor({state:'attached'});
    const writerValue=await writers.locator('select option').filter({hasText:writerEmail}).getAttribute('value');
    await writers.locator('select').selectOption(writerValue); await writers.getByRole('button',{name:'Add',exact:true}).click();
    await writers.getByRole('button',{name:/Move .* down/}).first().click();
    const teamLeader=form.getByLabel('Team Leader'); const choices=await teamLeader.locator('option').evaluateAll(options=>options.map(o=>o.value).filter(Boolean)); await teamLeader.selectOption(choices[0]);
    const mentors=form.locator('.people-builder').last();
    const mentorEmail=await mentors.locator('.ordered-person > span').innerText();
    await mentors.getByRole('button',{name:/Remove /}).click(); await mentors.locator('input[type=search]').fill(mentorEmail);
    await mentors.locator('select option').filter({hasText:mentorEmail}).waitFor({state:'attached'});
    const mentorValue=await mentors.locator('select option').filter({hasText:mentorEmail}).getAttribute('value'); await mentors.locator('select').selectOption(mentorValue); await mentors.getByRole('button',{name:'Add',exact:true}).click();
    await form.getByRole('button',{name:'Save Team',exact:true}).click(); await adminPage.locator('#adminDialog').waitFor({state:'hidden'});
    const saved=await json(admin,`/api/admin/v2/paper-teams/${config.paperId}`); assert.equal(saved.members.filter(m=>m.role==='mentor').length,1); assert.equal(saved.members.find(m=>m.is_leader).user_id,choices[0]);
    await openTeam(); await adminPage.locator('details > summary').filter({hasText:/^Edit Team$/}).click(); assert.equal(await adminPage.locator('#adminDialogBody .ordered-person').count(),3);
  });
  await check('Runtime Logs: Admin reads actual container events; Writer is denied',async()=>{
    const response=await admin.request.get(config.base+'/api/admin/v2/runtime-logs'); assert.equal(response.status(),200);
    const data=await response.json(); assert.ok(data.records.length>0); assert.ok(data.services.includes('worker'));
    if (config.requireFreshRuntimeLogs) assert.ok(data.records.some(record=>Date.parse(record.timestamp)>=startedAt&&record.service==='worker'),'No newly generated worker event reached Runtime Logs');
    assert.equal((await writer.request.get(config.base+'/api/admin/v2/runtime-logs')).status(),403);
    console.log(config.requireFreshRuntimeLogs?'NINE_LOGS_FRESH: newly generated worker events verified':'NINE_LOGS_LIMIT: retained actual container events verified; fresh deployed events require deployment acceptance');
  });
  console.log(`NINE_BROWSER_TOTAL: ${passed} passed; 0 failed`);
} finally { await browser.close(); }
