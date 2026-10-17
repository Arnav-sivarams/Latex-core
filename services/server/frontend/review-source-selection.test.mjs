import assert from 'node:assert/strict';
import test from 'node:test';
import {nativeSourceRange} from './review-source-selection.mjs';
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
