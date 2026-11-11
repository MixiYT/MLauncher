const UPSTREAM = 'https://openrouter.ai/api/v1/chat/completions';
const MAX_BODY = 300 * 1024;
const json = (value, status=200, headers={}) => Response.json(value,{status,headers:{'Cache-Control':'no-store',...headers}});
const error = (message,status=400) => json({error:{message}},status,status===429?{'Retry-After':'60'}:{});
async function boundedJson(stream, max) {
  if (!stream) throw new Error('Empty body');
  const reader=stream.getReader(); const chunks=[]; let size=0;
  try { for (;;) { const {value,done}=await reader.read(); if(done)break; size+=value.byteLength; if(size>max)throw new Error('Body too large'); chunks.push(value); } }
  finally { await reader.cancel().catch(()=>{}); }
  const bytes=new Uint8Array(size); let offset=0;
  for (const chunk of chunks) {bytes.set(chunk,offset);offset+=chunk.byteLength;}
  return JSON.parse(new TextDecoder().decode(bytes));
}
export async function handle(request,env,upstreamFetch=fetch) {
  const url=new URL(request.url);
  if(url.pathname==='/health' && request.method==='GET')return json({ok:true});
  if(url.pathname!=='/v1/chat/completions')return error('Not found',404);
  if(request.method!=='POST')return error('Method not allowed',405);
  // Desktop clients do not send Origin. No browser CORS access or caller-selected upstream URL.
  if(request.headers.has('Origin'))return error('Browser requests are disabled',403);
  if(!request.headers.get('Content-Type')?.toLowerCase().startsWith('application/json'))return error('JSON required',415);
  if(!env.PER_IP || !env.TOTAL)return error('Gateway rate limits are not configured',503);
  const ip=request.headers.get('CF-Connecting-IP')||'unknown';
  if(!(await env.PER_IP.limit({key:ip})).success || !(await env.TOTAL.limit({key:'shared'})).success)return error('Gateway rate limit reached',429);
  let body; try {body=await boundedJson(request.body,MAX_BODY);}catch{return error('Invalid or oversized JSON',413);}
  const allowed=(env.ALLOWED_MODELS||'openrouter/free').split(',').map(s=>s.trim()).filter(Boolean);
  if(!allowed.includes(body.model))return error('Model is not allowed');
  if(!Array.isArray(body.messages)||body.messages.length<1||body.messages.length>20||body.messages.some(m=>!['system','user','assistant'].includes(m.role)||typeof m.content!=='string'||m.content.length>256*1024))return error('Invalid messages');
  if(!Number.isInteger(body.max_tokens)||body.max_tokens<1||body.max_tokens>4000)return error('max_tokens must be between 1 and 4000');
  let keys;try{keys=JSON.parse(env.AI_KEYS||'[]');}catch{return error('Gateway keys are not configured',503);}
  if(!Array.isArray(keys)||keys.length<1||keys.length>32||keys.some(k=>typeof k!=='string'||!k.trim()||k.length>1024))return error('Gateway keys are not configured',503);
  // One configured credential per request. Never retry with another key after a quota/rate-limit response.
  const index=crypto.getRandomValues(new Uint32Array(1))[0]%keys.length;
  try {
    const response=await upstreamFetch(UPSTREAM,{method:'POST',redirect:'error',signal:AbortSignal.timeout(75000),headers:{'Content-Type':'application/json','Authorization':`Bearer ${keys[index].trim()}`},body:JSON.stringify({model:body.model,messages:body.messages.map(({role,content})=>({role,content})),max_tokens:body.max_tokens,stream:false})});
    if(!response.ok){await response.body?.cancel(); return error('AI provider returned an error',response.status===429?429:response.status===402?503:502);}
    const result=await boundedJson(response.body,1024*1024);
    const text=result?.choices?.[0]?.message?.content;
    if(typeof text!=='string')return error('Invalid provider response',502);
    // Do not pass through provider diagnostics, headers, usage metadata or credentials.
    if(keys.some(k=>text.includes(k)))return error('Invalid provider response',502);
    return json({choices:[{message:{role:'assistant',content:text}}]});
  }catch{return error('AI provider is unavailable',502);}
}
export default {fetch:(request,env)=>handle(request,env)};
