'use strict';
/* Kurasi UI core: helpers, API client, state, startup and splash, catalogue, filters, tabs,
   edit pane, tag graph and live refresh. Every record comes from the local API (contract
   section 6) through root-relative URLs. */

/* ---------- helpers ---------- */
const $=(s,r=document)=>r.querySelector(s), $$=(s,r=document)=>[...r.querySelectorAll(s)];
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
function el(tag,attrs={},...kids){const e=document.createElement(tag);for(const [k,v] of Object.entries(attrs||{})){if(v===false||v==null)continue;if(k==='class')e.className=v;else if(k==='html')e.innerHTML=v;else if(k.startsWith('on')&&typeof v==='function')e.addEventListener(k.slice(2),v);else if(k==='hidden')e.hidden=!!v;else e.setAttribute(k,v);}for(const k of kids.flat(Infinity)){if(k==null||k===false)continue;e.append(k.nodeType?k:document.createTextNode(k));}return e;}
const ls={get(k){try{return localStorage.getItem(k);}catch(e){return null;}},set(k,v){try{localStorage.setItem(k,v);}catch(e){}}};
const trunc=(s,n=22)=>{s=String(s);return s.length<=n?s:s.slice(0,Math.ceil(n*0.55))+'…'+s.slice(-(Math.floor(n*0.45)-1));};
/* filter-pane labels: cut at the end, nothing after the ellipsis, 75 letters at most */
const truncEnd=(s,n=75)=>{s=String(s);return s.length<=n?s:s.slice(0,n-1).trimEnd()+'…';};
const words=(s,n=8)=>{const w=String(s).split(/\s+/);return w.length<=n?String(s):w.slice(0,n).join(' ')+'…';};
const IMG_EXT=new Set(['png','jpg','jpeg','gif','webp','bmp']);
const isImgExt=e=>IMG_EXT.has(String(e||'').toLowerCase());
const baseName=p=>String(p||'').split(/[\\/]/).pop();
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
/* two animation frames, so the page is laid out and painted. The desktop window loads hidden behind
   the splash and a hidden page runs no animation frames: there the layout is forced and the promise
   resolves at once. The timer covers any other case where frames stop. */
const nextFrame=()=>new Promise(r=>{if(document.visibilityState==='hidden'){void document.body.offsetHeight;r();return;}let done=false;const f=()=>{if(!done){done=true;r();}};requestAnimationFrame(()=>requestAnimationFrame(f));setTimeout(f,150);});
const enc=encodeURIComponent;
/* append that skips null and false (Element.append would print them) */
const put=(p,...k)=>{p.append(...k.flat().filter(x=>x!=null&&x!==false));return p;};
const plural=(n,one,many)=>`${n} ${n===1?one:(many||one+'s')}`;
/* text order for every sort in the filter pane and the catalogue: localeCompare with no options, which V8 runs on a
   fast path (a cached Intl.Collator measured 3 to 8 times slower). Strings it holds equal fall back to code order, so
   a sort never depends on the order it started from. ISO timestamps sort in code order, which equals their time order. */
const byCode=(a,b)=>a<b?-1:a>b?1:0;
const az=(a,b)=>a.localeCompare(b)||byCode(a,b);
/* filter-pane order: Uncategorized first, Unknown last, A to Z between */
const paneAz=(a,b)=>(b==='Uncategorized')-(a==='Uncategorized')||(a==='Unknown')-(b==='Unknown')||az(a,b);
function fmtWhen(v){if(v==null||v==='')return '';const d=typeof v==='number'?new Date(v<1e12?v*1000:v):new Date(String(v));if(isNaN(d))return String(v);const p=n=>String(n).padStart(2,'0');const t=`${p(d.getHours())}:${p(d.getMinutes())}`;const now=new Date();if(d.toDateString()===now.toDateString())return t;return `${d.getFullYear()}-${p(d.getMonth()+1)}-${p(d.getDate())} ${t}`;}

/* ---------- API client ---------- */
class ApiError extends Error{constructor(msg,status){super(msg);this.status=status;}}
const NO_SERVER='Kurasi cannot reach its server. Check that the Kurasi window is still open, then try again.';
async function api(method,path,body,signal){
  const opt={method,headers:{}};if(signal)opt.signal=signal;
  if(body!==undefined){opt.headers['Content-Type']='application/json';opt.body=JSON.stringify(body);}
  let res;try{res=await fetch(path,opt);}catch(e){if(e&&e.name==='AbortError')throw e;throw new ApiError(NO_SERVER,0);}
  let data=null;const ct=res.headers.get('content-type')||'';
  try{data=ct.includes('json')?await res.json():await res.text();}catch(e){data=null;}
  if(!res.ok){const m=data&&typeof data==='object'&&data.error?String(data.error):`The server answered ${res.status} to ${method} ${path.split('?')[0]}. Try again; restart Kurasi if it repeats.`;throw new ApiError(m,res.status);}
  return data;}
/* signal: an AbortController signal; an aborted request rejects with the AbortError itself */
const GET=(p,signal)=>api('GET',p,undefined,signal), POST=(p,b)=>api('POST',p,b===undefined?{}:b), PUT=(p,b)=>api('PUT',p,b===undefined?{}:b), DEL=p=>api('DELETE',p);
async function apiRaw(path,signal){let res;try{res=await fetch(path,signal?{signal}:undefined);}catch(e){if(e&&e.name==='AbortError')throw e;throw new ApiError(NO_SERVER,0);}
  if(!res.ok){let m=`The server answered ${res.status} for this file.`;try{const j=await res.clone().json();if(j&&j.error)m=String(j.error);}catch(e){}throw new ApiError(m,res.status);}return res;}
const apiBlob=async(p,signal)=>(await apiRaw(p,signal)).blob();
const apiText=async p=>(await apiRaw(p)).text();
function fail(e){console.warn(e);toast(e&&e.message?e.message:String(e),{err:true,ms:9000});}

/* pywebview js_api, present only inside the desktop window */
function pyFn(name){const p=window.pywebview;return p&&p.api&&typeof p.api[name]==='function'?p.api[name].bind(p.api):null;}
function pyCall(name,...a){const f=pyFn(name);if(!f)return false;try{Promise.resolve(f(...a)).catch(()=>{});}catch(e){}return true;}
/* Tauri 2 bridge. window.__TAURI__ exists only when tauri.conf.json sets app.withGlobalTauri to true;
   without it the page behaves as in a plain browser. */
const tauri=()=>window.__TAURI__||null;
function tauriEmit(name,payload){const t=tauri();if(!t||!t.event||typeof t.event.emit!=='function')return false;try{Promise.resolve(t.event.emit(name,payload)).catch(()=>{});}catch(e){}return true;}
/* inside a desktop shell (pywebview or Tauri) the shell owns the splash window */
const hosted=()=>!!(window.pywebview||tauri());

/* ---------- state ---------- */
const S={project:null,projects:[],settings:{},demo:false,startup:null,counts:null,changeSeq:0,dataVer:0,offline:false,
 refs:[],byId:new Map(),cats:[],catPaths:[],catByPath:new Map(),tagRows:[],refTags:new Map(),hl:new Map(),
 q:'',sort:'relevance',tab:'reference',tabs:[],tabFile:new Map(),sel:null,
 f:{cats:new Set(),authors:new Set(),tags:new Set(),refs:new Set(),chips:[]},kf:{tags:new Set(),authors:new Set()},
 right:null,exp:new Set(['People','Organisations']),expInit:false,fclosed:new Set(),kopen:new Set(),kedit:new Set(),kdraft:new Map(),
 ui:{filterPane:ls.get('kurasi.filterPane')!=='0',theme:ls.get('kurasi.theme')||''},
 graph:{view:{zoom:1,panX:0,panY:0,fontPx:12,labelOp:100,edgeOp:60,spacing:100}},
 tour:{on:false,i:0},hintShown:new Set(),undo:[]};
const V=new Map(); /* per-document viewer state, keyed by reference id */
const CAT_TABS=['reference','authors','tags'];
const isDocTab=k=>!CAT_TABS.includes(k);
const refById=id=>S.byId.get(id)||null;
const docTabRef=()=>isDocTab(S.tab)?refById(S.tab):null;
const hlOf=id=>S.hl.get(id)||[];
const aiName=()=>String(S.settings.ai_author||'Claude');
const hintsOn=()=>S.settings.mcp_hints!==false;
const looseOn=()=>S.settings.loose!==false;
const autoMin=()=>S.project?(+(S.project.auto_scan_min??S.settings.auto_scan_min)||0):0;
const isRO=()=>!!(S.project&&(S.project.read_only||S.settings.read_only));
const lastScan=()=>S.project?(S.project.last_scan||S.settings.last_scan||''):'';

/* ---------- startup: splash overlay or the desktop splash window ---------- */
const Boot={t0:performance.now(),done:false,notified:false,status:'Starting',pending:new Set(['project state']),safety:null,
 start(){if(hosted())this.drop(true);
  window.addEventListener('pywebviewready',()=>{this.drop(true);this.say(this.status);if(this.done&&!this.notified)this.notified=pyCall('app_ready');});
  this.safety=setTimeout(()=>this.onSafety(),+window.KURASI_SAFETY_MS||30000);},
 say(text){this.status=text;const s=$('#splashStatus');if(s)s.textContent=text;pyCall('splash_status',text);tauriEmit('splash-status',{text});},
 drop(now){const sp=$('#splash');if(!sp)return;if(now){sp.remove();return;}sp.classList.add('out');setTimeout(()=>sp.remove(),450);},
 /* ready: the first catalogue page is rendered. The Tauri shell waits for kurasi-ready before it shows
    the main window; the data-ready attribute serves a shell that polls the document instead. */
 async ready(){if(this.done)return;this.done=true;clearTimeout(this.safety);this.pending.clear();this.notified=pyCall('app_ready');
  document.documentElement.dataset.ready='1';tauriEmit('kurasi-ready',{});
  if(!hosted())await sleep(Math.max(0,2000-(performance.now()-this.t0)));this.drop();},
 onSafety(){if(this.done)return;this.drop();const left=[...this.pending];
  toast(`Kurasi is still loading ${left.length?left.join(', '):'the project'}. You can start working; the rest fills in when it arrives.`,{ms:12000});}};

/* ---------- data loading ---------- */
async function loadState(){const st=await GET('/api/state')||{};const prev=S.project&&S.project.id;
 S.project=st.project&&st.project.id?st.project:null;S.projects=Array.isArray(st.projects)?st.projects:[];S.settings=st.settings&&typeof st.settings==='object'?st.settings:{};
 S.demo=!!st.demo;S.version=st.version||'';S.startup=st.startup||null;S.previousProjectId=st.previous_project_id||null;S.counts=st.counts||null;
 const seq=+st.change_seq||0;if(!S.project||(S.project&&S.project.id)!==prev||seq>S.changeSeq)S.changeSeq=seq;
 /* the graph view is restored from the project settings when a project opens; later state reloads
    (for example after this page saved the view) leave the live view alone */
 if((S.project&&S.project.id)!==prev)Graph.loadView();
 applyTheme();return st;}
