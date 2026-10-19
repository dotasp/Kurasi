/* Development only: drives headless Chrome over the DevTools protocol against the fake API and
   writes screenshots to ui/dev/shots/. A small static server mimics the backend routing:
   "/" serves ui/index.html and "/ui/..." serves the UI files.
   Usage: node shoot.mjs [scenario ...]   (no argument runs every scenario except bigui, which runs only when named)
   KURASI_BIG_REFS=8000 node shoot.mjs biggraph   lays out and draws a generated library of that many references */
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {spawn,spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const UI=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const SHOTS=path.join(UI,'dev','shots');
const CHROME='C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const PROFILE=process.env.KURASI_CHROME_PROFILE||path.join(process.env.TEMP||UI,'kurasi-ui-chrome');
fs.mkdirSync(SHOTS,{recursive:true});
const TYPES={'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8','.mjs':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.jpg':'image/jpeg','.png':'image/png','.json':'application/json','.txt':'text/plain'};
const server=http.createServer((req,res)=>{const u=new URL(req.url,'http://x');let p=decodeURIComponent(u.pathname);
  let file=p==='/'?path.join(UI,'index.html'):p.startsWith('/ui/')?path.join(UI,p.slice(4)):null;
  if(!file||!file.startsWith(UI)||!fs.existsSync(file)||fs.statSync(file).isDirectory()){res.writeHead(404,{'Content-Type':'application/json'});res.end('{"error":"not found"}');return;}
  res.writeHead(200,{'Content-Type':TYPES[path.extname(file)]||'application/octet-stream','Cache-Control':'no-store'});fs.createReadStream(file).pipe(res);});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const BASE=`http://127.0.0.1:${server.address().port}/`;
const port=9300+Math.floor(Math.random()*500);
const chrome=spawn(CHROME,['--headless=new',`--remote-debugging-port=${port}`,'--remote-allow-origins=*',`--user-data-dir=${PROFILE}`,'--no-first-run','--no-default-browser-check','--disable-extensions','--window-size=1536,864','about:blank'],{stdio:'ignore'});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
let target=null;for(let i=0;i<60&&!target;i++){await sleep(250);try{const list=await (await fetch(`http://127.0.0.1:${port}/json`)).json();target=list.find(t=>t.type==='page');}catch(e){}}
if(!target){console.error('Chrome did not start');process.exit(1);}
const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
let mid=0;const pend=new Map();const logs=[];
ws.onmessage=ev=>{const m=JSON.parse(ev.data);if(m.id&&pend.has(m.id)){const {res,rej}=pend.get(m.id);pend.delete(m.id);m.error?rej(new Error(m.error.message)):res(m.result);return;}
  if(m.method==='Runtime.consoleAPICalled'){const t=m.params.args.map(a=>a.value??a.description??'').join(' ');logs.push(`[${m.params.type}] ${t}`);}
  if(m.method==='Runtime.exceptionThrown'){const d=m.params.exceptionDetails;logs.push(`[exception] ${d.exception&&d.exception.description||d.text} @${d.url||''}:${d.lineNumber}`);}
  if(m.method==='Log.entryAdded'){const e=m.params.entry;if(e.level==='error'||e.level==='warning')logs.push(`[log ${e.level}] ${e.text} ${e.url||''}`);}};
const send=(method,params={})=>new Promise((res,rej)=>{const id=++mid;pend.set(id,{res,rej});ws.send(JSON.stringify({id,method,params}));});
await send('Page.enable');await send('Runtime.enable');await send('Log.enable');
await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});
async function nav(q){await send('Page.navigate',{url:BASE+q});await sleep(300);}
async function ev(expr){const r=await send('Runtime.evaluate',{expression:`(async()=>{${expr}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error('eval: '+(r.exceptionDetails.exception&&r.exceptionDetails.exception.description||r.exceptionDetails.text));return r.result.value;}
async function shot(name){const r=await send('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(SHOTS,name+'.png'),Buffer.from(r.data,'base64'));console.log('shot',name);}
async function waitFor(expr,ms=8000){const t0=Date.now();while(Date.now()-t0<ms){try{if(await ev(`return !!(${expr})`))return true;}catch(e){}await sleep(150);}console.log('timeout waiting for',expr);return false;}
async function mouse(type,x,y,buttons=1){await send('Input.dispatchMouseEvent',{type,x,y,button:'left',buttons,clickCount:1});}
async function click(sel){const p=await ev(`document.querySelectorAll(".toast").forEach(t=>t.remove());const e=document.querySelector(${JSON.stringify(sel)});if(!e)return null;const r=e.getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);if(!p){console.log('no element',sel);return false;}await mouse('mousePressed',p[0],p[1]);await mouse('mouseReleased',p[0],p[1],0);return true;}
const ready=()=>waitFor(`typeof Boot!=='undefined'&&Boot.done&&!document.getElementById('splash')`,15000);
const refId=re=>`S.refs.find(r=>${re}.test(r.title)).id`;
/* a generated library for the large-graph scenarios, as page code that sets S.refs and the tag rows. With text it
   also gives categories and search text on every third reference, hands the library to the fake backend
   (KurasiFake.adopt) and loads it through the routes, so search and partial refresh go through /api/search and
   /api/references?ids=. Titles, names and words are synthetic. The author pool grows faster than the library and
   larger libraries get co-authors, so most authors of a large library appear once. */
const bigLib=(N,text)=>`let h=7;const rn=()=>{h=(h*1103515245+12345)%2147483648;return h/2147483648;};let h2=11;const rn2=()=>{h2=(h2*1103515245+12345)%2147483648;return h2/2147483648;};
 const base=S.refs[0],refs=[],tags=new Map(),N=${N},AP=Math.round(90*Math.pow(N/300,1.7)),TP=Math.round(60*Math.sqrt(N/300)),CO=Math.min(3,Math.log10(N/300));
 const SY=['ka','lo','mi','ren','tas','vor','dun','pel','sio','gra','bel','tum','zer','fin','os','ul'],word=()=>{let w='';const k=2+Math.floor(rn2()*2);for(let j=0;j<k;j++)w+=SY[Math.floor(rn2()*SY.length)];return w;};
 for(let i=0;i<N;i++){const id='big'+i;const r={...base,id,title:'Synthetic study number '+i+' on organisational change',authors:['Author'+Math.floor(Math.pow(rn(),1.5)*AP)+', A.'],search_text:''};refs.push(r);
  const nt=1+Math.floor(rn()*4);for(let j=0;j<nt;j++){const t='topic'+Math.floor(Math.pow(rn(),2.2)*TP);(tags.get(t)||tags.set(t,new Set()).get(t)).add(id);}
  const co=Math.floor(rn2()*(CO+1));for(let j=0;j<co;j++){const a='Author'+Math.floor(Math.pow(rn2(),1.5)*AP)+', A.';if(!r.authors.includes(a))r.authors.push(a);}
  ${text?`r.categories=['Synthetic area '+(i%8)+'/Topic '+(i%5)];if(i%3===0){const w=[];while(w.join(' ').length<1500)w.push(word());r.search_text=w.join(' ');}`:''}}
 ${text?`const tagOf=new Map();tags.forEach((ids,t)=>ids.forEach(id=>(tagOf.get(id)||tagOf.set(id,[]).get(id)).push(t)));
  window.__textKB=Math.round(refs.reduce((n,r)=>n+r.search_text.length,0)/1024);
  KurasiFake.adopt(refs.map(r=>({id:r.id,title:r.title,authors:r.authors,categories:r.categories,text:r.search_text,tags:tagOf.get(r.id)||[]})));await loadCatalog();`
 :`setRefs(refs);setTags([...tags].map(([name,ids])=>({name,ref_ids:[...ids],hl_count:ids.size})));`}Graph.cache.clear();`;

const SC={
 async splash(){await nav('?fake=1&slow=1500');await sleep(700);await shot('00_splash_overlay');await ready();},
 async catalogue(){await nav('?fake=1');await ready();await sleep(400);await shot('01_catalogue');},
 async authors(){await ev(`switchTab('authors')`);await sleep(300);await shot('02_authors');},
 async graph(){await ev(`switchTab('tags')`);await waitFor(`document.querySelector('.gbox svg')`);await sleep(200);
  const p=await ev(`const n=document.querySelector('.gbox .node.tag .mark');const r=n.getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);await mouse('mouseMoved',p[0],p[1],0);await sleep(200);await shot('03_tag_graph');await mouse('mouseMoved',5,5,0);},
 async pdf(){await ev(`openDoc(${refId('/wealth of nations/i')})`);await waitFor(`document.querySelectorAll('.ppage img:not([hidden])').length>=2`);await sleep(600);
  await ev(`const r=docTabRef();const h=sortHls(docHls(r))[1];selectHl(r,V.get(r.id),h,true);`);await sleep(700);await shot('04_pdf_knowledge');
  /* drag across words on page 2 with the Highlight tool */
  await ev(`setTool(V.get(S.tab),'hl')`);await sleep(100);
  const pts=await ev(`const pg=V.get(S.tab).pages[1];pg.box.scrollIntoView({block:'start'});await new Promise(r=>setTimeout(r,500));const sp=pg.wordsEl.querySelectorAll('span');const a=sp[30].getBoundingClientRect(),b=sp[44].getBoundingClientRect();return [a.left+2,a.top+a.height/2,b.right-2,b.top+b.height/2,sp.length];`);
  await mouse('mousePressed',pts[0],pts[1]);for(let i=1;i<=6;i++)await mouse('mouseMoved',pts[0]+(pts[2]-pts[0])*i/6,pts[1]+(pts[3]-pts[1])*i/6);await sleep(100);await shot('05_pdf_drag');await mouse('mouseReleased',pts[2],pts[3],0);await sleep(300);await shot('06_highlight_dialog');
  await ev(`document.getElementById('hd-ana').value='(I) Test highlight from a drag across words.';`);await click('.modal .btn.pri');await sleep(900);
  console.log('hl count after save',await ev(`return docHls(docTabRef()).length`));await shot('07_pdf_after_save');},
 async rotated(){await ev(`openDoc(${refId('/origin of species/i')})`);await waitFor(`V.get(S.tab)&&V.get(S.tab).pages.length`);await ev(`V.get(S.tab).pages[2].box.scrollIntoView({block:'start'})`);await sleep(1200);await shot('08_pdf_rotated_page');},
 async markdown(){await ev(`openDoc(${refId('/reading plan/i')})`);await waitFor(`document.querySelector('.mdwrap mark')`);await sleep(300);await shot('09_markdown');await ev(`const v=V.get(S.tab);v.mdRaw=true;renderMdContent(docTabRef(),v);updateTb(v);`);await sleep(200);await shot('10_markdown_raw');await ev(`const v=V.get(S.tab);v.mdRaw=false;renderMdContent(docTabRef(),v);updateTb(v);`);},
 async image(){await ev(`openDoc(${refId('/Exhibit 1/')})`);await waitFor(`document.querySelector('.ilayer .hl')`);await sleep(300);await shot('11_image');await ev(`rotate(V.get(S.tab),null)`);await sleep(500);await shot('12_image_rotated');},
 async edit(){await ev(`switchTab('reference');openEdit(${refId('/leisure class/i')})`);await waitFor(`document.querySelector('#f-title')`);await sleep(200);await shot('13_edit_pane');},
 async copies(){await ev(`openDoc(${refId('/protestantische/i')})`);await sleep(1500);await shot('14_pdf_translation_copy');},
 async cloud(){await nav('?fake=1');await ready();await ev(`openDoc(${refId('/Principles of economics/')})`);await sleep(500);await shot('15_cloud_only');},
 async welcome(){await nav('?fake=1&fresh=1');await ready();await sleep(400);await shot('16_welcome');},
 async missing(){await nav('?fake=1&missing=1');await ready();await sleep(400);await shot('17_welcome_missing_folder');},
 async dark(){await nav('?fake=1');await ready();await ev(`setTheme('dark')`);await sleep(300);await shot('18_dark_catalogue');await ev(`openDoc(${refId('/scientific management/i')})`);await sleep(1600);await shot('19_dark_pdf');await ev(`setTheme('system')`);},
 async mcp(){await nav('?fake=1&mcp=1');await ready();await ev(`openDoc(${refId('/scientific management/i')})`);await sleep(9500);await shot('20_mcp_live_refresh');},
 async scan(){await nav('?fake=1');await ready();await ev(`runScan({start:true})`);await sleep(1300);await shot('21_scan_dialog');await sleep(2500);},
 async dialogs(){await ev(`mcpDialog()`);await sleep(500);await shot('22_mcp_dialog');await ev(`closeModal()`);await ev(`settingsDialog()`);await sleep(300);await shot('23_settings');await ev(`closeModal()`);await ev(`openMenuByName('mb-help')`);await sleep(200);await shot('24_help_menu');await ev(`closeMenus()`);},
 async py(){await nav('?fake=1&py=1');await waitFor(`typeof Boot!=='undefined'&&Boot.done`);await sleep(300);console.log('pywebview calls',JSON.stringify(await ev(`return window.__py`)),'overlay present',await ev(`return !!document.getElementById('splash')`));},
 async safety(){await nav('?fake=1&slow=4000&safety=1500');await sleep(2200);await shot('25_safety_toast');console.log('toast',await ev(`return [...document.querySelectorAll('.toast')].map(t=>t.textContent).join(' | ')`));await ready();},
 async interact(){await nav('?fake=1');await ready();const out=[];
  /* Markdown: select text with the Highlight tool */
  await ev(`openDoc(${refId('/open questions/i')})`);await waitFor(`document.querySelector('.mdwrap li')`);await ev(`setTool(V.get(S.tab),'hl')`);
  const md=await ev(`const li=document.querySelectorAll('.mdwrap li')[1];const t=li.firstChild;const r=document.createRange();r.setStart(t,9);r.setEnd(t,40);const s=getSelection();s.removeAllRanges();s.addRange(r);const b=li.getBoundingClientRect();return [b.left+20,b.top+b.height/2];`);
  await mouse('mouseReleased',md[0],md[1],0);await sleep(300);out.push('md dialog verbatim: '+await ev(`return document.getElementById('hd-verb')&&document.getElementById('hd-verb').value`));
  await ev(`document.getElementById('hd-ana').value='(I) Test Markdown highlight.'`);await click('.modal .btn.pri');await sleep(700);out.push('md marks: '+await ev(`return document.querySelectorAll('.mdwrap mark').length`));await shot('40_md_new_highlight');
  /* image: drag a region */
  await ev(`openDoc(${refId('/Exhibit 2/')})`);await waitFor(`V.get(S.tab)&&V.get(S.tab).iw`);await ev(`setTool(V.get(S.tab),'hl')`);
  const ib=await ev(`const b=V.get(S.tab).imgBox.getBoundingClientRect();return [b.left,b.top,b.width,b.height];`);await mouse('mousePressed',ib[0]+ib[2]*0.2,ib[1]+ib[3]*0.4);await mouse('mouseMoved',ib[0]+ib[2]*0.5,ib[1]+ib[3]*0.6);await mouse('mouseMoved',ib[0]+ib[2]*0.7,ib[1]+ib[3]*0.7);await mouse('mouseReleased',ib[0]+ib[2]*0.7,ib[1]+ib[3]*0.7,0);await sleep(300);
  await ev(`document.getElementById('hd-verb').value='Timeline from 1776 to 1918';document.getElementById('hd-ana').value='(F) Region test.'`);await click('.modal .btn.pri');await sleep(700);out.push('image quads: '+await ev(`return JSON.stringify(docHls(docTabRef()).map(h=>h.quads))`));await shot('41_image_new_region');
  /* PDF: rotate page 2 of Smith and check the overlay sits on the words */
  await ev(`openDoc(${refId('/wealth of nations/i')})`);await waitFor(`V.get(S.tab)&&V.get(S.tab).pages.length`);await ev(`await rotate(V.get(S.tab),2)`);await sleep(300);await ev(`V.get(S.tab).pages[1].box.scrollIntoView({block:'start'})`);await sleep(1200);await shot('42_pdf_rotated_overlay');
  out.push('rotated overlay vs words: '+await ev(`const pg=V.get(S.tab).pages[1];const h=pg.hls.querySelector('.hl').getBoundingClientRect();const r=docTabRef();const hl=docHls(r).find(x=>x.page===2);const w=pg.words.list.find(x=>Math.abs(x[0]-hl.quads[0][0])<1&&Math.abs(x[1]-hl.quads[0][1])<1);const i=pg.words.list.indexOf(w);const s=pg.wordsEl.children[i].getBoundingClientRect();return [Math.round(h.left),Math.round(h.top),Math.round(s.left),Math.round(s.top)].join(',');`));
  /* zoom with Ctrl + wheel */
  const sc=await ev(`const b=V.get(S.tab).sc.getBoundingClientRect();return [b.left+b.width/2,b.top+200];`);await send('Input.dispatchMouseEvent',{type:'mouseWheel',x:sc[0],y:sc[1],deltaX:0,deltaY:-120,modifiers:2});await sleep(400);out.push('zoom after ctrl+wheel: '+await ev(`return V.get(S.tab).z`));
  /* knowledge: inline edit and save, then delete with undo */
  await ev(`const r=docTabRef();const h=sortHls(docHls(r))[0];S.kedit.add(h.id);renderRight();`);await sleep(200);await shot('43_knowledge_edit');
  await ev(`const ta=document.querySelector('.kitem textarea[aria-label=Analysis]');ta.value+=' Edited.';ta.dispatchEvent(new Event('input'));`);await click('.kitem .kfoot .btn.pri');await sleep(800);out.push('edited: '+await ev(`return sortHls(docHls(docTabRef()))[0].analysis`));
  const before=await ev(`return docHls(docTabRef()).length`);await ev(`deleteHl(docTabRef(),sortHls(docHls(docTabRef()))[0])`);await sleep(200);await click('.modal .btn.danger');await sleep(800);const mid=await ev(`return docHls(docTabRef()).length`);await ev(`await undoLast()`);await sleep(500);out.push(`delete and undo: ${before} -> ${mid} -> `+await ev(`return docHls(docTabRef()).length`));
  /* suggestions on Veblen and Mill */
  await ev(`openDoc(${refId('/leisure class/i')})`);await waitFor(`document.querySelector('.ksug')`);await shot('44_delete_suggestion');await click('.ksug .btn');await sleep(700);out.push('after dismiss, open suggestions: '+await ev(`return document.querySelectorAll('.ksug').length`));
  await ev(`openDoc(${refId('/political economy/i')})`);await waitFor(`document.querySelector('.tagc.sug.ai')`);await click('.tagc.sug.ai');await sleep(700);out.push('accepted tag: '+await ev(`return JSON.stringify(docHls(docTabRef()).map(h=>h.tags))`));
  /* edit pane save and a new category */
  await ev(`switchTab('reference');openEdit(${refId('/leisure class/i')})`);await waitFor(`document.querySelector('#f-issue')`);await ev(`const i=document.querySelector('#f-pages_range');i.value='1-400';i.dispatchEvent(new Event('input'));`);await click('#right .rfoot .btn.pri');await sleep(900);out.push('status after save: '+await ev(`return refById(${refId('/leisure class/i')}).status`));
  await ev(`categoryDialog({})`);await sleep(200);await ev(`document.getElementById('cat-name').value='Methods'`);await click('.modal .btn.pri');await sleep(800);out.push('categories: '+await ev(`return S.catPaths.join('; ')`));
  await shot('45_after_edits');console.log(out.join('\n'));},
 async file(){await send('Page.navigate',{url:'file:///'+UI.split(path.sep).join('/')+'/index.html?fake=1'});await ready();await ev("switchTab('tags')");await waitFor("document.querySelector('.gbox svg')");await shot('26_file_url_graph');console.log('worker failed, page fallback used:',await ev('return Graph.failed'));},
 async links(){await nav('?fake=1');await ready();await ev("openDoc("+refId('/reading plan/i')+")");await waitFor("document.querySelector('.mdwrap a')");await click('.mdwrap a');await sleep(700);console.log('after project link, tab:',await ev('return docTabRef()&&docTabRef().title'));
  await ev("openDoc("+refId('/reading plan/i')+")");await sleep(300);await click('.mdwrap a[data-href^="https"]');await sleep(500);console.log('after web link:',await ev("return [...document.querySelectorAll('.toast')].map(t=>t.textContent).join(' | ')"));},
 async hover(){await nav('?fake=1');await ready();await ev("switchTab('tags')");await waitFor("document.querySelector('.gbox svg')");await sleep(300);await mouse('mouseMoved',5,5,0);await ev("const n=[...document.querySelectorAll('.gbox .node.tag')].find(g=>/quote/.test(g.textContent))||document.querySelector('.gbox .node.tag');const r=n.querySelector('.mark').getBoundingClientRect();const m=n.querySelector('.mark');m.dispatchEvent(new MouseEvent('mouseover',{bubbles:true}));m.dispatchEvent(new MouseEvent('mousemove',{clientX:r.left+r.width/2,clientY:r.top+r.height/2,bubbles:true}));");await sleep(200);await shot('03_tag_graph');},
 async newproj(){await nav('?fake=1&py=1');await ready();await ev('newProjectDialog()');await sleep(200);await click('.modal .btn:not(.pri)');await sleep(200);await shot('27_new_project_pywebview');console.log('picked folder:',await ev("return document.getElementById('np-folder').value"));await ev('closeModal()');},
 /* the Projects dialog with 150 projects: search, keys, Remove with its data and Undo, removing the open project,
    Reset on the sample row, and Open new folder in a plain browser, where the folder path is typed */
 async projects(){await nav('?fake=1&projects=150');await ready();const out=[];
  const key=async(k,code,vk,text)=>{await send('Input.dispatchKeyEvent',{type:'keyDown',key:k,code,windowsVirtualKeyCode:vk,text});await send('Input.dispatchKeyEvent',{type:'keyUp',key:k,code,windowsVirtualKeyCode:vk});await sleep(60);};
  const info=()=>ev(`const rows=[...document.querySelectorAll('#pd-list .prow')].filter(r=>!r.hidden);const s=document.querySelector('#pd-list .prow.sel');return document.querySelector('.projdlg .pcount').textContent+' | rows shown '+rows.length+' | selected '+(s?s.querySelector('b').textContent:'none')`);
  const undo=()=>ev(`const t=[...document.querySelectorAll('.toast')].find(t=>/removed/.test(t.textContent));if(!t)return 'no toast';t.querySelector('button').click();return 'undo clicked'`);
  await ev(`PD.q='';projectsDialog()`);await waitFor(`document.querySelectorAll('#pd-list .prow').length>100`);await sleep(300);out.push('list: '+await info());
  out.push('order by last opened: '+await ev(`const l=await GET('/api/projects');const ids=[...document.querySelectorAll('#pd-list .prow')].map(r=>r.id.slice(3));const lo=l.map(p=>p.last_opened||'');return ids.join()===l.map(p=>p.id).join()&&lo.every((x,i)=>!i||lo[i-1]>=x)`));
  await shot('80_projects_list');
  /* typing on a footer button goes to the search box */
  await ev(`document.querySelector('.modal .mfoot .btn:not(.pri)').focus()`);for(const c of 'soil')await key(c,'Key'+c.toUpperCase(),c.toUpperCase().charCodeAt(0),c);await sleep(150);
  out.push('typed "soil": search box '+JSON.stringify(await ev(`return document.getElementById('pd-q').value`))+', '+await info());await shot('81_projects_search');
  await key('ArrowDown','ArrowDown',40);await key('ArrowDown','ArrowDown',40);await sleep(100);const pick=await ev(`return document.querySelector('#pd-list .prow.sel b').textContent`);out.push('after Down Down: '+await info());await shot('82_projects_keyboard');
  await key('Enter','Enter',13,'\r');await waitFor(`!modalEl&&S.project&&S.project.name===${JSON.stringify(pick)}`);out.push('Enter opened: '+await ev(`return S.project.name`));
  /* Remove with the data box ticked, then Undo */
  await ev(`PD.q='';projectsDialog()`);await waitFor(`document.querySelectorAll('#pd-list .prow').length>100`);const victim=await ev(`const r=[...document.querySelectorAll('#pd-list .prow')].find(r=>!r.classList.contains('cur')&&!/Sample/.test(r.textContent));return r.querySelector('b').textContent`);
  await ev(`[...document.querySelectorAll('#pd-list .prow')].find(r=>r.querySelector('b').textContent===${JSON.stringify(victim)}).querySelector('button').click()`);await waitFor(`document.getElementById('rm-data')`);await sleep(200);await shot('83_projects_remove_unticked');
  await click('#rm-data');await sleep(200);out.push('remove dialog: '+await ev(`return document.getElementById('rm-paths').innerText.replace(/\\n+/g,' / ')+' | button '+document.getElementById('rm-go').textContent`));await shot('84_projects_remove_ticked');
  await click('#rm-go');await waitFor(`[...document.querySelectorAll('.toast')].some(t=>/removed/.test(t.textContent))`);await sleep(300);
  out.push('after remove: '+await ev(`return [...document.querySelectorAll('.toast')].map(t=>t.innerText.replace(/\\n/g,' ')).join(' | ')+' | in list '+S.projects.some(p=>p.name===${JSON.stringify(victim)})+' | dialog '+(modalEl&&modalEl._kind)`));await shot('85_projects_undo_toast');
  out.push(await undo());await waitFor(`S.projects.some(p=>p.name===${JSON.stringify(victim)})`);out.push('after Undo, in list: '+await ev(`return S.projects.some(p=>p.name===${JSON.stringify(victim)})`));
  /* Remove the open project: it closes and the welcome screen shows; Undo opens it again */
  await ev(`closeModal(true);PD.q='';projectsDialog()`);await waitFor(`document.querySelector('#pd-list .prow.cur')`);await ev(`document.querySelector('#pd-list .prow.cur button').click()`);await waitFor(`document.getElementById('rm-go')`);await sleep(200);
  out.push('open project warning: '+await ev(`return (document.querySelector('.modal .warnline')||{}).textContent`));await click('#rm-go');await waitFor(`modalEl&&modalEl._kind==='welcome'`);await sleep(300);
  out.push('after removing the open one: project '+await ev(`return S.project&&S.project.name`)+', dialog '+await ev(`return modalEl&&modalEl._kind`));await shot('86_projects_removed_open_welcome');
  out.push(await undo());await waitFor(`S.project&&S.project.name===${JSON.stringify(pick)}`);out.push('after Undo: open '+await ev(`return S.project&&S.project.name`)+', dialog '+await ev(`return modalEl&&modalEl._kind`));
  /* the sample row offers Reset */
  await ev(`closeModal(true);PD.q='sample';projectsDialog()`);await waitFor(`document.querySelector('#pd-list .prow')`);await sleep(200);out.push('sample row button: '+await ev(`const r=[...document.querySelectorAll('#pd-list .prow')].find(r=>!r.hidden);return r.querySelector('b').textContent+' -> '+r.querySelector('button').textContent`));await shot('87_projects_sample_reset');
  /* Open new folder in a plain browser: the typed path, checked and named from the folder */
  await ev(`PD.q=''`);await click('#pd-new');await waitFor(`document.getElementById('np-typed')`);await sleep(200);out.push('focus on folder field: '+await ev(`return document.activeElement&&document.activeElement.id`));
  await send('Input.insertText',{text:'D:\\Papers\\Soil-notes'});await waitFor(`!document.querySelector('.modal .fok').hidden`);await sleep(100);out.push('typed path: name '+await ev(`return document.getElementById('np-name').value+' | '+document.querySelector('.modal .fok').textContent`));await shot('88_open_new_folder_typed');
  await ev(`const f=document.getElementById('np-folder');f.value='D:\\\\Papers\\\\missing-folder';f.dispatchEvent(new Event('change'))`);await waitFor(`!document.querySelector('.modal .err').hidden`);out.push('bad path: '+await ev(`return document.querySelector('.modal .err').textContent`));
  await ev(`closeModal(true)`);console.log(out.join('\n'));},
 async tour(){await nav('?fake=1');await ready();await ev(`startDemo({reset:false,tour:true})`);await waitFor(`S.tour.on&&!document.getElementById('tour').hidden`);await sleep(300);await shot('30_tour_step1');
  const seen=[];for(let k=0;k<80;k++){const st=await ev(`return S.tour.on?STEPS[S.tour.i].id+'|'+(!!modalEl)+'|'+S.project.name:'off|'+(S.project&&S.project.name)`);
   if(!seen.length||seen[seen.length-1]!==st)seen.push(st);if(st.startsWith('off'))break;
   if(st.startsWith('search|'))await shot('31_tour_search');if(st.startsWith('markdown|'))await shot('32_tour_markdown');if(st.startsWith('knowledge|'))await shot('33_tour_knowledge');
   if(st.includes('|true|')){await sleep(400);const isNew=await ev(`return !!document.getElementById('np-name')`);if(isNew){await shot('34_tour_new_project');await click('.modal .btn.pri');}else await sleep(800);continue;}
   await click('#coach .cfoot .btn.pri');await sleep(st.startsWith('author|')||st.startsWith('card|')||st.startsWith('edit|')?900:600);}
  console.log('tour steps:',seen.join(' > '));await sleep(800);await shot('35_tour_finished');},
 /* Tauri shell stand-ins: the page emits kurasi-ready and sets data-ready; the splash window shows the error buttons */
 async tauri(){await nav('?fake=1&tauri=1');await waitFor(`typeof Boot!=='undefined'&&Boot.done`);await sleep(300);
  console.log('tauri events',JSON.stringify(await ev(`return window.__tauri.map(x=>x[0]+(x[1]&&x[1].text?':'+x[1].text:''))`)),'data-ready',await ev(`return document.documentElement.dataset.ready`),'overlay present',await ev(`return !!document.getElementById('splash')`));
  await send('Emulation.setDeviceMetricsOverride',{width:960,height:600,deviceScaleFactor:1,mobile:false});await send('Page.navigate',{url:BASE+'ui/splash.html?fake=1'});await sleep(1100);await shot('54_splash_window_status');await sleep(900);await shot('55_splash_window_error');
  await click('#retry');await click('#quit');console.log('splash invokes',JSON.stringify(await ev(`return window.__tauri`)),'status',await ev(`return document.getElementById('status').textContent`));
  await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});},
 /* tag graph view controls: wheel zoom, pan, node drag, sliders, keys, persistence, the folded toolbar */
 async graphctl(){await send('Emulation.setDeviceMetricsOverride',{width:1920,height:1080,deviceScaleFactor:1,mobile:false});await nav('?fake=1');await ready();await ev(`switchTab('tags')`);await waitFor(`document.querySelector('.gbox svg .node')`);await sleep(300);const out=[];
  const b=await ev(`const r=document.querySelector('.gbox').getBoundingClientRect();return [r.left,r.top,r.width,r.height];`);
  await send('Input.dispatchMouseEvent',{type:'mouseWheel',x:b[0]+b[2]*0.6,y:b[1]+b[3]*0.5,deltaX:0,deltaY:-300});await sleep(150);out.push('zoom after wheel: '+await ev(`return S.graph.view.zoom.toFixed(3)+' pan '+Math.round(S.graph.view.panX)+','+Math.round(S.graph.view.panY)`));
  /* left-drag on empty canvas (a point with no node under it) */
  const e0=await ev(`const box=document.querySelector('.gbox');const r=box.getBoundingClientRect();for(let y=r.bottom-12;y>r.top+60;y-=12)for(let x=r.right-12;x>r.left+12;x-=24){const t=document.elementFromPoint(x,y);if(t&&t.closest('.gbox')&&!t.closest('.node,.gtb,.ggear'))return [x,y];}return null;`);
  const p0=await ev(`return [S.graph.view.panX,S.graph.view.panY]`);await mouse('mousePressed',e0[0],e0[1]);await mouse('mouseMoved',e0[0]-40,e0[1]-20);await mouse('mouseMoved',e0[0]-80,e0[1]-40);await mouse('mouseReleased',e0[0]-80,e0[1]-40,0);await sleep(100);
  out.push('pan after drag: '+await ev(`return [Math.round(S.graph.view.panX-(${p0[0]})),Math.round(S.graph.view.panY-(${p0[1]}))].join(',')+' tab '+S.tab`));
  /* node drag moves the node and does not count as a click */
  const sel=`[...document.querySelectorAll('.gbox .node.author')].find(n=>{const r=n.querySelector('.mark').getBoundingClientRect();const t=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);return t&&t.closest('.node')===n;})`;
  const n0=await ev(`const n=${sel};window.__drag=n;const r=n.querySelector('.mark').getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);await mouse('mousePressed',n0[0],n0[1]);await mouse('mouseMoved',n0[0]+30,n0[1]+25);await mouse('mouseMoved',n0[0]+60,n0[1]+50);await mouse('mouseReleased',n0[0]+60,n0[1]+50,0);await sleep(100);
  const n1=await ev(`const n=window.__drag;const r=n.querySelector('.mark').getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);out.push(`node moved: ${Math.round(n1[0]-n0[0])},${Math.round(n1[1]-n0[1])} tab ${await ev('return S.tab')}`);
  /* sliders through the toolbar inputs */
  await ev(`const d=Graph.el;const set=(k,v)=>{d.inputs[k].value=v;d.inputs[k].dispatchEvent(new Event('input'));};set('fontPx',15);set('labelOp',85);set('edgeOp',40);`);await sleep(100);
  out.push('css vars: '+await ev(`const s=document.querySelector('.gbox').style;return [s.getPropertyValue('--gfont'),s.getPropertyValue('--glabel'),s.getPropertyValue('--gedge')].join(' ')`));
  /* node spacing applies on release: a new layout is drawn at the same zoom */
  const w0=await ev(`const xs=Graph.el.nodes.map(n=>n.x);return Math.round(Math.max(...xs)-Math.min(...xs))`);
  await ev(`const d=Graph.el;d.inputs.spacing.value=200;d.inputs.spacing.dispatchEvent(new Event('input'));d.inputs.spacing.dispatchEvent(new Event('change'));`);
  await waitFor(`Graph.el.nodes.length&&Graph.cache.has(Graph.key(Graph.filterSig()))&&Graph.key(Graph.filterSig()).includes('|s200|')`);await sleep(300);
  out.push('spacing 200: layout width '+w0+' -> '+await ev(`const xs=Graph.el.nodes.map(n=>n.x);return Math.round(Math.max(...xs)-Math.min(...xs))`)+', label '+await ev(`return Graph.el.outs.spacing.textContent`)+', zoom '+await ev(`return S.graph.view.zoom.toFixed(3)`)+', nodes in sight '+await ev(`return Graph.anyVisible()`));
  /* keys: arrow pans 40 px, Ctrl 0 fits */
  const p1=await ev(`return S.graph.view.panX`);await send('Input.dispatchKeyEvent',{type:'keyDown',key:'ArrowRight',code:'ArrowRight',windowsVirtualKeyCode:39});await send('Input.dispatchKeyEvent',{type:'keyUp',key:'ArrowRight',code:'ArrowRight',windowsVirtualKeyCode:39});await sleep(50);out.push('arrow right pan dx: '+await ev(`return Math.round(S.graph.view.panX-(${p1}))`));
  await send('Input.dispatchKeyEvent',{type:'keyDown',key:'0',code:'Digit0',windowsVirtualKeyCode:48,modifiers:2});await send('Input.dispatchKeyEvent',{type:'keyUp',key:'0',code:'Digit0',windowsVirtualKeyCode:48,modifiers:2});await sleep(50);out.push('fit zoom: '+await ev(`return S.graph.view.zoom.toFixed(3)`));
  await ev(`Graph.zoomStep(1)`);await sleep(700);out.push('saved: '+await ev(`return JSON.stringify(Object.fromEntries(Object.entries(KurasiFake.cur().settings).filter(([k])=>k.startsWith('graph.'))))`));
  await mouse('mouseMoved',5,5,0);await sleep(200);await shot('50_graph_toolbar_1920');
  /* reload: the view comes back from the settings */
  const saved=await ev(`const s=KurasiFake.cur().settings;return [s['graph.zoom'],s['graph.pan_x'],s['graph.pan_y'],s['graph.font_px'],s['graph.label_opacity'],s['graph.edge_opacity'],s['graph.spacing']].join(',')`);await nav('?fake=1');await ready();await ev(`switchTab('tags')`);await waitFor(`document.querySelector('.gbox svg .node')`);await sleep(200);
  const restored=await ev(`const v=S.graph.view;return [v.zoom,v.panX,v.panY,v.fontPx,v.labelOp,v.edgeOp,v.spacing].join(',')`);out.push(`restored equals saved: ${restored===saved} (${restored})`);
  /* level of detail: at 0.3x the labels keep a floor size and the busiest nodes keep theirs */
  await ev(`Graph.setView({zoom:0.3})`);await sleep(300);out.push('labels at 0.3x: '+await ev(`const d=Graph.el;return d.labs.length+' of '+d.nodes.length+' placed at '+getComputedStyle(d.gL.firstChild).fontSize+' graph units, first '+d.labs.slice(0,3).map(i=>d.nodes[i].label).join(', ')`));await ev(`Graph.fit()`);
  await ev(`setTheme('dark')`);await sleep(300);await shot('53_graph_dark_1920');await ev(`setTheme('system')`);
  /* narrow window: the toolbar folds behind the gear */
  await send('Emulation.setDeviceMetricsOverride',{width:1000,height:700,deviceScaleFactor:1,mobile:false});await sleep(400);await waitFor(`document.querySelector('.gbox svg .node')`);await sleep(200);
  out.push('gear visible at 1000: '+await ev(`const g=document.querySelector('.ggear');return g&&getComputedStyle(g).display!=='none'`)+', toolbar hidden: '+await ev(`return getComputedStyle(document.querySelector('.gtb')).display==='none'`));
  await shot('51_graph_toolbar_collapsed_1000');await click('.ggear');await sleep(200);await shot('52_graph_gear_open_1000');
  await ev(`Graph.resetView()`);await sleep(700);await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});console.log(out.join('\n'));},
 /* catalogue without pages: a chunk of cards first, more on scroll, the place kept across a redraw */
 async scroll(){await nav('?fake=1');await ready();await ev(`switchTab('reference')`);const out=[];
  await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};const base=S.refs;setRefs(Array.from({length:400},(_,i)=>{const b=base[i%base.length];return {...b,id:'syn'+i,title:b.title+' '+i};}));renderAll();`);await sleep(300);
  const c0=await ev(`return document.querySelectorAll('.grid .card').length`);out.push(`count label: ${await ev(`return document.querySelector('.cathead .pinfo').textContent`)}; first chunk ${c0} cards (under 400: ${c0<400})`);
  await ev(`const g=document.querySelector('.grid');g.scrollTop=g.scrollHeight;`);await sleep(250);const c1=await ev(`return document.querySelectorAll('.grid .card').length`);out.push(`after one scroll to the end: ${c1} cards (more: ${c1>c0})`);
  for(let i=0;i<40;i++){await ev(`const g=document.querySelector('.grid');g.scrollTop=g.scrollHeight;`);await sleep(60);}out.push('after scrolling through: '+await ev(`return document.querySelectorAll('.grid .card').length`)+' cards');
  await ev(`const g=document.querySelector('.grid');g.scrollTop=900;`);await sleep(150);await ev(`renderMain()`);await sleep(100);
  out.push('scrollTop after renderMain: '+await ev(`return Math.round(document.querySelector('.grid').scrollTop)`)+' (kept at 900), cards '+await ev(`return document.querySelectorAll('.grid .card').length`));
  await shot('54_catalogue_scroll');
  await ev(`S.sort='alpha';renderMain();`);await sleep(100);out.push('after a sort change: scrollTop '+await ev(`return document.querySelector('.grid').scrollTop`)+', cards '+await ev(`return document.querySelectorAll('.grid .card').length`));
  await ev(`switchTab('authors')`);await sleep(200);out.push('authors tab count label: '+await ev(`return document.querySelector('.cathead .pinfo').textContent`));
  await ev(`S.sort='relevance';switchTab('reference')`);console.log(out.join('\n'));},
 /* search chips: Enter and the + button keep the text as a filter chip, chips combine with AND and rank as one query
    holding all their words, the graph follows them, a chip comes off with its x, and a cached answer is used again */
 async chips(){await nav('?fake=1');await ready();await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};switchTab('reference')`);const out=[];
  await ev(`window.__sreq=[];const f0=window.fetch;window.fetch=(u,o)=>{if(String(u).includes('/api/search'))window.__sreq.push(decodeURIComponent(String(u).split('?')[1]));return f0(u,o);}`);
  const count=()=>ev(`return document.querySelector('.cathead .pinfo').textContent`);
  const settle=async()=>{await waitFor(`S.q===document.getElementById('q').value&&!FT.pending()`);await sleep(150);};
  const type=async t=>{await ev(`document.getElementById('q').focus()`);await send('Input.insertText',{text:t});await settle();};
  const enter=async()=>{await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await settle();};
  const state=()=>ev(`return JSON.stringify(qChips())+' box "'+document.getElementById('q').value+'"'`);
  out.push('library: '+await count());
  await type('labour');out.push('typed "labour": '+await count());await enter();out.push('after Enter: chips '+await state()+', '+await count());
  await type('smith');out.push('typed "smith": '+await count());await click('#qadd');await settle();out.push('after the + button: chips '+await state()+', '+await count()+', focus on '+await ev(`return document.activeElement.id`));
  out.push('chips in the filter bar: '+await ev(`return [...document.querySelectorAll('.cathead .chip.q')].map(c=>c.textContent).join(' ')`));
  await shot('62_search_chips');
  /* two chips rank as the one query "labour smith" */
  const rank=`return sorted(visibleRefs()).map(r=>r.id+':'+qScore(r,qEnts()).toFixed(6)).join(',')`;const byChips=await ev(rank);
  await ev(`window.__keep=qChips().slice();S.f.chips=[];setQuery('labour smith')`);await settle();const byQuery=await ev(rank);
  out.push('two chips rank as the joined query: '+(byChips===byQuery)+' ('+byChips.split(',').length+' references)');
  await ev(`setQuery('');S.f.chips=window.__keep;renderAll()`);await settle();
  /* empty text adds nothing; a duplicate in other letter case is left out and the box still empties */
  await type('   ');await enter();out.push('Enter on blank text: chips '+await state());await ev(`setQuery('')`);
  await type('SMITH');await enter();out.push('Enter on "SMITH": chips '+await state());
  /* the tag graph follows the chips through filterSig */
  await ev(`switchTab('tags')`);await waitFor(`document.querySelector('.gbox svg .node')||document.querySelector('.gbox .empty:not(:first-child)')`);await sleep(300);
  out.push('graph filterSig has the chips: '+await ev(`const s=Graph.filterSig();return s.includes('labour')&&s.includes('smith')`)+', graph nodes '+await ev(`return Graph.el&&Graph.el.nodes.length`)+' of '+await ev(`return (Graph.cache.get(Graph.key(''))||{nodes:[]}).nodes.length`)+' unfiltered');
  await shot('63_graph_with_chips');await ev(`switchTab('reference')`);await sleep(200);
  /* the x of the "smith" chip removes it and the list widens again */
  await ev(`[...document.querySelectorAll('.cathead .chip.q')].find(c=>/smith/.test(c.textContent)).querySelector('b').id='rmchip'`);
  await click('#rmchip');await settle();out.push('after removing "smith": chips '+await state()+', '+await count());await shot('64_search_chip_removed');
  /* typing next to a chip asks only for the new text; the chip answer is reused */
  const n0=await ev(`return window.__sreq.length`);await type('wealth');out.push('requests for "wealth" next to the chip: '+await ev(`return JSON.stringify(window.__sreq.slice(${n0}))`)+', '+await count());
  out.push('all search requests: '+await ev(`return window.__sreq.join(' ; ')`));
  await ev(`clearFilters()`);console.log(out.join('\n'));},
 /* the finder at the top of the filter pane narrows the Author and Tag entries only; a ticked entry stays; Enter does
    nothing; the clear control empties it */
 async panesearch(){await nav('?fake=1');await ready();await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};switchTab('reference')`);const out=[];
  const main=()=>ev(`return Graph.filterSig()+' | '+[...document.querySelectorAll('.grid .card.ref .ctitle')].map(e=>e.textContent).join(' / ')`);
  const pane=()=>ev(`const rows=k=>{const x=[...document.querySelectorAll('#filter .fsec')].find(s=>s.querySelector('.fhead span:nth-child(2)').textContent===k);if(!x)return '';const l=[...x.querySelectorAll('.trow .lbl')].map(e=>e.textContent);return (l.length?l.join(', '):x.querySelector('.fbody').textContent)+' ['+x.querySelector('.fhead .cnt').textContent+']';};return 'Author: '+rows('Author')+' | Tag: '+rows('Tag');`);
  await ev(`S.f.tags=new Set(['quote']);renderAll()`);await sleep(200);const m0=await main();out.push('pane with the tag "quote" ticked: '+await pane());
  await ev(`document.getElementById('fq').focus()`);await send('Input.insertText',{text:'taylr'});await sleep(400);
  out.push('finder "taylr": '+await pane());out.push('focus stays in the finder: '+await ev(`return document.activeElement&&document.activeElement.id==='fq'`)+'; main pane unchanged: '+(await main()===m0));
  await shot('65_pane_finder_author');
  await ev(`document.getElementById('fq').select()`);await send('Input.insertText',{text:'efficency'});await sleep(400);
  out.push('finder "efficency": '+await pane());out.push('main pane unchanged: '+(await main()===m0));await shot('66_pane_finder_tag');
  const snap=`return JSON.stringify([FP.q,document.getElementById('fq').value,Graph.filterSig(),qChips(),S.q,document.getElementById('filter').textContent])`;const s0=await ev(snap);
  await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await sleep(400);
  out.push('Enter in the finder changes nothing: '+(s0===await ev(snap))+'; main pane unchanged: '+(await main()===m0));
  await click('.fsearch .clr');await sleep(250);out.push('after the clear control: box "'+await ev(`return document.getElementById('fq').value`)+'", '+await pane());
  await ev(`clearFilters()`);console.log(out.join('\n'));},
 /* filter pane order: categories A to Z with Uncategorized first, authors by surname within People and by name within
    Organisations with Unknown last, tags A to Z. Two sterile organisation names are set on the exhibits first. */
 async paneorder(){await nav('?fake=1');await ready();await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};switchTab('reference')`);const out=[];
  await ev(`const ex=S.refs.filter(r=>/^Exhibit/.test(r.title));await PUT('/api/reference/'+encodeURIComponent(ex[0].id),{authors:['Zeta Sample Bureau']});await PUT('/api/reference/'+encodeURIComponent(ex[1].id),{authors:['Alpha Sample Office']});
   await refreshData({ids:null,cats:true});S.cats.forEach(n=>{if(n.c)S.exp.add(n.p);});renderFilter();`);await sleep(200);
  const r=await ev(`const C=new Intl.Collator(undefined,{sensitivity:'base',numeric:true}).compare,isAz=l=>l.every((x,i)=>!i||C(l[i-1],x)<=0);
   const sec=k=>[...document.querySelectorAll('#filter .fsec')].find(s=>s.querySelector('.fhead span:nth-child(2)').textContent===k);
   const rows=k=>[...sec(k).querySelectorAll('.trow')].map(r=>({d:+r.style.getPropertyValue('--d'),t:r.querySelector('.lbl').textContent}));
   const kids=(l,name)=>{const i=l.findIndex(x=>x.d===0&&x.t===name);if(i<0)return [];const o=[];for(let j=i+1;j<l.length&&l[j].d>0;j++)if(l[j].d===1)o.push(l[j].t);return o;};
   const cat=rows('Reference'),top=cat.filter(x=>x.d===0).map(x=>x.t),au=rows('Author'),people=kids(au,'People'),orgs=kids(au,'Organisations'),atop=au.filter(x=>x.d===0).map(x=>x.t),tags=rows('Tag').map(x=>x.t.slice(1));
   return ['categories: '+top.join(', ')+' / Management: '+kids(cat,'Management').join(', '),
    'Uncategorized first: '+(top[0]==='Uncategorized')+', the rest A to Z: '+isAz(top.slice(1))+', subcategories A to Z: '+isAz(kids(cat,'Management')),
    'People: '+people.join('; ')+' / A to Z by surname: '+isAz(people.map(p=>p.split(',')[0])),
    'Organisations: '+orgs.join('; ')+' / A to Z: '+isAz(orgs)+'; Unknown last: '+(atop[atop.length-1]==='Unknown'),
    'tags: '+tags.join(', ')+' / A to Z: '+isAz(tags)].join('\\n');`);
  out.push(r);await shot('67_pane_a_to_z');console.log(out.join('\n'));},
 /* pane rows under content-visibility: a long Author list keeps its scroll place, ticks, keyboard focus and the finder
    caret. Every check also runs with the containment switched off, and the two runs must agree. */
 async panerows(){await nav('?fake=1');await ready();const out=[];
  await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};${bigLib(2000,false)}switchTab('reference');renderAll();`);await sleep(400);
  const f='document.getElementById("filter")',mid=`const b=${f}.getBoundingClientRect();const e=document.elementFromPoint(b.left+60,b.top+b.height/2);const row=e&&e.closest('.trow');return [${f}.scrollTop,row&&row.textContent].join(' ')`;
  const run=async off=>{const r={};
   await ev(`clearFilters();let s=document.getElementById('cvoff');if(${off}&&!s){s=document.createElement('style');s.id='cvoff';s.textContent='.trow{content-visibility:visible!important}';document.head.append(s);}if(!${off}&&s)s.remove();renderFilter();`);await sleep(300);
   r.cv=await ev(`return getComputedStyle(document.querySelector('#filter .trow')).contentVisibility`);
   r.height=await ev(`return ${f}.scrollHeight`);
   await ev(`${f}.scrollTop=3000`);await sleep(300);r.mid=await ev(mid);await sleep(300);r.midLater=await ev(mid);
   /* keyboard: a checkbox far down takes the focus and scrolls into sight; Space ticks it */
   r.focus=await ev(`const row=[...document.querySelectorAll('#filter .trow')][1500];const cb=row.querySelector('input');cb.focus();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));const b=row.getBoundingClientRect(),p=${f}.getBoundingClientRect();window.__name=row.querySelector('.lbl').textContent;return [document.activeElement===cb,b.top>=p.top&&b.bottom<=p.bottom,window.__name,${f}.scrollTop].join(' ')`);
   await send('Input.dispatchKeyEvent',{type:'keyDown',key:' ',code:'Space',windowsVirtualKeyCode:32,text:' '});await send('Input.dispatchKeyEvent',{type:'keyUp',key:' ',code:'Space',windowsVirtualKeyCode:32});await sleep(400);
   r.tick=await ev(`const n=window.__name;const row=[...document.querySelectorAll('#filter .trow')].find(r=>r.querySelector('.lbl').textContent===n);return [S.f.authors.has(n),row&&row.querySelector('input').checked,${f}.scrollTop].join(' ')`);
   /* finder: focus and caret stay through the redraw */
   await ev(`const i=document.getElementById('fq');i.focus();i.value='';`);await send('Input.insertText',{text:'author1'});await sleep(400);
   r.finder=await ev(`const i=document.getElementById('fq');return [document.activeElement===i,i.selectionStart,i.value,[...document.querySelectorAll('#filter .fhead .cnt')].map(c=>c.textContent).join('/')].join(' ')`);
   if(!off)await shot('68_pane_rows_contained');
   await ev(`FP.q='';FP.inp.value='';FP.clr.hidden=true;clearFilters();`);return r;};
  const off=await run(true),on=await run(false);
  for(const k of Object.keys(on))out.push(`${k}: on ${on[k]} | off ${off[k]}${k==='cv'||on[k]===off[k]?'':'  DIFFERS'}`);
  console.log(out.join('\n'));},
 /* a synthetic library with skewed tags: overlaps, mark sizes, labels, smoothness and where the time goes.
    KURASI_BIG_REFS sets the number of references: 300 (the default) give 444 nodes and the shots 55 and 56;
    3000 give about 5,900 nodes and 8000 about 15,100, with the shots 57 to 59 named after the count */
 async biggraph(){const N=Math.max(10,+process.env.KURASI_BIG_REFS||300),big=N!==300;await send('Emulation.setDeviceMetricsOverride',{width:1920,height:1080,deviceScaleFactor:1,mobile:false});await nav('?fake=1');await ready();const out=[];
  await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};${bigLib(N,false)}window.__t0=performance.now();switchTab('tags');`);
  await waitFor(`document.querySelector('.gbox svg .node')`,600000);await sleep(300);
  out.push(`${N} references; first draw after switchTab: `+await ev(`return Math.round(performance.now()-window.__t0)`)+' ms (layout in the worker included)');
  const probe=`const g=Graph.cache.get(Graph.key(Graph.filterSig()));const t0=performance.now();drawGraph(Graph.el.box,g,{auto:false});const t1=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));const t2=performance.now();Graph.fit();Graph.labels();
   const d=Graph.el,C=64,grid=new Map();let ov=0;d.nodes.forEach(n=>{const k=Math.floor(n.x/C)+','+Math.floor(n.y/C);(grid.get(k)||grid.set(k,[]).get(k)).push(n);});
   for(const a of d.nodes){const cx=Math.floor(a.x/C),cy=Math.floor(a.y/C);for(let i=-1;i<=1;i++)for(let j=-1;j<=1;j++)for(const b of grid.get((cx+i)+','+(cy+j))||[])if(b.i>a.i&&Math.hypot(a.x-b.x,a.y-b.y)<a.size+b.size-0.5)ov++;}
   const cnt=t=>d.nodes.filter(n=>n.type===t).length,top=d.nodes.filter(n=>n.type==='tag').sort((a,b)=>b.deg-a.deg).slice(0,3).map(n=>n.label+' '+n.deg+' papers r '+n.r).join(', ');
   return g.nodes.length+' nodes ('+cnt('paper')+' papers, '+cnt('author')+' authors, '+cnt('tag')+' tags), '+g.edges.length+' edges; layout '+g.ms+' ms in the worker, '+g.iters+' rounds; draw '+Math.round(t1-t0)+' ms build + '+Math.round(t2-t1)+' ms to paint; overlapping marks '+ov+'; after the fit: zoom '+S.graph.view.zoom.toFixed(3)+', '+d.labs.length+' labels; busiest tags: '+top;`;
  /* frame intervals while the view moves one step per frame (after 1.5 s of rest, so the drawing just replaced has been collected), then hover on the busiest node and one label pass */
  const smooth=`const run=async f=>{const ts=[];for(let i=0;i<=40;i++){await new Promise(r=>requestAnimationFrame(r));ts.push(performance.now());f(i);}const d=ts.slice(1).map((t,k)=>t-ts[k]).sort((a,b)=>a-b);return 'median '+d[d.length>>1].toFixed(1)+' ms, p90 '+d[Math.floor(d.length*0.9)].toFixed(1)+' ms, max '+d[d.length-1].toFixed(1)+' ms';};
   const [cx,cy]=Graph.centre();const pan1=await run(i=>Graph.pan(i<20?8:-8,i<20?4:-4)),pan=await run(i=>Graph.pan(i<20?8:-8,i<20?4:-4));const zoom=await run(i=>Graph.zoomAt(i<20?1.04:1/1.04,cx,cy));
   const n=Graph.el.nodes.reduce((a,b)=>b.deg>a.deg?b:a);const t0=performance.now();Graph.hover(n.i);const t1=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));const t2=performance.now();Graph.unhover();
   const t3=performance.now();Graph.labels();const t4=performance.now();Graph.place(true);const t5=performance.now();
   return 'frame interval panning, first pass after the drawing: '+pan1+'; panning: '+pan+'; zooming: '+zoom+'; hover on '+n.label+' ('+n.deg+' links) '+Math.round(t1-t0)+' ms + '+Math.round(t2-t1)+' ms to paint; label pass '+Math.round(t4-t3)+' ms ('+Graph.el.labs.length+' labels); canvases painted again at rest '+Math.round(t5-t4)+' ms';`;
  out.push('spacing 100: '+await ev(probe));await sleep(300);
  if(!big){await shot('55_biggraph_spacing100');
   await ev(`Graph.setView({spacing:200})`);await waitFor(`Graph.key(Graph.filterSig()).includes('|s200|')&&Graph.cache.has(Graph.key(Graph.filterSig()))`,60000);await sleep(400);
   out.push('spacing 200: '+await ev(probe));await sleep(300);await shot('56_biggraph_spacing200');}
  else{await shot(`57_biggraph_${N}_fit`);await sleep(1500);out.push(await ev(smooth));
   /* 100% around the busiest tag, then hover on it with the mouse */
   const p=await ev(`const d=Graph.el,n=d.nodes.filter(n=>n.type==='tag').reduce((a,b)=>b.deg>a.deg?b:a);const [cx,cy]=Graph.centre();Graph.setView({zoom:1,panX:cx-n.x-120,panY:cy-n.y});Graph.labels();const r=d.els[n.i].querySelector('.mark').getBoundingClientRect();return [r.left+r.width/2,r.top+r.height/2];`);
   await sleep(400);out.push('at 100%: '+await ev(`return Graph.el.labs.length`)+' labels');await shot(`58_biggraph_${N}_zoom100`);
   await mouse('mouseMoved',p[0],p[1],0);await sleep(400);await shot(`59_biggraph_${N}_hover`);await mouse('mouseMoved',5,5,0);}
  await ev(`Graph.resetView()`);await sleep(700);await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});console.log(out.join('\n'));},
 /* one-off measurement with a large generated library (KURASI_BIG_REFS, default 10000) and search text on every
    third reference: a search keystroke, the filter pane, tab switches and the Tag tab. Opt-in: node shoot.mjs bigui */
 async bigui(){const N=+process.env.KURASI_BIG_REFS||10000;await send('Emulation.setDeviceMetricsOverride',{width:1920,height:1080,deviceScaleFactor:1,mobile:false});await nav('?fake=1');await ready();const out=[];
  await ev(`clearTimeout(Poll.timer);Poll.start=()=>{};${bigLib(N,true)}switchTab('reference');`);await sleep(500);
  const T=async(label,code)=>{const r=await ev(`const t0=performance.now();${code};const t1=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return [Math.round(t1-t0),Math.round(performance.now()-t1)];`);out.push(`${label}: ${r[0]} ms + ${r[1]} ms to paint`);};
  out.push(await ev(`return S.refs.length+' references, '+authorList(S.refs).length+' authors, '+S.tagRows.length+' tags, '+S.catPaths.length+' categories, search text '+window.__textKB+' KB (kept by the backend)'`));
  /* a keystroke draws the metadata matches at once; the full-text answer from /api/search follows after the
     debounce (FT.DELAY) and the fake's 20 to 60 ms latency and draws again */
  const K=async(label,q)=>{const r=await ev(`const st=FT.stamp,t0=performance.now();S.q=${JSON.stringify(q)};renderMain();const t1=performance.now();const n1=document.querySelector('.cathead .pinfo').textContent;
    await new Promise(r=>{const f=()=>FT.stamp!==st||performance.now()-t0>10000?r():setTimeout(f,2);f();});const t2=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
    const t3=performance.now(),f0=performance.now();KurasiFake.textSearch(KurasiFake.cur(),${JSON.stringify(q)},looseOn());const f1=performance.now();
    return [Math.round(t1-t0),n1,Math.round(t2-t0),Math.round(t3-t0),document.querySelector('.cathead .pinfo').textContent,Math.round(f1-f0),FT.DELAY];`);
   out.push(`${label}: metadata matches drawn in ${r[0]} ms (${r[1]}); full-text answer drawn at ${r[2]} ms, painted at ${r[3]} ms (${r[4]}); debounce ${r[6]} ms, fake search ${r[5]} ms`);};
  await K('search keystroke, first after load ("organisa")','organisa');await K('search keystroke, next ("organisat")','organisat');
  await K('search keystroke, a word found only in search text ("kalomir")','kalomir');
  /* one partial refresh after an edit, against the whole list for comparison */
  await ev(`S.q='';renderMain();await PUT('/api/reference/big5',{title:'Synthetic study number 5 on organisational change, revised'})`);
  const R=async(label,code)=>{const r=await ev(`const t0=performance.now();${code};const t1=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return [Math.round(t1-t0),Math.round(performance.now()-t1),refById('big5').title];`);out.push(`${label}: ${r[0]} ms + ${r[1]} ms to paint (${r[2]})`);};
  await R('partial refresh of one reference after an edit',`await refreshData({hlRefs:[],ids:['big5']})`);
  await R('whole-list refresh, for comparison',`await refreshData({ids:null})`);
  await T('search cleared',`S.q='';renderMain()`);await T('filter pane',`renderFilter()`);await T('status bar',`renderStatus()`);
  /* the filter-pane finder and the search chips */
  out.push('one sort, best of 5, as az (localeCompare, code order on ties) / Intl.Collator base and numeric / bare localeCompare: '+await ev(`const run=(l,f)=>{let b=1e9;for(let k=0;k<5;k++){const c=[...l];const t0=performance.now();c.sort(f);b=Math.min(b,performance.now()-t0);}return b.toFixed(1);},lc=(a,b)=>a.localeCompare(b),cb=new Intl.Collator(undefined,{sensitivity:'base',numeric:true}).compare;
   const names=authorList(S.refs).map(a=>a.name),tags=S.tagRows.map(t=>t.name),titles=S.refs.map(r=>r.title),all=l=>run(l,az)+' / '+run(l,cb)+' / '+run(l,lc)+' ms';
   return 'authors '+all(names)+', tags '+all(tags)+', titles '+all(titles)`));
  const cnt=async()=>out.push('  entries shown (Author, Tag): '+await ev(`return [...document.querySelectorAll('#filter .fsec .fhead .cnt')].slice(1).map(c=>c.textContent).join(', ')`));
  await T('filter pane, one finder keystroke that every synthetic name comes near ("author12")',`FP.node();FP.q='author12';FP.inp.value=FP.q;renderFilter()`);await cnt();
  await T('filter pane, one finder keystroke that leaves few entries ("zqx")',`FP.q='zqx';FP.inp.value=FP.q;renderFilter()`);await cnt();
  await T('filter pane, finder cleared',`FP.q='';FP.inp.value='';renderFilter()`);
  const ask=async w=>{await ev(`const q=document.getElementById('q');q.value=${JSON.stringify(w)};S.q=q.value;renderMain();const t0=performance.now();await new Promise(r=>{const f=()=>!FT.pending()||performance.now()-t0>10000?r():setTimeout(f,5);f();});`);};
  /* medians of three for the two costs that the pane row containment aims at */
  const M=async(label,setup,code)=>{const rs=[];for(let i=0;i<3;i++){if(setup)await setup();rs.push(await ev(`const t0=performance.now();${code};const t1=performance.now();await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return [Math.round(t1-t0),Math.round(performance.now()-t1)];`));}
   rs.sort((a,b)=>a[0]+a[1]-b[0]-b[1]);out.push(`${label}, median of 3: ${rs[1][0]} ms + ${rs[1][1]} ms to paint (runs ${rs.map(r=>r[0]+'+'+r[1]).join(', ')})`);};
  await M('one filter-pane redraw',null,`renderFilter()`);
  await M('first chip by Enter ("organisa", its answer already in)',async()=>{await ev(`clearFilters()`);await sleep(300);await ask('organisa');},`addChip()`);out.push('  '+await ev(`return document.querySelector('.cathead .pinfo').textContent`));
  await ask('kalomir');await T('second chip by Enter ("kalomir")',`addChip()`);out.push('  '+await ev(`return document.querySelector('.cathead .pinfo').textContent`));
  await K('search keystroke next to two chips ("synthetic")','synthetic');
  await T('remove a chip',`removeChip(qChips()[0])`);await ev(`clearFilters()`);await sleep(300);
  await T('switch to Authors',`switchTab('authors')`);await T('switch to Reference',`switchTab('reference')`);
  out.push('whole-project layout in the worker: '+await ev(`const t0=performance.now();const g=await Graph.request('',S.refs);return Math.round(performance.now()-t0)+' ms wait, '+g.ms+' ms in the worker, '+g.nodes.length+' nodes'`));
  await T('open the Tag tab (layout cached)',`switchTab('tags');await new Promise(r=>{const f=()=>document.querySelector('.gbox svg .node')?r():requestAnimationFrame(f);f();})`);
  await T('back to Reference',`switchTab('reference')`);await T('Tag tab again (same layout, drawing kept)',`switchTab('tags')`);
  await shot('60_bigui_tag');await ev(`switchTab('reference')`);await sleep(300);await shot('61_bigui_catalogue');
  await send('Emulation.setDeviceMetricsOverride',{width:1536,height:864,deviceScaleFactor:1.25,mobile:false});console.log(out.join('\n'));}
};
const want=process.argv.slice(2);const order=Object.keys(SC).filter(k=>k!=='bigui');const run=want.length?want:order;
try{if(!want.length||!want.includes('splash')&&!want.includes('catalogue')&&!['welcome','missing','cloud','dark','mcp','scan','tour','splash','catalogue'].includes(run[0])){await nav('?fake=1');await ready();}
 for(const n of run){if(!SC[n]){console.log('unknown scenario',n);continue;}try{await SC[n]();}catch(e){console.log('scenario',n,'failed:',e.message);}}}
finally{console.log(logs.length?'console:\n'+logs.slice(0,80).join('\n'):'console: clean');ws.close();
 /* Chrome keeps its child processes when only the parent is killed */
 try{spawnSync('taskkill',['/PID',String(chrome.pid),'/T','/F'],{stdio:'ignore'});}catch(e){chrome.kill();}server.close();}
process.exit(0);
