import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';

function parseCsv(text) {
  const rows=[];let row=[],cell='',quoted=false;
  for(let i=0;i<text.length;i++) {
    const ch=text[i];
    if(ch==='"') {if(quoted&&text[i+1]==='"'){cell+='"';i++;}else quoted=!quoted;}
    else if(ch===','&&!quoted){row.push(cell);cell='';}
    else if(ch==='\n'&&!quoted){row.push(cell.replace(/\r$/,''));rows.push(row);row=[];cell='';}
    else cell+=ch;
  }
  assert.equal(quoted,false);assert.equal(cell,'');return rows;
}
const quote=value=>`"${String(value).replaceAll('"','""')}"`;
const data=rows=>Buffer.from(rows.map(row=>row.map(quote).join(',')).join('\r\n')+'\r\n');

export async function credentialHandoff(admin, base) {
  const suffix=randomUUID().replaceAll('-','');
  const key=label=>`${label}${suffix}`, email=label=>`${label}-${suffix}@example.invalid`;
  const files=[
    ['students.csv',data([['reg_no','name','email'],...['a','b','c','u'].map(label=>[key(label),'Synthetic, "研究"',email(label)])])],
    ['faculty.csv',data([['faculty_id','name','email'],[key('m'),'Synthetic Mentor',email('m')]])],
    ['paper_teams.csv',data([['external_team_key','team_name','academic_year','semester','status'],[key('c'),'Zulu','2026','1','ACTIVE'],[key('b'),'Alpha, "研究"','2026','1','ACTIVE'],[key('a'),'Alpha, "研究"','2026','1','ACTIVE']])],
    ['paper_team_writers.csv',data([['external_team_key','student_reg_no','writer_order','is_leader'],...['c','b','a'].map(label=>[key(label),key(label),'1','true'])])],
    ['paper_team_mentors.csv',data([['external_team_key','faculty_id'],[key('a'),key('m')],[key('b'),key('m')]])],
  ];
  await admin.page.getByRole('button',{name:'Imports',exact:true}).click();
  await admin.page.locator('input[type=file]').setInputFiles(files.map(([name,buffer])=>({name,buffer,mimeType:'text/csv'})));
  const validated=admin.page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/import-batches/validate'));
  await admin.page.getByRole('button',{name:'Review changes',exact:true}).click();assert.equal((await validated).status(),201);
  const downloadEvent=admin.page.waitForEvent('download');
  const appliedEvent=admin.page.waitForResponse(r=>r.request().method()==='POST'&&/\/import-batches\/[^/]+\/apply$/.test(new URL(r.url()).pathname));
  await admin.page.getByRole('button',{name:'Add records',exact:true}).click();
  const applied=await appliedEvent;assert.equal(applied.status(),200);assert.equal(applied.headers()['cache-control'],'no-store');
  const permitted=(await applied.json()).account_provisioning.credentials;
  const download=await downloadEvent;const stream=await download.createReadStream();const chunks=[];for await(const chunk of stream)chunks.push(chunk);
  const csv=parseCsv(Buffer.concat(chunks).toString('utf8'));const header=csv.shift();
  assert.deepEqual(header,['email','password','role','team_name','team_id','team_key','team_role','writer_order','is_team_leader']);
  assert.equal(csv.length,6);
  const groups=new Set();let previous=null;const accountPasswords=new Map();
  for(const row of csv) {
    const item=permitted.find(item=>item.email===row[0]);assert.ok(item,'Export must contain only authorized one-time rows');
    assert.ok(row[1]===item.temporary_password,'CSV must preserve the same account password');
    if(accountPasswords.has(row[0]))assert.ok(accountPasswords.get(row[0])===row[1],'Shared account password changed');
    accountPasswords.set(row[0],row[1]);
    const group=row[4]||'Unassigned';if(group!==previous){assert.ok(!groups.has(group),'Team rows must be contiguous');groups.add(group);previous=group;}
  }
  assert.equal(csv.filter(row=>row[0]===email('m')).length,2);assert.ok(csv.some(row=>row[0]===email('u')&&row[3]==='Unassigned'));
  for(const [username,password] of accountPasswords) {
    const login=await admin.ctx.request.post(base+'/login',{form:{email:username,password},maxRedirects:0});
    assert.equal(login.status(),303);assert.equal(login.headers().location,'/change-password');
  }
  console.log('TEAM_GROUPED_CSV_BROWSER_PASS: actual Admin import, one-time download, scope, grouping, quoting, shared credentials and forced-change login');
}
