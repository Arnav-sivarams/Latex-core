import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import * as Y from 'yjs';
const writer=readFileSync(new URL('./writer.js',import.meta.url),'utf8');
const start=writer.indexOf('  async initializeOrMerge() {');
const method=writer.slice(start,writer.indexOf('\n  sendUpdate(update)',start));
function fixture() {
  let sync,storageStarted;
  const started=new Promise(resolve=>{storageStarted=resolve;});
  const synced=new Promise(resolve=>{sync=resolve;});
  const model={identity:{user_id:'fixture-user'},view:null,collaboration:null};
  const notices=[],sent=[],mounted=[];
  const context={Y,model,REMOTE_ORIGIN:Symbol('remote'),window:{setTimeout:()=>0},
    IndexeddbPersistence:class {constructor(){this.whenSynced=synced;storageStarted();} destroy(){}},
    notice:message=>notices.push(message),saveState:()=>{},
    mountEditor:text=>{mounted.push(text.toString());model.view={};}};
  const session=runInNewContext(`({${method}})`,context);
  const seed=new Y.Doc();seed.getText('source').insert(0,'baseline');
  Object.assign(session,{metadata:{document_epoch:1},initialState:Y.encodeStateAsUpdate(seed),initializing:false,doc:null,text:null,destroyed:false,
    access:'read_write',connected:true,requestedEditable:true,latex:true,localPersistenceAvailable:true,
    paper:{workspace_id:'fixture-workspace'},file:{file_id:'fixture-file'},bufferedRemote:[],
    recoverIncompatibleCache:async()=>{},resolveReady:()=>{},sendUpdate:update=>sent.push(update)});
  model.collaboration=session;
  return {session,model,seed,sync,started,notices,sent,mounted};
}
test('a reconnect during recovery storage loading merges complete handshake frames and retains the local update observer',async()=>{
  const f=fixture();const loading=f.session.initializeOrMerge();await f.started;
  assert.ok(f.session.doc);
  // connect() clears these fields while the old initialization awaits IndexedDB.
  f.session.initialState=null;f.session.metadata=null;
  f.seed.getText('source').insert(8,' remote');
  f.session.metadata={document_epoch:1};f.session.initialState=Y.encodeStateAsUpdate(f.seed);
  await f.session.initializeOrMerge();f.sync();await loading;await Promise.resolve();
  assert.equal(f.session.text.toString(),'baseline remote');assert.deepEqual(f.notices,[]);
  f.session.text.insert(f.session.text.length,' local');
  const peer=new Y.Doc();for(const update of f.sent)Y.applyUpdate(peer,update);
  assert.equal(peer.getText('source').toString(),'baseline remote local');
});
test('switching files during storage loading finishes the retained document without mounting it in another file',async()=>{
  const f=fixture();const loading=f.session.initializeOrMerge();await f.started;
  f.model.collaboration={};f.sync();await loading;
  assert.equal(f.session.text.toString(),'baseline');assert.equal(f.mounted.length,0);
  f.model.collaboration=f.session;await f.session.initializeOrMerge();
  f.session.text.insert(8,' retained');
  const peer=new Y.Doc();for(const update of f.sent)Y.applyUpdate(peer,update);
  assert.equal(peer.getText('source').toString(),'baseline retained');assert.deepEqual(f.notices,[]);
});
