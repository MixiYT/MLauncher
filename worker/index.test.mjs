import test from 'node:test';
import assert from 'node:assert/strict';
import {handle} from './index.mjs';
const env=()=>({AI_KEYS:JSON.stringify(['test-key-1','test-key-2']),ALLOWED_MODELS:'openrouter/free',PER_IP:{limit:async()=>({success:true})},TOTAL:{limit:async()=>({success:true})}});
const req=(body={},headers={})=>new Request('https://gateway.example/v1/chat/completions',{method:'POST',headers:{'Content-Type':'application/json',...headers},body:JSON.stringify({model:'openrouter/free',messages:[{role:'user',content:'Create a build'}],max_tokens:100,...body})});
test('forwards only allowed payload and keeps credentials on server',async()=>{
 let sent;
 const result=await handle(req({extra:'discard'}),env(),async(url,init)=>{sent={url,init};return Response.json({choices:[{message:{content:'OK'}}],private:'discard'});});
 assert.equal(result.status,200);assert.equal((await result.json()).choices[0].message.content,'OK');
 assert.equal(sent.url,'https://openrouter.ai/api/v1/chat/completions');assert.match(sent.init.headers.Authorization,/^Bearer test-key-[12]$/);assert.equal(JSON.parse(sent.init.body).extra,undefined);
});
test('validates input, rejects browser use and enforces rate limits',async()=>{
 const never=()=>{throw new Error('must not fetch');};
 assert.equal((await handle(req({model:'paid-model'}),env(),never)).status,400);
 assert.equal((await handle(req({}, {Origin:'https://other.site'}),env(),never)).status,403);
 assert.equal((await handle(req({max_tokens:4001}),env(),never)).status,400);
 assert.equal((await handle(req({messages:[{role:'user',content:'x'.repeat(310000)}]}),env(),never)).status,413);
 const limited=env();limited.PER_IP.limit=async()=>({success:false});assert.equal((await handle(req(),limited,never)).status,429);
});
test('does not rotate keys on upstream quota errors or leak diagnostics',async()=>{
 let calls=0;
 const result=await handle(req(),env(),async()=>{calls++;return new Response('test-key-1 confidential',{status:429});});
 assert.equal(calls,1);assert.equal(result.status,429);assert.ok(!(await result.text()).includes('test-key'));
 const leaked=await handle(req(),env(),async()=>Response.json({choices:[{message:{content:'test-key-1'}}]}));assert.equal(leaked.status,502);
});
import worker from './index.mjs';
test('Cloudflare execution context is not mistaken for fetch implementation',async()=>{
 const response=await worker.fetch(new Request('https://gateway.example/health'),env(),{waitUntil(){}});
 assert.equal(response.status,200);
});
