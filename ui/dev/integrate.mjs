/* Development only: integration run of the real UI against the real backend.
   Starts `target\release\kurasi.exe serve` (or the exe named by KURASI_EXE) on a free port with a
   temporary app-data folder (KURASI_HOME) and this ui folder (KURASI_UI_DIR),
   drives headless Chrome over the DevTools protocol, talks to the MCP server over stdio through
   ui/dev/mcp_client.py, checks PDF files with ui/dev/pdfcheck.py, and prints PASS or FAIL per check.
   Nothing outside the temporary folder is written, except screenshots in ui/dev/shots/integration.
   Usage: node integrate.mjs [scenario ...]
     KURASI_INT_ROOT keeps the temporary folder at a fixed place.
     KURASI_UI_DIR serves another copy of the ui folder (a snapshot taken while ui\ is being edited). */
import fs from 'node:fs';
import os from 'node:os';
import http from 'node:http';
import zlib from 'node:zlib';
import net from 'node:net';
import path from 'node:path';
import readline from 'node:readline';
import {spawn,execSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const UI=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const REPO=path.resolve(UI,'..');
const PY=path.join(REPO,'.venv','Scripts','python.exe');
const EXE=process.env.KURASI_EXE||path.join(REPO,'target','release','kurasi.exe');
const SERVE_UI=path.resolve(process.env.KURASI_UI_DIR||UI);
if(!fs.existsSync(path.join(SERVE_UI,'index.html'))){console.error('No index.html in '+SERVE_UI);process.exit(2);}
if(!fs.existsSync(EXE)){console.error('No kurasi.exe at '+EXE);process.exit(2);}
const CHROME='C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const SHOTS=path.join(UI,'dev','shots','integration');fs.mkdirSync(SHOTS,{recursive:true});
const ROOT=process.env.KURASI_INT_ROOT||fs.mkdtempSync(path.join(os.tmpdir(),'kurasi-int-'));
const HOME=path.join(ROOT,'home'),COPY=path.join(ROOT,'copy');fs.mkdirSync(HOME,{recursive:true});
/* isolation guard: never the real app data, never OneDrive */
const REAL=path.join(process.env.LOCALAPPDATA||'',"Kurasi").toLowerCase();
if(path.resolve(HOME).toLowerCase()===REAL||/^d:\\onedrive/i.test(path.resolve(ROOT))){console.error('Refusing to run: the temporary folder points at real data.');process.exit(2);}
console.log('exe',EXE,'| ui',SERVE_UI,'| temporary folder',ROOT);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const results=[];
function ok(name,cond,detail=''){results.push({name,pass:!!cond,detail});console.log(`${cond?'PASS':'FAIL'}  ${name}${detail!==''?'  ('+detail+')':''}`);return !!cond;}

/* ---------- backend ---------- */
function freePort(from){return new Promise(res=>{const s=net.createServer();s.once('error',()=>res(freePort(from+1)));s.listen(from,'127.0.0.1',()=>{s.close(()=>res(from));});});}
let server=null,PORT=0,BASE='';
async function startServer(){PORT=await freePort(7461);
 server=spawn(EXE,['serve','--port',String(PORT)],{cwd:ROOT,env:{...process.env,KURASI_HOME:HOME,KURASI_UI_DIR:SERVE_UI},stdio:['ignore','pipe','pipe']});
 const log=fs.createWriteStream(path.join(ROOT,'serve.log'),{flags:'a'});server.stdout.pipe(log);server.stderr.pipe(log);
 BASE=`http://127.0.0.1:${PORT}/`;for(let i=0;i<120;i++){try{const r=await fetch(BASE+'api/state');if(r.ok)return;}catch(e){}await sleep(250);}throw new Error('server did not start');}
async function stopServer(){if(!server)return;const s=server;server=null;spawn('taskkill',['/PID',String(s.pid),'/T','/F'],{stdio:'ignore'});await new Promise(r=>{s.once('exit',r);setTimeout(r,4000);});await sleep(300);}
async function api(method,p,body){const r=await fetch(BASE+p.replace(/^\//,''),{method,headers:body?{'Content-Type':'application/json'}:{},body:body?JSON.stringify(body):undefined});const t=await r.text();try{return JSON.parse(t);}catch(e){return t;}}
function rawGet(p,enc){return new Promise((res,rej)=>{const req=http.get(BASE+p.replace(/^\//,''),{headers:enc?{'Accept-Encoding':enc}:{}},r=>{const c=[];r.on('data',d=>c.push(d));r.on('end',()=>res({status:r.statusCode,headers:r.headers,body:Buffer.concat(c)}));});req.on('error',rej);});}
function py(args,env={}){return new Promise((res,rej)=>{const p=spawn(PY,args,{cwd:ROOT,env:{...process.env,KURASI_HOME:HOME,...env}});let out='',err='';p.stdout.on('data',d=>out+=d);p.stderr.on('data',d=>err+=d);p.on('exit',c=>c===0?res(out):rej(new Error(err||('exit '+c))));});}
const pdf=async rel=>JSON.parse(await py([path.join(UI,'dev','pdfcheck.py'),path.join(DEMO,rel)]));
async function poll(fn,ms=8000){const t0=Date.now();let v;while(Date.now()-t0<ms){try{v=await fn();if(v)return v;}catch(e){}await sleep(200);}return v;}
const near=(a,b,t=1.5)=>a.length===b.length&&a.every((v,i)=>Math.abs(v-b[i])<=t);

/* ---------- MCP client over stdio ---------- */
function mcpStart(folder){const p=spawn(PY,[path.join(UI,'dev','mcp_client.py'),folder,'kurasi-integration'],{cwd:ROOT,env:{...process.env,KURASI_HOME:HOME}});
 const rl=readline.createInterface({input:p.stdout});const q=[];let waiting=null;rl.on('line',l=>{let m;try{m=JSON.parse(l);}catch(e){return;}if(waiting){const w=waiting;waiting=null;w(m);}else q.push(m);});
 let err='';p.stderr.on('data',d=>err+=d);
 const next=ms=>new Promise((res,rej)=>{if(q.length){res(q.shift());return;}const t=setTimeout(()=>{waiting=null;rej(new Error('mcp timeout '+err.slice(-400)));},ms);waiting=m=>{clearTimeout(t);res(m);};});
 return {proc:p,next,call:async(tool,args)=>{p.stdin.write(JSON.stringify({tool,args})+'\n');return next(60000);},quit:async()=>{p.stdin.write('{"quit":true}\n');try{await next(10000);}catch(e){}p.kill();}};}

/* ---------- Chrome over DevTools ---------- */
const cport=9300+Math.floor(Math.random()*500);
const chrome=spawn(CHROME,['--headless=new',`--remote-debugging-port=${cport}`,'--remote-allow-origins=*',`--user-data-dir=${path.join(ROOT,'chrome')}`,'--no-first-run','--no-default-browser-check','--disable-extensions','--window-size=1536,864','about:blank'],{stdio:'ignore'});
let target=null;for(let i=0;i<240&&!target;i++){await sleep(250);try{const list=await (await fetch(`http://127.0.0.1:${cport}/json`)).json();target=list.find(t=>t.type==='page');}catch(e){}}
if(!target){console.error('Chrome did not start within 60 s');try{execSync(`taskkill /PID ${chrome.pid} /T /F`,{stdio:'ignore'});}catch(e){}process.exit(1);}
const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
let mid=0;const pend=new Map();const logs=[];const reqs=[];const resps=[];
const hdr=(h,k)=>{for(const [n,v] of Object.entries(h||{}))if(n.toLowerCase()===k)return String(v);return '';};
ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.id&&pend.has(m.id)){const {res,rej}=pend.get(m.id);pend.delete(m.id);m.error?rej(new Error(m.error.message)):res(m.result);return;}
 if(m.method==='Runtime.consoleAPICalled'&&['error','warning','warn'].includes(m.params.type)){logs.push(`[${m.params.type}] `+m.params.args.map(a=>a.value??a.description??'').join(' '));}
 if(m.method==='Runtime.exceptionThrown'){const d=m.params.exceptionDetails;logs.push(`[exception] ${d.exception&&d.exception.description||d.text} @${d.url||''}:${d.lineNumber}`);}
 if(m.method==='Log.entryAdded'){const x=m.params.entry;if(x.level==='error')logs.push(`[log] ${x.text} ${x.url||''}`);}
 if(m.method==='Network.requestWillBeSent')reqs.push(m.params.request.url);
 if(m.method==='Network.responseReceived'){const r=m.params.response;resps.push({url:r.url,status:r.status,type:r.mimeType,enc:hdr(r.headers,'content-encoding')});}};
const send=(method,params={})=>new Promise((res,rej)=>{const id=++mid;pend.set(id,{res,rej});ws.send(JSON.stringify({id,method,params}));});
await send('Page.enable');await send('Runtime.enable');await send('Log.enable');await send('Network.enable');
await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});
async function nav(q=''){await send('Page.navigate',{url:BASE+q});await sleep(150);}
async function ev(expr){const r=await send('Runtime.evaluate',{expression:`(async()=>{${expr}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error('eval: '+(r.exceptionDetails.exception&&r.exceptionDetails.exception.description||r.exceptionDetails.text));return r.result.value;}
async function shot(name){const r=await send('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(SHOTS,name+'.png'),Buffer.from(r.data,'base64'));}
async function waitFor(expr,ms=10000){const t0=Date.now();while(Date.now()-t0<ms){try{if(await ev(`return !!(${expr})`))return true;}catch(e){}await sleep(120);}return false;}
async function mouse(type,x,y,o={}){await send('Input.dispatchMouseEvent',{type,x,y,button:o.button||'left',buttons:o.buttons??(type==='mouseReleased'?0:1),clickCount:1,modifiers:o.modifiers||0,...(o.extra||{})});}
/* element given by a JS expression; returns its centre */
async function centre(expr){return ev(`const e=(${expr});if(!e)return null;e.scrollIntoView({block:'nearest'});const r=e.getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);}
async function clickEl(expr,label){if(!/toast/.test(expr))await ev(`document.querySelectorAll('.toast').forEach(t=>t.remove())`);const p=await centre(expr);if(!p){ok('element present: '+(label||expr),false);return false;}await mouse('mousePressed',p[0],p[1]);await mouse('mouseReleased',p[0],p[1]);return true;}
async function rclickEl(expr,label){await ev(`document.querySelectorAll('.toast').forEach(t=>t.remove())`);const p=await centre(expr);if(!p){ok('element present: '+(label||expr),false);return false;}await mouse('mousePressed',p[0],p[1],{button:'right',buttons:2});await mouse('mouseReleased',p[0],p[1],{button:'right',buttons:0});await sleep(150);return true;}
const q=s=>`document.querySelector(${JSON.stringify(s)})`;
const byText=(sel,re)=>`[...document.querySelectorAll(${JSON.stringify(sel)})].find(e=>${re}.test(e.textContent))`;
const click=(sel)=>clickEl(q(sel),sel);
const clickMenu=re=>clickEl(byText('.menu .mi',re),'menu item '+re);
async function typeInto(expr,text){await ev(`const e=(${expr});e.focus();if(e.select)e.select();`);await send('Input.insertText',{text});await sleep(60);}
async function key(k,code,vk){await send('Input.dispatchKeyEvent',{type:'keyDown',key:k,code,windowsVirtualKeyCode:vk});await send('Input.dispatchKeyEvent',{type:'keyUp',key:k,code,windowsVirtualKeyCode:vk});}
const enter=()=>key('Enter','Enter',13);
async function drag(a,b,steps=6){await mouse('mousePressed',a[0],a[1]);for(let i=1;i<=steps;i++)await mouse('mouseMoved',a[0]+(b[0]-a[0])*i/steps,a[1]+(b[1]-a[1])*i/steps);await sleep(60);await mouse('mouseReleased',b[0],b[1]);await sleep(250);}
async function waitToast(re,ms=6000){const t0=Date.now();while(Date.now()-t0<ms){const t=await ev(`return [...document.querySelectorAll('.toast')].map(t=>t.textContent).join(' | ')`);if(re.test(t))return {ms:Date.now()-t0,text:t};await sleep(100);}return null;}
const refByTitle=re=>`S.refs.find(r=>${re}.test(r.title))`;
async function refIdOf(re){return ev(`const r=${refByTitle(re)};return r&&r.id;`);}
async function openRef(re){const id=await refIdOf(re);await ev(`openDoc(${JSON.stringify(id)})`);await waitFor(`V.get(${JSON.stringify(id)})`);return id;}
let DEMO='';

const SC={
/* first start: splash overlay, 2 s minimum, welcome dialog with no project */
async fresh(){await startServer();await nav();const seen=[];let gone=null;const t0=Date.now();
 for(let i=0;i<120&&gone==null;i++){const s=await ev(`const sp=document.getElementById('splash');return [!!sp,(document.getElementById('splashStatus')||{}).textContent||'',performance.now()-(typeof Boot!=='undefined'?Boot.t0:0)];`).catch(()=>null);if(s){if(s[0]&&!seen.includes(s[1]))seen.push(s[1]);if(!s[0])gone=s[2];}if(i===3)await shot('i00_splash_overlay');await sleep(100);}
 ok('splash overlay shown at start',seen.length>0,seen.join(' > '));ok('overlay held for at least 2.0 s',gone!=null&&gone>=1950,Math.round(gone)+' ms');
 ok('welcome dialog with no project',await waitFor(`modalEl&&/Welcome/.test(document.querySelector('.modal .mhead').textContent)`));await shot('i01_welcome');},
/* sample project: real thumbnails and the catalogue that scrolls without pages */
async demo(){await ev(`closeModal(true);await startDemo({reset:false})`);ok('sample project open',await waitFor(`S.project&&S.project.id==='demo'&&document.querySelector('.card.ref')`,30000));
 DEMO=(await api('GET','api/state')).project.folder;const refs=await api('GET','api/references');
 const thumbs=await waitFor(`(()=>{const im=[...document.querySelectorAll('.card.ref img.timg')];return im.length&&im.every(i=>i.complete&&i.naturalWidth>0);})()`,15000);
 const tinfo=await ev(`const im=[...document.querySelectorAll('.card.ref img.timg')];return im.length+' images, first '+(im[0]&&im[0].naturalWidth+'x'+im[0].naturalHeight)`);
 ok('catalogue thumbnails decoded from /api/thumb',thumbs,tinfo);
 const pg=await ev(`const g=document.querySelector('.grid');const t=document.querySelector('.cathead .pinfo').textContent;const first=g.querySelectorAll('.card').length;for(let i=0;i<50&&g.querySelectorAll('.card').length<${refs.length};i++){g.scrollTop=g.scrollHeight;await new Promise(r=>setTimeout(r,50));}const all=g.querySelectorAll('.card').length;g.scrollTop=0;return {t,first,all,oy:getComputedStyle(g).overflowY,pager:!!document.querySelector('.pager')};`);
 ok('catalogue scrolls without pages',pg.t===`${refs.length} references`&&pg.first>0&&pg.all===refs.length&&pg.oy==='auto'&&!pg.pager,`${pg.t}; ${pg.first} cards first, ${pg.all} after scrolling`);
 await shot('i02_catalogue');},
async tabs(){const authors=await api('GET','api/authors'),tags=await api('GET','api/tags');
 await click('#tab-authors');await sleep(300);const na=await ev(`return document.querySelectorAll('.card.author').length`);ok('Authors tab cards',na===authors.length,`${na} cards, ${authors.length} from /api/authors`);await shot('i03_authors');
 await clickEl(byText('.card.author','/Taylor/'),'Taylor author card');await sleep(300);ok('author card filters Reference',await ev(`return S.tab==='reference'&&S.f.authors.has('Taylor, F. W.')&&document.querySelectorAll('.card.ref').length===1`));
 /* count the canvases drawn after the Tag tab opens: the first draw fits the view and saves it, and the
    page must not lay the graph out again because of its own settings save */
 await ev(`clearFilters();window.__gdraw=[];window.__gt0=performance.now();window.__gobs&&__gobs.disconnect();window.__gobs=new MutationObserver(ms=>{for(const m of ms)for(const n of m.addedNodes)if(n.nodeName==='svg'&&n.parentElement&&n.parentElement.classList.contains('gbox'))__gdraw.push(Math.round(performance.now()-__gt0));});__gobs.observe(document.getElementById('main'),{childList:true,subtree:true});`);
 const seq0=(await api('GET','api/changes?since=0&timeout=0')).seq;await click('#tab-tags');ok('Tag graph drawn',await waitFor(`document.querySelector('.gbox svg .node.tag')`,15000));
 await sleep(4000);const draws=await ev(`__gobs.disconnect();return __gdraw`);const selfCh=((await api('GET',`api/changes?since=${seq0}&timeout=0`)).changes||[]).map(c=>c.what);
 ok('Tag graph drawn once after the tab opens',draws.length===1,`canvas drawn at ${draws.join(', ')} ms; changes since the tab opened: ${selfCh.join(' | ')||'none'}`);
 const nt=await ev(`return document.querySelectorAll('.gbox .node.tag').length`);ok('graph tag nodes match /api/tags',nt===tags.length,`${nt} of ${tags.length}`);await shot('i04_tag_graph');
 const evo=byText('.gbox .node.tag','/^#?evolution$/')+'.querySelector(".mark")';
 const hit=await ev(`const m=${evo};if(!m)return 'no node';const r=m.getBoundingClientRect();const e=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);const n=e&&e.closest('.node');return (n?n.textContent:(e?e.tagName+'.'+e.getAttribute('class'):'nothing'))+' at '+Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);`);
 await clickEl(evo,'evolution tag node');const gclick=await waitFor(`S.tab==='reference'&&S.f.tags.has('evolution')&&document.querySelectorAll('.card.ref').length===1`,3000);
 ok('graph click filters Reference',gclick,gclick?'':await ev(`return 'under the pointer: ${hit.replace(/'/g,'')}; tab '+S.tab+', tags ['+[...S.f.tags]+'], authors ['+[...S.f.authors]+'], refs ['+[...S.f.refs]+'], '+document.querySelectorAll('.card.ref').length+' cards'`));await ev(`clearFilters()`);},
async filters(){const refs=await api('GET','api/references');await ev(`clearFilters();switchTab('reference')`);await sleep(200);
 const econ=refs.filter(r=>r.categories.some(c=>c==='Economics'||c.startsWith('Economics/'))).length;
 await clickEl(`[...document.querySelectorAll('#filter .trow')].find(r=>r.textContent.replace(/\\d+$/,'').trim()==='Economics').querySelector('input')`,'Economics checkbox');await sleep(300);
 const n1=await ev(`return document.querySelectorAll('.card.ref').length`);ok('category filter',n1===econ,`${n1} cards, ${econ} expected`);await ev(`clearFilters()`);
 await clickEl(`[...document.querySelectorAll('#filter .trow')].find(r=>/^#evolution/.test(r.textContent.trim())).querySelector('input')`,'#evolution checkbox');await sleep(300);
 ok('tag filter',await ev(`const c=[...document.querySelectorAll('.card.ref .ctitle')];return c.length===1&&/origin of species/i.test(c[0].textContent)`));await ev(`clearFilters()`);
 await typeInto(q('#q'),'scientfic managment');await sleep(500);const top=await ev(`const c=document.querySelector('.card.ref .ctitle');return c&&c.textContent`);ok('loose search tolerates typos',/scientific management/i.test(top||''),top);await shot('i05_loose_search');await ev(`setQuery('')`);},
async edit(){const id=await refIdOf('/leisure class/i');await ev(`setQuery('leisure')`);await sleep(300);
 await rclickEl(byText('.card.ref','/leisure class/i'),'Veblen card');await clickMenu('/Edit metadata/');ok('edit pane opens',await waitFor(q('#f-pages_range')));
 await typeInto(q('#f-pages_range'),'1-400');reqs.length=0;await click('#right .rfoot .btn.pri');await sleep(900);
 const rec=await api('GET','api/reference/'+id);ok('edit pane save sets Verified',rec.status==='Verified'&&rec.pages_range==='1-400',`status ${rec.status}, pages ${rec.pages_range}`);
 await sleep(2500);const lists=reqs.filter(u=>/\/api\/references(\?|$)/.test(u)).map(u=>u.replace(BASE,''));
 ok('edit pane save refreshes only that reference',lists.length>0&&lists.every(u=>u==='api/references?ids='+encodeURIComponent(id)),lists.join(', '));
 ok('card dot turns green',await waitFor(`(()=>{const c=${byText('.card.ref','/leisure class/i')};return c&&c.querySelector('.dot.v');})()`));await shot('i06_edit_saved');await ev(`closeRight();setQuery('')`);},
async deleteUndo(){const id=await refIdOf('/political economy/i');const before=(await api('GET','api/references')).length;await ev(`setQuery('political economy')`);await sleep(300);
 await rclickEl(byText('.card.ref','/political economy/i'),'Mill card');await clickMenu('/Delete reference/');await sleep(200);reqs.length=0;await click('.modal .btn.danger');await sleep(700);
 const mid=await api('GET','api/references');ok('reference deleted',mid.length===before-1&&!mid.some(r=>r.id===id),`${before} -> ${mid.length}`);
 await sleep(2500);const dl=reqs.filter(u=>/\/api\/references(\?|$)/.test(u)).map(u=>u.replace(BASE,''));
 ok('delete refreshes only that reference',dl.length>0&&dl.every(u=>u==='api/references?ids='+encodeURIComponent(id))&&await ev(`return !S.byId.has(${JSON.stringify(id)})&&S.refs.length===${before-1}`),dl.join(', '));
 await waitFor(byText('.toast button','/^Undo$/'));reqs.length=0;const u=await clickEl(byText('.toast button','/^Undo$/'),'Undo in the toast');const after=await poll(async()=>{const a=await api('GET','api/references');return a.length===before&&a;})||await api('GET','api/references');const r=after.find(x=>x.id===id);
 ok('Undo restores the reference',u&&r&&after.length===before&&r.hl_count===2,`${after.length} references, ${r&&r.hl_count} highlights`);
 await sleep(1500);const ul=reqs.filter(u=>/\/api\/references(\?|$)/.test(u)).map(u=>u.replace(BASE,''));
 ok('Undo refreshes the whole list',ul.includes('api/references')&&await ev(`return S.byId.has(${JSON.stringify(id)})&&S.refs.length===${before}`),ul.join(', '));await ev(`setQuery('')`);},
/* full-text search: the page scores the metadata and /api/search the highlight text; the merged scores and the
   ranking must equal those of the former page-side search over every field, loose and strict */
async fulltext(){await ev(`clearFilters();switchTab('reference');setQuery('')`);const refs=await api('GET','api/references');
 ok('catalogue list leaves out search_text',refs.length>0&&refs.every(r=>!('search_text' in r)&&!('slug' in r)&&!('usability' in r)),Object.keys(refs[0]||{}).length+' fields per reference');
 const full={};for(const r of refs)full[r.id]=(await api('GET','api/reference/'+encodeURIComponent(r.id))).search_text||'';
 ok('full record keeps search_text',Object.values(full).some(t=>t.length>0),Object.values(full).filter(Boolean).length+' references with highlight text');
 await ev(`window.__full=${JSON.stringify(full)}`);
 /* a word only the highlight text holds, a typo of it and a query that mixes it with an author */
 const pick=await ev(`const meta=S.refs.map(r=>refFields(r).map(f=>f.low).join(' ')).join(' ');
  for(const r of S.refs){const ws=[...new Set((__full[r.id]||'').toLowerCase().split(/[^\\p{L}\\p{N}-]+/u))].filter(w=>w.length>=8&&!meta.includes(w)&&S.refs.every(x=>score(x,w)===0)).sort();if(ws.length)return {id:r.id,word:ws[0],author:r.authors[0].split(/[ ,]/)[0].toLowerCase()};}return null;`);
 if(!ok('a word found only in the highlight text',pick,pick&&`${pick.word} in ${pick.id}`))return;
 /* a keystroke shows the metadata matches at once and says the highlight text is being searched */
 reqs.length=0;const first=await ev(`setQuery(${JSON.stringify(pick.word.slice(0,2))});setQuery(${JSON.stringify(pick.word.slice(0,4))});setQuery(${JSON.stringify(pick.word)});return [document.querySelectorAll('.card.ref').length,document.getElementById('main').textContent];`);
 ok('metadata matches at once, highlight text pending',first[0]===0&&/Searching the highlight text/.test(first[1]),first[1].slice(0,80));
 const found=await waitFor(`FT.key===${JSON.stringify(pick.word+'|true')}&&[...document.querySelectorAll('.card.ref')].length>0`,6000);
 const sreq=reqs.filter(u=>/\/api\/search\?/.test(u)).map(u=>u.replace(BASE,''));
 ok('full-text matches added after the answer',found&&await ev(`return visibleRefs().some(r=>r.id===${JSON.stringify(pick.id)})`),await ev(`return visibleRefs().length+' references'`));
 ok('three quick keystrokes send one search request',sreq.length===1,sreq.join(', '));
 const typo=pick.word.slice(0,3)+pick.word.slice(4);
 const queries=[pick.word,typo,pick.author+' '+pick.word,'division labour','scientfic managment','evolution',pick.word.slice(0,5),'xylophone'];
 const bad=[];const stats=[];
 for(const L of [true,false])for(const qq of queries){const r=await ev(`S.settings.loose=${L};const q=${JSON.stringify(qq)};setQuery(q);const key=q+'|'+looseOn();const t0=performance.now();
   while(FT.key!==key&&performance.now()-t0<6000)await new Promise(r=>setTimeout(r,20));
   const toks=q.toLowerCase().split(/\\s+/).filter(Boolean),thr=looseOn()?0.42:0.72;
   const old=r=>{const fs=[...refFields(r),{low:(__full[r.id]||'').toLowerCase(),w:1,words:null}];let s=0;for(const t of toks){let b=0;for(const f of fs){const x=fieldSim(t,f,thr)*f.w;if(x>b)b=x;}if(b===0)return 0;s+=b;}return s;};
   const diff=[];for(const r of S.refs){const a=old(r),b=score(r,q);if(Math.abs(a-b)>1e-9)diff.push(r.id+' '+a.toFixed(4)+'/'+b.toFixed(4));}
   const was=S.refs.filter(r=>old(r)>0).sort((a,b)=>old(b)-old(a)).map(r=>r.id).join(),now=sorted(visibleRefs()).map(r=>r.id).join();
   return {arrived:FT.key===key,diff,same:was===now,n:visibleRefs().length};`);
  stats.push(`${L?'loose':'strict'} "${qq}" ${r.n}`);if(!r.arrived||r.diff.length||!r.same)bad.push(`${L?'loose':'strict'} "${qq}": ${r.arrived?'':'no answer; '}${r.diff.slice(0,3).join(', ')}${r.same?'':' ranking differs'}`);}
 await ev(`S.settings.loose=true;setQuery('')`);
 ok('full-text scores and ranking equal the former page-side search',bad.length===0,bad.length?bad.join(' | '):stats.join(', '));},
/* a change from elsewhere that names a reference fetches that reference only; tags and the open highlights follow */
async refresh(){const id=await refIdOf('/origin of species/i');const hl=(await api('GET','api/highlights?ref_id='+encodeURIComponent(id)))[0];
 if(!ok('a highlight to edit',hl,id))return;await ev(`openDoc(${JSON.stringify(id)})`);await waitFor(`S.hl.get(${JSON.stringify(id)})`);
 reqs.length=0;const tags=[...hl.tags,'refreshcheck'];await api('PUT','api/highlights/'+encodeURIComponent(hl.id),{analysis:hl.analysis,label:hl.label||'',tags,verbatim:hl.verbatim});
 const seen=await waitFor(`S.tagRows.some(t=>t.name==='refreshcheck'&&t.ref_ids.includes(${JSON.stringify(id)}))&&S.refTags.get(${JSON.stringify(id)}).has('refreshcheck')&&S.hl.get(${JSON.stringify(id)}).some(h=>h.tags.includes('refreshcheck'))`,8000);
 const lists=reqs.filter(u=>/\/api\/references(\?|$)/.test(u)).map(u=>u.replace(BASE,''));
 ok('highlight change from elsewhere: tags and open highlights follow',seen,await ev(`return S.tagRows.length+' tags'`));
 ok('highlight change from elsewhere refreshes that reference only',lists.length>0&&lists.every(u=>u==='api/references?ids='+encodeURIComponent(id)),lists.join(', '));
 reqs.length=0;await api('PUT','api/highlights/'+encodeURIComponent(hl.id),{analysis:hl.analysis,label:hl.label||'',tags:hl.tags,verbatim:hl.verbatim});
 ok('tag removed again',await waitFor(`!S.tagRows.some(t=>t.name==='refreshcheck')`,8000));
 await ev(`closeTab(${JSON.stringify(id)});switchTab('reference')`);},
/* gzip for JSON and text bodies, none for page images */
async gzip(){const list=resps.filter(r=>/\/api\/references$/.test(r.url));
 ok('Chrome receives the catalogue list gzip-compressed',list.length>0&&list.every(r=>r.enc==='gzip'),list.map(r=>r.enc||'none').join(', '));
 const js=resps.filter(r=>/\/app\.js(\?|$)/.test(r.url));ok('Chrome receives app.js gzip-compressed',js.length>0&&js.every(r=>r.enc==='gzip'),js.map(r=>r.enc||'none').join(', '));
 const png=resps.filter(r=>/\/api\/thumb\/|\/page\/\d+\.png/.test(r.url));ok('thumbnails and page images are sent as they are',png.length>0&&png.every(r=>!r.enc),png.length+' images, '+[...new Set(png.map(r=>r.enc||'none'))].join(', '));
 const a=await rawGet('api/references','gzip, deflate, br'),b=await rawGet('api/references',null);let same=false;try{same=zlib.gunzipSync(a.body).equals(b.body);}catch(e){}
 ok('raw request: gzip on request, plain without it',a.headers['content-encoding']==='gzip'&&!b.headers['content-encoding']&&same&&/accept-encoding/i.test(a.headers.vary||''),`${b.body.length} bytes plain, ${a.body.length} gzip, vary ${a.headers.vary||'none'}`);
 const refs=JSON.parse(b.body.toString('utf8'));const f=refs.map(r=>r.files.find(x=>x.ext==='pdf')).find(Boolean);
 const pg=await rawGet(`api/file/${encodeURIComponent(f.id)}/page/1.png?scale=1`,'gzip');ok('raw request: page PNG without content-encoding',pg.status===200&&!pg.headers['content-encoding']&&pg.body.slice(1,4).toString()==='PNG',`${pg.headers['content-type']}, ${pg.body.length} bytes`);
 const sr=await rawGet('api/search?q=labour&loose=1','gzip');let sj=null;try{sj=JSON.parse((sr.headers['content-encoding']==='gzip'?zlib.gunzipSync(sr.body):sr.body).toString('utf8'));}catch(e){}
 ok('raw request: /api/search answers JSON',sr.status===200&&sj&&Array.isArray(sj.tokens)&&Array.isArray(sj.hits),sj?`${sj.hits.length} hits, ${sr.headers['content-encoding']||'plain'}`:String(sr.status));},
async category(){await clickEl(`document.querySelector('#filter a.fclear[title^="Create a category"]')`,'New category link');await sleep(200);await typeInto(q('#cat-name'),'Methods');await click('.modal .btn.pri');await sleep(800);
 const cats=await api('GET','api/categories');ok('category created',cats.some(c=>c.name==='Methods'&&c.parent_id==null),cats.map(c=>c.name).join(', '));
 ok('filter tree shows the category',await ev(`return [...document.querySelectorAll('#filter .trow')].some(r=>/^Methods/.test(r.textContent.trim()))`));},
/* PDF: lazy pages, drag highlight written into the file, edit, delete, Undo, region, rotation */
async pdf(){const rel='management\\taylor_1911_principles-of-scientific-management.pdf';const id=await refIdOf('/scientific management/i');
 reqs.length=0;await ev(`switchTab('reference');setQuery('scientific')`);await sleep(300);await clickEl(byText('.card.ref','/scientific management/i'),'Taylor card');
 ok('PDF pages render',await waitFor(`V.get(S.tab)&&V.get(S.tab).pages.length&&V.get(S.tab).pages[0].img.complete&&V.get(S.tab).pages[0].img.naturalWidth>0`,15000));await sleep(1200);
 const lazy=await ev(`const v=V.get(S.tab);return [v.pages.length,v.pages.filter(p=>p.loaded).length];`);const pngReq=reqs.filter(u=>/\/page\/\d+\.png/.test(u)).length;
 ok('pages load lazily',lazy[1]>0&&lazy[1]<lazy[0],`${lazy[1]} of ${lazy[0]} pages drawn, ${pngReq} page requests`);
 await ev(`const v=V.get(S.tab);v.sc.scrollTop=v.sc.scrollHeight;`);ok('scrolling loads the last page',await waitFor(`(()=>{const v=V.get(S.tab);return v.pages[v.pages.length-1].loaded>0;})()`,8000));
 ok('page URL carries the cache-buster',reqs.some(u=>/\/page\/\d+\.png\?scale=[\d.]+&v=\d+r\d+/.test(u)),reqs.filter(u=>/\.png\?/.test(u))[0]||'');
 await ev(`V.get(S.tab).pages[0].box.scrollIntoView({block:'start'})`);await sleep(600);await click('.vtb .tool[data-tool="hl"]');ok('words of page 1 drawn',await waitFor(`V.get(S.tab).pages[0].wordsDrawn`));
 /* the longest line on page 1: drag from its first word to its last */
 const line=await ev(`const pg=V.get(S.tab).pages[0];const L=pg.words.list;const rows=new Map();L.forEach((w,i)=>{const k=Math.round((w[1]+w[3])/2);(rows.get(k)||rows.set(k,[]).get(k)).push(i);});const best=[...rows.values()].sort((a,b)=>b.length-a.length)[0];const a=best[0],b=best[Math.min(best.length-1,5)];pg.wordsEl.children[a].scrollIntoView({block:'center'});await new Promise(r=>setTimeout(r,400));const sa=pg.wordsEl.children[a].getBoundingClientRect(),sb=pg.wordsEl.children[b].getBoundingClientRect();return {a,b,words:L.slice(a,b+1).map(w=>w[4]).join(' '),p0:[sa.left+1.5,sa.top+sa.height/2],p1:[sb.right-1.5,sb.top+sb.height/2]};`);
 await drag(line.p0,line.p1);ok('drag opens the highlight dialog',await waitFor(q('#hd-verb')));const verb=await ev(`return document.getElementById('hd-verb').value`);ok('verbatim prefilled from the words',verb===line.words,verb);await shot('i07_pdf_dialog');
 await typeInto(q('#hd-ana'),'(I) Integration test highlight.');await typeInto(q('.modal .ktags input'),'integration');await enter();await sleep(100);await click('.modal .btn.pri');
 let hls;let h=await poll(async()=>(hls=await api('GET','api/highlights?ref_id='+id)).find(x=>x.analysis==='(I) Integration test highlight.'));await sleep(500);
 ok('highlight saved by the API',h&&h.author==='You'&&h.origin==='human'&&h.page===1&&h.label==='I'&&JSON.stringify(h.tags)==='["integration"]',h?`${h.id} quads ${JSON.stringify(h.quads)}`:'missing');
 let doc=await pdf(rel);let an=doc.pages[0].annots.find(a=>/Integration test highlight/.test(a.content));
 ok('annotation written into the PDF',an&&an.type==='Highlight'&&an.title==='You'&&an.content==='(I) Integration test highlight.\n\nTags: #integration',an?JSON.stringify(an.content):'none');
 ok('annotation quads equal the highlight quads',an&&h&&an.rects.length===h.quads.length&&an.rects.every((r,i)=>near(r,h.quads[i])),an&&JSON.stringify(an.rects));
 ok('overlay drawn in the human colour',await ev(`const e=[...document.querySelectorAll('.hl[data-h="${h&&h.id}"]')];return e.length>0&&e.every(x=>!x.classList.contains('ai'))`));await shot('i08_pdf_after_save');
 /* edit in the knowledge pane */
 await clickEl(`[...document.querySelectorAll('.kitem[data-h="${h.id}"] .kfoot .btn')].find(b=>b.textContent==='Edit')`,'Edit button');await sleep(200);
 await typeInto(`document.querySelector('.kitem[data-h="${h.id}"] textarea[aria-label=Analysis]')`,'(I) Integration test highlight, edited.');await click(`.kitem[data-h="${h.id}"] .kfoot .btn.pri`);await sleep(1000);
 doc=await pdf(rel);an=doc.pages[0].annots.find(a=>/Integration test highlight/.test(a.content));ok('edit rewrites the annotation note',an&&an.content==='(I) Integration test highlight, edited.\n\nTags: #integration',an&&JSON.stringify(an.content));
 /* delete, then Undo from the toast */
 const nAnn=doc.pages[0].annots.length;await click(`.kitem[data-h="${h.id}"] .khead button[title="Delete highlight"]`);await sleep(200);await click('.modal .btn.danger');hls=await poll(async()=>{const l=await api('GET','api/highlights?ref_id='+id);return !l.some(x=>x.id===h.id)&&l;})||await api('GET','api/highlights?ref_id='+id);
 const delCopy=path.join(ROOT,'deleted-state.pdf');fs.copyFileSync(path.join(DEMO,rel),delCopy);await waitFor(byText('.toast button','/^Undo$/'));const u=await clickEl(byText('.toast button','/^Undo$/'),'Undo in the toast');
 doc=JSON.parse(await py([path.join(UI,'dev','pdfcheck.py'),delCopy]));ok('delete removes the annotation from the PDF',!doc.pages[0].annots.some(a=>/Integration test highlight/.test(a.content))&&doc.pages[0].annots.length===nAnn-1);
 ok('delete removes the highlight row',!hls.some(x=>x.id===h.id));
 hls=await poll(async()=>{const l=await api('GET','api/highlights?ref_id='+id);return l.some(x=>/Integration test highlight, edited/.test(x.analysis))&&l;})||[];await sleep(300);doc=await pdf(rel);an=doc.pages[0].annots.find(a=>/Integration test highlight/.test(a.content));
 hls=await api('GET','api/highlights?ref_id='+id);const h2=hls.find(x=>/Integration test highlight, edited/.test(x.analysis));
 ok('Undo restores the annotation and the row',u&&an&&h2&&an.rects.every((r,i)=>near(r,h2.quads[i]))&&an.title==='You',h2&&h2.id);
 ok('restored highlight drawn again',await waitFor(`document.querySelector('.hl[data-h="${h2&&h2.id}"]')`,4000));
 /* region from an empty spot below the words of page 1 */
 const reg=await ev(`const v=V.get(S.tab),pg=v.pages[0];const bottom=Math.max(...pg.words.list.map(w=>w[3]));const y0=Math.min(pg.h-120,bottom+40),x0=90;const b=pg.box.getBoundingClientRect();pg.box.scrollIntoView({block:'end'});return {y0,x0,h:pg.h};`);await sleep(400);
 const rp=await ev(`const v=V.get(S.tab),pg=v.pages[0];const b=pg.box.getBoundingClientRect();return [b.left+${reg.x0}*v.z,b.top+${reg.y0}*v.z,b.left+${reg.x0+160}*v.z,b.top+${reg.y0+60}*v.z,v.z];`);
 await drag([rp[0],rp[1]],[rp[2],rp[3]]);ok('empty-spot drag opens the region dialog',await waitFor(`modalEl&&/New region highlight/.test(document.querySelector('.modal .mhead').textContent)`));
 await typeInto(q('#hd-ana'),'(F) Integration region.');await click('.modal .btn.pri');hls=await poll(async()=>{const l=await api('GET','api/highlights?ref_id='+id);return l.some(x=>x.analysis==='(F) Integration region.')&&l;})||[];await sleep(300);
 const hr=hls.find(x=>x.analysis==='(F) Integration region.');doc=await pdf(rel);const ar=doc.pages[0].annots.find(a=>a.content.startsWith('(F) Integration region.'));
 ok('region highlight in the API and the PDF',hr&&hr.kind==='region'&&hr.quads.length===1&&ar&&near(ar.rects[0],hr.quads[0],2)&&near(hr.quads[0],[reg.x0,reg.y0,reg.x0+160,reg.y0+60],3),hr&&JSON.stringify(hr.quads)+' vs '+JSON.stringify(ar&&ar.rects));
 /* rotate page 3 from the page menu, then every page from the toolbar */
 await ev(`V.get(S.tab).pages[2].box.scrollIntoView({block:'center'})`);await sleep(500);await rclickEl(`V.get(S.tab).pages[2].box`,'page 3');await clickMenu('/Rotate this page/');await sleep(1200);
 doc=await pdf(rel);ok('rotate one page writes the PDF',doc.pages.map(p=>p.rotation).join(',')==='0,0,90,0,0,0',doc.pages.map(p=>p.rotation).join(','));
 ok('rotated page redrawn landscape',await waitFor(`(()=>{const pg=V.get(S.tab).pages[2];return pg.rot===90&&pg.img.naturalWidth>pg.img.naturalHeight;})()`,8000));
 await clickEl(byText('.vtb .tool','/Rotate/'),'Rotate toolbar button');await sleep(1500);doc=await pdf(rel);ok('rotate all pages writes the PDF',doc.pages.map(p=>p.rotation).join(',')==='90,90,180,90,90,90',doc.pages.map(p=>p.rotation).join(','));
 await ev(`V.get(S.tab).pages[1].box.scrollIntoView({block:'start'})`);await sleep(1500);
 const align=await ev(`const v=V.get(S.tab),pg=v.pages[1];const r=docTabRef();const hl=docHls(r).find(x=>x.page===2&&x.kind==='text');const el=pg.hls.querySelector('.hl[data-h="'+hl.id+'"]').getBoundingClientRect();const i=pg.words.list.findIndex(w=>Math.abs(w[0]-hl.quads[0][0])<1.5&&Math.abs(w[1]-hl.quads[0][1])<3);const s=pg.wordsEl.children[i].getBoundingClientRect();return [Math.round(el.left),Math.round(el.top),Math.round(s.left),Math.round(s.top),pg.rot];`);
 ok('overlay sits on its words after rotation',Math.abs(align[0]-align[2])<=3&&Math.abs(align[1]-align[3])<=3,align.join(','));await shot('i09_pdf_rotated');
 const sc=await ev(`const b=V.get(S.tab).sc.getBoundingClientRect();return [b.left+b.width/2,b.top+200,V.get(S.tab).z];`);await mouse('mouseWheel',sc[0],sc[1],{buttons:0,modifiers:2,extra:{deltaX:0,deltaY:-120}});await sleep(400);
 ok('Ctrl + wheel zooms',await ev(`return V.get(S.tab).z`)>sc[2]);},
async markdown(){const id=await openRef('/open questions/i');ok('Markdown rendered',await waitFor(`document.querySelector('.mdwrap li')`));await click('.vtb .tool[data-tool="hl"]');
 const before=await ev(`return document.querySelectorAll('.mdwrap mark').length`);
 const sel=await ev(`const li=[...document.querySelectorAll('.mdwrap li')].find(l=>l.firstChild&&l.firstChild.nodeType===3&&l.firstChild.nodeValue.length>40);const t=li.firstChild;const r=document.createRange();r.setStart(t,4);r.setEnd(t,36);const s=getSelection();s.removeAllRanges();s.addRange(r);const b=li.getBoundingClientRect();return {text:r.toString().trim(),x:b.left+20,y:b.top+b.height/2};`);
 await mouse('mouseReleased',sel.x,sel.y);ok('selection opens the dialog',await waitFor(q('#hd-verb')));const verb=await ev(`return document.getElementById('hd-verb').value`);ok('verbatim is the selection',verb===sel.text,verb);
 await typeInto(q('#hd-ana'),'(I) Integration Markdown highlight.');await click('.modal .btn.pri');const hls=await poll(async()=>{const l=await api('GET','api/highlights?ref_id='+id);return l.some(x=>x.analysis==='(I) Integration Markdown highlight.')&&l;})||[];
 const h=hls.find(x=>x.analysis==='(I) Integration Markdown highlight.');
 ok('Markdown highlight saved with anchor and empty quads',h&&h.kind==='md'&&h.page==null&&JSON.stringify(h.quads)==='[]'&&h.anchor&&h.anchor.text===sel.text,h&&JSON.stringify(h.anchor));
 ok('mark drawn in the note',await waitFor(`document.querySelectorAll('.mdwrap mark').length===${before+1}`,8000));
 const side=JSON.parse(fs.readFileSync(path.join(DEMO,'.kurasi','metadata.json'),'utf8'));ok('sidecar holds the Markdown highlight',JSON.stringify(side).includes('Integration Markdown highlight'));await shot('i10_markdown');
 await click('.vtb .tool[data-raw]');ok('Raw mode shows the source',await waitFor(`(()=>{const p=document.querySelector('.mdwrap pre.raw');return p&&/^#\\s/m.test(p.textContent);})()`));await shot('i11_markdown_raw');await click('.vtb .tool[data-raw]');
 /* a link to a project file opens a new tab; the web link only resolves as external */
 const plan=await openRef('/reading plan/i');await waitFor(`document.querySelector('.mdwrap a[data-href]')`);const links=await ev(`return [...document.querySelectorAll('.mdwrap a[data-href]')].map(a=>a.dataset.href)`);
 const fid=await ev(`return V.get(S.tab).fileId`);let target=null,ext=null;for(const l of links){const r=await api('GET',`api/resolve?href=${encodeURIComponent(l)}&from=${fid}`);if(r.ref_id&&!target)target={l,r};if(r.external&&/^https?:/.test(l))ext={l,r};}
 ok('web link resolves as external',ext&&ext.r.external===true,ext&&ext.l);
 await clickEl(`[...document.querySelectorAll('.mdwrap a[data-href]')].find(a=>a.dataset.href===${JSON.stringify(target&&target.l)})`,'project link');await sleep(800);
 ok('project-file link opens a new tab',target&&await ev(`return S.tab===${JSON.stringify(target&&target.r.ref_id)}&&S.tabs.includes(${JSON.stringify(plan)})`),target&&target.l);},
async image(){const id=await openRef('/Exhibit 2/');ok('image decoded',await waitFor(`V.get(S.tab).iw>0`));await click('.vtb .tool[data-tool="hl"]');
 const ib=await ev(`const b=V.get(S.tab).imgBox.getBoundingClientRect();return [b.left,b.top,b.width,b.height];`);await drag([ib[0]+ib[2]*0.2,ib[1]+ib[3]*0.4],[ib[0]+ib[2]*0.7,ib[1]+ib[3]*0.7]);
 ok('region drag opens the dialog',await waitFor(q('#hd-verb')));await typeInto(q('#hd-verb'),'Timeline from 1776 to 1918');await typeInto(q('#hd-ana'),'(F) Integration image region.');await click('.modal .btn.pri');await sleep(1000);
 const h=(await api('GET','api/highlights?ref_id='+id)).find(x=>x.analysis==='(F) Integration image region.');
 ok('image region saved as fractions, page null',h&&h.kind==='region'&&h.page==null&&h.quads.length===1&&near(h.quads[0],[0.2,0.4,0.5,0.3],0.02),h&&JSON.stringify(h.quads));
 const side=JSON.parse(fs.readFileSync(path.join(DEMO,'.kurasi','metadata.json'),'utf8'));ok('sidecar holds the image highlight',JSON.stringify(side).includes('Integration image region'));
 const fid=await ev(`return V.get(S.tab).fileId`);await clickEl(byText('.vtb .tool','/Rotate/'),'Rotate');const info=await poll(async()=>{const i=await api('GET',`api/file/${fid}/info`);return JSON.stringify(i.rotation)==='[90]'&&i;})||await api('GET',`api/file/${fid}/info`);await sleep(300);
 const vr=await ev(`const v=V.get(S.tab);const b=v.imgBox.getBoundingClientRect(),h=v.imgHls.querySelector('.hl').getBoundingClientRect();return [v.rot,v.imgLayer.style.transform,h.left>=b.left-1&&h.right<=b.right+1&&h.top>=b.top-1&&h.bottom<=b.bottom+1,Math.round(b.width),Math.round(b.height)];`);
 ok('image view rotation stored and drawn',JSON.stringify(info.rotation)==='[90]'&&vr[0]===90&&/rotate\(90deg\)/.test(vr[1])&&vr[2]&&vr[4]>vr[3]*0.9,JSON.stringify(info.rotation)+' '+vr.join(' '));await shot('i12_image_rotated');},
async review(){const tid=await openRef('/scientific management/i');ok('delete suggestion shown',await waitFor(`document.querySelector('.ksug')`,6000));const hid=await ev(`return document.querySelector('.ksug').closest('.kitem').dataset.h`);await shot('i13_delete_suggestion');
 await clickEl(byText('.ksug .btn','/Keep it/'),'Keep it');await sleep(800);const h=(await api('GET','api/highlights?ref_id='+tid)).find(x=>x.id===hid);
 ok('dismiss keeps the highlight',h&&h.suggestion&&h.suggestion.resolution==='dismissed'&&!(await ev(`return !!document.querySelector('.ksug')`)),h&&JSON.stringify(h.suggestion));
 const fol=await openRef('/new state/i');ok('suggested tag chip shown',await waitFor(`document.querySelector('.tagc.sug.ai')`,6000));const sh=await ev(`const c=document.querySelector('.tagc.sug.ai');return {h:c.closest('.kitem').dataset.h,t:c.textContent.replace(/^\\+ #/,'')}`);
 await click('.tagc.sug.ai');const h2=await poll(async()=>{const x=(await api('GET','api/highlights?ref_id='+fol)).find(x=>x.id===sh.h);return x&&x.tags.includes(sh.t)&&x;})||(await api('GET','api/highlights?ref_id='+fol)).find(x=>x.id===sh.h);
 ok('accepting a suggested tag',h2&&h2.tags.includes(sh.t)&&h2.suggested_tags.length===0,h2&&JSON.stringify(h2.tags));
 const doc=await pdf('management\\follett_1918_the-new-state.pdf');const an=doc.pages.flatMap(p=>p.annots).find(a=>a.content.includes('#'+sh.t)&&a.title!=='You');ok('accepted tag written into the PDF note',!!an,an&&JSON.stringify(an.content.split('\n').pop()));},
/* MCP over stdio; the UI must show every write within a few seconds */
async mcp(){const timing=[];for(let run=1;run<=2;run++){const t0=Date.now();const c=mcpStart(DEMO);const r=await c.next(90000);timing.push({run,wall_s:(Date.now()-t0)/1000,...r});
  if(run===1){await c.quit();continue;}
  console.log('MCP start',JSON.stringify(timing.map(t=>({run:t.run,wall_s:t.wall_s,client_import_s:t.client_import_s,spawn_to_initialized_s:t.spawn_to_initialized_s,connect:t.connect,protocol:t.protocol}))));
  ok('MCP server lists 14 tools',r.tools&&r.tools.length===14,r.tools&&r.tools.join(' '));
  const s=await c.call('catalog_search',{query:'scientfic managment',limit:3});ok('catalog_search',s.ok&&/scientific management/i.test(s.data.results[0].title),s.ok&&s.data.results[0].title);
  const smith=await refIdOf('/wealth of nations/i');const txt=await c.call('reference_text',{ref_id:smith,page:2});ok('reference_text',txt.ok&&txt.data.words.length>10,txt.ok&&txt.data.words.length+' words');
  const w=txt.data.words;const quote=w.slice(Math.max(0,w.length-9),w.length-2).map(x=>x[4]).join(' ');
  await ev(`openDoc(${JSON.stringify(smith)})`);await waitFor(`V.get(S.tab)&&V.get(S.tab).pages.length`);await ev(`document.querySelectorAll('.toast').forEach(t=>t.remove())`);
  const add=await c.call('highlight_add',{ref_id:smith,page:2,quote,analysis:'(I) Added through MCP by the integration run.',label:'I',tags:['mcp-test'],purpose:'Integration run'});
  ok('highlight_add with a quote',add.ok&&add.data.author==='Claude'&&add.data.origin==='ai'&&add.data.verbatim===quote,add.ok?add.data.id:JSON.stringify(add.data));
  const t1=await waitToast(/Claude via MCP: Added a highlight/,6000);ok('UI toast for highlight_add',t1,t1&&t1.ms+' ms');
  ok('blue highlight in the open document',await waitFor(`document.querySelector('.kitem.ai[data-h="${add.data.id}"]')&&V.get(S.tab).pages[1].hls.querySelector('.hl.ai[data-h="${add.data.id}"]')`,6000));await shot('i14_mcp_highlight');
  const doc=await pdf('economics\\smith_1776_wealth-of-nations.pdf');const an=doc.pages[1].annots.find(a=>a.content.includes('#mcp-test'));ok('MCP highlight written into the PDF',an&&an.title==='Claude',an&&an.title);
  const inbox=await refIdOf('/^scan_0007$/');await ev(`switchTab('reference');setQuery('scan_0007')`);await sleep(400);await ev(`document.querySelectorAll('.toast').forEach(t=>t.remove())`);
  const cat=await c.call('reference_categorize',{ref_ids:[inbox],category_paths:['Economics/Inbox test'],purpose:'Integration run'});ok('reference_categorize on an inbox file',cat.ok,JSON.stringify(cat.data).slice(0,120));
  const t2=await waitToast(/Claude via MCP: Proposed categories/,6000);ok('UI toast for reference_categorize',t2,t2&&t2.ms+' ms');
  ok('card shows the blue dot',await waitFor(`(()=>{const c=document.querySelector('.card.ref');return c&&/scan_0007/.test(c.textContent)&&c.querySelector('.dot.s');})()`,6000));
  ok('filter tree shows the new category',await waitFor(`[...document.querySelectorAll('#filter .trow')].some(r=>/^Inbox test/.test(r.textContent.trim()))||S.catPaths.includes('Economics/Inbox test')`,6000));await shot('i15_mcp_categorized');await ev(`setQuery('')`);
  await ev(`openDoc(${JSON.stringify(smith)})`);await sleep(500);
  const pd=await c.call('highlight_propose_delete',{highlight_id:add.data.id,reason:'Integration run: a test highlight to remove.'});ok('highlight_propose_delete',pd.ok);
  ok('UI shows the delete proposal',await waitFor(`document.querySelector('.kitem[data-h="${add.data.id}"] .ksug')`,6000));
  await clickEl(byText(`.kitem[data-h="${add.data.id}"] .ksug .btn`,'/Delete highlight/'),'accept the proposal');await sleep(1000);
  const gone=!(await api('GET','api/highlights?ref_id='+smith)).some(x=>x.id===add.data.id);const doc2=await pdf('economics\\smith_1776_wealth-of-nations.pdf');ok('accepting the proposal deletes it from the PDF',gone&&!doc2.pages[1].annots.some(a=>a.content.includes('#mcp-test')));
  await c.quit();
  const log=await api('GET','api/ai-log');const mine=log.filter(x=>x.client==='kurasi-integration').map(x=>x.tool);
  ok('ai_log has the rows with the client name',['catalog_search','reference_text','highlight_add','reference_categorize','highlight_propose_delete'].every(t=>mine.includes(t)),mine.length?mine.join(', '):'clients in the log: '+[...new Set(log.map(x=>x.client))].join(', '));
  await ev(`mcpDialog()`);const seenDlg=await waitFor(`[...document.querySelectorAll('.modal table.tools td')].some(td=>td.textContent==='kurasi-integration')`,5000);await shot('i16_mcp_dialog');
  ok('MCP dialog lists the calls',seenDlg,seenDlg?'':await ev(`return document.querySelectorAll('.modal table.tools tr').length+' rows; '+[...new Set([...document.querySelectorAll('.modal table.tools td')].map(td=>td.textContent).filter(t=>/client|claude|integration/i.test(t)))].join(', ')`).catch(e=>e.message));await ev(`closeModal()`);}
 fs.writeFileSync(path.join(ROOT,'mcp_timing.json'),JSON.stringify(timing,null,1));},
/* a real project made with the New project dialog; the scan dialog shows real progress */
async newProject(){fs.cpSync(DEMO,COPY,{recursive:true,filter:s=>!/[\\/]\.kurasi([\\/]|$)/.test(s)});await ev(`switchTab('reference')`);
 await click('#mb-file');await sleep(150);await click('#mi-new');ok('New project dialog',await waitFor(q('#np-folder')));await typeInto(q('#np-name'),'Integration copy');await typeInto(q('#np-folder'),COPY);
 await clickEl(byText('.modal .btn','/Create and scan/'),'Create and scan');const prog=[];let sawDialog=false;
 for(let i=0;i<150;i++){const s=await ev(`const m=document.querySelector('.modal .scanlog');if(!m)return null;return [document.querySelector('.modal .bar i').style.width,m.children.length];`);if(s){sawDialog=true;prog.push(s.join('/'));}else if(sawDialog)break;await sleep(100);}
 ok('scan dialog shows progress',sawDialog&&prog.length>0,[...new Set(prog)].slice(0,6).join(' '));const st=await api('GET','api/state');
 ok('new project open and scanned',st.project&&st.project.name==='Integration copy'&&st.counts.references===15,st.project&&`${st.counts.references} references, ${st.counts.highlights} highlights`);await shot('i17_new_project');},
async tour(){const before=(await api('GET','api/state')).project;await click('#mb-help');await sleep(150);await click('#mi-tour');await sleep(200);await clickEl(byText('.modal .btn','/Start tour/'),'Start tour');
 ok('tour starts on the sample project',await waitFor(`S.tour.on&&S.project&&S.project.id==='demo'&&!document.getElementById('tour').hidden`,30000));await shot('i18_tour_start');
 const seen=[],missing=[];for(let k=0;k<90;k++){const st=await ev(`if(!S.tour.on)return 'off';const s=STEPS[S.tour.i];const t=s.target();if(!t||!t.isConnected)window.__miss=(window.__miss||[]).concat(s.id);return s.id+'|'+(!!modalEl);`);
  if(!seen.length||seen[seen.length-1]!==st)seen.push(st);if(st==='off')break;
  if(st.endsWith('|true')){await sleep(400);const isNew=await ev(`return !!document.getElementById('np-name')`);if(isNew)await clickEl(byText('.modal .btn','/Create and scan/'),'tour Create and scan');else await sleep(800);continue;}
  if(st.startsWith('search|'))await shot('i19_tour_search');if(st.startsWith('knowledge|'))await shot('i20_tour_knowledge');
  await click('#coach .cfoot .btn.pri');await sleep(/^(author|card|edit|kfilter|markdown|graph)\|/.test(st)?1000:650);}
 const miss=await ev(`return window.__miss||[]`);const ids=[...new Set(seen.map(s=>s.split('|')[0]))];
 ok('tour walks every step',ids.includes('tools')&&ids[ids.length-1]==='off',ids.join(' > '));ok('every tour step found its target',miss.length===0,miss.join(', '));
 await poll(()=>ev(`return S.project&&S.project.id===${JSON.stringify(before.id)}`),15000);const st=await api('GET','api/state');ok('leaving the tour returns to the previous project',st.project&&st.project.id===before.id&&await ev(`return S.project.id===${JSON.stringify(before.id)}`),st.project&&st.project.name);
 const t=await waitToast(/Tour finished/,3000);ok('tour finished toast names the project',t&&t.text.includes(before.name),t&&t.text);await shot('i21_tour_back');},
/* restart: the last project reopens; the desktop handover protocol seen from the page */
async restart(){const last=(await api('GET','api/state')).project;await stopServer();await startServer();const st=await api('GET','api/state');
 ok('backend reopens the last project',st.project&&st.project.id===last.id&&st.startup.opened_last===true,st.project&&st.project.name);
 const reg=await send('Page.addScriptToEvaluateOnNewDocument',{source:`window.__py=[];window.pywebview={api:{splash_status:t=>{__py.push(['status',t,Math.round(performance.now())]);return Promise.resolve(true);},app_ready:()=>{__py.push(['ready','',Math.round(performance.now())]);return Promise.resolve(true);},pick_folder:()=>Promise.resolve(null)}};`});
 await nav();ok('page ready after restart',await waitFor(`typeof Boot!=='undefined'&&Boot.done`,20000));const calls=await ev(`return window.__py`);
 const statuses=calls.filter(c=>c[0]==='status').map(c=>c[1]);const readyIdx=calls.findIndex(c=>c[0]==='ready');
 ok('desktop protocol: statuses then app_ready',readyIdx===calls.length-1&&statuses.some(s=>/^Opening /.test(s))&&statuses.includes('Loading references')&&statuses.includes('Building tag graph'),statuses.join(' > ')+' > app_ready');
 ok('no overlay inside the desktop window',await ev(`return !document.getElementById('splash')`));ok('catalogue of the last project',await waitFor(`S.project&&S.project.id===${JSON.stringify(last.id)}&&document.querySelector('.card.ref')`));
 await send('Page.removeScriptToEvaluateOnNewDocument',{identifier:reg.identifier});await nav();await waitFor(`typeof Boot!=='undefined'&&Boot.done`,20000);await shot('i22_restart');}
};

const want=process.argv.slice(2);const order=Object.keys(SC);
let crashed=null;
try{for(const n of (want.length?want:order)){if(!SC[n]){console.log('unknown scenario',n);continue;}console.log('---',n);try{await SC[n]();}catch(e){crashed=crashed||n;ok('scenario '+n+' ran',false,e.message.split('\n')[0]);}}}
finally{const fails=results.filter(r=>!r.pass);console.log(`\n${results.length-fails.length} of ${results.length} checks passed.${fails.length?' Failed: '+fails.map(f=>f.name).join('; '):''}`);
 console.log(logs.length?'console:\n'+[...new Set(logs)].slice(0,40).join('\n'):'console: clean');console.log('temporary folder:',ROOT);
 fs.writeFileSync(path.join(ROOT,'results.json'),JSON.stringify({results,logs},null,1));ws.close();spawn('taskkill',['/PID',String(chrome.pid),'/T','/F'],{stdio:'ignore'});await stopServer();}
process.exit(results.some(r=>!r.pass)?1:0);
