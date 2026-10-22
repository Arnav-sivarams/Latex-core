import assert from 'node:assert/strict';
import test from 'node:test';
import {nativeSourceRange, openSourceContextMenu} from './review-source-selection.mjs';
const start={},end={},outside={};
const view={contentDOM:{contains:node=>node===start||node===end},posAtDOM:(node,offset)=>(node===start?10:100)+offset};
test('read-only native selections retain their exact source range in either direction',()=>{
 const selection={rangeCount:1,isCollapsed:false,anchorNode:start,anchorOffset:2,focusNode:end,focusOffset:5};
 assert.deepEqual(nativeSourceRange(view,selection),{from:12,to:105,empty:false});
 assert.deepEqual(nativeSourceRange(view,{...selection,anchorNode:end,anchorOffset:5,focusNode:start,focusOffset:2}),{from:12,to:105,empty:false});
});
test('selection outside this editor, including another file or the comment textbox, cannot become an anchor',()=>{
 const selection={rangeCount:1,isCollapsed:false,anchorNode:start,anchorOffset:0,focusNode:outside,focusOffset:2};
 assert.equal(nativeSourceRange(view,selection),null);
 assert.equal(nativeSourceRange(view,{...selection,isCollapsed:true}),null);
 assert.equal(nativeSourceRange(view,{...selection,rangeCount:0}),null);
});

test('valid source context menu is prevented synchronously, before anchor work', async () => {
 const calls=[]; let resolve;
 const anchor = new Promise(done => { resolve=done; });
 const completed=openSourceContextMenu({preventDefault(){calls.push('prevent');}},()=>{calls.push('resolve');return anchor;},value=>calls.push(value),error=>{throw error;});
 assert.deepEqual(calls,['prevent']);
 await Promise.resolve(); assert.deepEqual(calls,['prevent','resolve']);
 resolve('popup'); await completed;
 assert.deepEqual(calls,['prevent','resolve','popup']);
});
test('anchor exceptions reach the actionable failure handler without opening a popup', async () => {
 for(const resolver of [()=>{throw new Error('anchor failed');},()=>Promise.reject(new Error('anchor failed'))]) {
  const calls=[];
  await openSourceContextMenu({preventDefault(){calls.push('prevent');}},resolver,()=>calls.push('popup'),error=>calls.push(error.message));
  assert.deepEqual(calls,['prevent','anchor failed']);
 }
});
