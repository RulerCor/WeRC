// Owned loopback transport only. No native provider, real token or user store.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {once} from 'node:events';
import {mkdir,writeFile,readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
const root=resolve(import.meta.dirname,'..');
const executable=resolve(process.argv[2]??resolve(root,'target/debug/werc-mcp-stdio.exe'));
const token='owned-stdio-automation-token-'.repeat(2);
const requests=[];
const server=createServer(async(req,res)=>{
  assert.equal(req.headers.authorization,`Bearer ${token}`);
  assert.equal(req.url,'/mcp/read');
  let body='';for await(const part of req)body+=part;
  const message=JSON.parse(body);requests.push(message);
  if(message.method==='notifications/initialized'){res.writeHead(202).end();return;}
  // Simulate a request whose effects happened but its response was lost.
  if(message.method==='owned/response-lost'){req.socket.destroy();return;}
  res.setHeader('Content-Type','application/json');
  res.end(JSON.stringify({jsonrpc:'2.0',id:message.id,result:message.method==='tools/list'?{tools:[]}:{owned:true}}));
});
server.listen(0,'127.0.0.1');await once(server,'listening');
const endpoint=`http://127.0.0.1:${server.address().port}/mcp/read`;
async function run(url,input){
  const child=spawn(executable,[],{windowsHide:true,stdio:['pipe','pipe','pipe'],env:{...process.env,WERC_MCP_URL:url,WERC_API_TOKEN:token}});
  let output='',error='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>error+=b);
  const timeout=setTimeout(()=>child.kill(),15000);
  try{
    child.stdin.on('error',()=>{});child.stdin.end(input);
    const [code]=await once(child,'close');
    return {code,error,output,rows:output.trim()?output.trim().split('\n').map(line=>JSON.parse(line)):[]};
  }finally{clearTimeout(timeout);}
}
try{
  const messages=[
    {jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'owned test',version:'1'}}},
    {jsonrpc:'2.0',method:'notifications/initialized'},
    {jsonrpc:'2.0',id:'owned-tools',method:'tools/list'},
    {jsonrpc:'2.0',id:'owned-once',method:'owned/response-lost'},
    {jsonrpc:'2.0',id:4,method:'ping'},
  ];
  const result=await run(endpoint,messages.map(m=>JSON.stringify(m)).join('\r\n')+'\r\n{invalid}\n');
  assert.equal(result.code,0);assert.equal(result.rows.length,5);
  assert.deepEqual(result.rows.map(r=>r.id),[1,'owned-tools','owned-once',4,null]);
  assert.equal(result.rows[1].result.tools.length,0);
  assert.equal(result.rows[2].error.code,-32000);
  assert.equal(result.rows[4].error.code,-32700);
  assert.equal(requests.filter(r=>r.method==='owned/response-lost').length,1,'response loss must never replay a request');
  assert.equal(result.output.includes(token),false);assert.equal(result.error.includes(token),false);
  const before=requests.length;
  const rejected=await run('http://example.invalid/mcp',JSON.stringify(messages[0])+'\n');
  assert.equal(rejected.code,1);assert.equal(requests.length,before);assert.equal(rejected.rows.length,0);
  await mkdir(resolve(root,'target'),{recursive:true});
  await writeFile(resolve(root,process.argv[3]??'target/remaining-stdio-result.json'),JSON.stringify({passed:true,sha256:createHash('sha256').update(await readFile(executable)).digest('hex'),real_service:false,native_connected:false,checks:['CRLF JSON lines','IDs preserved','notifications without reply','lost response no retry','parse error','subsequent request works','non-loopback rejected','token not printed']},null,2));
  console.log('Owned STDIO transport passed; no real service or WeChat used.');
}finally{server.closeAllConnections();await new Promise(ok=>server.close(ok));}
