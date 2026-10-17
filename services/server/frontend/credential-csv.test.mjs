import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const admin=readFileSync(new URL('../static/admin.js',import.meta.url),'utf8');
const code=admin.slice(admin.indexOf('function csvCell('),admin.indexOf('function downloadCredentials('));
const csv=runInNewContext(`${code}; credentialsCsv`);
const team=(name,id,role,order=null,leader=false)=>({team_name:name,team_id:id,team_key:`key-${id}`,team_role:role,writer_order:order,is_leader:leader});
const credential=(email,teams,password='Synthetic-Only-1')=>({email,teams,temporary_password:password,credential_role:'student'});
test('one-time CSV keeps compatible columns and groups deterministic Team membership without duplicating accounts',()=>{
 const a=team('Alpha','a','writer',2), b=team('Alpha','b','writer',1,true), c=team('Zulu','c','writer',1,true);
 const source=[credential('z@example.invalid',[c]),credential('shared@example.invalid',[{...a,team_role:'mentor',writer_order:null},{...c,team_role:'mentor',is_leader:false,writer_order:null}]),credential('writer3@example.invalid',[{...a,writer_order:3}]),credential('writer2@example.invalid',[a]),credential('leader@example.invalid',[{...a,writer_order:1,is_leader:true}]),credential('duplicate@example.invalid',[b]),credential('unassigned@example.invalid',[]),credential('reused@example.invalid',[a],null)];
 const out=csv(source); assert.equal(out,csv([...source].reverse()));
 const rows=out.trimEnd().split('\r\n'); assert.equal(rows[0],'email,password,role,team_name,team_id,team_key,team_role,writer_order,is_team_leader');
 assert.deepEqual(rows.slice(1).map(row=>row.split(',')[0]),['leader@example.invalid','writer2@example.invalid','writer3@example.invalid','shared@example.invalid','duplicate@example.invalid','unassigned@example.invalid','z@example.invalid','shared@example.invalid']);
 assert.equal(source.length,8); assert.ok(!out.includes('reused@example.invalid')); assert.equal(out.match(/shared@example.invalid,Synthetic-Only-1/g).length,2);
 assert.ok(out.includes(',Unassigned,,,,,false')); assert.ok(!out.includes('\r\n\r\n'));
});
test('CSV quotes commas, quotes, newlines and Unicode, and neutralizes spreadsheet formula fields',()=>{
 const out=csv([credential('user@example.invalid',[team('研究, "Team"\nsecond line','id','writer',1,true)])]);
 assert.ok(out.includes('"研究, ""Team""\nsecond line"')); assert.ok(out.includes('Synthetic-Only-1'));
 for(const field of ['=CMD()','+SUM(1)','-12','@SUM(2)','  =FORMULA']) assert.ok(csv([credential(field,[])]).includes(`'${field}`));
});

test('handoffs without membership metadata retain the original format instead of inventing Unassigned membership',()=>{
 const out=csv([{email:'account@example.invalid',temporary_password:'Synthetic-Only-1',credential_role:'student'}]);
 assert.equal(out,'email,password,role\r\naccount@example.invalid,Synthetic-Only-1,student\r\n');
});
