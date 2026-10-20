import assert from 'node:assert/strict';
import test from 'node:test';
import {createHash} from 'node:crypto';
import {digest} from 'lib0/hash/sha256';
import {uuidv4} from 'lib0/random';
import {readFileSync} from 'node:fs';
test('review compatibility digest is exact SHA-256 for Unicode and empty context',()=>{
 for(const text of ['', 'selected source', '研究\r\n\\chapter{One}', '😀\0'.repeat(100), 'x'.repeat(10000)]) assert.equal(Buffer.from(digest(new TextEncoder().encode(text))).toString('hex'),createHash('sha256').update(text).digest('hex'));
});
test('review anchors and publication retain the LAN-compatible cryptographic helpers',()=>{
 const review=readFileSync(new URL('./review.js',import.meta.url),'utf8');
 assert.match(review,/import \{ digest \} from 'lib0\/hash\/sha256'/);
 assert.match(review,/import \{ uuidv4 \} from 'lib0\/random'/);
 assert.match(review,/digest\(new TextEncoder\(\)\.encode\(value\)\)/);
 assert.match(review,/model\.pendingSubmissionId \|\| uuidv4\(\)/);
 assert.doesNotMatch(review,/crypto\.(?:subtle\.|randomUUID\()|Math\.random\(/);
});
test('review operation IDs use secure version 4 UUIDs',()=>{
 const ids=new Set(Array.from({length:128},()=>uuidv4())); assert.equal(ids.size,128);
 for(const id of ids) assert.match(id,/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});