function normRef(r){const o={...r};o.title=String(r.title||'(untitled)');
 o.authors=Array.isArray(r.authors)&&r.authors.length?r.authors.map(String):['Unknown'];
 o.categories=Array.isArray(r.categories)&&r.categories.length?r.categories.map(String):['Uncategorized'];
 o.files=(Array.isArray(r.files)?r.files:[]).map(f=>({...f,ext:String(f.ext||baseName(f.relpath).split('.').pop()||'').replace(/^\./,'').toLowerCase()}));
 if(!o.files.length&&r.primary_file_id)o.files=[{id:r.primary_file_id,relpath:'',role:'primary',ext:String(r.ext||'').toLowerCase(),pages:r.pages,missing:!!r.missing,cloud_only:false}];
 o.ext=String(r.ext||(o.files[0]&&o.files[0].ext)||'').replace(/^\./,'').toLowerCase();
 o.hl_count=+r.hl_count||0;o.top_tags=(Array.isArray(r.top_tags)?r.top_tags:[]).map(t=>String(t).replace(/^#/,''));
 o.created_at=String(r.created_at||'');o.updated_at=String(r.updated_at||'');o.status=r.status||'Auto';delete o.search_text;o._f=null;return o;}
function setRefs(list,normed){S.refs=(Array.isArray(list)?list:[]).map(r=>normed?r:normRef(r));S.byId=new Map(S.refs.map(r=>[r.id,r]));S.dataVer++;SI.reset();
 S.tabs=S.tabs.filter(id=>S.byId.has(id));[...V.keys()].forEach(id=>{if(!S.byId.has(id))disposeViewer(id);});
 if(isDocTab(S.tab)&&!S.byId.has(S.tab))S.tab='reference';if(S.right&&S.right.mode==='edit'&&!S.byId.has(S.right.id))S.right=null;
 [...S.f.refs].forEach(id=>{if(!S.byId.has(id))S.f.refs.delete(id);});}
/* partial refresh: the references named by `ids` take the records the server sent, and those it left out (moved to
   the trash) leave the list; every other reference keeps its place */
function patchRefs(ids,list){const got=new Map((Array.isArray(list)?list:[]).map(r=>{const o=normRef(r);return [o.id,o];}));const want=new Set(ids);const out=[];
 for(const r of S.refs){if(!want.has(r.id)){out.push(r);continue;}const n=got.get(r.id);if(n){out.push(n);got.delete(r.id);}}
 got.forEach(n=>out.push(n));setRefs(out,true);}
function setCats(list){let tree=Array.isArray(list)?list:[];
 /* a flat list with parent_id becomes a tree */
 if(tree.length&&tree.every(c=>!Array.isArray(c.children))&&tree.some(c=>'parent_id' in c)){const m=new Map(tree.map(c=>[c.id,{...c,children:[]}]));const roots=[];m.forEach(c=>{const p=c.parent_id!=null&&m.get(c.parent_id);(p?p.children:roots).push(c);});tree=roots;}
 const conv=(c,prefix)=>{const n=String(c.name||'');const p=c.path?String(c.path):(prefix?prefix+'/'+n:n);const kids=(Array.isArray(c.children)?c.children:[]).map(ch=>conv(ch,p));return {id:c.id,n,p,c:kids.length?kids:null,parent:null};};
 /* A to Z at every level, Uncategorized first */
 const order=ns=>{ns.sort((a,b)=>paneAz(a.n,b.n));ns.forEach(n=>{if(n.c)order(n.c);});return ns;};
 S.cats=order(tree.map(c=>conv(c,'')));
 S.catByPath=new Map();S.catPaths=[];const walk=(ns,parent)=>ns.forEach(n=>{n.parent=parent;S.catByPath.set(n.p,n);S.catPaths.push(n.p);if(n.c)walk(n.c,n);});walk(S.cats,null);
 if(!S.expInit&&S.cats.length){S.expInit=true;S.cats.forEach(n=>{if(n.c)S.exp.add(n.p);});}
 [...S.f.cats].forEach(p=>{if(!S.catByPath.has(p))S.f.cats.delete(p);});}
function setTags(rows){S.tagRows=(Array.isArray(rows)?rows:[]).filter(t=>t&&t.name).map(t=>({name:String(t.name).replace(/^#/,''),ref_ids:Array.isArray(t.ref_ids)?t.ref_ids.map(String):[],hl_count:+t.hl_count||0}));
 S.refTags=new Map();S.tagRows.forEach(t=>t.ref_ids.forEach(id=>{let s=S.refTags.get(id);if(!s)S.refTags.set(id,s=new Set());s.add(t.name);}));
 S.refs.forEach(r=>{r._f=null;});S.dataVer++;SI.reset();[...S.f.tags].forEach(t=>{if(!S.refTags.size||!S.tagRows.some(x=>x.name===t))S.f.tags.delete(t);});}
async function loadCatalog(){if(!S.project){setRefs([]);setCats([]);setTags([]);return;}
 const [refs,cats,tags]=await Promise.all([GET('/api/references'),GET('/api/categories').catch(e=>{fail(e);return [];}),GET('/api/tags').catch(e=>{fail(e);return [];})]);
 setRefs(refs);setCats(cats);setTags(tags);}
function parseJ(v,dflt){if(v==null||v==='')return dflt;if(typeof v==='string'){try{return JSON.parse(v);}catch(e){return dflt;}}return v;}
function normHl(h){const o={...h};let tags=parseJ(h.tags,[]);if(!Array.isArray(tags))tags=String(tags||'').split(/[\s,]+/);o.tags=tags.map(t=>String(t).replace(/^#/,'')).filter(Boolean);
 o.quads=parseJ(h.quads,[]);if(!Array.isArray(o.quads))o.quads=[];o.quads=o.quads.filter(q=>Array.isArray(q)&&q.length>=4).map(q=>q.map(Number));
 o.anchor=parseJ(h.anchor,null);o.suggestion=parseJ(h.suggestion,null);
 let st=parseJ(h.suggested_tags,[]);if(!Array.isArray(st))st=[];o.suggested_tags=st.map(t=>String(t).replace(/^#/,'')).filter(t=>t&&!o.tags.includes(t));
 o.page=h.page==null||h.page===''?null:+h.page;o.analysis=String(h.analysis||'');o.verbatim=String(h.verbatim||'');o.author=String(h.author||'Unknown');o.origin=h.origin==='ai'?'ai':'human';o.label=h.label||'';o.kind=h.kind||'text';return o;}
async function loadHl(refId){const rows=await GET('/api/highlights?ref_id='+enc(refId));S.hl.set(refId,(Array.isArray(rows)?rows:[]).map(normHl));return S.hl.get(refId);}
/* after any write or a change from elsewhere: references and tags, open highlights, then redraw. `ids` names the
   references the change touched and only those are fetched again; a write from the viewer passes hlRefs alone, which
   then names them. `ids:null`, a category reload without ids and a list longer than PARTIAL_MAX fetch the whole
   catalogue. */
const PARTIAL_MAX=200;
async function refreshData({cats=false,hlRefs=null,ids}={}){
 if(ids===undefined)ids=!cats&&Array.isArray(hlRefs)&&hlRefs.length?hlRefs:null;
 const part=Array.isArray(ids)&&ids.length>0&&ids.length<=PARTIAL_MAX&&S.refs.length>0;
 try{const ps=[GET(part?'/api/references?ids='+ids.map(enc).join(','):'/api/references'),GET('/api/tags')];if(cats)ps.push(GET('/api/categories'));const [refs,tags,c]=await Promise.all(ps);if(part)patchRefs(ids,refs);else setRefs(refs);setTags(tags);if(cats)setCats(c);}catch(e){fail(e);}
 const want=(hlRefs||S.tabs).filter(id=>S.tabs.includes(id)&&S.byId.has(id));
 await Promise.all(want.map(id=>loadHl(id).catch(()=>{})));
 renderAll();want.forEach(id=>{const v=V.get(id);if(v)viewerRefresh(v);});[...V.values()].forEach(v=>{if(v.needsReload&&!want.includes(v.id))viewerRefresh(v);});Graph.schedule();}
async function reloadAll(){try{await loadState();await loadCatalog();await Promise.all(S.tabs.map(id=>loadHl(id).catch(()=>{})));}catch(e){fail(e);}renderAll();Graph.schedule();}

/* ---------- search scoring: loose bigram similarity per token, weighted by field ---------- */
/* The metadata fields are scored here. The highlight text (search_text) stays on the server: /api/search scores it per
   token with the same rules, and score() takes, per token, the higher of the two, so a query ranks as it would with
   every field scored in the page. A search has parts: each search chip and the text in the box. A reference must
   match every part and scores the sum of the part scores, which equals the score of one query holding the words of
   every part. Scores are cached per part text. */
const SI={qs:new Map(),last:null,sims:new Map(),bg:new Map(),reset(){this.qs.clear();this.last=null;this.sims.clear();},
 drop(key){this.qs.delete(key);if(this.last&&this.last.key===key)this.last=null;}};
function bigrams(s){let b=SI.bg.get(s);if(b)return b;b=new Set();for(let i=0;i<s.length-1;i++)b.add(s.slice(i,i+2));if(SI.bg.size>200000)SI.bg.clear();SI.bg.set(s,b);return b;}
function dice(a,b){if(a===b)return 1;const A=bigrams(a),B=bigrams(b);if(!A.size||!B.size)return 0;let n=0;for(const x of A)if(B.has(x))n++;return 2*n/(A.size+B.size);}
function refFields(r){if(r._f)return r._f;const tags=[...(S.refTags.get(r.id)||[])].join(' ');
 r._f=[[r.title,3],[r.authors.join(' '),2],[tags,2],[r.pub||'',1],[r.register_id||'',1],[r.categories.join(' '),1]].map(([t,w])=>({low:String(t).toLowerCase(),w,words:null}));return r._f;}
function fieldSim(tok,f,thr){if(!f.low)return 0;if(f.low.includes(tok))return 1;if(!f.words)f.words=[...new Set(f.low.split(/[^\p{L}\p{N}-]+/u).filter(Boolean))];
 let m=SI.sims.get(tok);if(!m)SI.sims.set(tok,m=new Map());let best=0;for(const w of f.words){let d=m.get(w);if(d===undefined){d=dice(tok,w);m.set(w,d);}if(d>best)best=d;}return best>=thr?best*0.9:0;}
/* the score cache of one query text at one looseness; the last one used is found without building its key */
function qEnt(q,L){const l=SI.last;if(l&&l.q===q&&l.L===L)return l;const key=q+'|'+L;let e=SI.qs.get(key);
 if(!e){if(SI.qs.size>=64)SI.drop(SI.qs.keys().next().value);e={q,L,key,toks:q.toLowerCase().split(/\s+/).filter(Boolean),scores:new Map()};SI.qs.set(key,e);}SI.last=e;return e;}
function entScore(r,e){let s=e.scores.get(r.id);if(s!==undefined)return s;
 const toks=e.toks;if(!toks.length)s=1;else{const thr=e.L?0.42:0.72;const fields=refFields(r);const ft=FT.of(r.id,e.key);s=0;for(let i=0;i<toks.length;i++){const t=toks[i];let best=ft?ft[i]:0;for(const f of fields){const x=fieldSim(t,f,thr)*f.w;if(x>best)best=x;}if(best===0){s=0;break;}s+=best;}}
 e.scores.set(r.id,s);return s;}
function score(r,q){return entScore(r,qEnt(q,looseOn()));}
/* the search parts: every search chip in the order added, then the text in the box */
const qChips=()=>S.f.chips||(S.f.chips=[]);
function qParts(){const p=qChips().slice();if(S.q.trim())p.push(S.q);return p;}
function qEnts(){const L=looseOn();return qParts().map(q=>qEnt(q,L));}
/* a reference matches when every part scores; its score is the sum of the part scores */
function qScore(r,ents){let t=0;for(const e of ents){const s=entScore(r,e);if(s<=0)return 0;t+=s;}return t;}
/* full-text scores from /api/search, one answer per part text (each search chip and the text in the box). The
   catalogue shows the metadata matches at once and draws again when an answer arrives. Requests go out DELAY ms after
   the parts settle. A text no longer wanted (an older query, a removed chip, another project) has its request aborted
   and its answer dropped. Answers stay cached per project and per text until a data change asks again, so a chip made
   from the box reuses the answer the box already has. FT.key names the text in the box once its answer is in. */
const FT={key:'',stamp:0,timer:null,DELAY:90,ent:new Map(),MAX:32,
 pid(){return S.project?S.project.id:null;},
 wanted(){const L=looseOn(),m=new Map();for(const q of qParts())m.set(q+'|'+L,q);return m;},
 of(id,key){const e=this.ent.get(key);return e&&e.map&&e.pid===this.pid()?e.map.get(id)||null:null;},
 pending(){const pid=this.pid();for(const key of this.wanted().keys()){const e=this.ent.get(key);if(!e||!e.map||e.pid!==pid)return true;}return false;},
 /* drops the request of a text: an answer still on its way finds e.n moved on and is ignored */
 stop(e){if(e.ctl){e.ctl.abort();e.ctl=null;}e.n++;e.want=-1;e.reqVer=-1;},
 sync(){const want=this.wanted(),pid=this.pid(),ver=S.dataVer;let need=false;
  for(const [key,e] of this.ent){if(e.pid!==pid){this.stop(e);this.ent.delete(key);SI.drop(key);}else if(!want.has(key)&&e.want>=0)this.stop(e);}
  if(pid!=null)for(const [key,q] of want){let e=this.ent.get(key);if(!e){e={key,q,pid,ver:-1,map:null,want:-1,reqVer:-1,failed:-1,ctl:null,n:0};this.ent.set(key,e);}
   if((e.map&&e.ver===ver)||e.want===ver||e.failed===ver)continue;e.want=ver;need=true;}
  /* at most MAX texts stay cached; the oldest ones no longer wanted leave first */
  if(this.ent.size>this.MAX)for(const [key,e] of this.ent){if(this.ent.size<=this.MAX)break;if(!want.has(key)){this.stop(e);this.ent.delete(key);SI.drop(key);}}
  const live=this.ent.get(S.q+'|'+looseOn());if(live&&live.map)this.key=live.key;
  if(need){clearTimeout(this.timer);this.timer=setTimeout(()=>this.fire(),this.DELAY);}else if(!want.size)clearTimeout(this.timer);},
 fire(){for(const key of this.wanted().keys()){const e=this.ent.get(key);if(e&&e.want>=0&&!(e.ctl&&e.reqVer===e.want))this.fetchOne(e,e.want);}},
 async fetchOne(e,ver){if(e.ctl)e.ctl.abort();const ctl=e.ctl=new AbortController(),n=++e.n;e.reqVer=ver;let res;
  try{const r=await fetch('/api/search?q='+enc(e.q)+'&loose='+(e.key.endsWith('|true')?1:0),{signal:ctl.signal});if(!r.ok)throw new Error('status '+r.status);res=await r.json();}
  catch(x){if(n===e.n){e.ctl=null;e.reqVer=-1;if(e.want===ver)e.want=-1;if(!(x&&x.name==='AbortError'))e.failed=ver;}return;}
  if(n!==e.n)return;e.ctl=null;e.reqVer=-1;if(e.want===ver)e.want=-1;
  if(this.ent.get(e.key)!==e||e.pid!==this.pid()||!this.wanted().has(e.key))return;
  const toks=e.q.toLowerCase().split(/\s+/).filter(Boolean),st=Array.isArray(res&&res.tokens)?res.tokens:[],at=toks.map(t=>st.indexOf(t));
  e.map=new Map((Array.isArray(res&&res.hits)?res.hits:[]).map(h=>[h.id,at.map(j=>j<0?0:+(h.scores&&h.scores[j])||0)]));
  e.ver=ver;e.failed=-1;this.stamp++;SI.drop(e.key);if(e.key===S.q+'|'+looseOn())this.key=e.key;if(!docTabRef())renderMain();}};
function visibleRefs(){const f=S.f,ents=qEnts();return S.refs.filter(r=>{if(f.cats.size&&!r.categories.some(c=>f.cats.has(c)))return false;if(f.authors.size&&!r.authors.some(a=>f.authors.has(a)))return false;
 if(f.tags.size){const t=S.refTags.get(r.id);if(!t||![...f.tags].some(x=>t.has(x)))return false;}if(f.refs.size&&!f.refs.has(r.id))return false;if(ents.length&&qScore(r,ents)<=0)return false;return true;});}
function sorted(list){const l=[...list];if(S.sort==='alpha')l.sort((a,b)=>az(a.title,b.title));else if(S.sort==='added')l.sort((a,b)=>byCode(b.created_at,a.created_at));
 else{const ents=qEnts();if(ents.length)l.sort((a,b)=>qScore(b,ents)-qScore(a,ents));else l.sort((a,b)=>b.hl_count-a.hl_count||byCode(b.updated_at,a.updated_at));}return l;}
function authorList(list){const m=new Map();for(const r of list)for(const a of r.authors){const o=m.get(a)||{name:a,refs:[],hl:0};o.refs.push(r);o.hl+=r.hl_count;m.set(a,o);}const out=[...m.values()];
 out.forEach(o=>{o.top=[...o.refs].sort((a,b)=>b.hl_count-a.hl_count)[0];});return out.sort((a,b)=>a.name==='Unknown'?1:b.name==='Unknown'?-1:b.hl-a.hl||az(a.name,b.name));}
function tagList(list){const ids=new Set(list.map(r=>r.id));return S.tagRows.map(t=>({name:t.name,refs:new Set(t.ref_ids.filter(id=>ids.has(id))),hl:t.hl_count})).filter(t=>t.refs.size).sort((a,b)=>b.refs.size-a.refs.size||az(a.name,b.name));}
const isPerson=a=>/,\s/.test(a);
const isUnknown=r=>r.authors.length===1&&r.authors[0]==='Unknown';
const statusCls=s=>s==='Verified'?'v':s==='Suggested'?'s':'a';
const statusText=s=>s==='Verified'?'Verified by you':s==='Suggested'?`Proposed by ${aiName()} via MCP, review and save`:'Read from the file at scan, needs review';
function apa(r){const au=r.authors.filter(a=>a!=='Unknown');let a='';if(au.length===1)a=au[0];else if(au.length===2)a=au.join(', & ');else if(au.length>2)a=au.slice(0,-1).join(', ')+', & '+au[au.length-1];
 let s=`${a}${a?' ':''}(${r.year||'n.d.'}). ${r.title}.`;if(r.pub){s+=` ${r.pub}`;if(r.vol){s+=`, ${r.vol}`;if(r.issue)s+=`(${r.issue})`;}if(r.pages_range)s+=`, ${r.pages_range}`;s+='.';}
 if(r.doi)s+=` https://doi.org/${String(r.doi).replace(/^https?:\/\/(dx\.)?doi\.org\//i,'')}`;else if(r.url)s+=` ${r.url}`;return s;}
async function copyApa(r){try{const full=await GET('/api/reference/'+enc(r.id));copyText(apa(normRef({...r,...full})));}catch(e){fail(e);}}

/* ---------- toasts, clipboard, prompts for Claude ---------- */
function toast(msg,o={}){const t=el('div',{class:'toast'+(o.mcp?' mcp':'')+(o.err?' err':''),role:o.err?'alert':'status'},el('span',{},msg));
 if(o.action)t.append(el('button',{class:'btn sm',onclick:()=>{o.action.fn();t.remove();}},o.action.label));
 t.append(el('button',{class:'btn sm ico',title:'Dismiss',onclick:()=>t.remove()},'✕'));$('#toasts').append(t);
 const all=$$('.toast');if(all.length>5)all[0].remove();if(!o.sticky)setTimeout(()=>t.remove(),o.ms||6000);return t;}
function copyText(t,done){const ok=()=>toast(done||'Copied to the clipboard.');
 const legacy=()=>{const ta=el('textarea',{style:'position:fixed;left:-9999px;top:0'});ta.value=t;document.body.append(ta);ta.select();let good=false;try{good=document.execCommand('copy');}catch(e){}ta.remove();if(good)ok();else toast(el('span',{},'Copy this text: ',el('code',{},t)),{ms:15000});};
 if(navigator.clipboard&&navigator.clipboard.writeText)navigator.clipboard.writeText(t).then(ok,legacy);else legacy();}
const PROMPT_DONE='Prompt copied. Paste it into Claude Code, where the Kurasi MCP server is registered.';
function copyPrompt(text){copyText(text,PROMPT_DONE);}
const Prompt={
 head(){return `Use the Kurasi MCP server for the project "${S.project?S.project.name:''}".`;},
 categorize(refs){return `${this.head()} For each reference below, read page 1 with reference_text, then call reference_update_metadata with the APA 7 fields (title, authors, year, date, publication) and reference_categorize with the best category path from categories_list. Each change waits for my review. Purpose: label new references.\nReference IDs: ${refs.map(r=>r.id).join(', ')}`;},
 metadata(r){return `${this.head()} Read page 1 of reference ${r.id} ("${words(r.title,12)}") with reference_text, then call reference_update_metadata(ref_id="${r.id}", ...) with its APA 7 fields: title, authors, year, date, publication, volume, issue, pages, DOI or URL, language. Purpose: fill the metadata for my review.`;},
 highlight(r,d){const where=d.page?` on page ${d.page}`:'';return `${this.head()} Call highlight_add(ref_id="${r.id}"${d.page?`, page=${d.page}`:''}, quote=${JSON.stringify(d.verbatim||'')}, ...) to highlight this passage${where} with an analysis that starts with (F), (I) or (P) and with tags from tags_list. Purpose: draft the analysis of this passage for my review.`;},
 highlights(r){return `${this.head()} Read reference ${r.id} ("${words(r.title,12)}") page by page with reference_text and call highlight_add for each passage that bears on my question, with an analysis that starts with (F), (I) or (P) and tags from tags_list. Purpose: mark the key passages for my review. My question: `;}};
function mcpHint(key,text,tool,promptFn){if(!hintsOn()||S.tour.on||S.hintShown.has(key))return;S.hintShown.add(key);
 toast(el('span',{html:`MCP available: <code>${esc(tool)}</code>. ${esc(text)}`}),{mcp:true,ms:9000,action:promptFn?{label:'Copy prompt for Claude',fn:()=>copyPrompt(promptFn())}:null});}

/* ---------- modal ---------- */
let modalEl=null;
function openModal({title,body,buttons=[],width,onClose,sticky}){closeModal(true);const foot=el('div',{class:'mfoot'});
 buttons.forEach(b=>{if(b.left){foot.append(el('span',{class:'lft'},b.left));return;}foot.append(el('button',{class:'btn'+(b.primary?' pri':'')+(b.danger?' danger':''),id:b.id||null,disabled:b.disabled?true:null,onclick:async ev=>{const r=b.action?await b.action(ev):undefined;if(r!==false&&!b.keep)closeModal();}},b.label));});
 const m=el('div',{class:'modal',role:'dialog','aria-modal':'true',style:width?`--mw:${width}px`:null},el('div',{class:'mhead'},title),el('div',{class:'mbody'},typeof body==='string'?el('div',{html:body}):body),buttons.length?foot:null);
 modalEl=el('div',{class:'backdrop'},m);modalEl._onClose=onClose;modalEl._sticky=!!sticky;document.body.append(modalEl);$('#tour').hidden=true;
 const f=m.querySelector('input:not([readonly]),textarea,select');if(f)setTimeout(()=>f.focus(),0);return m;}
function closeModal(silent){if(!modalEl)return;const f=modalEl._onClose;modalEl.remove();modalEl=null;if(f&&!silent)f();if(S.tour.on&&!modalEl)tourShow(S.tour.i);}
function confirmDialog(title,text,label,fn,danger=true){openModal({title,body:`<p>${text}</p>`,buttons:[{label:'Cancel'},{label,primary:!danger,danger,action:fn}]});}

/* ---------- menus and context menus ---------- */
let menuEls=[];
function closeMenus(){menuEls.forEach(m=>m.remove());menuEls=[];$$('.mb.open').forEach(b=>b.classList.remove('open'));}
function showMenu(items,x,y,level=0){const m=el('div',{class:'menu',role:'menu'});items.filter(Boolean).forEach(it=>{if(it.sep){m.append(el('div',{class:'mi sep'}));return;}
 const row=el('div',{class:'mi'+(it.disabled?' dis':'')+(it.danger?' danger':''),id:it.id||null,role:'menuitem',title:it.title||null},it.checked?el('span',{class:'chk'},'✓'):null,it.radio?el('span',{class:'chk'},'●'):null,el('span',{},it.label),it.shortcut?el('span',{class:'sc'},it.shortcut):null,it.sub?el('span',{class:'arr'},'›'):null);
 const trim=()=>{menuEls.filter(e=>+e.dataset.level>level).forEach(e=>e.remove());menuEls=menuEls.filter(e=>+e.dataset.level<=level);};
 if(it.sub){row.addEventListener('mouseenter',()=>{trim();const r=row.getBoundingClientRect();showMenu(it.sub,r.right-2,r.top-4,level+1);});}
 else if(!it.disabled){row.addEventListener('click',()=>{closeMenus();it.action&&it.action();});row.addEventListener('mouseenter',trim);}
 m.append(row);});
 m.dataset.level=level;document.body.append(m);const mw=m.offsetWidth,mh=m.offsetHeight;m.style.left=Math.max(4,Math.min(x,innerWidth-mw-4))+'px';m.style.top=Math.max(4,Math.min(y,innerHeight-mh-4))+'px';menuEls.push(m);return m;}
function menubarDef(){const has=!!S.project,auto=autoMin();
 const autoItem=n=>({label:n?`Every ${n} minutes`:'Off',radio:has&&auto===n,disabled:!has,action:()=>setAutoScan(n)});
 const recent=[...S.projects].sort((a,b)=>String(b.last_opened||'').localeCompare(String(a.last_opened||''))).slice(0,8).map(p=>({label:p.name||p.folder,radio:has&&p.id===S.project.id,title:p.folder,action:()=>openProject(p.id)}));
 return [
 {id:'mb-file',label:'File',sig:'menu:file',items:[
  {id:'mi-new',label:'New project…',shortcut:'Ctrl+N',action:newProjectDialog},{id:'mi-projects',label:'Projects…',shortcut:'Ctrl+O',action:projectsDialog},{label:'Recent',sub:recent.length?recent:[{label:'No projects yet',disabled:true}]},{sep:true},
  {id:'mi-scan',label:'Scan folder now',shortcut:'F5',disabled:!has,action:()=>runScan({start:true})},{label:'Auto-scan',sub:[autoItem(0),autoItem(5),autoItem(15),autoItem(30),autoItem(60),{sep:true},{label:'Custom…',disabled:!has,action:customIntervalDialog}]},{sep:true},
  {label:'Export references',sub:[{label:'APA 7 (Markdown)',disabled:!has,action:()=>doExport('apa-md')},{label:'BibTeX',disabled:!has,action:()=>doExport('bibtex')},{label:'CSL JSON',disabled:!has,action:()=>doExport('csl-json')}]},{label:'Export highlights (JSON)',disabled:!has,action:()=>doExport('highlights-json')},{sep:true},
  {label:'Exit',shortcut:'Alt+F4',action:exitApp}]},
 {id:'mb-edit',label:'Edit',items:[{label:S.undo.length?`Undo: ${S.undo[S.undo.length-1].label}`:'Undo',shortcut:'Ctrl+Z',disabled:!S.undo.length,action:undoLast},{label:'Redo',shortcut:'Ctrl+Y',disabled:true},{sep:true},{label:'Find',shortcut:'Ctrl+F',action:()=>{$('#q').focus();$('#q').select();}},{label:'Clear filters',action:clearFilters}]},
 {id:'mb-view',label:'View',items:[{label:'Filter pane',checked:S.ui.filterPane,action:toggleFilterPane},{label:'Collapse copies',checked:true,action:()=>toast('Copies of one source (original, print, translation, excerpt) share one reference card. Switch between them in the document toolbar.')},{sep:true},
  {label:'Theme',sub:['system','light','dark'].map(t=>({label:t[0].toUpperCase()+t.slice(1),radio:curTheme()===t,action:()=>setTheme(t)}))},
  {label:'Reset viewer zoom',disabled:!V.get(S.tab),action:()=>{const v=V.get(S.tab);if(v)setZoom(v,1);}},
  {label:'Reset graph view',disabled:S.tab!=='tags'||!S.project,action:()=>Graph.resetView()}]},
 {id:'mb-tools',label:'Tools',items:[{id:'mi-mcp',label:'MCP server…',action:mcpDialog},{label:'Thumbnail cache…',action:()=>toast('Thumbnails and rendered pages are cached by SHA-256 in %LOCALAPPDATA%\\Kurasi. The page cache is trimmed to 500 MB, oldest first.')},{label:'Reload catalogue',disabled:!has,action:reloadAll},{sep:true},{id:'mi-settings',label:'Settings…',disabled:!has,action:settingsDialog}]},
 {id:'mb-help',label:'Help',items:[{id:'mi-tour',label:'Guided tour',action:restartTour},{label:'Open sample project',action:()=>startDemo({reset:false})},{label:'Reset sample project…',action:resetDemoDialog},{sep:true},{label:'Keyboard shortcuts',action:shortcutsDialog},{sep:true},{label:'About Kurasi',action:aboutDialog}]}];}
function renderMenubar(){const bar=$('#menubar');bar.innerHTML='';menubarDef().forEach(m=>{const b=el('div',{class:'mb',id:m.id,onclick:()=>openMenuByName(m.id)},m.label);b.addEventListener('mouseenter',()=>{if(menuEls.length&&!b.classList.contains('open'))openMenuByName(m.id);});bar.append(b);});}
function openMenuByName(id){const def=menubarDef().find(m=>m.id===id);const b=$('#'+id);if(!def||!b)return;closeMenus();b.classList.add('open');const r=b.getBoundingClientRect();showMenu(def.items,r.left,r.bottom+2);if(def.sig)signal(def.sig);}
async function setAutoScan(n){try{await PUT('/api/settings',{auto_scan_min:n});S.project.auto_scan_min=n;S.settings.auto_scan_min=n;renderStatus();toast(n?`Auto-scan every ${n} minutes.`:'Auto-scan off. File, Scan folder now still runs a scan by hand.');}catch(e){fail(e);}}
async function openProject(id){closeMenus();try{await POST('/api/project/open',{id});}catch(e){fail(e);return;}await switchedProject();}
async function switchedProject({attachScan=true}={}){[...V.keys()].forEach(disposeViewer);S.tabs=[];S.tabFile.clear();S.hl.clear();S.tab='reference';S.right=null;S.q='';$('#q').value='';$('#qclr').hidden=true;S.undo=[];S.expInit=false;
 S.f={cats:new Set(),authors:new Set(),tags:new Set(),refs:new Set()};try{await loadState();await loadCatalog();}catch(e){fail(e);}renderAll();Graph.schedule();
 if(attachScan){try{const st=await GET('/api/scan/status');if(st&&st.running)runScan({start:false});}catch(e){}}}
function exitApp(){try{window.close();}catch(e){}setTimeout(()=>toast('Close the window with the button in its title bar, or press Alt+F4.'),300);}
function toggleFilterPane(){S.ui.filterPane=!S.ui.filterPane;ls.set('kurasi.filterPane',S.ui.filterPane?'1':'0');renderAll();}

/* ---------- undo (deleted references and highlights) ---------- */
function pushUndo(trash_id,label,refId){if(trash_id==null)return;S.undo.push({trash_id,label,refId});if(S.undo.length>50)S.undo.shift();}
async function undoLast(){const u=S.undo.pop();if(!u){toast('Nothing to undo.');return;}try{await POST('/api/undo',{trash_id:u.trash_id});toast(`Restored: ${u.label}.`);}catch(e){fail(e);}await refreshData({hlRefs:u.refId?[u.refId]:null,ids:null});}
function curTheme(){return S.ui.theme||S.settings.theme||'system';}
function applyTheme(){const t=curTheme();if(t==='light'||t==='dark')document.documentElement.dataset.theme=t;else delete document.documentElement.dataset.theme;}
function setTheme(t){S.ui.theme=t;ls.set('kurasi.theme',t);applyTheme();if(S.project)PUT('/api/settings',{theme:t}).then(()=>{S.settings.theme=t;}).catch(()=>{});if(S.tab==='tags')renderMain();}

/* ---------- tabs ---------- */
function docFile(r){if(!r)return null;const fid=S.tabFile.get(r.id);let f=fid&&r.files.find(x=>x.id===fid);
 if(!f){f=r.files.find(x=>x.id===r.primary_file_id)||r.files[0]||null;if(f&&(f.cloud_only||f.missing)){const alt=r.files.find(x=>!x.cloud_only&&!x.missing);if(alt)f=alt;}}return f;}
/* the three catalogue views sit in a segmented switch; open documents follow it as closable file tabs */
function renderTabs(){const ts=$('#tabstrip');ts.innerHTML='';const seg=el('div',{class:'vtabs',role:'tablist','aria-label':'Catalogue view'});
 [['reference','Reference'],['authors','Authors'],['tags','Tag']].forEach(([k,l])=>seg.append(el('div',{class:'vtab'+(S.tab===k?' active':''),id:'tab-'+k,role:'tab','aria-selected':String(S.tab===k),onclick:()=>switchTab(k)},l)));ts.append(seg);
 S.tabs.forEach(id=>{const r=refById(id);if(!r)return;const f=docFile(r);ts.append(el('div',{class:'tab doc'+(S.tab===id?' active':''),title:r.title,onclick:()=>switchTab(id),onauxclick:e=>{if(e.button===1)closeTab(id);}},el('span',{class:'ty'},String((f&&f.ext)||r.ext||'?').toUpperCase()),el('span',{class:'tl'},words(r.title,5)),el('span',{class:'x',title:'Close tab',onclick:e=>{e.stopPropagation();closeTab(id);}},'✕')));});}
function switchTab(k){S.tab=k;S.sel=null;if(!isDocTab(k)){if(S.right&&S.right.mode!=='edit')S.right=null;}renderAll();if(refById(k))signal('doc:open');}
function closeTab(id){S.tabs=S.tabs.filter(t=>t!==id);disposeViewer(id);S.tabFile.delete(id);if(S.tab===id)S.tab=S.tabs.length?S.tabs[S.tabs.length-1]:'reference';renderAll();}
function openDoc(id,fileId){const r=refById(id);if(!r)return;
 if(fileId&&r.files.some(f=>f.id===fileId)&&S.tabFile.get(id)!==fileId){S.tabFile.set(id,fileId);disposeViewer(id);}
 if(!S.tabs.includes(id))S.tabs.push(id);S.kf={tags:new Set(),authors:new Set()};S.right=null;
 loadHl(id).then(()=>{if(S.tab===id){renderRight();renderFilter();}const v=V.get(id);if(v)viewerRefresh(v);if(S.tour.on)requestAnimationFrame(positionTour);}).catch(fail);switchTab(id);}

/* ---------- filter pane ---------- */
function subtree(n,path,inner){if(!n.c||!n.c.length)return [path];const kids=n.c.flatMap(ch=>subtree(ch,ch.p||path+'/'+ch.n,inner));return inner?[path,...kids]:kids;}
function fsec(title,body,key,extra){const sec=el('div',{class:'fsec'+(S.fclosed.has(key)?' closed':'')});sec.append(el('div',{class:'fhead',onclick:()=>{S.fclosed.has(key)?S.fclosed.delete(key):S.fclosed.add(key);sec.classList.toggle('closed');}},el('span',{class:'chev'},'▼'),el('span',{},title),extra!=null?el('span',{class:'cnt'},extra):null));sec.append(el('div',{class:'fbody'},body));return sec;}
function tree(nodes,checked,countFn,onChange,o={}){const prefix=o.prefix||'',depth=o.depth||0;const frag=document.createDocumentFragment();
 nodes.forEach(n=>{const path=n.p||(prefix?prefix+'/'+n.n:n.n);const leaves=subtree(n,path,o.inner);const cnt=countFn(leaves);const nChecked=leaves.filter(l=>checked.has(l)).length;
  const cb=el('input',{type:'checkbox'});cb.checked=nChecked===leaves.length&&leaves.length>0;cb.indeterminate=nChecked>0&&nChecked<leaves.length;
  cb.addEventListener('change',()=>{leaves.forEach(l=>cb.checked?checked.add(l):checked.delete(l));onChange();});const kids=n.c&&n.c.length;
  /* o.open shows every branch open (the pane finder is in use) */
  const open=kids&&(o.open||S.exp.has(path));
  const row=el('div',{class:'trow',style:`--d:${depth}`,oncontextmenu:o.ctx?e=>{e.preventDefault();o.ctx(n,e.clientX,e.clientY);}:null},
   kids?el('span',{class:'tg',onclick:()=>{S.exp.has(path)?S.exp.delete(path):S.exp.add(path);renderFilter();}},open?'▼':'▶'):el('span',{class:'tg'}),
   el('label',{title:n.n+(cnt!=null?` (${cnt})`:'')},cb,el('span',{class:'lbl'},truncEnd(n.n,75))),cnt!=null?el('span',{class:'cnt'},cnt):null);
  frag.append(row);if(open)frag.append(tree(n.c,checked,countFn,onChange,{...o,prefix:path,depth:depth+1}));});return frag;}
/* a parent category counts as ticked when every category below it is ticked */
function normCatSel(){const walk=n=>{if(!n.c)return S.f.cats.has(n.p);let all=true;n.c.forEach(ch=>{if(!walk(ch))all=false;});if(all)S.f.cats.add(n.p);else S.f.cats.delete(n.p);return all;};S.cats.forEach(walk);}
/* filter-pane finder: narrows the Author and Tag entries of the pane and leaves the catalogue and the filters alone.
   Every word typed must sit inside the name or come near one of its words (bigram Dice at the loose threshold, 0.42);
   names split at every character other than a letter or a digit. Ticked entries always stay. Enter does nothing. */
const FP={q:'',pid:null,box:null,inp:null,clr:null,timer:null,
 toks(){return this.q.toLowerCase().split(/\s+/).map(t=>t.replace(/^#+/,'')).filter(Boolean);},
 match(name,toks){const low=name.toLowerCase();let ws=null;for(const t of toks){if(low.includes(t))continue;if(!ws)ws=low.split(/[^\p{L}\p{N}]+/u).filter(Boolean);if(!ws.some(w=>dice(t,w)>=0.42))return false;}return true;},
 /* one box element, kept across redraws of the pane */
 node(){if(this.box)return this.box;const inp=el('input',{type:'text',id:'fq',placeholder:'Find an author or tag',autocomplete:'off',spellcheck:'false','aria-label':'Find an author or tag in the filter pane'});
  const clr=el('button',{class:'clr',type:'button',title:'Clear the finder','aria-label':'Clear the finder',hidden:true},'✕');
  inp.addEventListener('input',()=>{clr.hidden=!inp.value;clearTimeout(this.timer);this.timer=setTimeout(()=>{this.q=inp.value;renderFilter();},120);});
  inp.addEventListener('keydown',e=>{if(e.key==='Enter')e.preventDefault();});
  clr.addEventListener('click',()=>{clearTimeout(this.timer);inp.value='';clr.hidden=true;this.q='';renderFilter();inp.focus();});
  this.inp=inp;this.clr=clr;this.box=el('div',{class:'fsearch'},el('span',{class:'ic','aria-hidden':'true'},'\u{1F50D}︎'),inp,clr);return this.box;}};
function renderFilter(){const f=$('#filter');f.hidden=!S.ui.filterPane;
 /* the finder keeps the focus and the caret through a redraw */
 const fin=FP.inp,foc=fin&&document.activeElement===fin?[fin.selectionStart,fin.selectionEnd]:null;
 f.innerHTML='';const r=docTabRef();if(r){renderDocFilter(f,r);return;}
 if(!S.project){f.append(el('div',{class:'fdoc'},'Open a project to see its categories, authors and tags.'));return;}
 if(FP.pid!==S.project.id){FP.pid=S.project.id;FP.q='';clearTimeout(FP.timer);if(fin){fin.value='';FP.clr.hidden=true;}}
 f.append(FP.node());if(foc){fin.focus({preventScroll:true});try{fin.setSelectionRange(foc[0],foc[1]);}catch(e){}}
 const any=S.f.cats.size||S.f.authors.size||S.f.tags.size||S.f.refs.size||qChips().length;if(any)f.append(el('a',{class:'fclear',id:'fclearall',onclick:clearFilters},'Clear all filters'));
 const change=()=>{renderAll();};
 /* counts come from one index per section (category or author to its references), so a section with
    thousands of rows costs one pass over the library */
 const index=key=>{const m=new Map();for(const x of S.refs)for(const k of key(x)){const l=m.get(k);if(l)l.push(x.id);else m.set(k,[x.id]);}return m;};
 const union=(m,keys)=>{if(keys.length===1){const l=m.get(keys[0]);return l?new Set(l).size:0;}const u=new Set();for(const k of keys)for(const id of m.get(k)||[])u.add(id);return u.size;};
 const cidx=index(x=>x.categories);
 const cbody=el('div',{},tree(S.cats,S.f.cats,ls_=>union(cidx,ls_),()=>{normCatSel();change();},{inner:true,ctx:catMenu}),el('a',{class:'fclear',title:'Create a category for references',onclick:()=>categoryDialog({})},'+ New category'));
 f.append(fsec('Reference',cbody,'cats',S.refs.length));
 /* authors A to Z by surname within People, by name within Organisations, Unknown last; the finder narrows them */
 const ft=FP.toks(),fon=ft.length>0,keep=(name,ticked)=>!fon||ticked||FP.match(name,ft),near=()=>`"${FP.q.trim()}"`;
 const au=authorList(S.refs),sur=n=>{const i=n.indexOf(',');return i<0?n:n.slice(0,i);};
 const people=au.filter(a=>isPerson(a.name)&&keep(a.name,S.f.authors.has(a.name))).map(a=>({n:a.name,s:sur(a.name)})).sort((x,y)=>az(x.s,y.s)||az(x.n,y.n));
 const orgs=au.filter(a=>!isPerson(a.name)&&a.name!=='Unknown'&&keep(a.name,S.f.authors.has(a.name))).map(a=>({n:a.name})).sort((x,y)=>paneAz(x.n,y.n));
 const unk=au.find(a=>a.name==='Unknown'&&keep(a.name,S.f.authors.has(a.name)));
 const atree=[];if(people.length)atree.push({n:'People',c:people});if(orgs.length)atree.push({n:'Organisations',c:orgs});if(unk)atree.push({n:'Unknown'});
 const aidx=index(x=>x.authors),acount=ls_=>union(aidx,ls_.map(l=>l.split('/').pop()));
 const aset={has:p=>S.f.authors.has(p.split('/').pop()),add:p=>S.f.authors.add(p.split('/').pop()),delete:p=>S.f.authors.delete(p.split('/').pop())};
 const ashown=people.length+orgs.length+(unk?1:0);
 f.append(fsec('Author',atree.length?tree(atree,aset,acount,change,{open:fon}):el('div',{class:'fdoc'},fon&&au.length?`No author comes near ${near()}.`:'No authors yet.'),'authors',fon?`${ashown} of ${au.length}`:au.length));
 const tl=tagList(S.refs),tmap=new Map(tl.map(t=>[t.name,t]));const tnodes=tl.filter(t=>keep(t.name,S.f.tags.has(t.name))).sort((a,b)=>paneAz(a.name,b.name)).map(t=>({n:'#'+t.name}));const tset={has:p=>S.f.tags.has(p.slice(1)),add:p=>S.f.tags.add(p.slice(1)),delete:p=>S.f.tags.delete(p.slice(1))};
 f.append(fsec('Tag',tnodes.length?tree(tnodes,tset,ls_=>{const o=tmap.get(ls_[0].slice(1));return o?o.refs.size:0;},change):el('div',{class:'fdoc'},fon&&tl.length?`No tag comes near ${near()}.`:'No tags yet. Tags come from highlights.'),'tags',fon?`${tnodes.length} of ${tl.length}`:tl.length));}
function renderDocFilter(f,r){const file=docFile(r);const d=el('div',{class:'fdoc'});put(d,el('b',{},words(r.title,10)),el('div',{},r.authors.join('; ')),el('div',{},r.categories.map(c=>c.split('/').pop()).join(', ')),el('div',{class:'path',title:'Register ID'},r.register_id||'No register ID yet'),file&&file.relpath?el('div',{class:'path',title:'Path in the project folder'},file.relpath):null);
 f.append(fsec('This document',d,'doc'));const hls=docHls(r);
 const tags=new Map();hls.forEach(h=>h.tags.forEach(t=>tags.set(t,(tags.get(t)||0)+1)));const tnodes=[...tags.keys()].sort(az).map(t=>({n:'#'+t}));
 const tset={has:p=>S.kf.tags.has(p.slice(1)),add:p=>S.kf.tags.add(p.slice(1)),delete:p=>S.kf.tags.delete(p.slice(1))};
 f.append(fsec('Tags in this document',tnodes.length?tree(tnodes,tset,ls_=>tags.get(ls_[0].slice(1)),()=>renderRight()):el('div',{class:'fdoc'},'No tags yet.'),'ktags',tags.size));
 const lbl=a=>{const h=hls.find(x=>x.author===a);return h&&h.origin==='ai'?a+' (MCP)':a;};const unl=p=>p.replace(/ \(MCP\)$/,'');
 const who=[...new Set(hls.map(h=>h.author))].sort(az).map(a=>({n:lbl(a)}));const wset={has:p=>S.kf.authors.has(unl(p)),add:p=>S.kf.authors.add(unl(p)),delete:p=>S.kf.authors.delete(unl(p))};
 f.append(fsec('Highlight author',who.length?tree(who,wset,ls_=>hls.filter(h=>h.author===unl(ls_[0])).length,()=>renderRight()):el('div',{class:'fdoc'},'No highlights yet.'),'kwho'));}
function catMenu(n,x,y){const fixed=n.id===1||n.p==='Uncategorized';showMenu([{label:'Show only this category',action:()=>{S.f.cats=new Set(subtree(n,n.p,true));switchTab('reference');}},{sep:true},{label:'New subcategory…',action:()=>categoryDialog({parent:n})},{label:'Rename…',disabled:fixed,action:()=>categoryDialog({node:n})},{sep:true},{label:'Delete category',danger:true,disabled:fixed,action:()=>deleteCategory(n)}],x,y);}
function clearFilters(){S.f={cats:new Set(),authors:new Set(),tags:new Set(),refs:new Set(),chips:[]};S.q='';$('#q').value='';$('#qclr').hidden=true;renderAll();}

/* ---------- status bar ---------- */
function renderStatus(){const sb=$('#statusbar');sb.innerHTML='';const mcp=el('span',{class:'mcp',title:'MCP server: registration command and AI-use log',onclick:mcpDialog},el('span',{class:'dot'}),'MCP (stdio)');
 if(S.project){const au=authorList(S.refs).length,tg=S.tagRows.length,hl=S.refs.reduce((n,r)=>n+r.hl_count,0),auto=autoMin();
  put(sb,el('span',{},plural(S.refs.length,'reference')),el('span',{},plural(au,'author')),el('span',{},plural(tg,'tag')),el('span',{},plural(hl,'highlight')),el('span',{class:'sp'}),
   isRO()?el('span',{class:'ro',title:'Kurasi writes nothing into the project folder; the database still records your work'},'Read-only project'):null,
   el('span',{title:String(lastScan())},`Last scan ${fmtWhen(lastScan())||'not run yet'}`),el('span',{},auto?`Auto-scan every ${auto} min`:'Auto-scan off'),S.offline?el('span',{class:'off'},'Server unreachable'):null,mcp);}
 else put(sb,el('span',{},'No project open'),el('span',{class:'sp'}),S.offline?el('span',{class:'off'},'Server unreachable'):null,mcp);
 const pn=$('#projname');pn.textContent=S.project?S.project.name:'No project';pn.title=S.project?S.project.folder:'';
 const mb=$('#modebadge');mb.className='muted modebadge'+(isRO()&&!S.tour.on?' ro':'');mb.textContent=S.tour.on?`Guided tour ${tourPos().n} of ${tourPos().of}`:isRO()?'Read-only':S.demo?'Demo project':'';}
function renderAll(){renderMenubar();renderTabs();renderFilter();renderMain();renderRight();renderStatus();if(S.tour.on)requestAnimationFrame(positionTour);}

/* ---------- catalogue (e-commerce style) ---------- */
function filterChips(head){const chip=(label,fn,cls)=>el('span',{class:'chip'+(cls?' '+cls:''),title:label},trunc(label,28),el('b',{title:'Remove this filter',onclick:fn},'✕'));const re=()=>{renderAll();};
 const shown=new Set();[...S.f.cats].forEach(c=>{const n=S.catByPath.get(c);if(n&&n.parent&&S.f.cats.has(n.parent.p))return;shown.add(c);});
 shown.forEach(c=>head.append(chip(c.split('/').pop(),()=>{const n=S.catByPath.get(c);(n?subtree(n,c,true):[c]).forEach(x=>S.f.cats.delete(x));normCatSel();re();})));
 S.f.authors.forEach(a=>head.append(chip(a,()=>{S.f.authors.delete(a);re();})));S.f.tags.forEach(t=>head.append(chip('#'+t,()=>{S.f.tags.delete(t);re();})));
 S.f.refs.forEach(id=>{const r=refById(id);head.append(chip('Paper: '+(r?words(r.title,5):id),()=>{S.f.refs.delete(id);re();}));});
 qChips().forEach(c=>head.append(chip(`"${c}"`,()=>removeChip(c),'q')));
 if(S.q)head.append(chip(`"${S.q}"`,()=>setQuery(''),'live'));}
/* search chips: Enter in the search box or its + button keeps the text as a filter and empties the box. Empty text
   adds nothing, and a chip equal to one already there (letter case aside) is left out. The catalogue draws again; the
   filter pane only gains or loses its "Clear all filters" link. */
function addChip(){const box=$('#q'),t=box.value.trim().replace(/\s+/g,' ');if(!t)return false;const cs=qChips();
 if(!cs.some(c=>c.toLowerCase()===t.toLowerCase()))cs.push(t);S.q='';box.value='';$('#qclr').hidden=true;chipsChanged();return true;}
function removeChip(c){S.f.chips=qChips().filter(x=>x!==c);chipsChanged();}
function chipsChanged(){const f=S.f,any=f.cats.size||f.authors.size||f.tags.size||f.refs.size||qChips().length,link=$('#fclearall');
 if(any&&!link&&FP.box&&FP.box.parentNode===$('#filter'))FP.box.after(el('a',{class:'fclear',id:'fclearall',onclick:clearFilters},'Clear all filters'));else if(!any&&link)link.remove();
 if(!docTabRef())renderMain();if(S.tour.on)requestAnimationFrame(positionTour);}
function renderMain(){const m=$('#main');const r=docTabRef();
 if(r){const v=getViewer(r);if(m.firstChild===v.el)return;detachMain(m);m.append(v.el);requestAnimationFrame(()=>{if(v.saveTop!=null){v.sc.scrollTop=v.saveTop;v.sc.scrollLeft=v.saveLeft||0;}});return;}
 detachMain(m);
 if(!S.project){m.append(el('div',{class:'empty'},el('div',{},S.offline?'Kurasi cannot reach its server.':'No project is open.'),el('div',{},el('button',{class:'btn pri',onclick:newProjectDialog},'Open a folder'),' ',el('button',{class:'btn',onclick:()=>startDemo({reset:false,tour:true})},'Guided tour'))));return;}
 FT.sync();if(S.tab==='tags'){renderGraph(m);return;}
 const cat=el('div',{class:'cat'});const head=el('div',{class:'cathead'});const grid=el('div',{class:'grid'});cat.append(head,grid);m.append(cat);
 const list=sorted(visibleRefs());const items=S.tab==='reference'?list.map(r=>({kind:'ref',r})):authorList(list).map(a=>({kind:'author',a}));
 filterChips(head);head.append(el('span',{class:'sp'}));
 const uncat=S.refs.filter(x=>x.categories.includes('Uncategorized'));
 if(S.tab==='reference'&&uncat.length&&!S.tour.on&&hintsOn())head.append(el('span',{class:'mcphint',style:'margin:0'},el('span',{class:'t',html:`${uncat.length} uncategorized. <code>reference_categorize</code> can propose metadata.`}),el('button',{class:'btn sm',onclick:()=>copyPrompt(Prompt.categorize(uncat))},'Copy prompt for Claude')));
 head.append(el('span',{class:'pinfo'},plural(items.length,S.tab==='reference'?'reference':'author')));
 const qs=qParts(),qtxt=qs.map(x=>`"${x.trim()}"`).join(' and ');
 if(!items.length){grid.append(el('div',{class:'empty',style:'grid-column:1/-1'},!S.refs.length?el('div',{},'No files are indexed yet. ',el('button',{class:'btn sm',onclick:()=>runScan({start:true})},'Scan folder now')):qs.length&&FT.pending()?`Searching the highlight text for ${qtxt}`:qs.length?`Nothing matches ${qtxt} at this looseness. ${qChips().length?'Remove a search term':'Try fewer words'}, or set a looser threshold in Tools, Settings.`:'No references match the filters.'));return;}
 /* chunk: about two screens of cards, measured with a probe card */
 let chunk=40;const probe=card(items[0]);probe.style.visibility='hidden';grid.append(probe);
 {const gap=10,gw=grid.clientWidth,gh=grid.clientHeight,cw=probe.offsetWidth||150,ch=probe.offsetHeight||110;probe.remove();const cols=Math.max(1,Math.floor((gw+gap)/(cw+gap))),rows=Math.max(1,Math.ceil((gh+gap)/(ch+gap)));chunk=Math.max(20,cols*rows*2);}
 const key=[S.tab,S.sort,Graph.filterSig()].join('|');if(List.key!==key){List.key=key;List.n=0;List.top=0;}
 const end=el('div',{class:'gend'});grid.append(end);let shown=0;
 const add=n=>{const stop=Math.min(items.length,shown+n);const frag=document.createDocumentFragment();for(let i=shown;i<stop;i++)frag.append(card(items[i]));grid.insertBefore(frag,end);shown=stop;List.n=Math.max(List.n,shown);};
 const fill=()=>{while(shown<items.length&&grid.scrollTop+grid.clientHeight>=grid.scrollHeight-400)add(chunk);};
 add(Math.max(chunk,Math.min(List.n,items.length)));grid.scrollTop=List.top;fill();
 grid.addEventListener('scroll',()=>{List.top=grid.scrollTop;fill();},{passive:true});}
/* The catalogue scrolls without pages: cards are added a chunk at a time as the grid nears its end.
   A redraw of the same list (same tab, sort, search and filters) keeps the cards already shown and
   the scroll position; any other list starts at the top. */
const List={key:'',n:0,top:0};
function detachMain(m){const cur=m.firstChild;if(cur&&cur._viewer){const v=cur._viewer;v.saveTop=v.sc.scrollTop;v.saveLeft=v.sc.scrollLeft;}m.innerHTML='';}
function thumb(r){const f=r.files.find(x=>x.id===r.primary_file_id)||r.files[0]||{};const ext=r.ext;const img=isImgExt(ext);const t=el('div',{class:'thumb'+(ext==='md'?' md':img?' img':'')});
 if(ext==='md')t.append(el('div',{class:'tp'},el('div',{class:'tt'},'# '+r.title),el('div',{class:'au'},isUnknown(r)?'':r.authors.join(', ')),el('div',{class:'body'})));
 else if(img)t.append(el('div',{class:'tp'},el('div',{class:'blk'}),el('div',{class:'cap'}),el('div',{class:'cap',style:'width:60%'})));
 else t.append(el('div',{class:'tp'+(['News article','Regulatory filing','Press release','Statute'].includes(r.kind)||r.status==='Auto'?' sans':'')},el('div',{class:'tt'},r.title),el('div',{class:'au'},isUnknown(r)?'':r.authors.join(', ')),el('div',{class:'body'})));
 if(r.thumb_url&&ext!=='md'&&!f.cloud_only&&!f.missing){const im=el('img',{class:'timg',alt:'',loading:'lazy',src:r.thumb_url});im.addEventListener('error',()=>im.remove());t.append(im);}
 if(r.lang&&String(r.lang).toLowerCase()!=='en')t.append(el('span',{class:'lang',title:`${r.lang} language source`},String(r.lang).toUpperCase()));
 t.append(el('span',{class:'ext'},String(ext||'?').toUpperCase()));
 if(r.missing||f.missing)t.append(el('span',{class:'flag miss',title:'The file is missing from the project folder; its highlights are kept'},'Missing'));
 else if(f.cloud_only)t.append(el('span',{class:'flag',title:'Online-only in OneDrive. Kurasi does not open it, so OneDrive does not download it'},'☁'));return t;}
function card(it){if(it.kind==='author')return authorCard(it.a);const r=it.r;const n=r.hl_count;const pct=Math.min(100,15+n*14);
 const src=[r.kind,r.pub].filter(Boolean).join(' · ');const all=S.refTags.get(r.id);const top=(r.top_tags.length?r.top_tags:[...(all||[])]).slice(0,3);const more=all&&all.size>top.length?all.size-top.length:0;
 const c=el('div',{class:'card ref'+(S.sel===r.id?' sel':''),title:`${r.title}\n${r.authors.join('; ')}\n${statusText(r.status)}`,onclick:()=>openDoc(r.id),oncontextmenu:e=>{e.preventDefault();S.sel=r.id;$$('.card.sel').forEach(x=>x.classList.remove('sel'));c.classList.add('sel');refMenu(r,e.clientX,e.clientY);}},
  thumb(r),el('div',{class:'ctext'},
   el('div',{class:'ctitle'},words(r.title,18)),
   el('div',{class:'cauth'+(isUnknown(r)?' unk':'')},r.authors.join('; ')),
   el('div',{class:'csrc',title:src},src||(r.status==='Auto'?'Type and publication not yet labelled':'')),
   el('div',{class:'cmeta'},el('span',{class:'dot '+statusCls(r.status),title:statusText(r.status)}),el('span',{},r.date||r.year||''),r.ext==='pdf'?(r.pages?el('span',{},`${r.pages} pp`):null):el('span',{},r.ext==='md'?'MD':isImgExt(r.ext)?'Image':String(r.ext).toUpperCase()),el('span',{class:'hlc',style:`--pct:${pct}%`,title:plural(n,'highlight')},'■ '+n)),
   el('div',{class:'ctags'},...top.map(t=>el('span',{class:'tagc',title:'#'+t},'#'+t)),more?el('span',{class:'more',title:'more tags'},'+'+more):null)));return c;}
function authorCard(a){const unk=a.name==='Unknown';let name;if(isPerson(a.name)){const [sur,giv]=a.name.split(/,\s*/);name=el('div',{class:'bigname'},el('span',{class:'given'},giv),el('span',{},sur));}else name=el('div',{class:'bigname org'},a.name);
 return el('div',{class:'card author',title:unk?'References with no author yet':a.name,onclick:()=>clickAuthor(a.name),oncontextmenu:e=>{e.preventDefault();showMenu([{label:'Show references',action:()=>clickAuthor(a.name)},{label:'Copy name',action:()=>copyText(a.name)},{sep:true},{label:'Authors are kept; delete their references instead',disabled:true}],e.clientX,e.clientY);}},
  name,el('div',{class:'cmeta'},`${plural(a.refs.length,'reference')} · `,el('span',{class:'hlc',style:`--pct:${Math.min(100,15+a.hl*8)}%`},'■ '+a.hl)),el('div',{class:'sub',title:a.top?a.top.title:''},a.top&&a.hl?'Most highlighted: '+words(a.top.title,7):unk?'Label these in the edit pane':'No highlights yet'));}
function clickAuthor(name){S.f.authors=new Set([name]);S.tab='reference';renderAll();signal('author:clicked');}
function refMenu(r,x,y){const f=r.files.find(x=>x.id===r.primary_file_id)||r.files[0];
 showMenu([{label:'Open',action:()=>openDoc(r.id)},{id:'mi-edit',label:'Edit metadata',action:()=>openEdit(r.id)},{label:'Copy APA 7 citation',action:()=>copyApa(r)},{label:'Copy register ID',disabled:!r.register_id,action:()=>copyText(r.register_id)},{label:'Copy reference ID',action:()=>copyText(r.id)},
  {label:'Reveal in File Explorer',disabled:!f,action:()=>POST('/api/open',{file_id:f.id,mode:'reveal'}).catch(fail)},{label:'Open with the Windows default app',disabled:!f||f.cloud_only,action:()=>POST('/api/open',{file_id:f.id,mode:'default'}).catch(fail)},{sep:true},
  {label:'Delete reference',danger:true,action:()=>deleteRef(r)}],x,y);}
function deleteRef(r){confirmDialog('Delete reference',`Remove <b>${esc(words(r.title,10))}</b> from the project?<br><br>The file stays on disk. The reference, its metadata and its ${plural(r.hl_count,'highlight')} move to the trash; Edit, Undo restores them.`,'Delete',async()=>{
 try{const res=await DEL('/api/reference/'+enc(r.id));pushUndo(res&&res.trash_id,'reference '+words(r.title,4),r.id);if(S.right&&S.right.id===r.id)S.right=null;await refreshData({ids:[r.id]});toast('Reference moved to the trash.',{action:res&&res.trash_id!=null?{label:'Undo',fn:undoLast}:null});}catch(e){fail(e);}});}
async function doExport(format){const refs=visibleRefs();try{const res=await POST('/api/export',{format,ref_ids:refs.map(r=>r.id)});toast(`Written: ${res&&res.path||'the _exports folder'} (${plural(refs.length,'entry','entries')} from the current filter).`,{ms:9000});}catch(e){fail(e);}}

/* ---------- tag graph: tag, author, paper; layout computed in graph_worker.js and cached ---------- */
const Graph={worker:null,failed:false,cache:new Map(),wait:new Map(),msgs:new Map(),queue:[],busy:null,timer:null,sigVer:-1,sigVal:'',
 settingsSig(){return `s${S.graph.view.spacing}`;},
 /* the search chips count in the order added, since that order fixes how their scores add up */
 filterSig(){const f=S.f;const p=[S.q.trim(),[...f.cats].sort().join(','),[...f.authors].sort().join(','),[...f.tags].sort().join(','),[...f.refs].sort().join(','),qChips().join('\n')];return p.join('').length?JSON.stringify(p):'';},
 /* the whole-project layout is keyed by a hash of what the graph shows (ids, titles, authors, tags), so a change
    that leaves those alone (an analysis, a rotation, a setting) keeps the cached layout; a filtered layout
    follows every data change, since a search can match any field */
 sig(){if(this.sigVer!==S.dataVer){let h=2166136261;const mix=s=>{s=String(s);for(let i=0;i<s.length;i++){h^=s.charCodeAt(i);h=Math.imul(h,16777619);}h=Math.imul(h^31,16777619);};
   for(const r of S.refs){mix(r.id);mix(r.title);r.authors.forEach(mix);const t=S.refTags.get(r.id);if(t)t.forEach(mix);mix('');}this.sigVer=S.dataVer;this.sigVal=(h>>>0).toString(36)+'.'+S.refs.length;}return this.sigVal;},
 key(fs){return `${fs?S.changeSeq+'.'+S.dataVer+'.'+FT.stamp:this.sig()}|${this.settingsSig()}|${fs}`;},
 size(){const box=$('.gbox');if(box&&box.clientWidth)return {W:box.clientWidth,H:box.clientHeight};const m=$('#main');const W=Math.max(480,(m&&m.clientWidth||900)-30),H=Math.max(320,(m&&m.clientHeight||600)-86);return {W,H};},
 request(fs,refs){const key=this.key(fs);const hit=this.cache.get(key);if(hit)return Promise.resolve(hit);const w=this.wait.get(key);if(w)return w.p;
  let res,rej;const p=new Promise((a,b)=>{res=a;rej=b;});this.wait.set(key,{p,res,rej});const {W,H}=this.size();
  const msg={key,fs,refs:refs.map(r=>({id:r.id,title:r.title,authors:r.authors,tags:[...(S.refTags.get(r.id)||[])]})),spacing:S.graph.view.spacing/100,W,H};this.msgs.set(key,msg);this.post(msg);return p;},
 /* the worker takes one job at a time. A newer job for the same filter replaces one that still waits and its
    callers get the newer result, so a burst of changes in a large library is laid out once */
 post(msg){const q=this.queue;for(let i=q.length-1;i>=0;i--){const m=q[i];if(m.fs!==msg.fs)continue;q.splice(i,1);this.msgs.delete(m.key);const w=this.wait.get(m.key);this.wait.delete(m.key);if(w)w.res(this.wait.get(msg.key).p);}
  q.push(msg);this.pump();},
 pump(){if(this.busy||!this.queue.length)return;const msg=this.queue.shift();this.busy=msg.key;const w=this.getWorker();if(w)w.postMessage(msg);else setTimeout(()=>this.local(msg),0);},
 local(msg){let out;try{out=kurasiGraphCompute(msg);}catch(e){out={key:msg.key,error:String(e&&e.message||e)};}this.onResult(out);},
 getWorker(){if(this.failed)return null;if(this.worker)return this.worker;
  try{this.worker=new Worker('graph_worker.js');}catch(e){this.failed=true;return null;}
  this.worker.onmessage=e=>this.onResult(e.data);
  /* a worker that cannot start (for example on a page opened from disk) hands its jobs to the page */
  this.worker.onerror=e=>{if(e&&e.preventDefault)e.preventDefault();this.failed=true;try{this.worker.terminate();}catch(x){}this.worker=null;const m=this.busy&&this.msgs.get(this.busy);this.busy=null;if(m)this.queue.unshift(m);this.pump();};
  return this.worker;},
 onResult(out){if(!out||!out.key)return;if(this.busy===out.key)this.busy=null;const w=this.wait.get(out.key);this.wait.delete(out.key);this.msgs.delete(out.key);
  if(out.error){if(w)w.rej(new Error(out.error));}else{this.cache.set(out.key,out);while(this.cache.size>8)this.cache.delete(this.cache.keys().next().value);if(w)w.res(out);}this.pump();},
 /* after changes: recompute the whole-project layout in the background */
 schedule(){clearTimeout(this.timer);if(!S.project)return;this.timer=setTimeout(()=>{this.request('',S.refs).then(()=>{if(S.tab==='tags'&&!$('.gbox svg'))renderMain();}).catch(()=>{});},400);},
 /* node spacing changed: lay the graph out again inside the open canvas. The layout grows or shrinks
    around the centre of the canvas and the zoom stays, so the marks keep their size on screen and the
    gaps between them change; the view fits when asked or when no node is left in sight. */
 relayout(fit){this.schedule();const d=this.el;if(S.tab!=='tags'||!d||!d.box.isConnected)return;const box=d.box,fs=this.filterSig(),key=this.key(fs);
  this.request(fs,fs?visibleRefs():S.refs).then(g=>{if(S.tab==='tags'&&box.isConnected&&this.key(this.filterSig())===key){drawGraph(box,g,{auto:false});if(fit||!this.anyVisible())this.fit();}}).catch(fail);},

 /* ---- view: zoom and pan as transforms on the SVG and on the two canvases (see applyView and place),
    label size and opacities as CSS variables. Node coordinates stay as the worker laid them out. Node spacing is a
    layout input: a change lays the graph out again. Persisted in the project settings. ---- */
 VIEW_DEF:{zoom:1,panX:0,panY:0,fontPx:12,labelOp:100,edgeOp:60,spacing:100},
 KEYS:{zoom:'graph.zoom',panX:'graph.pan_x',panY:'graph.pan_y',fontPx:'graph.font_px',labelOp:'graph.label_opacity',edgeOp:'graph.edge_opacity',spacing:'graph.spacing'},
 LIM:{zoom:[0.2,6],panX:[-1e6,1e6],panY:[-1e6,1e6],fontPx:[8,24],labelOp:[0,100],edgeOp:[10,100],spacing:[50,300]},
 el:null,dirty:{},saveTimer:null,suppressClick:false,
 clamp(k,n){const [lo,hi]=this.LIM[k];return Math.max(lo,Math.min(hi,n));},
 loadView(){const v={...this.VIEW_DEF};for(const [k,key] of Object.entries(this.KEYS)){const raw=S.settings[key];const n=typeof raw==='string'?parseFloat(raw):+raw;if(raw!=null&&raw!==''&&isFinite(n))v[k]=this.clamp(k,n);}S.graph.view=v;this.dirty={};clearTimeout(this.saveTimer);this.applyView();},
 setView(patch,{save=true,fit=false}={}){const v=S.graph.view,sp=v.spacing;for(const [k,n] of Object.entries(patch)){if(k in this.LIM&&isFinite(n)){v[k]=this.clamp(k,n);if(save)this.dirty[k]=1;}}this.applyView();if(save)this.saveSoon();if(v.spacing!==sp)this.relayout(fit);},
 saveSoon(){clearTimeout(this.saveTimer);this.saveTimer=setTimeout(()=>this.saveNow(),500);},
 saveNow(){clearTimeout(this.saveTimer);const ks=Object.keys(this.dirty);this.dirty={};if(!ks.length||!S.project)return;const v=S.graph.view,body={};
  ks.forEach(k=>{const n=k==='zoom'?Math.round(v[k]*1000)/1000:Math.round(v[k]);body[this.KEYS[k]]=n;S.settings[this.KEYS[k]]=n;});PUT('/api/settings',body).catch(()=>{});},
 /* while the view moves, the SVG follows with a CSS transform on the svg element, relative to the view last
    written into its .view group, which spares the layout of every node group; the canvases follow on the
    next frame. Once the view has rested for 120 ms the view is written into the group and the labels are
    placed again; above 1000 nodes they are hidden while the zoom changes. */
 applyView(){const d=this.el;if(!d||!d.box.isConnected)return;const v=S.graph.view,c=d.vc;
  if(d.vg){if(!c)this.commit();else{const s=v.zoom/c.zoom;d.svg.style.transform=s===1&&v.panX===c.panX&&v.panY===c.panY?'':`translate(${v.panX-c.panX*s}px,${v.panY-c.panY*s}px) scale(${s})`;}}
  d.box.style.setProperty('--gfont',v.fontPx+'px');d.box.style.setProperty('--glabel',String(v.labelOp/100));d.box.style.setProperty('--gedge',String(v.edgeOp/100));d.zoomLbl.textContent=Math.round(v.zoom*100)+'%';
  Object.entries(d.inputs).forEach(([k,inp])=>{if(document.activeElement!==inp)inp.value=v[k];d.outs[k].textContent=v[k]+d.units[k];});
  if(d.vg){if(d.az!==v.zoom&&d.az!=null&&d.nodes.length>1000)d.box.classList.add('gz');d.az=v.zoom;this.paint();clearTimeout(d.lt);d.lt=setTimeout(()=>{this.settle();this.commit();this.labels();},120);}},
 commit(){const d=this.el;if(!d||!d.vg)return;const v=S.graph.view,c=d.vc;d.svg.style.transform='';if(c&&c.zoom===v.zoom&&c.panX===v.panX&&c.panY===v.panY)return;d.vg.setAttribute('transform',`translate(${v.panX} ${v.panY}) scale(${v.zoom})`);d.vc={zoom:v.zoom,panX:v.panX,panY:v.panY};},
 /* zoom by a factor around a point of the box (box pixels) */
 zoomAt(f,cx,cy){const v=S.graph.view;const z=this.clamp('zoom',v.zoom*f);if(z===v.zoom)return;const k=z/v.zoom;this.setView({zoom:z,panX:cx-(cx-v.panX)*k,panY:cy-(cy-v.panY)*k});},
 centre(){const b=this.el&&this.el.box;return b?[b.clientWidth/2,b.clientHeight/2]:[0,0];},
 zoomStep(dir){const [cx,cy]=this.centre();this.zoomAt(dir>0?1.25:0.8,cx,cy);},
 one(){const [cx,cy]=this.centre();const v=S.graph.view;const k=1/v.zoom;this.setView({zoom:1,panX:cx-(cx-v.panX)*k,panY:cy-(cy-v.panY)*k});},
 /* edges and marks are painted into two canvases under the SVG, each larger than the view by 30% on every
    side and painted at the zoom of that moment. Panning and zooming only move and scale the two canvases
    with a CSS transform, which costs no repaint; they are painted again when the view runs past them and
    once the view has rested at another zoom. Edge opacity and the hover fade are CSS opacity on them. */
 paint(){const d=this.el;if(!d||!d.ce||d.raf)return;if(document.visibilityState==='hidden'){this.place();return;}d.raf=requestAnimationFrame(()=>{d.raf=0;this.place();});},
 place(force){const d=this.el;if(!d||!d.ce||!d.box.isConnected)return;const v=S.graph.view,z=v.zoom,W=d.box.clientWidth,H=d.box.clientHeight;let b=d.buf;
  if(force||!b||b.dpr!==(window.devicePixelRatio||1)||-v.panX/z<b.x0||-v.panY/z<b.y0||(W-v.panX)/z>b.x1||(H-v.panY)/z>b.y1)b=this.rebuild();
  const tr=`translate(${b.x0*z+v.panX}px,${b.y0*z+v.panY}px) scale(${z/b.z})`;d.ce.style.transform=tr;d.cm.style.transform=tr;},
 /* at rest: paint again at the new zoom, or when the view has drifted towards the edge of the canvases */
 settle(){const d=this.el;if(!d||!d.buf||!d.box.isConnected)return;const v=S.graph.view,z=v.zoom,b=d.buf,W=d.box.clientWidth,H=d.box.clientHeight;
  if(b.z!==z||Math.abs((W/2-v.panX)/z-(b.x0+b.x1)/2)*z>0.15*W||Math.abs((H/2-v.panY)/z-(b.y0+b.y1)/2)*z>0.15*H)this.place(true);},
 rebuild(){const d=this.el,v=S.graph.view,z=v.zoom,W=Math.max(1,d.box.clientWidth),H=Math.max(1,d.box.clientHeight),dpr=window.devicePixelRatio||1,mx=Math.round(W*0.3),my=Math.round(H*0.3),k=z*dpr;
  const b={z,dpr,x0:(-v.panX-mx)/z,y0:(-v.panY-my)/z,x1:(W+mx-v.panX)/z,y1:(H+my-v.panY)/z},cw=Math.round((W+2*mx)*dpr),ch=Math.round((H+2*my)*dpr),N=d.nodes,E=d.edges,sk=d.skip;d.buf=b;
  for(const cv of [d.ce,d.cm]){if(cv.width!==cw||cv.height!==ch){cv.width=cw;cv.height=ch;}cv.style.width=cw/dpr+'px';cv.style.height=ch/dpr+'px';}
  const cs=getComputedStyle(d.box),col=p=>cs.getPropertyValue(p).trim();let c=d.ce.getContext('2d');c.setTransform(1,0,0,1,0,0);c.clearRect(0,0,cw,ch);c.setTransform(k,0,0,k,-b.x0*k,-b.y0*k);c.strokeStyle=col('--line-2')||'#c8c8c8';c.lineWidth=1/z;c.beginPath();
  for(let e=0;e<E.length;e++){const i=E[e][0],j=E[e][1];if(i===sk||j===sk)continue;const p=N[i],q=N[j];if((p.x<b.x0&&q.x<b.x0)||(p.x>b.x1&&q.x>b.x1)||(p.y<b.y0&&q.y<b.y0)||(p.y>b.y1&&q.y>b.y1))continue;c.moveTo(p.x,p.y);c.lineTo(q.x,q.y);}c.stroke();
  /* marks: one sprite per type and size on screen, drawn once into an atlas and copied to whole pixels */
  const fills={t:col('--n-tag'),a:col('--n-author'),p:col('--n-paper')},halo=col('--surface'),sig=[k,halo,fills.t,fills.a,fills.p].join();if(!d.atlas||d.atlas.sig!==sig)d.atlas={sig,map:new Map(),cv:null};
  const A=d.atlas,vis=[],ent=[];let grown=!A.cv;for(const n of N){const s=n.size;if(n.i===sk||n.x+s<b.x0||n.x-s>b.x1||n.y+s<b.y0||n.y-s>b.y1)continue;const q=Math.round(s*k*4)/4,key=n.type[0]+q;let e=A.map.get(key);if(!e){e={t:n.type[0],q,m:Math.ceil(q*1.2+1.5*dpr)};A.map.set(key,e);grown=true;}vis.push(n);ent.push(e);}
  if(grown){let x=0,y=0,rh=0,aw=1;for(const e of A.map.values()){const w=2*e.m+2;if(x+w>2048&&x>0){x=0;y+=rh;rh=0;}e.x=x;e.y=y;x+=w;aw=Math.max(aw,x);rh=Math.max(rh,w);}
   const cv=A.cv||(A.cv=document.createElement('canvas'));cv.width=aw;cv.height=Math.max(1,y+rh);const g=cv.getContext('2d');g.lineWidth=2*dpr;g.strokeStyle=halo;
   for(const e of A.map.values()){const q=e.q;g.setTransform(1,0,0,1,e.x+e.m+1,e.y+e.m+1);g.beginPath();if(e.t==='t')g.arc(0,0,q,0,2*Math.PI);else if(e.t==='a'){const h=q*0.8*Math.SQRT2;g.moveTo(0,-h);g.lineTo(h,0);g.lineTo(0,h);g.lineTo(-h,0);g.closePath();}else g.roundRect(-q,-q,2*q,2*q,Math.min(q,1.5*k));g.fillStyle=fills[e.t];g.fill();g.stroke();}}
  c=d.cm.getContext('2d');c.setTransform(1,0,0,1,0,0);c.clearRect(0,0,cw,ch);const ox=-b.x0*k,oy=-b.y0*k;
  for(let t=0;t<vis.length;t++){const n=vis[t],e=ent[t],w=2*e.m+2;c.drawImage(A.cv,e.x,e.y,w,w,Math.round(n.x*k+ox)-e.m-1,Math.round(n.y*k+oy)-e.m-1,w,w);}return b;},
 /* label size on screen: the chosen size times the zoom, never under 9 px (or the chosen size when smaller) */
 fontAt(z){const f=S.graph.view.fontPx;return Math.max(f*z,Math.min(f,9));},
 /* label width at 100 px, measured once per node with the page font (tags are bold) */
 lw(n){const d=this.el;let w=d.lw100[n.i];if(w>=0)return w;if(!d.mctx){d.mctx=document.createElement('canvas').getContext('2d');d.mfam=getComputedStyle(d.box).fontFamily||'sans-serif';}
  d.mctx.font=(n.type==='tag'?'600 ':'')+'100px '+d.mfam;w=d.mctx.measureText(n.label).width;d.lw100[n.i]=w;return w;},
 /* Level of detail: labels are placed in priority order (tags and authors by the papers they touch, then
    papers) and a label whose box would overlap one already placed is skipped. A paper label also needs a
    spot clear of other marks. Only nodes in sight take part, 900 labels at most. */
 labels(){const d=this.el;if(!d||!d.gL||!d.box.isConnected)return;d.box.classList.remove('gz');const v=S.graph.view,z=v.zoom,W=d.box.clientWidth,H=d.box.clientHeight,N=d.nodes,F=this.fontAt(z);
  const C=48,cols=Math.ceil(W/C)+1,rows=Math.ceil(H/C)+1,occ=new Array(cols*rows),marks=new Array(cols*rows);
  const cells=(x0,y0,x1,y1,f)=>{const a0=Math.max(0,Math.floor(x0/C)),a1=Math.min(cols-1,Math.floor(x1/C)),b0=Math.max(0,Math.floor(y0/C)),b1=Math.min(rows-1,Math.floor(y1/C));for(let b=b0;b<=b1;b++)for(let a=a0;a<=a1;a++)if(f(b*cols+a))return true;return false;};
  const add=(grid,bx)=>cells(bx[0],bx[1],bx[2],bx[3],c=>{(grid[c]||(grid[c]=[])).push(bx);return false;});
  const hit=(grid,x0,y0,x1,y1,self)=>cells(x0,y0,x1,y1,c=>{const l=grid[c];if(l)for(const q of l)if(q[4]!==self&&q[0]<x1&&q[2]>x0&&q[1]<y1&&q[3]>y0)return true;return false;});
  for(const n of N){const r=n.size*z;if(r<2)continue;const sx=n.x*z+v.panX,sy=n.y*z+v.panY;if(sx+r>=0&&sx-r<=W&&sy+r>=0&&sy-r<=H)add(marks,[sx-r,sy-r,sx+r,sy+r,n.i]);}
  const out=[];for(const i of d.order){const n=N[i];const sx=n.x*z+v.panX,sy=n.y*z+v.panY;if(sx<-400||sx>W||sy<-F||sy>H+F)continue;
   const f=n.type==='paper'?F*0.85:F,x0=sx+n.size*z+f/3,y0=sy-f*0.7,y1=sy+f*0.35,x1=x0+this.lw(n)*f/100;if(x1<0||x0>W)continue;
   if(hit(occ,x0,y0,x1,y1,-1)||(n.type==='paper'&&hit(marks,x0,y0,x1,y1,i)))continue;add(occ,[x0,y0,x1,y1,i]);out.push(i);if(out.length>=900)break;}
  const zf=F/z;d.lzf=zf;d.gL.setAttribute('font-size',zf);d.gL.setAttribute('stroke-width',3/z);
  d.gL.innerHTML=out.map(i=>{const n=N[i],p=this.labPos(n);return `<text data-i="${i}" class="${n.type[0]}" x="${p[0]}" y="${p[1]}">${esc(n.label)}</text>`;}).join('');
  d.labs=out;d.labEl=new Map();for(const t of d.gL.children)d.labEl.set(+t.dataset.i,t);if(d.hi>=0)this.markHover();},
 labPos(n){const d=this.el,f=d.lzf*(n.type==='paper'?0.85:1);return [Math.round((n.x+n.size+f/3)*10)/10,Math.round((n.y+f*0.33)*10)/10];},
 /* hover: the two canvases fade (CSS opacity); the node's edges and copies of the node and its neighbours
    sit above them in the SVG, so the cost follows the number of neighbours */
 hover(i){const d=this.el;if(!d||!d.gN||d.hi===i)return;if(d.hi>=0)this.unhover();d.hi=i;const n=d.nodes[i],nb=d.adj[i];d.box.classList.add('hov');
  const frag=document.createDocumentFragment();for(const j of nb)frag.append(d.els[j].cloneNode(true));d.hclone=d.els[i].cloneNode(true);d.hclone.classList.add('lit');frag.append(d.hclone);d.gH.innerHTML='<path class="hedge"/>';d.hpath=d.gH.firstChild;d.gH.append(frag);
  this.hedges();this.markHover();
  const papers=nb.filter(j=>d.nodes[j].type==='paper').length;const r=n.ref&&refById(n.ref);const tip=d.tip;tip.innerHTML='';
  if(n.type==='paper'){const au=r?r.authors.filter(a=>a!=='Unknown').length:0;tip.append(el('b',{},r?r.title:n.label),el('span',{class:'k'},`${r?r.authors.join('; '):''} · ${plural(Math.max(0,nb.length-au),'tag')}`));}
  else tip.append(el('b',{},n.label),el('span',{class:'k'},`${n.type==='tag'?'Tag':'Author'} · ${plural(papers,'paper')} · click to filter Reference`));tip.hidden=false;},
 hedges(){const d=this.el,i=d.hi;if(i<0||!d.hpath)return;const a=d.nodes[i];d.hpath.setAttribute('d',d.adj[i].map(j=>`M${a.x} ${a.y}L${d.nodes[j].x} ${d.nodes[j].y}`).join(''));},
 markHover(){const d=this.el,i=d.hi;if(i<0)return;const nb=new Set(d.adj[i]);for(const t of d.gL.children){const j=+t.dataset.i;t.classList.toggle('on',j===i||nb.has(j));}
  if(!d.labEl.has(i)&&!d.gL.querySelector('.tmp')){const n=d.nodes[i],p=this.labPos(n);d.gL.insertAdjacentHTML('beforeend',`<text class="${n.type[0]} tmp on" x="${p[0]}" y="${p[1]}">${esc(n.label)}</text>`);}},
 unhover(){const d=this.el;if(!d||d.hi<0)return;d.hi=-1;d.hclone=d.hpath=null;d.box.classList.remove('hov');d.gH.textContent='';d.gL.querySelectorAll('.tmp').forEach(t=>t.remove());d.gL.querySelectorAll('.on').forEach(t=>t.classList.remove('on'));d.tip.hidden=true;},
 /* fit every node with its label into the canvas, below the toolbar (or the gear when the toolbar is
    folded); maxZoom caps the zoom, so the automatic fit only zooms out. Labels keep a floor size on screen,
    so their width in graph units depends on the zoom and the fit is solved in a few rounds. */
 fit(maxZoom){const d=this.el;if(!d||!d.nodes.length)return;const v=S.graph.view;const W=d.box.clientWidth,H=d.box.clientHeight,pad=24,ctl=d.gear.offsetParent?d.gear:d.tb,top=Math.max(pad,ctl.offsetTop+ctl.offsetHeight+8);
  let z=v.zoom,x0,y0,x1,y1;const box=z=>{const F=this.fontAt(z);x0=y0=Infinity;x1=y1=-Infinity;for(const n of d.nodes){const s=n.size,f=n.type==='paper'?F*0.85:F,lw=n.lab?(f/3+this.lw(n)*f/100)/z:0;if(n.x-s<x0)x0=n.x-s;if(n.y-s<y0)y0=n.y-s;if(n.x+s+lw>x1)x1=n.x+s+lw;if(n.y+s>y1)y1=n.y+s;}};
  for(let k=0;k<5;k++){box(z);z=this.clamp('zoom',Math.min(maxZoom||Infinity,(W-2*pad)/Math.max(1,x1-x0),(H-top-pad)/Math.max(1,y1-y0)));}box(z);
  const bw=Math.max(1,x1-x0),bh=Math.max(1,y1-y0);this.setView({zoom:z,panX:(W-bw*z)/2-x0*z,panY:top+(H-top-pad-bh*z)/2-y0*z});},
 /* true when at least one node sits inside the canvas at the current view */
 anyVisible(){const d=this.el;if(!d||!d.nodes.length)return true;const v=S.graph.view,W=d.box.clientWidth,H=d.box.clientHeight;
  return d.nodes.some(n=>{const x=n.x*v.zoom+v.panX,y=n.y*v.zoom+v.panY;return x>=0&&x<=W&&y>=0&&y<=H;});},
 pan(dx,dy){const v=S.graph.view;this.setView({panX:v.panX+dx,panY:v.panY+dy});},
 resetView(){this.setView({...this.VIEW_DEF});},
 /* drag of the whole canvas: left button on empty canvas, middle button anywhere */
 startPan(e){const v=S.graph.view,x0=e.clientX,y0=e.clientY,px=v.panX,py=v.panY,box=this.el&&this.el.box;if(box)box.classList.add('panning');
  const move=ev=>{this.setView({panX:px+(ev.clientX-x0),panY:py+(ev.clientY-y0)});};
  const up=()=>{document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',up);if(box)box.classList.remove('panning');};
  document.addEventListener('mousemove',move);document.addEventListener('mouseup',up);},
 /* drag of one node: the node stays lit while it moves, the canvases are painted once without it and its
    edges, and its copy, label and edges follow the pointer in the SVG; the change is written back into the
    cached layout */
 startNodeDrag(e,i){const d=this.el,n=d.nodes[i],g_=d.els[i],v=S.graph.view,x0=e.clientX,y0=e.clientY,nx=n.x,ny=n.y;let moved=false;
  const move=ev=>{if(!moved&&Math.hypot(ev.clientX-x0,ev.clientY-y0)<3)return;if(!moved){moved=true;if(d.hi!==i)this.hover(i);d.drag=true;d.skip=i;this.place(true);}
   n.x=nx+(ev.clientX-x0)/v.zoom;n.y=ny+(ev.clientY-y0)/v.zoom;const tr=`translate(${n.x},${n.y})`;g_.setAttribute('transform',tr);if(d.hclone)d.hclone.setAttribute('transform',tr);
   const t=d.labEl.get(i)||d.gL.querySelector('.tmp');if(t){const p=this.labPos(n);t.setAttribute('x',p[0]);t.setAttribute('y',p[1]);}this.hedges();};
  const up=()=>{document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',up);if(moved){d.drag=false;d.skip=-1;this.place(true);const src=d.g.nodes[i];if(src){src.x=Math.round((n.x-d.ox)*10)/10;src.y=Math.round((n.y-d.oy)*10)/10;}this.suppressClick=true;setTimeout(()=>{this.suppressClick=false;},0);}};
  document.addEventListener('mousemove',move);document.addEventListener('mouseup',up);}};
/* the view toolbar at the top of the canvas; below 1100 px the stylesheet folds it behind the gear button */
function graphToolbar(box){const v=S.graph.view;const inputs={},outs={},units={fontPx:' px',labelOp:'%',edgeOp:'%',spacing:'%'};
 const btn=(label,title,fn)=>el('button',{class:'tool',title,onclick:fn},label);
 const zoomLbl=el('span',{class:'zl',title:'Zoom level'},Math.round(v.zoom*100)+'%');
 /* a live slider applies while it moves; node spacing lays the graph out again, so it applies when the
    slider is let go */
 const slider=(k,label,min,max,step=1,live=true)=>{const inp=el('input',{type:'range',min,max,step,value:v[k],'aria-label':label});const out=el('span',{class:'val'},v[k]+units[k]);
  if(live)inp.addEventListener('input',()=>{Graph.setView({[k]:+inp.value});});
  else{inp.addEventListener('input',()=>{out.textContent=inp.value+units[k];});inp.addEventListener('change',()=>{Graph.setView({[k]:+inp.value});});}
  inputs[k]=inp;outs[k]=out;return el('label',{class:'gs'},el('span',{class:'gl'},label),inp,out);};
 const tb=el('div',{class:'gtb',role:'toolbar','aria-label':'Graph view'},
  el('div',{class:'grp'},btn('+','Zoom in (Ctrl +)',()=>Graph.zoomStep(1)),zoomLbl,btn('-','Zoom out (Ctrl -)',()=>Graph.zoomStep(-1)),btn('Fit','Fit all nodes in the canvas (Ctrl 0)',()=>Graph.fit()),btn('1:1','Zoom 100% around the centre',()=>Graph.one())),
  el('div',{class:'grp sl'},slider('spacing','Node spacing',50,300,10,false),slider('fontPx','Label size',8,24),slider('labelOp','Label opacity',0,100),slider('edgeOp','Edge opacity',10,100)),
  el('div',{class:'grp'},btn('Reset view','Zoom 100%, no pan, node spacing 100%, labels 12 px at 100%, edges at 60%',()=>Graph.resetView())));
 const gear=el('button',{class:'tool ggear',title:'Graph view controls','aria-label':'Graph view controls','aria-expanded':'false',onclick:()=>{const o=box.classList.toggle('tbopen');gear.setAttribute('aria-expanded',String(o));}},'⚙︎');
 box.append(gear,tb);Graph.el={box,tb,gear,inputs,outs,units,zoomLbl,vg:null,nodes:[],hi:-1};Graph.applyView();}
function renderGraph(m){const fs=Graph.filterSig(),key=Graph.key(fs),d=Graph.el;
 /* the same layout in a canvas of the same size: the drawing already made goes back in place as it was */
 if(d&&d.vg&&d.key===key&&d.wrap&&d.mw===m.clientWidth&&d.mh===m.clientHeight){Graph.unhover();m.append(d.wrap);d.buf=null;Graph.applyView();return;}
 const wrap=el('div',{class:'gwrap'});const head=el('div',{class:'cathead'});filterChips(head);
 const box=el('div',{class:'gbox'});if(head.childNodes.length)wrap.append(head);wrap.append(box);m.append(wrap);graphToolbar(box);Object.assign(Graph.el,{wrap,mw:m.clientWidth,mh:m.clientHeight});
 /* canvas gestures: wheel zooms around the pointer; left-drag on empty canvas and middle-drag anywhere pan */
 box.addEventListener('wheel',e=>{e.preventDefault();const b=box.getBoundingClientRect();Graph.zoomAt(Math.exp(-e.deltaY*0.0015),e.clientX-b.left,e.clientY-b.top);},{passive:false});
 box.addEventListener('mousedown',e=>{if(e.target.closest('.gtb,.ggear'))return;if(e.button===1||(e.button===0&&!e.target.closest('.node'))){e.preventDefault();Graph.startPan(e);}});
 const hit=Graph.cache.get(key);
 if(hit){requestAnimationFrame(()=>drawGraph(box,hit));return;}
 const note=el('div',{class:'empty'},'Laying out the graph');box.append(note);
 Graph.request(fs,fs?visibleRefs():S.refs).then(g=>{if(S.tab==='tags'&&box.isConnected&&Graph.key(Graph.filterSig())===key)drawGraph(box,g);}).catch(e=>{note.textContent='The graph could not be laid out: '+e.message;});}
/* draws a layout into the canvas, replacing an earlier drawing. Edges and marks are painted on two canvases
   (see Graph.place). The SVG above them holds one invisible group per node, which takes the pointer events
   and is found by the tests and the tour as .node.tag .mark and so on, with the label text in a desc
   element; the hover copies and the labels sit in their own groups. Pointer events are handled once on the
   SVG. auto:false skips the automatic fit, which zooms out when no view was saved yet or when every node
   lies outside the canvas. */
function drawGraph(box,g,{auto=true}={}){const d=Graph.el;if(!d||d.box!==box)return;box.querySelectorAll(':scope>svg,:scope>canvas,:scope>.gtip,:scope>.empty').forEach(x=>x.remove());box.classList.remove('hov');
 Object.assign(d,{vg:null,gN:null,gL:null,ce:null,cm:null,buf:null,vc:null,nodes:[],hi:-1,key:null});
 if(!g.nodes.length){box.append(el('div',{class:'empty'},'No highlights with tags in the current filter.'));return;}
 /* the layout is centred on the box it was computed for; a canvas of another size only shifts it */
 const W=box.clientWidth||g.W,H=box.clientHeight||g.H;const ox=(W-g.W)/2,oy=(H-g.H)/2;const nodes=g.nodes.map((n,i)=>({...n,i,x:n.x+ox,y:n.y+oy,size:n.r||6}));
 const adj=nodes.map(()=>[]);for(const [a,b] of g.edges){adj[a].push(b);adj[b].push(a);}
 /* a diamond is a square turned 45 degrees; a half side of 0.8 r keeps its corners within the gap the layout leaves around r */
 let h='<g class="view"><g class="gnodes">';for(const n of nodes){const s=n.size;let mark;
  if(n.type==='tag')mark=`<circle class="mark" r="${s}"/>`;else if(n.type==='author'){const q=Math.round(s*0.8*10)/10;mark=`<rect class="mark" x="${-q}" y="${-q}" width="${q*2}" height="${q*2}" transform="rotate(45)"/>`;}else mark=`<rect class="mark" x="${-s}" y="${-s}" width="${s*2}" height="${s*2}" rx="1.5"/>`;
  h+=`<g class="node ${n.type}" data-i="${n.i}" transform="translate(${n.x},${n.y})"><desc>${esc(n.label)}</desc>${mark}</g>`;}
 h+='</g><g class="ghover"></g><g class="glabels"></g></g>';
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('width',W);svg.setAttribute('height',H);svg.innerHTML=h;
 const ce=el('canvas',{class:'gedges'}),cm=el('canvas',{class:'gmarks'}),tip=el('div',{class:'gtip',hidden:true});box.append(ce,cm,svg,tip);
 const vg=svg.firstChild,[gN,gH,gL]=vg.children;
 const order=nodes.map(n=>n.i).sort((a,b)=>{const A=nodes[a],B=nodes[b];return (A.type==='paper')-(B.type==='paper')||B.deg-A.deg||(A.type==='tag'?0:1)-(B.type==='tag'?0:1)||a-b;});
 Object.assign(d,{svg,ce,cm,vg,gN,gH,gL,tip,els:[...gN.children],nodes,edges:g.edges,adj,order,g,ox,oy,hi:-1,hclone:null,hpath:null,skip:-1,drag:false,buf:null,labs:[],labEl:new Map(),lw100:new Float32Array(nodes.length).fill(-1),key:g.key});
 const nodeOf=e=>{const t=e.target&&e.target.closest&&e.target.closest('.node');return t&&t.parentNode===gN?+t.dataset.i:-1;};
 svg.addEventListener('mouseover',e=>{const i=nodeOf(e);if(i>=0&&!d.drag)Graph.hover(i);});
 svg.addEventListener('mouseout',e=>{if(d.hi<0||d.drag)return;const r=e.relatedTarget,t=r&&r.closest&&r.closest('.node');if(t&&t.parentNode===gN&&+t.dataset.i===d.hi)return;Graph.unhover();});
 svg.addEventListener('mousemove',e=>{if(d.hi<0)return;const b=box.getBoundingClientRect();tip.style.left=Math.max(4,Math.min(b.width-310,e.clientX-b.left+14))+'px';tip.style.top=(e.clientY-b.top+14)+'px';});
 svg.addEventListener('mousedown',e=>{if(e.button!==0)return;const i=nodeOf(e);if(i<0)return;e.stopPropagation();e.preventDefault();Graph.startNodeDrag(e,i);});
 svg.addEventListener('click',e=>{const i=nodeOf(e);if(i>=0&&!Graph.suppressClick)graphClick(nodes[i]);});
 Graph.applyView();if(auto&&(S.settings[Graph.KEYS.zoom]==null||!Graph.anyVisible()))Graph.fit(1);Graph.labels();}
/* the system theme changed: the canvases are painted again in the new colours */
matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{const d=Graph.el;if(d&&d.ce){d.buf=null;Graph.paint();}});
function graphClick(n){if(n.type==='tag')S.f.tags=new Set([n.id.slice(2)]);else if(n.type==='author')S.f.authors=new Set([n.id.slice(2)]);else S.f.refs=new Set([n.ref]);S.tab='reference';renderAll();signal('graph:clicked');}

/* ---------- edit pane (right, 30%) ---------- */
const KINDS=['','Journal article','Book','Book chapter','Report','Regulatory filing','News article','Press release','Statute','Statistical release','Thesis','Working note','Exhibit','Web page'];
const LANGS=['','en','id','ko','ja','zh','de','fr','nl','es'];
const USAB=['','Full text','Abstract only','Excerpt','Translated'];
const EDIT_KEYS=['title','kind','year','date','pub','vol','issue','pages_range','doi','url','lang','usability'];
function openEdit(id){S.right={mode:'edit',id,rec:null,draft:null,err:null};renderRight();signal('edit:open');loadEditRec(id);}
function loadEditRec(id){GET('/api/reference/'+enc(id)).then(rec=>{if(S.right&&S.right.id===id){S.right.rec=normRef({...refById(id),...rec});S.right.err=null;renderRight();}}).catch(e=>{if(S.right&&S.right.id===id){S.right.err=e.message;renderRight();}});}
function closeRight(){S.right=null;renderRight();}
function renderRight(){renderRightPane();document.body.classList.toggle('rp',!$('#right').hidden);}
function renderRightPane(){const R=$('#right');const doc=docTabRef();if(doc){R.hidden=false;renderKnowledge(R,doc);return;}
 if(!S.right||S.right.mode!=='edit'){R.hidden=true;R.innerHTML='';return;}const base=refById(S.right.id);if(!base){R.hidden=true;R.innerHTML='';return;}R.hidden=false;renderEdit(R,base);}
function renderEdit(R,base){const st=S.right;const top=R.querySelector('.rbody')?R.querySelector('.rbody').scrollTop:0;R.innerHTML='';
 R.append(el('div',{class:'rhead'},'Edit reference',el('span',{class:'cnt'},String(base.ext||'').toUpperCase()),el('span',{class:'sp'}),el('button',{class:'btn sm ico',title:'Close',onclick:closeRight},'✕')));
 const body=el('div',{class:'rbody'});R.append(body);
 if(!st.rec){body.append(st.err?el('div',{class:'empty'},el('div',{},st.err),el('button',{class:'btn sm',onclick:()=>{st.err=null;renderRight();loadEditRec(st.id);}},'Try again')):el('div',{class:'empty'},'Loading the reference'));return;}
 const r=st.rec;if(!st.draft){st.draft={authors:r.authors.filter(a=>a!=='Unknown').join('\n'),cats:r.categories.filter(c=>c!=='Uncategorized')};EDIT_KEYS.forEach(k=>st.draft[k]=r[k]==null?'':String(r[k]));}const D=st.draft;
 const fld=(k,label,opts={})=>{const val=k in D?D[k]:(r[k]??'');let inp;
  if(opts.type==='select'){const o=[...opts.options];if(val&&!o.includes(val))o.push(val);inp=el('select',{},...o.map(x=>el('option',{value:x,selected:x===val?true:null},x||'(none)')));}
  else if(opts.type==='textarea')inp=el('textarea',{},val);else inp=el('input',{type:'text',value:val,readonly:opts.readonly?true:null,placeholder:opts.ph||null});
  inp.id='f-'+k;if(!opts.readonly){const upd=()=>{D[k]=inp.value;st.dirty=true;};inp.addEventListener('input',upd);inp.addEventListener('change',upd);}return el('div',{class:'fld'},el('label',{for:'f-'+k},label),inp);};
 if(hintsOn()&&!S.tour.on)body.append(el('div',{class:'mcphint'},el('span',{class:'t',html:`<code>reference_update_metadata</code>: ${esc(aiName())} can fill these fields from the first page or a DOI lookup.`}),el('button',{class:'btn sm',onclick:()=>copyPrompt(Prompt.metadata(r))},'Copy prompt for Claude')));
 if(r.status==='Suggested')body.append(el('div',{class:'tourhint'},`Proposed by ${aiName()} via MCP. Save to accept, or edit first.`));
 body.append(fld('title','Title',{type:'textarea'}),fld('authors','Authors (one per line, Surname, Initials)',{type:'textarea'}),
  el('div',{class:'row3'},fld('kind','Type',{type:'select',options:KINDS}),fld('year','Year'),fld('date','Document date',{ph:'yyyy-mm-dd'})),
  fld('pub','Publication or publisher'),el('div',{class:'row3'},fld('vol','Volume'),fld('issue','Issue'),fld('pages_range','Pages')),
  el('div',{class:'row2'},fld('doi','DOI'),fld('url','URL')),
  el('div',{class:'row2'},fld('lang','Language',{type:'select',options:LANGS}),fld('usability','Usability',{type:'select',options:USAB})));
 const cats=el('div',{class:'cats'});const drawCats=()=>{cats.innerHTML='';
  D.cats.forEach(c=>cats.append(el('span',{class:'tagc',title:c},c.split('/').pop(),el('b',{title:'Remove',onclick:()=>{D.cats=D.cats.filter(x=>x!==c);st.dirty=true;drawCats();}},'✕'))));
  const sel=el('select',{class:'sel',style:'height:24px;font-size:12px;max-width:100%','aria-label':'Add category'},el('option',{value:''},'+ Add category'),...S.catPaths.filter(p=>p!=='Uncategorized'&&!D.cats.includes(p)).map(p=>el('option',{value:p},p)),el('option',{value:'__new'},'New category…'));
  sel.addEventListener('change',()=>{if(sel.value==='__new'){categoryDialog({onCreated:p=>{if(!D.cats.includes(p))D.cats.push(p);st.dirty=true;renderRight();}});sel.value='';return;}if(sel.value){D.cats.push(sel.value);st.dirty=true;drawCats();}});cats.append(sel);};drawCats();
 body.append(el('div',{class:'fld'},el('label',{},'Categories'),cats));
 body.append(fld('register_id','Register ID',{readonly:true}));
 const folder=S.project?S.project.folder:'';body.append(el('div',{class:'fld'},el('label',{},'Files (never moved by the app)'),el('div',{class:'files'},...r.files.map(f=>el('div',{title:folder+'\\'+f.relpath},el('span',{class:'role'},f.role||'file'),el('span',{class:'fp'},f.relpath||'(no path)'),f.cloud_only?el('span',{class:'role'},'online-only'):null,f.missing?el('span',{class:'role'},'missing'):null)))));
 body.append(el('div',{class:'row2'},el('div',{class:'fld'},el('label',{},'Added'),el('input',{type:'text',readonly:true,value:fmtWhen(r.created_at)})),el('div',{class:'fld'},el('label',{},'Updated'),el('input',{type:'text',readonly:true,value:fmtWhen(r.updated_at)}))));
 body.scrollTop=top;
 const save=async()=>{const authors=D.authors.split(/\n/).map(s=>s.trim()).filter(Boolean);const out={authors:authors.length?authors:['Unknown'],categories:D.cats.length?D.cats:['Uncategorized']};EDIT_KEYS.forEach(k=>out[k]=String(D[k]??'').trim());if(!out.title)out.title=r.title;
  try{await PUT('/api/reference/'+enc(r.id),out);toast('Saved to the project database. The file itself was left untouched.');st.draft=null;st.dirty=false;st.rec=null;loadEditRec(r.id);}catch(e){fail(e);}await refreshData({hlRefs:[],ids:[r.id]});};
 R.append(el('div',{class:'rfoot'},el('span',{class:'note'},el('span',{class:'dot '+statusCls(r.status)}),' '+r.status),el('button',{class:'btn',onclick:closeRight},'Cancel'),el('button',{class:'btn pri',onclick:save},'Save')));}

/* ---------- live refresh: poll /api/changes every 2 s ---------- */
const Poll={timer:null,lost:null,
 start(){clearTimeout(this.timer);this.timer=setTimeout(()=>this.tick(),2000);},
 async tick(){if(!S.project){this.start();return;}let r;
  try{r=await GET('/api/changes?since='+S.changeSeq);}catch(e){if(!this.lost&&e.status===0){S.offline=true;renderStatus();this.lost=toast('Lost the connection to the Kurasi server. Kurasi retries every 2 seconds; start Kurasi again if this message stays.',{sticky:true,err:true});}this.start();return;}
  if(this.lost){this.lost.remove();this.lost=null;S.offline=false;renderStatus();toast('Connected to the Kurasi server again.');}
  try{const ch=Array.isArray(r&&r.changes)?r.changes:[];if(ch.length){S.changeSeq=Math.max(S.changeSeq,...ch.map(c=>+c.seq||0),+r.seq||0);await applyChanges(ch);}else if(r&&r.seq!=null&&+r.seq<S.changeSeq)S.changeSeq=+r.seq;}catch(e){console.warn(e);}
  this.start();}};
function changeIds(c){let ids=c.ids;if(typeof ids==='string'){try{ids=JSON.parse(ids);}catch(e){ids=ids.split(/[\s,]+/);}}return Array.isArray(ids)?ids.map(String).filter(Boolean):[];}
/* a settings change made in this page (the graph view saves itself after every zoom, pan or fit) or one that only
   touches graph.* keys comes back through the feed; the page holds those values already, so it redraws nothing */
function ownSettings(c){const w=String(c.what||'');if(!/^settings\b/i.test(w))return false;const keys=w.replace(/^settings( changed)?:?\s*/i,'').split(/,\s*/).filter(Boolean);return c.origin==='ui'||(keys.length>0&&keys.every(k=>k.startsWith('graph.')));}
async function applyChanges(all){const ch=all.filter(c=>!ownSettings(c));if(!ch.length)return;const mcp=ch.filter(c=>c.origin==='mcp');
 mcp.slice(0,3).forEach(c=>{const n=changeIds(c).length;toast(`${aiName()} via MCP: ${String(c.what||'a change').replace(/_/g,' ')}${n>1?` (${n} items)`:''}`,{mcp:true,ms:8000});});
 if(mcp.length>3)toast(`${aiName()} via MCP: ${mcp.length-3} more changes.`,{mcp:true});
 const whats=ch.map(c=>String(c.what||'').toLowerCase()).join(' ');const ids=new Set(ch.flatMap(changeIds));
 if(/project|setting/.test(whats)){try{await loadState();}catch(e){}}
 const touchesHl=[...ids].some(x=>x.startsWith('h-'))||/highlight|annot|scan|undo|restore/.test(whats);
 const hlRefs=S.tabs.filter(id=>{const r=refById(id);if(!r)return false;if(!ids.size||touchesHl||ids.has(id))return true;return r.files.some(f=>ids.has(f.id));});
 V.forEach(v=>{if(v.fileId&&ids.has(v.fileId)&&/rotat|file|scan|chang|annot|modif/.test(whats))v.needsReload=true;});
 /* partial refresh when every change names references; scans, imports, undo, project switches and changes without
    reference ids fetch the whole list. The kind of change is read from the start of its text, which can go on with a
    reference title. An AI log entry writes only the AI log and takes no part in the choice. */
 const whole=ch.some(c=>{const w=String(c.what||'').trim().toLowerCase();if(w.startsWith('ai log entry'))return false;
  return /^(scan|import|undo|restor|project|sample)/.test(w)||!changeIds(c).some(x=>x.startsWith('r-'));});
 await refreshData({cats:/categor|project|scan|import|undo|restore/.test(whats),hlRefs,ids:whole?null:[...ids].filter(x=>x.startsWith('r-'))});
 if(mcp.length)signal('mcp:categorized');}

/* ---------- startup ---------- */
const GRAPH_WAIT_MS=1500;
function setQuery(q){S.q=q;$('#q').value=q;$('#qclr').hidden=!q;if(!docTabRef())renderMain();}
function showOffline(e){S.offline=true;const m=$('#main');m.innerHTML='';m.append(el('div',{class:'vmsg'},el('b',{},'Kurasi cannot reach its server'),el('div',{},e&&e.message?e.message:NO_SERVER),el('div',{},'Start Kurasi again, then reload this page.'),el('div',{class:'acts'},el('button',{class:'btn pri',onclick:()=>location.reload()},'Reload'))));renderStatus();}
function bindGlobal(){const q=$('#q');let qt=null;q.addEventListener('input',()=>{$('#qclr').hidden=!q.value;clearTimeout(qt);qt=setTimeout(()=>{S.q=q.value;if(!docTabRef())renderMain();},120);});
 $('#qclr').addEventListener('click',()=>setQuery(''));$('#sortsel').addEventListener('change',e=>{S.sort=e.target.value;if(!docTabRef())renderMain();});
 /* Enter in the box or the + button beside it keeps the text as a search chip */
 const qadd=el('button',{class:'qadd',id:'qadd',type:'button',title:'Add as filter (Enter)','aria-label':'Add the search text as a filter'},'+');$('#searchbox').after(qadd);
 const keep=()=>{if(addChip())clearTimeout(qt);};
 q.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.isComposing){e.preventDefault();keep();}});qadd.addEventListener('click',()=>{keep();q.focus();});
 document.addEventListener('mousedown',e=>{if(!e.target.closest('.menu')&&!e.target.closest('.mb'))closeMenus();});
 document.addEventListener('keydown',e=>{const typing=e.target.closest&&e.target.closest('input,textarea,select,[contenteditable]');
  if(e.key==='Escape'){if(menuEls.length){closeMenus();return;}if(modalEl&&!modalEl._sticky){closeModal();return;}}
  if(e.key==='F5'){e.preventDefault();if(S.project)runScan({start:true});}
  if(e.ctrlKey&&!e.altKey&&e.key.toLowerCase()==='f'){e.preventDefault();q.focus();q.select();}
  if(e.ctrlKey&&!e.altKey&&e.key.toLowerCase()==='n'){e.preventDefault();newProjectDialog();}
  if(e.ctrlKey&&!e.altKey&&e.key.toLowerCase()==='o'){e.preventDefault();projectsDialog();}
  if(e.ctrlKey&&!e.altKey&&e.key.toLowerCase()==='z'&&!typing&&!modalEl){e.preventDefault();undoLast();}
  /* tag graph: Ctrl + / Ctrl - zoom, Ctrl 0 fits, arrows pan 40 px */
  if(S.tab==='tags'&&S.project&&!docTabRef()&&!typing&&!modalEl&&!menuEls.length){
   if(e.ctrlKey&&!e.altKey&&['+','=','-','_','0'].includes(e.key)){e.preventDefault();if(e.key==='0')Graph.fit();else Graph.zoomStep(e.key==='-'||e.key==='_'?-1:1);return;}
   const ar={ArrowLeft:[40,0],ArrowRight:[-40,0],ArrowUp:[0,40],ArrowDown:[0,-40]}[e.key];if(ar&&!e.ctrlKey&&!e.altKey){e.preventDefault();Graph.pan(ar[0],ar[1]);}}});
 window.addEventListener('contextmenu',e=>{if(!e.target.closest('input,textarea'))e.preventDefault();});
 let rt=null,lastSize='';window.addEventListener('resize',()=>{clearTimeout(rt);rt=setTimeout(()=>{const m=$('#main');const sz=m.clientWidth+'x'+m.clientHeight;if(sz!==lastSize&&!docTabRef())renderMain();lastSize=sz;if(S.tour.on)positionTour();},80);});
 document.addEventListener('scroll',()=>{if(S.tour.on)positionTour();},true);}
async function init(){applyTheme();bindGlobal();Boot.start();renderAll();Boot.say('Starting Kurasi');
 try{await loadState();}catch(e){Boot.pending.clear();showOffline(e);await Boot.ready();Poll.start();return;}
 Boot.pending.delete('project state');
 if(S.project){Boot.say(`Opening ${S.project.name}`);Boot.pending.add('references');await nextFrame();Boot.say('Loading references');
  try{await loadCatalog();}catch(e){fail(e);}Boot.pending.delete('references');Boot.pending.add('first catalogue page');renderAll();await nextFrame();Boot.pending.delete('first catalogue page');
  /* the whole-project graph layout is waited for at most GRAPH_WAIT_MS; a large library finishes it in the worker
     after the window shows, and the Tag tab draws it when it arrives */
  Boot.pending.add('tag graph');Boot.say('Building tag graph');await Promise.race([Graph.request('',S.refs).catch(e=>console.warn(e)),sleep(+window.KURASI_GRAPH_WAIT_MS||GRAPH_WAIT_MS)]);Boot.pending.delete('tag graph');
  try{const st=await GET('/api/scan/status');if(st&&st.running)S.scanBackground=true;}catch(e){}}
 else renderAll();
 await Boot.ready();Poll.start();
 if(!S.project)welcomeDialog();}
