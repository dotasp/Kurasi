'use strict';
/* Development only. Loaded by index.html when the page URL carries ?fake=1, before app.js.
   It replaces window.fetch for /api/... with an in-memory server that answers in the shapes of
   docs/CONTRACT.md section 6, using a small public-domain sample (contract section 9): page
   images and word boxes are drawn on a canvas so selection and overlays line up.
   URL switches: fresh=1 (no project yet), missing=1 (last project folder unreachable),
   demo=1 (open the sample project), mcp=1 (simulated MCP writes after a few seconds),
   slow=<ms> (delay /api/references), safety=<ms> (shorter splash safety timer),
   readonly=1 (open project is read-only), projects=<n> (n more entries in the project list, each opened with the
   small sample library on demand; the Projects dialog and its remove, restore and reset routes).
   The catalogue list leaves out search_text and the other fields only the full record carries; /api/references?ids=
   answers a partial refresh and /api/search scores the highlight text with the rules of app.js, as the real server
   does. KurasiFake.adopt() replaces the open project with a generated library (shoot.mjs bigui). */
(function(){
const Q=new URLSearchParams(location.search);
if(Q.get('safety'))window.KURASI_SAFETY_MS=+Q.get('safety');
/* py=1: a stand-in for the pywebview js_api, recording the calls in window.__py */
if(Q.get('py')==='1'){window.__py=[];window.pywebview={api:{splash_status:t=>{window.__py.push(['splash_status',t,Math.round(performance.now())]);return Promise.resolve(null);},
 app_ready:()=>{window.__py.push(['app_ready',Math.round(performance.now())]);return Promise.resolve(null);},pick_folder:()=>Promise.resolve('D:\\Papers\\Picked folder')}};}
/* tauri=1: a stand-in for the Tauri 2 global (app.withGlobalTauri), recording emits and invokes in window.__tauri */
if(Q.get('tauri')==='1'){window.__tauri=[];window.__TAURI__={event:{emit:(name,payload)=>{window.__tauri.push([name,payload,Math.round(performance.now())]);return Promise.resolve();},listen:()=>Promise.resolve(()=>{})},
 core:{invoke:(cmd,args)=>{window.__tauri.push(['invoke',cmd,args||null]);return Promise.resolve(null);}}};}
const realFetch=window.fetch.bind(window);
/* graph view settings survive a page reload within the browser session, so the restore path can be checked */
const GRAPH_DEF={'graph.zoom':1,'graph.pan_x':0,'graph.pan_y':0,'graph.font_px':12,'graph.label_opacity':100,'graph.edge_opacity':60,'graph.spacing':100};
const graphStore={get(id){try{return JSON.parse(sessionStorage.getItem('kurasi.fake.graph.'+id)||'null')||{};}catch(e){return {};}},set(id,settings){try{const o={};Object.keys(GRAPH_DEF).forEach(k=>{if(k in settings)o[k]=settings[k];});sessionStorage.setItem('kurasi.fake.graph.'+id,JSON.stringify(o));}catch(e){}}};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const now=()=>{const d=new Date(),p=n=>String(n).padStart(2,'0');return `${d.getFullYear()}-${p(d.getMonth()+1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;};
function hex12(s){let h1=0x811c9dc5,h2=0x1b873593;for(const c of String(s)){h1=Math.imul(h1^c.charCodeAt(0),16777619);h2=Math.imul(h2^c.charCodeAt(0),2246822507)^(h2>>>13);}return ((h1>>>0).toString(16).padStart(8,'0')+(h2>>>0).toString(16).padStart(8,'0')).slice(0,12);}
function rng(seed){let h=2166136261;for(const ch of String(seed)){h^=ch.charCodeAt(0);h=Math.imul(h,16777619);}return()=>{h+=0x6D2B79F5;let t=h;t=Math.imul(t^t>>>15,t|1);t^=t+Math.imul(t^t>>>7,t|61);return((t^t>>>14)>>>0)/4294967296;};}

/* ---------- sample content: public-domain classics, placeholder body text ---------- */
const POOL=[
'This page stands in for a page of the source; the title block is real metadata and the body text is placeholder.',
'The viewer renders the PDF with its text layer, so a drag across words selects the words beneath the pointer.',
'Highlights are written into the PDF as annotations and indexed in the project database, so another PDF reader shows them too.',
'The annotation author separates a human highlight from an AI highlight, and the colour on screen follows the author.',
'A note carries a verbatim quote, an analysis with a label of fact, interpretation or proposal, and a final line of tags.',
'Copies of one source share a reference: the original, the print, the translation and the excerpt open in the same tab.',
'A changed file is recognised by its hash, so a renamed or moved file keeps its highlights and its identity.',
'The knowledge pane on the right lists every highlight on this document and filters by tag and by author.',
'Zoom with Ctrl and the mouse wheel, pan with the hand tool, and right-click a page to rotate it.',
'An AI agent reaches the same database through the MCP server; its writes carry its name and wait for review.',
'A scan walks the folder recursively and skips files that live only in the cloud, so nothing is downloaded by accident.',
'Every reference starts in Uncategorized until someone labels it, by hand or through a proposal from the agent.'];
const SAMPLE_LINE='Sample file generated by Kurasi. The body text is placeholder.';
const CATS=[['Uncategorized'],['Economics'],['Management',['Scientific management','Administration']],['Sociology'],['Biology'],['Notes'],['Exhibits']];
/* hl: [page, sentence, author, label, analysis, tags, extra] for PDFs */
const REFS=[
 {rel:'economics/smith_1776_wealth-of-nations.pdf',title:'An inquiry into the nature and causes of the wealth of nations',authors:['Smith, A.'],year:'1776',date:'1776-03-09',kind:'Book',pub:'W. Strahan and T. Cadell',cat:'Economics',status:'Verified',usability:'Full text',lang:'en',pages:6,created:'2026-10-10T09:12:00',
  hl:[[2,1,'You','F','(F) The pin factory: ten workers who divide the steps make far more pins than ten who each work alone.',['division-of-labour','quote']],
      [3,4,'Claude','I','(I) The three causes of the gain (dexterity, saved time, machinery) return in [Taylor (1911)](../management/taylor_1911_principles-of-scientific-management.pdf).',['division-of-labour','efficiency']],
      [4,2,'You','P','(P) Read next to Babbage on paying only for the skill each task needs.',['method','to-read']]]},
 {rel:'economics/mill_1848_principles-of-political-economy.pdf',title:'Principles of political economy',authors:['Mill, J. S.'],year:'1848',date:'1848',kind:'Book',pub:'John W. Parker',cat:'Economics',status:'Verified',usability:'Full text',lang:'en',pages:5,created:'2026-10-10T10:02:00',
  hl:[[2,3,'You','F','(F) Distinguishes the laws of production from the laws of distribution.',['institutions']],
      [3,0,'Claude','I','(I) Distribution rests on institutions people can change, which opens the question of incentives.',['critique','incentives'],{suggested_tags:['institutions']}]]},
 {rel:'economics/veblen_1899_theory-of-the-leisure-class.pdf',title:'The theory of the leisure class',authors:['Veblen, T.'],year:'1899',date:'1899',kind:'Book',pub:'Macmillan',cat:'Economics',status:'Suggested',usability:'Full text',lang:'en',pages:4,created:'2026-10-11T08:40:00',
  hl:[[2,5,'You','F','(F) Conspicuous consumption: spending to display standing.',['quote','critique']],
      [3,1,'Claude','I','(I) Status spending works against the efficiency the engineers of the period promised.',['critique'],{suggestion:{action:'delete',reason:'It repeats the point of the highlight on page 2.',at:'2026-10-16T21:10:00',resolution:null}}]]},
 {rel:'management/taylor_1911_principles-of-scientific-management.pdf',title:'The principles of scientific management',authors:['Taylor, F. W.'],year:'1911',date:'1911',kind:'Book',pub:'Harper & Brothers',cat:'Management/Scientific management',status:'Verified',usability:'Full text',lang:'en',pages:6,created:'2026-10-12T14:00:00',
  hl:[[1,2,'You','F','(F) Each task is studied, timed and taught as the one best way.',['efficiency','method']],
      [2,4,'Claude','I','(I) The bonus for meeting the task links pay to measured output.',['efficiency','incentives']],
      [3,1,'You','P','(P) Compare the bonus with Mill on distribution.',['incentives','to-read']],
      [4,3,'Claude','F','(F) Four duties of management are listed in the second chapter.',['method','quote']]]},
 {rel:'management/gilbreth_1911_motion-study.pdf',title:'Motion study',authors:['Gilbreth, F. B.'],year:'1911',date:'1911',kind:'Book',pub:'D. Van Nostrand',cat:'Management/Scientific management',status:'Verified',usability:'Full text',lang:'en',pages:4,created:'2026-10-12T15:30:00',
  hl:[[2,1,'You','F','(F) Bricklaying motions cut from eighteen to five per brick.',['efficiency','method']],
      [3,2,'Claude','I','(I) Motion study measures the worker; time study measures the task.',['method']]]},
 {rel:'management/follett_1918_the-new-state.pdf',title:'The new state: Group organization the solution of popular government',authors:['Follett, M. P.'],year:'1918',date:'1918',kind:'Book',pub:'Longmans, Green',cat:'Management/Administration',status:'Suggested',usability:'Full text',lang:'en',pages:5,created:'2026-10-13T09:00:00',
  hl:[[2,0,'You','I','(I) Integration of interests as an alternative to compromise.',['institutions']],
      [3,3,'Claude','F','(F) The group process is described as the source of collective will.',['institutions','quote']]]},
 {rel:'management/babbage_1832_economy-of-machinery_excerpt-pp1-12_highlighted.pdf',title:'On the economy of machinery and manufactures',authors:['Babbage, C.'],year:'1832',date:'1832',kind:'Book',pub:'Charles Knight',cat:'Management',status:'Verified',usability:'Excerpt',lang:'en',pages:12,role:'excerpt',created:'2026-10-13T11:20:00',
  hl:[[2,2,'You','F','(F) The Babbage principle: split the work so each part is paid at the skill it needs.',['division-of-labour','efficiency']],
      [5,1,'Claude','I','(I) Extends Smith with a cost argument for dividing labour.',['division-of-labour']]]},
 {rel:'sociology/weber_1905_protestantische-ethik.pdf',slug:'weber_1905_protestantische-ethik',title:'Die protestantische Ethik und der Geist des Kapitalismus',authors:['Weber, M.'],year:'1905',date:'1905',kind:'Journal article',pub:'Archiv f\u00FCr Sozialwissenschaft und Sozialpolitik',vol:'20-21',cat:'Sociology',status:'Verified',usability:'Translated',lang:'de',pages:4,created:'2026-10-14T10:00:00',
  copies:[{rel:'sociology/weber_1905_protestantische-ethik.print.translated-en.pdf',role:'translation',pages:4,title:'The Protestant ethic and the spirit of capitalism (English translation)',
   hl:[[2,1,'You','F','(F) The calling as a duty pursued for its own sake. Translated from German.',['institutions','quote']],
       [3,3,'Claude','I','(I) Discipline in work spread beyond its religious origin.',['institutions','critique']]]}],hl:[]},
 {rel:'biology/darwin_1859_origin-of-species.pdf',title:'On the origin of species by means of natural selection',authors:['Darwin, C.'],year:'1859',date:'1859-11-24',kind:'Book',pub:'John Murray',cat:'Biology',status:'Verified',usability:'Full text',lang:'en',pages:24,rot:{3:90},created:'2026-10-14T16:45:00',
  hl:[[2,0,'You','F','(F) Natural selection preserves variations that help an organism survive.',['evolution','quote']],
      [4,2,'Claude','I','(I) The division of labour reappears as the division of physiological work between organs.',['evolution','division-of-labour']],
      [6,1,'You','P','(P) Check the later editions for changed wording.',['evolution','to-read']]]},
 {rel:'inbox/scan_0007.pdf',raw:true,pageTitle:'A new view of society',pageAuthors:'Robert Owen',pagePub:'London, 1813',pages:1,created:'2026-10-16T22:14:00',hl:[]},
 {rel:'inbox/2026-03-02_download.pdf',raw:true,pageTitle:'Principles of economics',pageAuthors:'Alfred Marshall',pagePub:'Macmillan, 1890',pages:1,date:'2026-03-02',created:'2026-10-16T22:15:00',hl:[]},
 {rel:'notes/reading-plan.md',title:'Reading plan',authors:['Unknown'],year:'2026',date:'2026-10-15',kind:'Working note',cat:'Notes',status:'Verified',usability:'Full text',lang:'en',created:'2026-10-15T08:00:00',
  md:`# Reading plan

## This month
- Finish [Smith (1776)](../economics/smith_1776_wealth-of-nations.pdf), Book I, chapters 1 to 3.
- Compare with [Taylor (1911)](../management/taylor_1911_principles-of-scientific-management.pdf) on the division of work.
- Look at [Exhibit 1](../exhibits/exhibit-1_division-of-labour.png) before the seminar.

## Order of reading
| Week | Source | Why |
| --- | --- | --- |
| 1 | Smith (1776) | The pin factory and the three causes of productivity |
| 2 | Babbage (1832) | Paying only for the skill each task needs |
| 3 | Taylor (1911) | Time study and the task idea |
| 4 | Follett (1918) | Groups and the integration of interests |

## Notes
Division of labour raises output when each task is small enough to learn quickly. The open questions live in [open-questions.md](open-questions.md). Public-domain copies of most titles are on [Project Gutenberg](https://www.gutenberg.org/).
`,mdhl:[['You','I','(I) The core claim the reading plan tests against each source.',['division-of-labour'],'Division of labour raises output when each task is small enough to learn quickly'],
       ['Claude','F','(F) Babbage (1832) prices each part of the work at the skill it needs.',['division-of-labour','incentives'],'Paying only for the skill each task needs']]},
 {rel:'notes/open-questions.md',title:'Open questions',authors:['Unknown'],year:'2026',date:'2026-10-16',kind:'Working note',cat:'Notes',status:'Verified',usability:'Full text',lang:'en',created:'2026-10-16T09:00:00',
  md:`# Open questions

1. Does the pin factory example hold when tasks need judgement?
2. How does Follett's integration differ from Taylor's harmony?
3. Which of these sources has a reliable English translation?

> Weber is read here through the English copy; the German title page is kept with it.

See the [reading plan](reading-plan.md).
`,mdhl:[]},
 {rel:'exhibits/exhibit-1_division-of-labour.png',title:'Exhibit 1. Division of labour, illustrative numbers',authors:['Unknown'],year:'2026',date:'2026-10-16',kind:'Exhibit',cat:'Exhibits',status:'Verified',usability:'Full text',lang:'en',created:'2026-10-16T10:00:00',
  imghl:[['You','F','(F) Output per worker rises sharply once the steps are divided. Illustrative numbers.',['division-of-labour','efficiency'],'Pins per worker per day: alone against in a team of ten',[0.06,0.2,0.56,0.62]]]},
 {rel:'exhibits/exhibit-2_reading-timeline.png',title:'Exhibit 2. Reading timeline',authors:['Unknown'],year:'2026',date:'2026-10-16',kind:'Exhibit',cat:'Exhibits',status:'Verified',usability:'Full text',lang:'en',created:'2026-10-16T10:05:00',imghl:[]}
];
const EXTRA_READING=[{rel:'archive/marshall_1890_principles-of-economics.pdf',title:'Principles of economics',authors:['Marshall, A.'],year:'1890',date:'1890',kind:'Book',pub:'Macmillan',cat:'Economics',status:'Verified',usability:'Full text',lang:'en',pages:8,cloud:true,created:'2026-10-09T08:00:00',hl:[]}];
const AI_LOG=[
 {at:'2026-10-16T22:14:00',tool:'catalog_scan',client:'claude-code',purpose:'Rescan after two files landed in the inbox',args:'{"full": false}',result:'2 new, 0 changed, 0 moved'},
 {at:'2026-10-16T22:31:00',tool:'highlight_add',client:'claude-code',purpose:'Mark the pin factory passage in Smith (1776)',args:'{"ref_id": "..."}',result:'1 highlight, author Claude'},
 {at:'2026-10-17T08:05:00',tool:'highlight_suggest_tags',client:'claude-code',purpose:'Propose tags for untagged highlights',args:'{}',result:'1 proposal pending review'},
 {at:'2026-10-17T08:07:00',tool:'highlight_propose_delete',client:'claude-code',purpose:'Flag a repeated highlight in Veblen (1899)',args:'{}',result:'1 proposal pending review'}];
const MCP_TOOLS=[['catalog_search','Loose search over metadata, tags and highlights','read'],['reference_get','Metadata, files and highlight count of one reference','read'],['reference_text','Page text plus word boxes','read'],['highlights_list','Highlights, filtered by reference, tag or author','read'],['tags_list','The tag vocabulary','read'],['categories_list','The category tree','read'],
 ['reference_update_metadata','Applies metadata as a proposal (status Suggested)','propose'],['reference_categorize','Adds categories as a proposal (status Suggested)','propose'],['highlight_propose_delete','Marks a highlight for deletion with a reason','propose'],['highlight_suggest_tags','Proposes tags for a highlight','propose'],
 ['catalog_scan','Runs a scan','write'],['highlight_add','Adds a highlight from a quote or quads; author is the AI name','write'],['export_references','APA 7 Markdown, BibTeX, CSL JSON or highlights JSON','write'],['log_append','Adds a row to the AI-use log','write']];

/* ---------- page layout on a canvas: word boxes in points ---------- */
const PG={w:612,h:792,ml:62,mt:56,mr:62,mb:64};
const mctx=document.createElement('canvas').getContext('2d');
const F={title:['700','16','Georgia'],au:['400','10.5','Georgia'],pub:['italic 400','9.5','Georgia'],sample:['400','8','Segoe UI'],body:['400','10.5','Georgia']};
const font=f=>`${f[0]} ${f[1]}px ${f[2]}`;
function pageSentences(seed,p){const rnd=rng(seed+':'+p);const n=12+Math.floor(rnd()*5);const out=[];let last=-1;for(let i=0;i<n;i++){let k;do{k=Math.floor(rnd()*POOL.length);}while(k===last);last=k;out.push(POOL[k]);}return out;}
const LAYOUT=new Map();
function layout(file,p){const key=file.id+':'+p;if(LAYOUT.has(key))return LAYOUT.get(key);const words=[],sent=new Map();let y=PG.mt;const maxX=PG.w-PG.mr;
 const flow=(text,f,lh,sIdx)=>{mctx.font=font(f);const size=+f[1];const sp=mctx.measureText(' ').width;let x=PG.ml;for(const w of String(text).split(/\s+/).filter(Boolean)){const ww=mctx.measureText(w).width;if(x+ww>maxX&&x>PG.ml){x=PG.ml;y+=lh;}if(y+size>PG.h-PG.mb)return false;const i=words.push([+(x).toFixed(2),+(y).toFixed(2),+(x+ww).toFixed(2),+(y+size*1.18).toFixed(2),w,f])-1;if(sIdx!=null){if(!sent.has(sIdx))sent.set(sIdx,[]);sent.get(sIdx).push(i);}x+=ww+sp;}y+=lh;return true;};
 if(p===1){flow(file.pageTitle,F.title,20);y+=2;if(file.pageAuthors)flow(file.pageAuthors,F.au,14);if(file.pagePub)flow(file.pagePub,F.pub,13);y+=4;flow(SAMPLE_LINE,F.sample,11);y+=12;}
 if(!(file.single&&p===1)){const ss=pageSentences(file.seed,p);const rnd=rng('para'+file.seed+p);let i=0;let ok=true;
  while(i<ss.length&&ok){const n=3+Math.floor(rnd()*3);mctx.font=font(F.body);let x=PG.ml;const lh=15.2;
   for(let j=0;j<n&&i<ss.length&&ok;j++,i++){for(const w of ss[i].split(' ')){const ww=mctx.measureText(w).width;const sp=mctx.measureText(' ').width;if(x+ww>maxX&&x>PG.ml){x=PG.ml;y+=lh;}if(y+10.5>PG.h-PG.mb){ok=false;break;}const k=words.push([+x.toFixed(2),+y.toFixed(2),+(x+ww).toFixed(2),+(y+12.4).toFixed(2),w,F.body])-1;if(!sent.has(i))sent.set(i,[]);sent.get(i).push(k);x+=ww+sp;}}
   y+=lh+9;}}
 const out={words,sent};LAYOUT.set(key,out);return out;}
function lineQuads(ws){const out=[];let cur=null;ws.forEach(w=>{if(cur&&Math.abs(w[1]-cur[1])<2){cur[2]=w[2];cur[3]=Math.max(cur[3],w[3]);}else{cur=[w[0],w[1],w[2],w[3]];out.push(cur);}});return out.map(q=>q.map(n=>+n.toFixed(2)));}
async function renderPage(file,p,scale){const L=layout(file,p);const rot=((file.rotation[p-1]||0)%360+360)%360;const W=PG.w*scale,H=PG.h*scale;const c=document.createElement('canvas');c.width=Math.round(rot%180?H:W);c.height=Math.round(rot%180?W:H);const g=c.getContext('2d');
 if(rot===90)g.setTransform(0,scale,-scale,0,c.width,0);else if(rot===180)g.setTransform(-scale,0,0,-scale,c.width,c.height);else if(rot===270)g.setTransform(0,-scale,scale,0,0,c.height);else g.setTransform(scale,0,0,scale,0,0);
 g.fillStyle='#ffffff';g.fillRect(0,0,PG.w,PG.h);g.textBaseline='top';
 for(const w of L.words){g.font=font(w[5]);g.fillStyle=w[5]===F.sample?'#8a8a8a':w[5]===F.au||w[5]===F.pub?'#555':'#1a1a1a';g.fillText(w[4],w[0],w[1]);}
 g.font='9px Georgia';g.fillStyle='#777';g.fillText(String(p),PG.w-PG.mr-8,PG.h-36);
 return new Promise(r=>c.toBlob(r,'image/png'));}
function drawExhibit(file,W=900,H=560){const c=document.createElement('canvas');c.width=W;c.height=H;const g=c.getContext('2d');g.fillStyle='#fff';g.fillRect(0,0,W,H);g.fillStyle='#1a1a1a';g.font='700 22px Segoe UI';g.fillText(file.ex===1?'Exhibit 1. Division of labour':'Exhibit 2. Reading timeline',40,48);g.font='13px Segoe UI';g.fillStyle='#555';
 if(file.ex===1){g.fillText('Pins per worker per day: alone against in a team of ten. Illustrative numbers.',40,76);const bars=[['Alone',20],['Team of ten',4800]];bars.forEach(([l,v],i)=>{const h=Math.max(6,v/4800*300);g.fillStyle=i?'#2f8f6a':'#b8560f';g.fillRect(80+i*240,440-h,160,h);g.fillStyle='#1a1a1a';g.font='13px Segoe UI';g.fillText(l,80+i*240,464);g.fillText(String(v),80+i*240,430-h);});
  g.fillStyle='#555';g.font='12px Segoe UI';g.fillText('Illustrative numbers after the pin factory example in Smith (1776).',40,530);}
 else{g.fillText('Publication years of the sources in the sample project.',40,76);const ys=[['Smith',1776],['Owen',1813],['Babbage',1832],['Mill',1848],['Darwin',1859],['Marshall',1890],['Veblen',1899],['Weber',1905],['Taylor',1911],['Follett',1918]];
  const x=y=>60+(y-1770)/(1925-1770)*780;g.strokeStyle='#888';g.lineWidth=2;g.beginPath();g.moveTo(50,300);g.lineTo(860,300);g.stroke();ys.forEach(([n,y],i)=>{g.fillStyle='#0f6cbd';g.beginPath();g.arc(x(y),300,6,0,7);g.fill();g.fillStyle='#1a1a1a';g.font='12px Segoe UI';const up=i%2===0;g.fillText(`${n} ${y}`,x(y)-24,up?270:330);});}
 return c;}
const blobOf=c=>new Promise(r=>c.toBlob(r,'image/png'));

/* ---------- in-memory projects ---------- */
const projects=new Map();let openId=null;let prevId=null;
function slugOf(rel){const name=rel.split('/').pop();let s=name.split('.')[0];s=s.replace(/_(highlighted|annotated|reading|hl)$/,'').replace(/_excerpt(-pp[\w-]+)?$/,'');return s;}
function buildProject(id,name,folder,set,demo){const P={project:{id,name,folder,read_only:false,auto_scan_min:15,last_scan:'2026-10-17T08:02:00',file_types:['.pdf','.md','.png','.jpg','.jpeg'],exclude:['.git','.kurasi','.venv','node_modules','__pycache__','~$*']},
  settings:{ai_authors:['claude'],ai_author:'Claude',hl_colour:'#ffd84a',loose:true,mcp_hints:true,theme:'system',...GRAPH_DEF,...graphStore.get(id)},demo:!!demo,cats:[],refs:new Map(),files:new Map(),hls:new Map(),trash:new Map(),changes:[],seq:40,aiLog:AI_LOG.map((r,i)=>({id:i+1,...r})).reverse(),scan:{running:false,pct:100,message:'Idle',log:[],result:null}};
 let cid=0;const addCat=(name,parent)=>{const c={id:++cid,parent_id:parent,name,sort:cid};P.cats.push(c);return c;};CATS.forEach(([n,kids])=>{const c=addCat(n,null);(kids||[]).forEach(k=>addCat(k,c.id));});
 set.forEach(d=>{const slug=d.slug||slugOf(d.rel);const rid='r-'+hex12(id+slug);const ext=d.rel.split('.').pop();
  const ref={id:rid,slug,title:d.raw?d.rel.split('/').pop().replace(/\.[^.]+$/,''):d.title,kind:d.kind||'',year:d.year||(d.date||'').slice(0,4)||(d.raw?'':''),date:d.date||'',pub:d.pub||'',vol:d.vol||'',issue:'',pages_range:'',doi:'',url:'',lang:d.lang||'',usability:d.usability||'',status:d.raw?'Auto':d.status,register_id:'',created_at:d.created,updated_at:d.created,authors:d.raw?['Unknown']:d.authors,catIds:[catByPath(P,d.raw?'Uncategorized':d.cat).id],deleted:false};
  if(d.raw&&d.date)ref.year=d.date.slice(0,4);P.refs.set(rid,ref);
  const mkFile=(rel,role,f)=>{const fid='f-'+hex12(id+rel);const file={id:fid,ref_id:rid,relpath:rel.replace(/\//g,'\\'),ext,pages:ext==='pdf'?f.pages:null,role,cloud_only:!!f.cloud,missing:false,rotation:[],view_rotation:0,seed:fid,
    pageTitle:f.pageTitle||f.title||d.pageTitle||d.title,pageAuthors:f.pageAuthors||d.pageAuthors||(d.authors||[]).join(', '),pagePub:f.pagePub||d.pagePub||[d.pub,d.year].filter(Boolean).join(', '),single:!!d.raw,md:d.md,ex:/exhibit-1/.test(rel)?1:/exhibit-2/.test(rel)?2:0};
   if(f.rot)Object.entries(f.rot).forEach(([pp,deg])=>file.rotation[+pp-1]=deg);P.files.set(fid,file);return file;};
  const main=mkFile(d.rel,d.role||'primary',d);
  const addHls=(file,list)=>list.forEach(([p,s,au,label,analysis,tags,extra])=>{const L=layout(file,p);const idx=L.sent.get(s)||L.sent.get(0)||[];const ws=idx.map(k=>L.words[k]);
   addHl(P,{file_id:file.id,ref_id:rid,page:p,kind:'text',quads:lineQuads(ws),anchor:null,verbatim:ws.map(w=>w[4]).join(' '),analysis,label,tags,author:au,...(extra||{})});});
  addHls(main,d.hl||[]);(d.copies||[]).forEach(cp=>{const f=mkFile(cp.rel,cp.role,cp);addHls(f,cp.hl||[]);});
  (d.mdhl||[]).forEach(([au,label,analysis,tags,text])=>addHl(P,{file_id:main.id,ref_id:rid,page:null,kind:'md',quads:[],anchor:{text,occurrence:0},verbatim:text,analysis,label,tags,author:au}));
  (d.imghl||[]).forEach(([au,label,analysis,tags,text,q])=>addHl(P,{file_id:main.id,ref_id:rid,page:null,kind:'region',quads:[q],anchor:null,verbatim:text,analysis,label,tags,author:au}));});
 return P;}
function catPath(P,c){const parts=[];while(c){parts.unshift(c.name);c=P.cats.find(x=>x.id===c.parent_id);}return parts.join('/');}
function catByPath(P,path){let c=P.cats.find(x=>catPath(P,x)===path);if(c)return c;let parent=null;for(const part of path.split('/')){let n=P.cats.find(x=>x.name===part&&x.parent_id===(parent?parent.id:null));if(!n){n={id:Math.max(0,...P.cats.map(x=>x.id))+1,parent_id:parent?parent.id:null,name:part,sort:99};P.cats.push(n);}parent=n;}return parent;}
let hlN=0;
function addHl(P,h){const id='h-'+hex12(P.project.id+'h'+(++hlN)+h.file_id+(h.verbatim||''));const t=h.created_at||'2026-10-16T2'+(hlN%10)+':1'+(hlN%6)+':00';
 const o={id,file_id:h.file_id,ref_id:h.ref_id,page:h.page??null,kind:h.kind||'text',quads:h.quads||[],anchor:h.anchor||null,verbatim:h.verbatim||'',analysis:h.analysis||'',label:h.label||'',tags:h.tags||[],author:h.author||'You',origin:/^claude$/i.test(h.author)?'ai':'human',created_at:t,updated_at:t,suggestion:h.suggestion||null,suggested_tags:h.suggested_tags||[]};P.hls.set(id,o);return o;}
function initProjects(){const read=buildProject('p-'+hex12('reading'),'Reading room','D:\\Papers\\Reading room',REFS.filter(d=>!d.rel.startsWith('inbox/')).concat(EXTRA_READING),false);
 const demo=buildProject('p-'+hex12('demo'),'Sample: Classics of work and organisation','C:\\Users\\you\\AppData\\Local\\Kurasi\\demo',REFS,true);
 projects.set(read.project.id,read);projects.set(demo.project.id,demo);read.project.last_opened='2026-10-17T08:00:00';demo.project.last_opened='2026-10-16T20:00:00';
 if(Q.get('readonly')==='1')read.project.read_only=true;
 if(Q.get('fresh')==='1'){projects.clear();openId=null;}else if(Q.get('missing')==='1'){openId=null;}else openId=Q.get('demo')==='1'?demo.project.id:read.project.id;}
const cur=()=>openId&&projects.get(openId);
function change(P,what,ids,origin='ui'){P.seq++;P.changes.push({seq:P.seq,at:now(),what,ids,origin});}

/* ---------- records in contract shapes ---------- */
const ROLE_ORDER=['translation','primary','print','excerpt','rendered'];
const GRP=new WeakMap();
function grp(P){const k=P.seq+':'+P.hls.size+':'+P.files.size+':'+P.refs.size;let g=GRP.get(P);if(g&&g.k===k)return g;g={k,files:new Map(),hls:new Map(),text:new Map()};
 P.files.forEach(f=>{(g.files.get(f.ref_id)||g.files.set(f.ref_id,[]).get(f.ref_id)).push(f);});P.hls.forEach(h=>{(g.hls.get(h.ref_id)||g.hls.set(h.ref_id,[]).get(h.ref_id)).push(h);});GRP.set(P,g);return g;}
function refHls(P,rid){return grp(P).hls.get(rid)||[];}
function refFiles(P,rid){return grp(P).files.get(rid)||[];}
/* the text /api/search reads, built as the server builds search_text: "verbatim analysis" per highlight, trimmed,
   one per line, at most 20,000 characters */
function refText(P,rid){const g=grp(P);let t=g.text.get(rid);if(t===undefined){t=[...refHls(P,rid).map(h=>(h.verbatim+' '+h.analysis).trim()).filter(Boolean).join('\n')].slice(0,20000).join('');g.text.set(rid,t);}return t;}
function primaryFile(P,ref){const files=[...refFiles(P,ref.id)];const n=f=>refHls(P,ref.id).filter(h=>h.file_id===f.id).length;return files.sort((a,b)=>n(b)-n(a)||ROLE_ORDER.indexOf(a.role)-ROLE_ORDER.indexOf(b.role))[0];}
const THUMBS=new Map();
function thumbUrl(f){if(f.ext==='md'||f.cloud_only)return null;if(THUMBS.has(f.id))return THUMBS.get(f.id);let c;
 if(f.ext==='pdf'){const L=layout(f,1);const s=120/PG.w;c=document.createElement('canvas');c.width=120;c.height=Math.round(PG.h*s);const g=c.getContext('2d');g.setTransform(s,0,0,s,0,0);g.fillStyle='#fff';g.fillRect(0,0,PG.w,PG.h);g.textBaseline='top';L.words.forEach(w=>{g.font=font(w[5]);g.fillStyle='#1a1a1a';g.fillText(w[4],w[0],w[1]);});}
 else{const big=drawExhibit(f);c=document.createElement('canvas');c.width=120;c.height=Math.round(120*560/900);c.getContext('2d').drawImage(big,0,0,c.width,c.height);}
 const u=c.toDataURL('image/png');THUMBS.set(f.id,u);return u;}
/* the list record (full false) has the fields of the server's catalogue list; the full record adds usability, the
   detail fields and search_text */
function refRecord(P,ref,full){const files=refFiles(P,ref.id);const pf=primaryFile(P,ref);const hls=refHls(P,ref.id);const freq=new Map();hls.forEach(h=>h.tags.forEach(t=>freq.set(t,(freq.get(t)||0)+1)));
 const o={id:ref.id,title:ref.title,authors:ref.authors,kind:ref.kind,pub:ref.pub,year:ref.year,date:ref.date,lang:ref.lang,status:ref.status,register_id:ref.register_id,
  categories:ref.catIds.map(id=>catPath(P,P.cats.find(c=>c.id===id))).filter(Boolean),ext:pf?pf.ext:'',pages:pf?pf.pages:null,hl_count:hls.length,top_tags:[...freq.entries()].sort((a,b)=>b[1]-a[1]||a[0].localeCompare(b[0])).slice(0,3).map(e=>e[0]),
  missing:files.every(f=>f.missing),files:files.map(f=>({id:f.id,relpath:f.relpath,role:f.role,ext:f.ext,pages:f.pages,missing:!!f.missing,cloud_only:!!f.cloud_only})),primary_file_id:pf?pf.id:null,thumb_url:pf?thumbUrl(pf):null,
  created_at:ref.created_at,updated_at:ref.updated_at};
 if(full)Object.assign(o,{usability:ref.usability,doi:ref.doi,url:ref.url,vol:ref.vol,issue:ref.issue,pages_range:ref.pages_range,search_text:refText(P,ref.id)});return o;}
/* /api/search: per token, 1 for a substring of the text, otherwise 0.9 times the best bigram Dice over its words when
   that reaches the threshold (0.42 loose, 0.72 strict); a reference is a hit when one token scores */
const FBG=new Map();
function fBigrams(s){let b=FBG.get(s);if(b)return b;b=new Set();for(let i=0;i<s.length-1;i++)b.add(s.slice(i,i+2));if(FBG.size>200000)FBG.clear();FBG.set(s,b);return b;}
function fDice(a,b){if(a===b)return 1;const A=fBigrams(a),B=fBigrams(b);if(!A.size||!B.size)return 0;let n=0;for(const x of A)if(B.has(x))n++;return 2*n/(A.size+B.size);}
function textSearch(P,q,loose){const tokens=String(q).toLowerCase().split(/\s+/).filter(Boolean),thr=loose?0.42:0.72,hits=[];
 if(tokens.length){const sims=tokens.map(()=>new Map());const g=grp(P);if(!g.words)g.words=new Map();
  for(const r of liveRefs(P)){const low=refText(P,r.id).toLowerCase();if(!low)continue;let words=g.words.get(r.id);
   const scores=tokens.map((t,i)=>{if(low.includes(t))return 1;if(!words)g.words.set(r.id,words=[...new Set(low.split(/[^\p{L}\p{N}-]+/u).filter(Boolean))]);
    let best=0;for(const w of words){let d=sims[i].get(w);if(d===undefined){d=fDice(t,w);sims[i].set(w,d);}if(d>best)best=d;}return best>=thr?best*0.9:0;});
   if(scores.some(x=>x>0))hits.push({id:r.id,score:scores.reduce((a,b)=>a+b,0),scores});}}
 hits.sort((a,b)=>b.score-a.score||(a.id<b.id?-1:a.id>b.id?1:0));return {query:String(q),loose,tokens,hits};}
function catTree(P){const node=c=>({id:c.id,name:c.name,path:catPath(P,c),children:P.cats.filter(x=>x.parent_id===c.id).sort((a,b)=>a.sort-b.sort).map(node)});return P.cats.filter(c=>c.parent_id==null).sort((a,b)=>a.sort-b.sort).map(node);}
const liveRefs=P=>[...P.refs.values()].filter(r=>!r.deleted);
const liveHl=P=>[...P.hls.values()].filter(h=>{const r=P.refs.get(h.ref_id);return r&&!r.deleted;});
/* ---------- the project list: entries without a catalogue behind them (projects=<n>), removals waiting for Undo ---------- */
const LIST_ONLY=new Map(),REMOVED=new Map();
function addListOnly(n){const kinds=['Thesis reading','Course notes','Literature pool','Seminar papers','Lab journal','Field reports'];
 const topics=['Soil chemistry','Coastal ecology','Urban transport','Medieval trade','Labour history','Water policy','Bridge design','Crop genetics','Public finance','Glacier survey','River sediment','Rail freight','Housing policy','Seed banks','Tidal energy','Forest fire','Grain storage','Port logistics'];
 const t0=Date.parse('2026-10-16T18:00:00');
 for(let i=0;i<n;i++){const kind=kinds[i%kinds.length],topic=topics[(i*7)%topics.length],id='p-'+hex12('list'+i);
  LIST_ONLY.set(id,{id,name:`${kind}: ${topic} ${String(i+1).padStart(3,'0')}`,folder:`D:\\Papers\\${kind.replace(/ /g,'-')}\\${topic.toLowerCase().replace(/ /g,'-')}-${i+1}`,
   created:`2026-0${1+i%9}-1${i%10}T08:00:00`,last_opened:i%23===22?null:new Date(t0-i*7.3*3600e3).toISOString().slice(0,19),demo:false,exists:i%31!==30});}}
function projectList(){const P=cur();const list=[...projects.values()].map(x=>({id:x.project.id,name:x.project.name,folder:x.project.folder,created:'2026-10-10T08:00:00',last_opened:x.project.last_opened,demo:!!x.demo,exists:true}))
  .concat([...LIST_ONLY.values()].filter(e=>!projects.has(e.id)).map(e=>({...e})));
 list.forEach(e=>{e.open=!!P&&e.id===P.project.id;});
 return list.sort((a,b)=>String(b.last_opened||'').localeCompare(String(a.last_opened||''))||String(b.created||'').localeCompare(String(a.created||'')));}
/* what "Also delete Kurasi's data" would remove, in the shape of GET /api/projects/<id> */
function projectData(e){if(e.demo)return {};return {app:{path:`C:\\Users\\you\\AppData\\Local\\Kurasi\\projects\\${e.id}`,files:['kurasi.sqlite','write.lock'],whole:true},sidecar:{path:`${e.folder}\\.kurasi`,files:['AI_USE_LOG.md','metadata.json'],whole:true}};}
function stateRec(){const P=cur();const list=projectList();
 if(Q.get('missing')==='1')list.push({id:'p-'+hex12('thesis'),name:'Thesis reading',folder:'E:\\Thesis\\Reading',created:'2026-09-01T08:00:00',last_opened:'2026-10-17T07:00:00'});
 const tags=new Set();if(P)liveHl(P).forEach(h=>h.tags.forEach(t=>tags.add(t)));const authors=new Set();if(P)liveRefs(P).forEach(r=>r.authors.forEach(a=>authors.add(a)));
 return {project:P?{...P.project,file_types:P.project.file_types,exclude:P.project.exclude}:null,projects:list,settings:P?{...P.settings,name:P.project.name,folder:P.project.folder,file_types:P.project.file_types,exclude:P.project.exclude,auto_scan_min:P.project.auto_scan_min,read_only:P.project.read_only,last_scan:P.project.last_scan}:{},
  counts:P?{references:liveRefs(P).length,authors:authors.size,tags:tags.size,highlights:liveHl(P).length}:{references:0,authors:0,tags:0,highlights:0},change_seq:P?P.seq:0,demo:!!(P&&P.demo),previous_project_id:prevId,version:'0.4.0',
  startup:{opened_last:!!P&&Q.get('fresh')!=='1',folder_missing:Q.get('missing')==='1'&&!P,missing_name:Q.get('missing')==='1'?'Thesis reading':null,missing_folder:Q.get('missing')==='1'?'E:\\Thesis\\Reading':null}};}

/* ---------- routes ---------- */
const J=(o,status=200)=>new Response(JSON.stringify(o),{status,headers:{'Content-Type':'application/json'}});
const E=(status,msg)=>J({error:msg},status);
const NOPROJ=()=>E(404,'No project is open. Open a project or create one, then try again.');
function resolveRel(P,from,href){const f=from&&P.files.get(from);const dir=f?f.relpath.split('\\').slice(0,-1):[];const parts=dir.concat(String(href).split(/[?#]/)[0].split(/[\\/]/));const out=[];parts.forEach(p=>{if(!p||p==='.')return;if(p==='..')out.pop();else out.push(p);});return out.join('\\').toLowerCase();}
function startScan(P,label){P.scan={running:true,pct:0,message:'Starting',log:[],result:null,t0:Date.now(),label};}
function scanStatus(P){const s=P.scan;if(!s.running)return s;const el_=Date.now()-s.t0;const steps=[[0,8,'Walking the folder with os.scandir; online-only files are skipped'],[300,30,`${liveRefs(P).length+3} files seen, ${liveRefs(P).length} match the file types`],[700,52,'Comparing size and modified time with the database'],[1100,70,'Hashing 2 new or changed files (SHA-256)'],[1500,86,'Reading annotations from the PDFs'],[1900,96,'Rendering 2 thumbnails'],[2300,100,'Done']];
 const reached=steps.filter(x=>el_>=x[0]);const last=reached[reached.length-1];s.pct=last[1];s.message=last[2];s.log=reached.map(x=>`${String(x[1]).padStart(3)}%  ${x[2]}`);
 if(el_>=2400){s.running=false;s.result={new:0,changed:0,moved:0,missing:0};P.project.last_scan=now();change(P,'scan',[],'scan');}return s;}
async function route(method,url,body){const p=url.pathname.replace(/^.*?\/api\//,'/api/');const P=cur();const m=(re)=>p.match(re);let x;
 if(p==='/api/state')return J(stateRec());
 if(p==='/api/mcp')return J({command:P?`claude mcp add kurasi -s user -- "D:\\PythonScripts\\Kurasi\\target\\release\\kurasi.exe" mcp --project "${P.project.folder}" --mode write`:null,tools:MCP_TOOLS.map(([name,description,mode])=>({name,description,mode})),resources:['kurasi://references','kurasi://reference/{ref_id}','kurasi://reference/{ref_id}/highlights','kurasi://tags']});
 if(p==='/api/demo'&&method==='POST'){const id='p-'+hex12('demo');if(openId!==id)prevId=openId;if(body.reset||!projects.has(id)){LAYOUT.clear();THUMBS.clear();projects.set(id,buildProject(id,'Sample: Classics of work and organisation','C:\\Users\\you\\AppData\\Local\\Kurasi\\demo',REFS,true));}openId=id;projects.get(id).project.last_opened=now();return J({ok:true,project_id:id});}
 if(p==='/api/projects'&&method==='GET')return J(projectList());
 if(p==='/api/project/check'&&method==='POST'){const f=String(body.folder||'').trim().replace(/^"|"$/g,'');const no=error=>J({ok:false,folder:f,error});
  if(!f)return no('Enter the folder to scan, for example D:\\Papers\\Thesis.');
  if(!/^([A-Za-z]:\\|\\\\)/.test(f))return no(`Enter the full path of the folder, with its drive letter, for example D:\\Papers\\${f}.`);
  if(/missing|nothing/i.test(f))return no(`The folder ${f} does not exist. Check the path, or reconnect the drive that holds it.`);
  if(/\.(pdf|md|png|jpe?g|txt|docx?)$/i.test(f))return no(`${f} is a file. Choose the folder that holds it.`);
  const clean=f.replace(/\\+$/,'');const hit=projectList().find(e=>e.folder.toLowerCase()===clean.toLowerCase());
  return J({ok:true,folder:clean,name:clean.split('\\').pop()||clean,...(hit?{project_id:hit.id,project_name:hit.name}:{})});}
 if(p==='/api/projects/restore'&&method==='POST'){const r=REMOVED.get(body.removal_id);if(!body.removal_id)return E(400,'Send the removal_id to restore.');if(!r)return E(404,"That removal is no longer in Kurasi's trash, so it cannot be undone.");
  REMOVED.delete(body.removal_id);if(r.full)projects.set(r.entry.id,r.full);else LIST_ONLY.set(r.entry.id,r.entry);return J({project:r.entry});}
 if((x=m(/^\/api\/projects\/([^/]+)\/reset$/))&&method==='POST'){const e=projectList().find(q=>q.id===x[1]);if(!e||!e.demo)return E(400,'Only the sample project can be reset. Remove takes another project off the list.');
  const id=x[1],open=openId===id,last=projects.get(id).project.last_opened;LAYOUT.clear();THUMBS.clear();const D=buildProject(id,'Sample: Classics of work and organisation','C:\\Users\\you\\AppData\\Local\\Kurasi\\demo',REFS,true);D.project.last_opened=last;projects.set(id,D);
  return J({project:{id,name:D.project.name,folder:D.project.folder},open});}
 if((x=m(/^\/api\/projects\/([^/]+)$/))){const e=projectList().find(q=>q.id===x[1]);if(!e)return E(404,`The project ${x[1]} is not in the list.`);
  if(method==='GET')return J({...e,data:projectData(e)});
  if(method==='DELETE'){if(e.demo)return E(400,'The sample project stays in the list. Reset rebuilds it from scratch.');const withData=/^(1|true)$/.test(url.searchParams.get('delete_data')||'');
   const rid='rm-'+hex12('rm'+e.id+Date.now()),closed=openId===e.id,{open,...entry}=e;REMOVED.set(rid,{entry,full:projects.get(e.id)||null});projects.delete(e.id);LIST_ONLY.delete(e.id);
   if(closed)openId=null;if(prevId===e.id)prevId=null;const d=projectData(e);
   return J({removal_id:rid,project:entry,with_data:withData,moved:withData?[d.app.path,...d.sidecar.files.map(f=>d.sidecar.path+'\\'+f)]:[],closed});}}
 if(p==='/api/project/open'&&method==='POST'&&!projects.has(body.id)&&LIST_ONLY.has(body.id)){const e=LIST_ONLY.get(body.id);
  if(e.exists===false)return E(404,`The folder ${e.folder} of the project ${e.name} is unreachable. Reconnect the drive, then open it again.`);
  const NP=buildProject(e.id,e.name,e.folder,REFS.filter(d=>!d.rel.startsWith('exhibits/')),false);NP.project.last_opened=e.last_opened;projects.set(e.id,NP);LIST_ONLY.delete(e.id);}
 if(p==='/api/project/open'&&method==='POST'){if(!projects.has(body.id))return E(404,'That project is unknown. Pick one from File, Open project.');if(openId!==body.id)prevId=openId;openId=body.id;projects.get(openId).project.last_opened=now();return J({ok:true});}
 if(p==='/api/project'&&method==='POST'){if(!body.folder)return E(400,'Enter the folder to scan.');const id='p-'+hex12(body.folder);const NP=buildProject(id,body.name||body.folder,body.folder,REFS.filter(d=>!d.rel.startsWith('exhibits/')),false);NP.project.file_types=body.file_types||NP.project.file_types;NP.project.auto_scan_min=+body.auto_scan_min||0;projects.set(id,NP);prevId=openId;openId=id;NP.project.last_opened=now();startScan(NP);return J({ok:true,project_id:id});}
 if(!P)return NOPROJ();
 if(p==='/api/references'){const d=+Q.get('slow')||0;if(d)await sleep(d);const raw=url.searchParams.get('ids');
  if(raw!=null){const want=new Set(raw.split(',').map(x=>x.trim()).filter(Boolean));return J(liveRefs(P).filter(r=>want.has(r.id)).map(r=>refRecord(P,r,false)));}
  return J(liveRefs(P).map(r=>refRecord(P,r,false)));}
 if(p==='/api/search'){const l=url.searchParams.get('loose');const loose=l==null||l===''?P.settings.loose!==false:!/^(0|false|no|off)$/i.test(l.trim());return J(textSearch(P,url.searchParams.get('q')||'',loose));}
 if((x=m(/^\/api\/reference\/([^/]+)$/))){const ref=P.refs.get(decodeURIComponent(x[1]));if(!ref||ref.deleted)return E(404,'That reference is gone. Reload the catalogue.');
  if(method==='GET')return J(refRecord(P,ref,true));
  if(method==='PUT'){['title','kind','year','date','pub','vol','issue','pages_range','doi','url','lang','usability','register_id'].forEach(k=>{if(k in body)ref[k]=String(body[k]??'');});if(Array.isArray(body.authors))ref.authors=body.authors.length?body.authors:['Unknown'];
   if(Array.isArray(body.categories))ref.catIds=(body.categories.length?body.categories:['Uncategorized']).map(c=>catByPath(P,c).id);ref.status=body.status||'Verified';ref.updated_at=now();change(P,'reference_update',[ref.id]);return J(refRecord(P,ref,true));}
  if(method==='DELETE'){ref.deleted=true;const tid=P.trash.size+1;P.trash.set(tid,{kind:'reference',id:ref.id});change(P,'reference_delete',[ref.id]);return J({trash_id:tid});}}
 if(p==='/api/undo'&&method==='POST'){const t=P.trash.get(+body.trash_id);if(!t)return E(404,'Nothing in the trash matches. It may have been restored already.');P.trash.delete(+body.trash_id);if(t.kind==='reference')P.refs.get(t.id).deleted=false;else P.hls.set(t.h.id,t.h);change(P,'undo',[t.id||t.h.id]);return J({ok:true});}
 if(p==='/api/categories'){if(method==='GET')return J(catTree(P));if(method==='POST'){const n=String(body.name||'').trim();if(!n)return E(400,'Enter a name for the category.');if(P.cats.some(c=>c.name===n&&c.parent_id===(body.parent_id??null)))return E(409,'A category with this name already exists there. Choose another name.');const c={id:Math.max(...P.cats.map(c=>c.id))+1,parent_id:body.parent_id??null,name:n,sort:99};P.cats.push(c);change(P,'category_add',[]);return J({id:c.id,name:c.name,path:catPath(P,c)});}}
 if((x=m(/^\/api\/categories\/(\d+)$/))){const c=P.cats.find(y=>y.id===+x[1]);if(!c)return E(404,'That category is gone.');if(c.id===1)return E(400,'Uncategorized cannot be renamed or deleted.');
  if(method==='PUT'){c.name=String(body.name||c.name);if('parent_id' in body)c.parent_id=body.parent_id;change(P,'category_update',[]);return J({id:c.id,name:c.name,path:catPath(P,c)});}
  if(method==='DELETE'){const sub=[];const walk=id=>{sub.push(id);P.cats.filter(y=>y.parent_id===id).forEach(y=>walk(y.id));};walk(c.id);const to=c.parent_id??1;liveRefs(P).forEach(r=>{if(r.catIds.some(i=>sub.includes(i))){r.catIds=[...new Set(r.catIds.map(i=>sub.includes(i)?to:i))];}});P.cats=P.cats.filter(y=>!sub.includes(y.id));change(P,'category_delete',[]);return J({ok:true});}}
 if(p==='/api/authors'){const m_=new Map();liveRefs(P).forEach(r=>r.authors.forEach(a=>{const o=m_.get(a)||{name:a,ref_ids:[],hl_count:0};o.ref_ids.push(r.id);o.hl_count+=refHls(P,r.id).length;m_.set(a,o);}));return J([...m_.values()].map(o=>({...o,top_ref_id:o.ref_ids[0]})));}
 if(p==='/api/tags'){const m_=new Map();liveHl(P).forEach(h=>h.tags.forEach(t=>{const o=m_.get(t)||{name:t,ref_ids:new Set(),hl_count:0};o.ref_ids.add(h.ref_id);o.hl_count++;m_.set(t,o);}));return J([...m_.values()].map(o=>({name:o.name,ref_ids:[...o.ref_ids],hl_count:o.hl_count})).sort((a,b)=>b.ref_ids.length-a.ref_ids.length));}
 if((x=m(/^\/api\/file\/([^/]+)\/(info|raw|rotate|page\/(\d+)\.png|page\/(\d+)\/words)$/))){const f=P.files.get(decodeURIComponent(x[1]));if(!f)return E(404,'That file is not in this project. Scan the folder again.');if(f.cloud_only&&x[2]!=='rotate')return E(409,'The file is online-only in OneDrive. Make it available on this device, then scan again.');
  if(x[2]==='info'){if(f.ext==='pdf')return J({ext:'pdf',pages:f.pages,page_sizes:Array.from({length:f.pages},()=>[PG.w,PG.h]),rotation:Array.from({length:f.pages},(_,i)=>f.rotation[i]||0)});if(f.ext==='md')return J({ext:'md',pages:1,page_sizes:[],rotation:[f.view_rotation]});return J({ext:f.ext,pages:1,page_sizes:[[900,560]],rotation:[f.view_rotation]});}
  if(x[2]==='raw'){if(f.ext==='md')return new Response(f.md||'',{headers:{'Content-Type':'text/markdown; charset=utf-8'}});if(f.ext==='png')return new Response(await blobOf(drawExhibit(f)),{headers:{'Content-Type':'image/png'}});return E(400,'Raw bytes are served for Markdown and images.');}
  if(x[2]==='rotate'&&method==='POST'){if(P.project.read_only&&f.ext==='pdf')return E(423,'The project is read-only, so Kurasi cannot write the rotation into the PDF. Turn off Read-only in Tools, Settings to rotate pages.');const d=+body.delta||90;
   if(f.ext==='pdf'){const pages=body.page==null?Array.from({length:f.pages},(_,i)=>i+1):[+body.page];pages.forEach(pp=>f.rotation[pp-1]=(((f.rotation[pp-1]||0)+d)%360+360)%360);}else f.view_rotation=((f.view_rotation+d)%360+360)%360;change(P,'file_rotate',[f.ref_id,f.id]);return J({ok:true});}
  if(x[3]){const pn=+x[3];if(pn<1||pn>f.pages)return E(404,'That page does not exist.');await sleep(60+Math.random()*120);const sc=Math.min(6,Math.max(0.25,+url.searchParams.get('scale')||1.5));return new Response(await renderPage(f,pn,sc),{headers:{'Content-Type':'image/png'}});}
  if(x[4]){const pn=+x[4];const L=layout(f,pn);return J({width:PG.w,height:PG.h,rotation:f.rotation[pn-1]||0,words:L.words.map(w=>w.slice(0,5))});}}
 if((x=m(/^\/api\/thumb\/([^/]+)\.png$/))){const f=P.files.get(decodeURIComponent(x[1]));const u=f&&thumbUrl(f);if(!u)return E(404,'No thumbnail for this file.');return new Response(await (await realFetch(u)).blob(),{headers:{'Content-Type':'image/png'}});}
 if(p==='/api/highlights'){if(method==='GET'){const rid=url.searchParams.get('ref_id'),fid=url.searchParams.get('file_id');return J(liveHl(P).filter(h=>(!rid||h.ref_id===rid)&&(!fid||h.file_id===fid)));}
  if(method==='POST'){const f=P.files.get(body.file_id);if(!f)return E(400,'The highlight names a file that is not in this project.');if(P.project.read_only&&f.ext==='pdf'&&body.kind!=='note')return E(423,'The project is read-only, so Kurasi cannot write the highlight into the PDF. Turn off Read-only in Tools, Settings, then save again.');
   const h=addHl(P,{...body,ref_id:f.ref_id,author:'You',created_at:now()});h.origin='human';const ref=P.refs.get(f.ref_id);ref.updated_at=now();change(P,'highlight_add',[h.id,h.ref_id]);return J(h);}}
 if((x=m(/^\/api\/highlights\/([^/]+)(?:\/(suggestion|accept-tag))?$/))){const h=P.hls.get(decodeURIComponent(x[1]));if(!h)return E(404,'That highlight is gone. Reload the document.');
  if(x[2]==='suggestion'){if(body.resolution==='accept'){P.hls.delete(h.id);const tid=P.trash.size+1;P.trash.set(tid,{kind:'highlight',h});}else h.suggestion={...h.suggestion,resolution:'dismissed'};change(P,'highlight_suggestion',[h.id,h.ref_id]);return J({ok:true});}
  if(x[2]==='accept-tag'){const t=String(body.tag||'');if(t&&!h.tags.includes(t))h.tags.push(t);h.suggested_tags=h.suggested_tags.filter(y=>y!==t);change(P,'highlight_accept_tag',[h.id,h.ref_id]);return J(h);}
  if(method==='PUT'){['analysis','label','verbatim'].forEach(k=>{if(k in body)h[k]=String(body[k]??'');});if(Array.isArray(body.tags))h.tags=body.tags;h.updated_at=now();change(P,'highlight_update',[h.id,h.ref_id]);return J(h);}
  if(method==='DELETE'){const f=P.files.get(h.file_id);if(P.project.read_only&&f&&f.ext==='pdf')return E(423,'The project is read-only, so Kurasi cannot remove the annotation from the PDF. Turn off Read-only in Tools, Settings, then delete again.');P.hls.delete(h.id);const tid=P.trash.size+1;P.trash.set(tid,{kind:'highlight',h});change(P,'highlight_delete',[h.id,h.ref_id]);return J({trash_id:tid});}}
 if(p==='/api/scan'&&method==='POST'){if(P.scan.running)return E(409,'A scan is already running.');startScan(P);return J({ok:true});}
 if(p==='/api/scan/status'){const s=scanStatus(P);return J({running:s.running,pct:s.pct,message:s.message,log:s.log,result:s.result});}
 if(p==='/api/settings'&&method==='PUT'){Object.entries(body).forEach(([k,v])=>{if(k in P.project&&k!=='id')P.project[k]=v;P.settings[k]=v;});if(Object.keys(body).some(k=>k.startsWith('graph.')))graphStore.set(P.project.id,P.settings);change(P,'settings',[]);return J({ok:true});}
 if(p==='/api/changes'){const since=+url.searchParams.get('since')||0;return J({seq:P.seq,changes:P.changes.filter(c=>c.seq>since)});}
 if(p==='/api/ai-log')return J(P.aiLog);
 if(p==='/api/ai-log/export'&&method==='POST'){if(P.project.read_only)return E(423,'The project is read-only, so Kurasi cannot write the log into its folder. Turn off Read-only in Tools, Settings first.');return J({path:P.project.folder+'\\.kurasi\\AI_USE_LOG.md'});}
 if(p==='/api/export'&&method==='POST'){if(P.project.read_only)return E(423,'The project is read-only, so Kurasi cannot write the export into its folder. Turn off Read-only in Tools, Settings first.');const name={'apa-md':'references_apa.md','bibtex':'references.bib','csl-json':'references.csl.json','highlights-json':'highlights.json'}[body.format]||'export.txt';return J({path:P.project.folder+'\\_exports\\'+name});}
 if(p==='/api/open'&&method==='POST')return J({ok:true});
 if(p==='/api/resolve'){const href=url.searchParams.get('href')||'',from=url.searchParams.get('from');if(/^[a-z][a-z0-9+.-]*:/i.test(href))return J({external:true});const want=resolveRel(P,from,href);const f=[...P.files.values()].find(y=>y.relpath.toLowerCase()===want);return J(f?{file_id:f.id,ref_id:f.ref_id}:{external:true});}
 return E(404,`The fake API has no route for ${method} ${p}.`);}
window.fetch=async function(input,init){const u=typeof input==='string'?input:input.url;const url=new URL(u,location.href);
 if(!/\/api\//.test(url.pathname)||url.protocol==='data:'||url.protocol==='blob:')return realFetch(input,init);
 const method=(init&&init.method)||(typeof input!=='string'&&input.method)||'GET';let body={};try{if(init&&init.body)body=JSON.parse(init.body);}catch(e){}
 await sleep(20+Math.random()*40);try{return await route(method.toUpperCase(),url,body);}catch(e){console.error(e);return E(500,'The fake API failed: '+e.message);}};
initProjects();
if(+Q.get('projects')>0)addListOnly(Math.min(5000,Math.round(+Q.get('projects'))));
/* simulated MCP writes, so the toast and the live refresh can be checked */
if(Q.get('mcp')==='1'){setTimeout(()=>{const P=cur();if(!P)return;const h=liveHl(P).find(x=>x.origin==='human');if(h){h.suggested_tags=['method'];change(P,'highlight_suggest_tags',[h.id,h.ref_id],'mcp');}},5000);
 setTimeout(()=>{const P=cur();if(!P)return;const f=[...P.files.values()].find(x=>x.ext==='pdf'&&/taylor/.test(x.relpath));if(!f)return;const L=layout(f,2);const idx=L.sent.get(6)||[];const ws=idx.map(k=>L.words[k]);
  const h=addHl(P,{file_id:f.id,ref_id:f.ref_id,page:2,kind:'text',quads:lineQuads(ws),verbatim:ws.map(w=>w[4]).join(' '),analysis:'(I) Added through MCP during the session.',label:'I',tags:['method'],author:'Claude'});change(P,'highlight_add',[h.id,h.ref_id],'mcp');},8000);}
/* replaces the open project's references with generated ones: one Markdown file each and one highlight carrying the
   reference's tags and its text as the verbatim. refs are list records (id, title, authors, categories, text, tags). */
function adopt(refs){const P=cur();if(!P)return 0;P.refs.clear();P.files.clear();P.hls.clear();P.trash.clear();const t='2026-10-16T09:00:00';
 refs.forEach((r,i)=>{const cats=(r.categories&&r.categories.length?r.categories:['Uncategorized']).map(c=>catByPath(P,c).id);
  P.refs.set(r.id,{id:r.id,title:r.title,authors:r.authors,kind:'article',pub:'',year:'2001',date:'',lang:'en',usability:'Full',status:'Verified',register_id:'',catIds:cats,doi:'',url:'',vol:'',issue:'',pages_range:'',created_at:t,updated_at:t,deleted:false});
  const fid='f-'+hex12('big'+r.id);P.files.set(fid,{id:fid,ref_id:r.id,relpath:'synthetic\\'+r.id+'.md',role:'primary',ext:'md',pages:1,rotation:[],view_rotation:0,md:'',missing:false,cloud_only:false});
  if((r.tags&&r.tags.length)||r.text){const hid='h-'+hex12('bighl'+r.id);P.hls.set(hid,{id:hid,file_id:fid,ref_id:r.id,page:null,kind:'text',quads:[],anchor:null,verbatim:r.text||'',analysis:'',label:'',tags:r.tags||[],author:'You',origin:'human',created_at:t,updated_at:t,suggestion:null,suggested_tags:[]});}});
 change(P,'scan',[],'scan');return P.refs.size;}
window.KurasiFake={projects,layout,cur,adopt,textSearch};
})();
