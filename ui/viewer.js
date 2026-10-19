'use strict';
/* Document viewer (PDF pages, Markdown, images), highlight overlays, knowledge pane and the
   highlight dialog. PDF pages come from /api/file/<fid>/page/<n>.png, word boxes from
   /words, quads are unrotated page points; image quads are [x, y, w, h] fractions of the
   unrotated image; Markdown highlights carry an anchor {text, occurrence}. */

/* rotation from /info: one value per page, or a single value for every page */
const rotList=(info,n)=>{const r=info&&info.rotation;return Array.isArray(r)?r:Array.from({length:Math.max(1,n||1)},()=>+r||0);};
const normRot=d=>(((Math.round((+d||0)/90)*90)%360)+360)%360;
function vmsg(title,text,...acts){return el('div',{class:'vmsg'},el('b',{},title),text?el('div',{},text):null,acts.length?el('div',{class:'acts'},...acts):null);}
function docHls(r){const v=V.get(r.id);const f=v?{id:v.fileId}:docFile(r);const fid=f&&f.id;return hlOf(r.id).filter(h=>!h.file_id||!fid||h.file_id===fid);}
function sortHls(list){const y=h=>(h.quads[0]||[0,0])[1];return [...list].sort((a,b)=>(a.page??0)-(b.page??0)||y(a)-y(b)||String(a.created_at||'').localeCompare(String(b.created_at||'')));}

/* ---------- viewer construction ---------- */
function getViewer(r){const f=docFile(r);let v=V.get(r.id);if(v&&v.fileId===(f?f.id:null))return v;if(v)disposeViewer(r.id);
 const ext=String((f&&f.ext)||r.ext||'').toLowerCase();
 v={id:r.id,fileId:f?f.id:null,file:f,ext,kind:ext==='pdf'?'pdf':ext==='md'?'md':isImgExt(ext)?'img':'other',z:1,tool:'select',mdRaw:false,sel:null,pages:[],ver:0,io:null};v.avail=!!(f&&!f.cloud_only&&!f.missing&&v.kind!=='other');
 v.el=buildViewer(r,v);v.el._viewer=v;V.set(r.id,v);return v;}
function disposeViewer(id){const v=V.get(id);if(!v)return;if(v.io)v.io.disconnect();v.pages.forEach(pg=>{if(pg.url)URL.revokeObjectURL(pg.url);});if(v.imgUrl)URL.revokeObjectURL(v.imgUrl);V.delete(id);}
function buildViewer(r,v){const tb=el('div',{class:'vtb'});v.tb=tb;const sep=()=>el('span',{class:'vsep'});
 if(v.avail){[['select','Select','Click a highlight to select it'],['hl','Highlight',v.kind==='md'?'Select text with the mouse, then describe it':'Drag across words for a text highlight, or from an empty spot for a region'],['hand','Hand','Drag to pan']].forEach(([k,l,t])=>tb.append(el('button',{class:'tool','data-tool':k,title:t,onclick:()=>setTool(v,k)},l)));
 tb.append(sep(),el('button',{class:'tool',title:'Zoom out (Ctrl + wheel)',onclick:()=>zoomBy(v,1/1.15)},'−'),el('span',{class:'pg','data-z':''},'100%'),el('button',{class:'tool',title:'Zoom in (Ctrl + wheel)',onclick:()=>zoomBy(v,1.15)},'+'),el('button',{class:'tool',title:'Zoom so the page fills the width',onclick:()=>fitWidth(v)},'Fit width'),sep());
 if(v.kind==='pdf')tb.append(el('span',{class:'pg','data-pg':''},`1 / ${(v.file&&v.file.pages)||r.pages||'?'}`),sep());
 if(v.kind==='pdf')tb.append(el('button',{class:'tool',title:'Rotate every page 90 degrees and save the rotation into the PDF (right-click a page to rotate one page)',onclick:()=>rotate(v,null)},'↻ Rotate'));
 if(v.kind==='img')tb.append(el('button',{class:'tool',title:'Rotate the view 90 degrees; the image file stays as it is',onclick:()=>rotate(v,null)},'↻ Rotate'));
 if(v.kind==='md')tb.append(el('button',{class:'tool','data-raw':'',title:'Show the Markdown source',onclick:()=>{v.mdRaw=!v.mdRaw;renderMdContent(r,v);updateTb(v);}},'Raw Markdown'));}
 if(r.files.length>1){const s=el('select',{class:'tool',title:'Copies of this reference: original, print, translation, excerpt'},...r.files.map(f=>el('option',{value:f.id,selected:f.id===v.fileId?true:null},`${f.role||'file'}: ${baseName(f.relpath)||f.id}`)));s.addEventListener('change',()=>openDoc(r.id,s.value));tb.append(sep(),s);}
 tb.append(el('span',{class:'sp'}),el('span',{class:'muted','data-hint':''},''));
 const sc=el('div',{class:'vscroll'});v.sc=sc;const f=v.file;
 const openBtn=label=>el('button',{class:'btn',onclick:()=>POST('/api/open',{file_id:f.id,mode:'reveal'}).catch(fail)},label);
 if(!f)sc.append(vmsg('This reference has no file','The scan found no file for it. Scan the folder again, or delete the reference.'));
 else if(f.cloud_only)sc.append(vmsg('This file is online-only in OneDrive','Kurasi does not open it, so OneDrive does not download it. To read it here, right-click it in File Explorer, choose Always keep on this device, then scan again.',openBtn('Reveal in File Explorer')));
 else if(f.missing)sc.append(vmsg('The file is missing from the project folder',`Kurasi last saw it at ${f.relpath||'its old path'}. Its highlights are kept. Put the file back, or move it inside the project folder, then scan again.`));
 else if(v.kind==='pdf')buildPdf(r,v);else if(v.kind==='md')sc.append(buildMd(r,v));else if(v.kind==='img')sc.append(buildImg(r,v));
 else sc.append(vmsg('Kurasi shows PDF, Markdown and image files','Open this file with its Windows app.',el('button',{class:'btn',onclick:()=>POST('/api/open',{file_id:f.id,mode:'default'}).catch(fail)},'Open with the Windows default app')));
 sc.addEventListener('wheel',e=>{if(e.ctrlKey){e.preventDefault();zoomBy(v,e.deltaY<0?1.1:1/1.1,e.clientX,e.clientY);}},{passive:false});
 sc.addEventListener('mousedown',e=>{if(v.tool!=='hand'||e.button!==0)return;e.preventDefault();const x0=e.clientX,y0=e.clientY,l0=sc.scrollLeft,t0=sc.scrollTop;const mv=ev=>{sc.scrollLeft=l0-(ev.clientX-x0);sc.scrollTop=t0-(ev.clientY-y0);};const up=()=>{document.removeEventListener('mousemove',mv);document.removeEventListener('mouseup',up);};document.addEventListener('mousemove',mv);document.addEventListener('mouseup',up);});
 let raf=0;sc.addEventListener('scroll',()=>{if(!raf)raf=requestAnimationFrame(()=>{raf=0;updateTb(v);});});
 requestAnimationFrame(()=>updateTb(v));return el('div',{class:'viewer'},tb,sc);}
function setTool(v,k){v.tool=k;v.pages.forEach(pg=>pg.box.classList.toggle('hlmode',k==='hl'));if(v.imgBox)v.imgBox.classList.toggle('hlmode',k==='hl');v.sc.classList.toggle('hand',k==='hand');if(v.md)v.md.classList.toggle('hlmode',k==='hl');updateTb(v);
 const r=refById(v.id);if(k==='hl'&&r)mcpHint('hl',`${aiName()} can add highlights with analysis through MCP; they are drawn in blue with the author ${aiName()}.`,'highlight_add',()=>Prompt.highlights(r));}
function updateTb(v){$$('.tool[data-tool]',v.tb).forEach(b=>b.classList.toggle('on',b.dataset.tool===v.tool));const z=$('[data-z]',v.tb);if(z)z.textContent=Math.round(v.z*100)+'%';const raw=$('[data-raw]',v.tb);if(raw)raw.classList.toggle('on',v.mdRaw);
 const hint=$('[data-hint]',v.tb);if(hint)hint.textContent=!v.avail?'':v.tool==='hl'?(v.kind==='md'?'Select text with the mouse to highlight it':v.kind==='img'?'Drag a region':'Drag across words, or from an empty spot for a region'):v.tool==='hand'?'Drag to pan':v.kind==='pdf'?'Right-click a page to rotate it':v.kind==='img'?'Right-click the image to rotate it':'';
 const pg=$('[data-pg]',v.tb);if(pg&&v.pages.length){const top=v.sc.scrollTop+v.sc.clientHeight*0.4;let lo=0,hi=v.pages.length-1;while(lo<hi){const mid=(lo+hi)>>1;const b=v.pages[mid].box;if(b.offsetTop+b.offsetHeight>top)hi=mid;else lo=mid+1;}pg.textContent=`${lo+1} / ${v.pages.length}`;}}
function zoomBy(v,f,ax,ay){setZoom(v,v.z*f,ax,ay);}
function setZoom(v,z,ax,ay){z=Math.min(4,Math.max(0.25,z));const sc=v.sc,old=v.z;if(Math.abs(z-old)<1e-4)return;const r=sc.getBoundingClientRect();const mx=ax==null?sc.clientWidth/2:ax-r.left,my=ay==null?sc.clientHeight/2:ay-r.top;const cx=sc.scrollLeft+mx,cy=sc.scrollTop+my;
 v.z=z;layoutPages(v);const k=z/old;sc.scrollLeft=cx*k-mx;sc.scrollTop=cy*k-my;updateTb(v);}
function baseWidth(v){if(v.kind==='pdf'&&v.pages.length)return Math.max(...v.pages.slice(0,20).map(pg=>pg.rot%180?pg.h:pg.w));if(v.kind==='img'&&v.iw)return v.rot%180?v.ih:v.iw;if(v.kind==='md')return 780;return 0;}
function fitWidth(v){const w=baseWidth(v);if(!w)return;setZoom(v,(v.sc.clientWidth-48)/w);}
function layoutPages(v){if(v.kind==='pdf'){v.pages.forEach(pg=>sizePage(v,pg));clearTimeout(v.rt);v.rt=setTimeout(()=>v.pages.forEach(pg=>{if(pg.visible)ensurePage(v,pg);}),180);}else if(v.kind==='img')sizeImg(v);else if(v.kind==='md'&&v.md)v.md.style.zoom=v.z;}
/* the layer holds unrotated content (word boxes, highlights) and is turned to match the rendered page */
function rotTransform(rot,W,H){return rot===90?`translate(${H}px,0) rotate(90deg)`:rot===180?`translate(${W}px,${H}px) rotate(180deg)`:rot===270?`translate(0,${W}px) rotate(270deg)`:'none';}
function sizeBox(box,layer,w,h,rot,z){const W=w*z,H=h*z;box.style.width=(rot%180?H:W)+'px';box.style.height=(rot%180?W:H)+'px';layer.style.width=W+'px';layer.style.height=H+'px';layer.style.transform=rotTransform(rot,W,H);}
/* client point to unrotated page units */
function toUnrot(box,w,h,rot,z,cx,cy){const b=box.getBoundingClientRect();const dx=(cx-b.left)/z,dy=(cy-b.top)/z;return rot===90?[dy,h-dx]:rot===180?[w-dx,h-dy]:rot===270?[w-dy,dx]:[dx,dy];}

/* ---------- PDF ---------- */
function buildPdf(r,v){const pages=el('div',{class:'pages'});v.pagesEl=pages;v.sc.append(pages);const note=el('div',{class:'pageend'},'Opening the document');pages.append(note);
 GET(`/api/file/${enc(v.fileId)}/info`).then(info=>{note.remove();setupPdf(r,v,info||{});}).catch(e=>{note.remove();pages.append(vmsg('Kurasi could not open this PDF',e.message,el('button',{class:'btn',onclick:()=>POST('/api/open',{file_id:v.fileId,mode:'default'}).catch(fail)},'Open with the Windows default app')));});}
function setupPdf(r,v,info){const sizes=Array.isArray(info.page_sizes)?info.page_sizes:[];const n=+info.pages||sizes.length||0;const rots=rotList(info,n);
 if(!n){v.pagesEl.append(vmsg('This PDF has no pages','The file may be damaged. Open it with another PDF reader to check.'));return;}
 for(let i=1;i<=n;i++){const sz=sizes[i-1]||sizes[sizes.length-1]||[612,792];const pg={n:i,w:+sz[0]||612,h:+sz[1]||792,rot:normRot(rots[i-1]),loaded:0,busy:0,url:null,words:null,visible:false};
  pg.img=el('img',{alt:'',hidden:true});pg.ld=el('div',{class:'pload'},`Page ${i}`);pg.paper=el('div',{class:'ppage'},pg.img,pg.ld);
  pg.hls=el('div',{class:'hls'});pg.wordsEl=el('div',{class:'words'});pg.layer=el('div',{class:'layer'},pg.hls,pg.wordsEl);
  pg.box=el('div',{class:'pagebox'+(v.tool==='hl'?' hlmode':''),'data-p':i},pg.paper,pg.layer);pg.box._pg=pg;v.pagesEl.append(pg.box);v.pages.push(pg);}
 const avail=v.sc.clientWidth-48;const bw=baseWidth(v);v.z=avail>100&&bw?Math.max(0.4,Math.min(1,avail/bw)):1;
 v.pages.forEach(pg=>sizePage(v,pg));
 v.io=new IntersectionObserver(es=>es.forEach(en=>{const pg=en.target._pg;if(!pg)return;if(en.isIntersecting){pg.visible=true;ensurePage(v,pg);}else unloadPage(pg);}),{root:v.sc,rootMargin:'1200px 0px'});
 v.pages.forEach(pg=>v.io.observe(pg.box));
 v.pagesEl.addEventListener('mousedown',e=>pdfDown(r,v,e));
 v.pagesEl.addEventListener('contextmenu',e=>{const hd=e.target.closest('.hl[data-h]');const box=e.target.closest('.pagebox');if(!box)return;e.preventDefault();
  if(hd){const h=hlOf(r.id).find(x=>x.id===hd.dataset.h);if(h){hlMenu(r,v,h,e.clientX,e.clientY);return;}}const pg=box._pg;const p=toUnrot(box,pg.w,pg.h,pg.rot,v.z,e.clientX,e.clientY);const h=hlAt(r,v,pg,p);if(h){hlMenu(r,v,h,e.clientX,e.clientY);return;}pageMenu(r,v,pg.n,e.clientX,e.clientY);});
 renderHls(r,v);updateTb(v);if(S.tour.on)requestAnimationFrame(positionTour);}
const sizePage=(v,pg)=>sizeBox(pg.box,pg.layer,pg.w,pg.h,pg.rot,v.z);
function scaleFor(v){const s=v.z*(window.devicePixelRatio||1);return Math.min(6,Math.max(0.5,Math.ceil(s*4)/4));}
function ensurePage(v,pg){if(!V.has(v.id))return;
 if(!pg.words&&!pg.wordsBusy){pg.wordsBusy=true;GET(`/api/file/${enc(v.fileId)}/page/${pg.n}/words`).then(w=>{pg.words=normWords(w,pg);pg.wordsBusy=false;if(pg.visible)drawWords(pg);}).catch(()=>{pg.wordsBusy=false;pg.words={w:pg.w,h:pg.h,list:[]};});}
 else if(pg.words&&!pg.wordsDrawn)drawWords(pg);
 const s=scaleFor(v);if(pg.loaded===s&&pg.ver===v.ver||pg.busy===s&&pg.reqVer===v.ver)return;pg.busy=s;const ver=v.ver;pg.reqVer=ver;const tok=pg.tok=(pg.tok||0)+1;
 apiBlob(`/api/file/${enc(v.fileId)}/page/${pg.n}.png?scale=${s}&v=${ver}r${pg.rot}`).then(b=>{if(pg.tok!==tok)return;pg.busy=0;if(v.ver!==ver||!V.has(v.id)||!pg.visible)return;
  const url=URL.createObjectURL(b);const old=pg.url;pg.url=url;pg.img.onload=()=>{if(old&&old!==pg.url)URL.revokeObjectURL(old);};pg.img.src=url;pg.img.hidden=false;pg.ld.hidden=true;pg.loaded=s;pg.ver=ver;})
 .catch(e=>{if(pg.tok!==tok)return;pg.busy=0;pg.ld.hidden=false;pg.ld.textContent=`Page ${pg.n} could not be drawn. ${e.message}`;});}
function unloadPage(pg){pg.visible=false;pg.tok=(pg.tok||0)+1;if(pg.url){pg.img.removeAttribute('src');URL.revokeObjectURL(pg.url);pg.url=null;}pg.img.hidden=true;pg.ld.hidden=false;pg.loaded=0;pg.busy=0;if(pg.wordsDrawn){pg.wordsEl.innerHTML='';pg.wordsDrawn=false;}}
function normWords(w,pg){const list=(w&&Array.isArray(w.words)?w.words:[]).filter(x=>Array.isArray(x)&&x.length>=5).map(x=>[+x[0],+x[1],+x[2],+x[3],String(x[4])]);return {w:+(w&&w.width)||pg.w,h:+(w&&w.height)||pg.h,list};}
function drawWords(pg){const W=pg.words;pg.wordsEl.innerHTML='';const frag=document.createDocumentFragment();W.list.forEach((x,i)=>{const s=document.createElement('span');s.style.cssText=`left:${x[0]/W.w*100}%;top:${x[1]/W.h*100}%;width:${(x[2]-x[0])/W.w*100}%;height:${(x[3]-x[1])/W.h*100}%`;s.dataset.i=i;frag.append(s);});pg.wordsEl.append(frag);pg.wordsDrawn=true;}
/* word under a point; with nearest, the closest word on the same line */
function wordAt(list,p,nearest){const t=1.5;for(let i=0;i<list.length;i++){const w=list[i];if(p[0]>=w[0]-t&&p[0]<=w[2]+t&&p[1]>=w[1]-t&&p[1]<=w[3]+t)return i;}
 if(!nearest)return -1;let best=-1,bd=Infinity;list.forEach((w,i)=>{if(p[1]<w[1]-t||p[1]>w[3]+t)return;const d=p[0]<w[0]?w[0]-p[0]:p[0]>w[2]?p[0]-w[2]:0;if(d<bd){bd=d;best=i;}});return best;}
/* one quad per line, as in the PDF annotation */
function lineBoxes(ws){const out=[];let cur=null;ws.forEach(w=>{const cy=(w[1]+w[3])/2;if(cur&&cy>=cur[1]&&cy<=cur[3]&&w[0]>=cur[2]-2){cur[2]=Math.max(cur[2],w[2]);cur[1]=Math.min(cur[1],w[1]);cur[3]=Math.max(cur[3],w[3]);}else{cur=[w[0],w[1],w[2],w[3]];out.push(cur);}});return out.map(q=>q.map(n=>Math.round(n*100)/100));}
/* the scan rule: a word belongs to a region when it overlaps by half its height and 30% of its width */
function inRegion(w,q){const oh=Math.min(w[3],q[3])-Math.max(w[1],q[1]),ow=Math.min(w[2],q[2])-Math.max(w[0],q[0]);return oh>=0.5*(w[3]-w[1])&&ow>=0.3*(w[2]-w[0]);}
function hlAt(r,v,pg,p){return docHls(r).find(h=>h.page===pg.n&&h.quads.some(q=>p[0]>=q[0]&&p[0]<=q[2]&&p[1]>=q[1]&&p[1]<=q[3]))||null;}
const quadCss=(q,w,h)=>`left:${q[0]/w*100}%;top:${q[1]/h*100}%;width:${(q[2]-q[0])/w*100}%;height:${(q[3]-q[1])/h*100}%`;
function pdfDown(r,v,e){if(e.button!==0||v.tool!=='hl')return;const box=e.target.closest('.pagebox');if(!box||!box._pg)return;e.preventDefault();const pg=box._pg;
 const W=pg.words?pg.words.list:[];const pt=(cx,cy)=>toUnrot(box,pg.w,pg.h,pg.rot,v.z,cx,cy);const p0=pt(e.clientX,e.clientY);const i0=wordAt(W,p0,false);const x0=e.clientX,y0=e.clientY;
 let moved=false,i1=i0,rub=null,tmp=[];
 const drawTmp=qs=>{tmp.forEach(t=>t.remove());tmp=qs.map(q=>{const d=el('div',{class:'hl tmp',style:quadCss(q,pg.w,pg.h)});pg.hls.append(d);return d;});};
 const mv=ev=>{if(!moved&&Math.abs(ev.clientX-x0)+Math.abs(ev.clientY-y0)<4)return;moved=true;
  if(i0>=0){const j=wordAt(W,pt(ev.clientX,ev.clientY),true);if(j>=0)i1=j;drawTmp(lineBoxes(W.slice(Math.min(i0,i1),Math.max(i0,i1)+1)));}
  else{if(!rub){rub=el('div',{class:'rub'});box.append(rub);}const b=box.getBoundingClientRect();Object.assign(rub.style,{left:(Math.min(x0,ev.clientX)-b.left)+'px',top:(Math.min(y0,ev.clientY)-b.top)+'px',width:Math.abs(ev.clientX-x0)+'px',height:Math.abs(ev.clientY-y0)+'px'});}};
 const up=ev=>{document.removeEventListener('mousemove',mv);document.removeEventListener('mouseup',up);tmp.forEach(t=>t.remove());if(rub)rub.remove();
  if(!moved){const h=hlAt(r,v,pg,p0);if(h)selectHl(r,v,h);return;}
  if(i0>=0){const ws=W.slice(Math.min(i0,i1),Math.max(i0,i1)+1);hlDialog(r,{file_id:v.fileId,page:pg.n,kind:'text',quads:lineBoxes(ws),verbatim:ws.map(w=>w[4]).join(' ')});return;}
  const p1=pt(ev.clientX,ev.clientY);const c=(x,m)=>Math.max(0,Math.min(m,x));const q=[c(Math.min(p0[0],p1[0]),pg.w),c(Math.min(p0[1],p1[1]),pg.h),c(Math.max(p0[0],p1[0]),pg.w),c(Math.max(p0[1],p1[1]),pg.h)].map(n=>Math.round(n*100)/100);
  if(q[2]-q[0]<4||q[3]-q[1]<4)return;hlDialog(r,{file_id:v.fileId,page:pg.n,kind:'region',quads:[q],verbatim:W.filter(w=>inRegion(w,q)).map(w=>w[4]).join(' ')});};
 document.addEventListener('mousemove',mv);document.addEventListener('mouseup',up);}
async function rotate(v,page){if(v.kind!=='pdf'&&v.kind!=='img')return;try{await POST(`/api/file/${enc(v.fileId)}/rotate`,{page:page==null?null:page,delta:90});}catch(e){fail(e);return;}
 try{await reloadFileInfo(v);}catch(e){fail(e);}toast(v.kind==='pdf'?(page==null?'Every page rotated 90 degrees and saved into the PDF.':`Page ${page} rotated 90 degrees and saved into the PDF.`):'View rotated 90 degrees. The image file stays as it is.');}
async function reloadFileInfo(v){const info=await GET(`/api/file/${enc(v.fileId)}/info`)||{};const rots=rotList(info,v.pages.length);
 if(v.kind==='pdf'){v.ver++;v.pages.forEach((pg,i)=>{pg.rot=normRot(rots[i]);pg.loaded=0;pg.busy=0;sizePage(v,pg);if(pg.visible)ensurePage(v,pg);});}
 else if(v.kind==='img'){v.rot=normRot(rots[0]);sizeImg(v);}updateTb(v);}
function viewerRefresh(v){const r=refById(v.id);if(!r)return;if(v.needsReload){v.needsReload=false;if(v.kind==='pdf'||v.kind==='img')reloadFileInfo(v).catch(()=>{});else if(v.kind==='md'&&v.fileId){apiText(`/api/file/${enc(v.fileId)}/raw`).then(t=>{v.mdSrc=t;renderMdContent(r,v);}).catch(()=>{});return;}}renderHls(r,v);}

/* ---------- image ---------- */
function buildImg(r,v){const pages=el('div',{class:'pages'});const img=el('img',{alt:''});const hls=el('div',{class:'hls'});const layer=el('div',{class:'layer ilayer'},img,hls);
 const box=el('div',{class:'pagebox','data-p':1},el('div',{class:'ppage'}),layer);pages.append(box);Object.assign(v,{imgBox:box,imgLayer:layer,imgEl:img,imgHls:hls,rot:0,iw:0,ih:0});
 const note=el('div',{class:'pageend'},'Opening the image');pages.append(note);
 Promise.all([GET(`/api/file/${enc(v.fileId)}/info`).catch(()=>null),apiBlob(`/api/file/${enc(v.fileId)}/raw`)]).then(([info,blob])=>{v.rot=normRot(rotList(info,1)[0]);const url=URL.createObjectURL(blob);v.imgUrl=url;
  img.onload=()=>{note.remove();v.iw=img.naturalWidth||1;v.ih=img.naturalHeight||1;const avail=v.sc.clientWidth-48;const bw=baseWidth(v);v.z=avail>100&&bw?Math.max(0.1,Math.min(1,avail/bw)):1;sizeImg(v);renderHls(r,v);updateTb(v);if(S.tour.on)requestAnimationFrame(positionTour);};
  img.onerror=()=>{note.textContent='The image could not be decoded. Open it with its Windows app to check the file.';};img.src=url;})
 .catch(e=>{note.remove();pages.append(vmsg('Kurasi could not open this image',e.message));});
 box.addEventListener('mousedown',e=>imgDown(r,v,e));
 box.addEventListener('contextmenu',e=>{e.preventDefault();const hd=e.target.closest('.hl[data-h]');const h=hd?hlOf(r.id).find(x=>x.id===hd.dataset.h):imgHlAt(r,v,e.clientX,e.clientY);if(h)hlMenu(r,v,h,e.clientX,e.clientY);else pageMenu(r,v,1,e.clientX,e.clientY);});
 return pages;}
function sizeImg(v){if(!v.iw)return;sizeBox(v.imgBox,v.imgLayer,v.iw,v.ih,v.rot,v.z);}
function imgPt(v,cx,cy){const p=toUnrot(v.imgBox,v.iw,v.ih,v.rot,v.z,cx,cy);return [p[0]/v.iw,p[1]/v.ih];}
function imgHlAt(r,v,cx,cy){if(!v.iw)return null;const p=imgPt(v,cx,cy);return docHls(r).find(h=>h.quads.some(q=>p[0]>=q[0]&&p[0]<=q[0]+q[2]&&p[1]>=q[1]&&p[1]<=q[1]+q[3]))||null;}
function imgDown(r,v,e){if(e.button!==0||v.tool!=='hl'||!v.iw)return;e.preventDefault();const box=v.imgBox;const x0=e.clientX,y0=e.clientY;let rub=null;
 const mv=ev=>{if(!rub){rub=el('div',{class:'rub'});box.append(rub);}const b=box.getBoundingClientRect();Object.assign(rub.style,{left:(Math.min(x0,ev.clientX)-b.left)+'px',top:(Math.min(y0,ev.clientY)-b.top)+'px',width:Math.abs(ev.clientX-x0)+'px',height:Math.abs(ev.clientY-y0)+'px'});};
 const up=ev=>{document.removeEventListener('mousemove',mv);document.removeEventListener('mouseup',up);if(rub)rub.remove();
  if(Math.abs(ev.clientX-x0)<6||Math.abs(ev.clientY-y0)<6){const h=imgHlAt(r,v,ev.clientX,ev.clientY);if(h)selectHl(r,v,h);return;}
  const a=imgPt(v,x0,y0),b=imgPt(v,ev.clientX,ev.clientY);const c=x=>Math.max(0,Math.min(1,x));const x=c(Math.min(a[0],b[0])),y=c(Math.min(a[1],b[1]));const q=[x,y,c(Math.max(a[0],b[0]))-x,c(Math.max(a[1],b[1]))-y].map(n=>Math.round(n*10000)/10000);
  hlDialog(r,{file_id:v.fileId,page:null,kind:'region',quads:[q],verbatim:''});};
 document.addEventListener('mousemove',mv);document.addEventListener('mouseup',up);}

/* ---------- highlight overlays ---------- */
function renderHls(r,v){if(v.kind==='md'){renderMdContent(r,v);return;}const list=docHls(r);
 const mk=(h,style)=>el('div',{class:'hl'+(h.origin==='ai'?' ai':'')+(v.sel===h.id?' sel':''),'data-h':h.id,title:h.verbatim||h.analysis.slice(0,120)||'highlight',style,onclick:e=>{e.stopPropagation();selectHl(r,v,h);}});
 if(v.kind==='pdf')v.pages.forEach(pg=>{pg.hls.innerHTML='';list.filter(h=>h.page===pg.n).forEach(h=>h.quads.forEach(q=>pg.hls.append(mk(h,quadCss(q,pg.w,pg.h)))));});
 else if(v.kind==='img'&&v.imgHls){v.imgHls.innerHTML='';list.forEach(h=>h.quads.forEach(q=>v.imgHls.append(mk(h,`left:${q[0]*100}%;top:${q[1]*100}%;width:${q[2]*100}%;height:${q[3]*100}%`))));}}
function selectHl(r,v,h,fromPane){v.sel=h.id;$$('.hl[data-h], .kitem[data-h], mark[data-h]').forEach(x=>x.classList.toggle('sel',x.dataset.h===h.id));
 if(fromPane){if(v.kind==='pdf'&&h.page&&v.pages[h.page-1]&&!$(`.hl[data-h="${h.id}"]`,v.el))v.pages[h.page-1].box.scrollIntoView({block:'center',behavior:'smooth'});const t=$(`.hl[data-h="${h.id}"], mark[data-h="${h.id}"]`,v.el);if(t)t.scrollIntoView({block:'center',behavior:'smooth'});}
 else{const k=$(`.kitem[data-h="${h.id}"]`);if(k){if(!S.kopen.has(h.id)){S.kopen.add(h.id);renderRight();}const k2=$(`.kitem[data-h="${h.id}"]`);if(k2)k2.scrollIntoView({block:'nearest',behavior:'smooth'});}}}
function pageMenu(r,v,p,x,y){const pdf=v.kind==='pdf';showMenu([{label:pdf?'Rotate this page 90°':'Rotate 90°',title:pdf?'Saved into the PDF':'View only; the image file stays as it is',action:()=>rotate(v,pdf?p:null)},pdf?{label:'Rotate all pages 90°',title:'Saved into the PDF',action:()=>rotate(v,null)}:null,{sep:true},
 {label:'Highlight tool',checked:v.tool==='hl',action:()=>setTool(v,v.tool==='hl'?'select':'hl')},{label:'Fit width',action:()=>fitWidth(v)},{sep:true},
 {label:'Open with the Windows default app',action:()=>POST('/api/open',{file_id:v.fileId,mode:'default'}).catch(fail)},{label:'Reveal in File Explorer',action:()=>POST('/api/open',{file_id:v.fileId,mode:'reveal'}).catch(fail)}],x,y);}
function hlMenu(r,v,h,x,y){showMenu([{label:'Show in knowledge pane',action:()=>selectHl(r,v,h)},{label:'Edit',action:()=>{S.kedit.add(h.id);S.kopen.add(h.id);renderRight();selectHl(r,v,h);}},{label:'Copy verbatim text',disabled:!h.verbatim,action:()=>copyText(h.verbatim)},{label:'Copy highlight ID',action:()=>copyText(h.id)},{sep:true},{label:'Delete highlight',danger:true,action:()=>deleteHl(r,h)}],x,y);}
function hlIndex(r,h){return sortHls(docHls(r)).indexOf(h)+1;}
function deleteHl(r,h){const pdf=(docFile(r)||{}).ext==='pdf';confirmDialog('Delete highlight',`Remove highlight #${hlIndex(r,h)||''} and its analysis? ${pdf?'The annotation is removed from the PDF and kept in the trash; Edit, Undo restores it.':'It moves to the trash; Edit, Undo restores it.'}`,'Delete',async()=>{
 try{const res=await DEL('/api/highlights/'+enc(h.id));pushUndo(res&&res.trash_id,'highlight',r.id);S.kedit.delete(h.id);S.kdraft.delete(h.id);toast('Highlight deleted.',{action:res&&res.trash_id!=null?{label:'Undo',fn:undoLast}:null});}catch(e){fail(e);}await refreshData({hlRefs:[r.id]});});}
async function saveHl(r,h,patch,msg){const body={analysis:h.analysis,label:h.label||'',tags:h.tags,verbatim:h.verbatim,...patch};try{await PUT('/api/highlights/'+enc(h.id),body);if(msg)toast(msg);await refreshData({hlRefs:[r.id]});return true;}catch(e){fail(e);await refreshData({hlRefs:[r.id]});return false;}}

/* ---------- Markdown: built-in renderer (everything escaped) ---------- */
function inline(s){s=esc(s);s=s.replace(/`([^`]+)`/g,'<code>$1</code>');s=s.replace(/\*\*([^*]+)\*\*/g,'<b>$1</b>');s=s.replace(/(^|[^*\w])\*([^*\n]+)\*/g,'$1<i>$2</i>');s=s.replace(/(^|[^\w])_([^_\n]+)_(?!\w)/g,'$1<i>$2</i>');
 s=s.replace(/!?\[([^\]]+)\]\(([^)\s]+)(?:\s+&quot;[^)]*&quot;)?\)/g,(m,t,u)=>`<a href="#" data-href="${u}" title="${u}">${t}</a>`);s=s.replace(/(^|\s)(#[a-z][\w-]*)/g,'$1<span class="tagi">$2</span>');return s;}
function md(src){const lines=String(src).split(/\r?\n/);const out=[];let i=0,list=null,para=[];const flushP=()=>{if(para.length){out.push('<p>'+inline(para.join(' '))+'</p>');para=[];}};const closeL=()=>{if(list){out.push(`</${list}>`);list=null;}};
 while(i<lines.length){const L=lines[i];
  if(/^\s*```/.test(L)){flushP();closeL();let j=i+1;const code=[];while(j<lines.length&&!/^\s*```/.test(lines[j])){code.push(lines[j]);j++;}out.push('<pre><code>'+esc(code.join('\n'))+'</code></pre>');i=j+1;continue;}
  const h=L.match(/^(#{1,6})\s+(.*)$/);if(h){flushP();closeL();out.push(`<h${h[1].length}>${inline(h[2])}</h${h[1].length}>`);i++;continue;}
  if(/^\s*>/.test(L)){flushP();closeL();const q=[];while(i<lines.length&&/^\s*>/.test(lines[i])){q.push(lines[i].replace(/^\s*>\s?/,''));i++;}out.push('<blockquote>'+md(q.join('\n'))+'</blockquote>');continue;}
  if(/^\s*[-*+]\s+/.test(L)){flushP();if(list!=='ul'){closeL();out.push('<ul>');list='ul';}out.push('<li>'+inline(L.replace(/^\s*[-*+]\s+(\[[ xX]\]\s+)?/,''))+'</li>');i++;continue;}
  if(/^\s*\d+[.)]\s+/.test(L)){flushP();if(list!=='ol'){closeL();out.push('<ol>');list='ol';}out.push('<li>'+inline(L.replace(/^\s*\d+[.)]\s+/,''))+'</li>');i++;continue;}
  if(/^\s*\|/.test(L)){flushP();closeL();const rows=[];while(i<lines.length&&/^\s*\|/.test(lines[i])){rows.push(lines[i]);i++;}const body=rows.filter(r=>!/^\s*\|[\s:|-]+\|?\s*$/.test(r));const cells=r=>r.trim().replace(/^\|/,'').replace(/\|$/,'').split('|').map(c=>inline(c.trim()));out.push('<table>'+body.map((r,k)=>'<tr>'+cells(r).map(c=>k===0?`<th>${c}</th>`:`<td>${c}</td>`).join('')+'</tr>').join('')+'</table>');continue;}
  if(/^\s*(---+|\*\*\*+)\s*$/.test(L)){flushP();closeL();out.push('<hr>');i++;continue;}
  if(/^\s*$/.test(L)){flushP();closeL();i++;continue;}
  if(list&&/^\s{2,}\S/.test(L)){out[out.length-1]=out[out.length-1].replace(/<\/li>$/,' '+inline(L.trim())+'</li>');i++;continue;}
  para.push(L.trim());i++;}
 flushP();closeL();return out.join('\n');}
function buildMd(r,v){const wrap=el('div',{class:'mdwrap'+(v.tool==='hl'?' hlmode':'')});v.md=wrap;wrap.append(el('div',{class:'muted'},'Opening the note'));
 apiText(`/api/file/${enc(v.fileId)}/raw`).then(t=>{v.mdSrc=t;renderMdContent(r,v);layoutPages(v);if(S.tour.on)requestAnimationFrame(positionTour);}).catch(e=>{wrap.innerHTML='';wrap.append(vmsg('Kurasi could not open this note',e.message));});
 wrap.addEventListener('mouseup',()=>{if(v.tool!=='hl'||v.mdRaw||v.mdSrc==null)return;const a=mdSelection(wrap);if(!a)return;getSelection().removeAllRanges();hlDialog(r,{file_id:v.fileId,page:null,kind:'md',quads:[],anchor:a,verbatim:a.text});});
 wrap.addEventListener('click',e=>{const a=e.target.closest('a[data-href]');if(a){e.preventDefault();if(v.tool!=='hl'||!getSelection().toString())openLink(a.dataset.href,v.fileId);return;}const m=e.target.closest('mark[data-h]');if(m){const h=hlOf(r.id).find(x=>x.id===m.dataset.h);if(h)selectHl(r,v,h);}});
 wrap.addEventListener('contextmenu',e=>{const m=e.target.closest('mark[data-h]');if(m){e.preventDefault();const h=hlOf(r.id).find(x=>x.id===m.dataset.h);if(h)hlMenu(r,v,h,e.clientX,e.clientY);}});return wrap;}
function renderMdContent(r,v){if(!v.md||v.mdSrc==null)return;v.md.classList.toggle('hlmode',v.tool==='hl');if(v.mdRaw){v.md.innerHTML='';v.md.append(el('pre',{class:'raw'},v.mdSrc));return;}
 v.md.innerHTML=md(v.mdSrc);docHls(r).filter(h=>h.kind!=='note'&&(h.anchor&&h.anchor.text||h.verbatim)).forEach(h=>markAnchor(v.md,h,v));}
/* find the anchor text (the nth exact match, else a whitespace-tolerant match) and wrap it in <mark> */
function markAnchor(root,h,v){const a=h.anchor&&h.anchor.text?h.anchor:{text:h.verbatim,occurrence:0};const text=String(a.text||'');if(!text)return false;const full=root.textContent;
 const hits=[];let i=full.indexOf(text);while(i>=0){hits.push(i);i=full.indexOf(text,i+1);}let s=-1,e=-1;
 if(hits.length){s=hits[Math.min(Math.max(0,+a.occurrence||0),hits.length-1)];e=s+text.length;}
 else{const map=[];let norm='';for(let k=0;k<full.length;k++){const ch=full[k];if(/\s/.test(ch)){if(norm.endsWith(' ')||!norm.length)continue;norm+=' ';}else norm+=ch;map.push(k);}const t=text.replace(/\s+/g,' ').trim();const j=norm.indexOf(t);if(j<0)return false;s=map[j];e=map[j+t.length-1]+1;}
 const attrs={'data-h':h.id,class:(h.origin==='ai'?'ai':'')+(v.sel===h.id?' sel':''),title:'Highlight by '+h.author};const tw=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);let pos=0,n;const parts=[];
 while((n=tw.nextNode())){const len=n.nodeValue.length;const a0=pos,b0=pos+len;pos=b0;if(b0<=s||a0>=e)continue;if(!n.nodeValue.trim())continue;parts.push([n,Math.max(s,a0)-a0,Math.min(e,b0)-a0]);}
 parts.forEach(([node,x,y])=>{let mid=node;if(x>0)mid=node.splitText(x);if(y-x<mid.nodeValue.length)mid.splitText(y-x);const m=el('mark',attrs);mid.parentNode.insertBefore(m,mid);m.append(mid);});return parts.length>0;}
function mdSelection(root){const sel=getSelection();if(!sel.rangeCount||sel.isCollapsed)return null;const rg=sel.getRangeAt(0);if(!root.contains(rg.commonAncestorContainer))return null;
 const pre=document.createRange();pre.setStart(root,0);pre.setEnd(rg.startContainer,rg.startOffset);let s=pre.toString().length;let text=rg.toString();s+=text.length-text.trimStart().length;text=text.trim();if(text.length<2)return null;
 const full=root.textContent;let occ=0,i=full.indexOf(text);while(i>=0&&i<s){occ++;i=full.indexOf(text,i+1);}return {text,occurrence:occ};}
function fileRel(fid){for(const r of S.refs){const f=r.files.find(x=>x.id===fid);if(f)return f.relpath||'';}return '';}
function joinRel(dir,href){const parts=(dir?dir.split(/[\\/]/):[]).concat(String(href).split(/[?#]/)[0].split(/[\\/]/));const out=[];parts.forEach(p=>{if(!p||p==='.')return;if(p==='..')out.pop();else out.push(p);});return out.join('\\');}
async function openLink(href,from){href=String(href||'').trim();if(!href||href.startsWith('#'))return;
 try{const res=await GET(`/api/resolve?href=${enc(href)}${from?'&from='+enc(from):''}`);if(res&&res.ref_id&&refById(res.ref_id)){openDoc(res.ref_id,res.file_id||null);return;}}catch(e){console.warn(e);}
 const scheme=(href.match(/^([a-z][a-z0-9+.-]*):/i)||[])[1];
 try{if(scheme){if(!/^(https?|mailto)$/i.test(scheme)){toast(`Kurasi opens web links, mail links and project files. It does not open ${scheme}: links.`,{err:true});return;}await POST('/api/open',{url:href,mode:'default'});toast('Opened with the Windows default app: '+href);}
  else{const dir=fileRel(from).split(/[\\/]/).slice(0,-1).join('\\');const rel=joinRel(dir,decodeURIComponent(href));await POST('/api/open',{relpath:rel,mode:'default'});toast('Opened with the Windows default app: '+rel);}}catch(e){fail(e);}}

/* ---------- knowledge pane (right, 30%) ---------- */
function kfMatch(h){if(S.kf.tags.size&&!h.tags.some(t=>S.kf.tags.has(t)))return false;if(S.kf.authors.size&&!S.kf.authors.has(h.author))return false;return true;}
function renderKnowledge(R,r){const prev=R.querySelector('.rbody');const top=prev&&R.dataset.ref===r.id?prev.scrollTop:0;R.innerHTML='';R.dataset.ref=r.id;const v=V.get(r.id);const f=docFile(r);
 const all=sortHls(docHls(r));const items=all.filter(kfMatch);const other=hlOf(r.id).length-all.length;
 R.append(el('div',{class:'rhead'},'Knowledge',el('span',{class:'cnt'},`${items.length} of ${all.length}`),el('span',{class:'sp'}),el('button',{class:'btn sm',title:'A note on the whole document, without a region',disabled:f&&(!v||v.avail)?null:true,onclick:()=>hlDialog(r,{file_id:f.id,page:null,kind:'note',quads:[],verbatim:''})},'+ Note')));
 const body=el('div',{class:'rbody'});
 if(!S.hl.has(r.id))body.append(el('div',{class:'empty'},'Loading highlights'));
 else if(!items.length)body.append(el('div',{class:'empty'},all.length?'No highlight matches the tag filter on the left.':v&&!v.avail?'This file cannot be opened here, so it takes no new highlights.':v&&v.kind==='md'?'No highlights yet. Choose Highlight in the toolbar and select text with the mouse.':'No highlights yet. Choose Highlight in the toolbar and drag across the page.'));
 items.forEach(h=>body.append(kItem(r,v,h,all.indexOf(h)+1,all.length)));
 if(other>0)body.append(el('div',{class:'kmore'},`${plural(other,'more highlight')} on other copies of this reference. Switch the copy in the document toolbar.`));
 R.append(body);body.scrollTop=top;
 R.append(el('div',{class:'rfoot'},el('span',{class:'note'},f&&f.ext==='pdf'?'Highlights live in the file; the database indexes them.':'Highlights on this file live in the project database.'),el('button',{class:'btn sm',title:'Write the highlights of this reference to a JSON file in the _exports folder',onclick:async()=>{try{const res=await POST('/api/export',{format:'highlights-json',ref_ids:[r.id]});toast(`Written: ${res&&res.path||'the _exports folder'}.`,{ms:9000});}catch(e){fail(e);}}},'Export')));}
function tagSuggest(r,have){const freq=new Map();hlOf(r.id).forEach(h=>h.tags.forEach(t=>freq.set(t,(freq.get(t)||0)+3)));S.tagRows.forEach(t=>freq.set(t.name,(freq.get(t.name)||0)+t.ref_ids.length/100));
 return [...freq.entries()].sort((a,b)=>b[1]-a[1]).map(e=>e[0]).filter(t=>!have.has(t));}
function tagEditor(tags,sugg,onChange){const box=el('div',{class:'ktags'});const listId='taglist-all';
 const draw=()=>{box.innerHTML='';tags.forEach(t=>box.append(el('span',{class:'tagc'},'#'+t,el('b',{title:'Remove',onclick:()=>{tags.delete(t);onChange();draw();}},'✕'))));
  sugg(tags).slice(0,3).forEach(t=>box.append(el('span',{class:'tagc sug',title:'Used elsewhere in this project; click to add',onclick:()=>{tags.add(t);onChange();draw();}},'+ #'+t)));
  const inp=el('input',{list:listId,placeholder:'add tag, Enter','aria-label':'Add a tag'});inp.addEventListener('keydown',e=>{if(e.key==='Enter'&&inp.value.trim()){e.preventDefault();inp.value.split(/[\s,]+/).map(x=>x.replace(/^#/,'').toLowerCase()).filter(Boolean).forEach(x=>tags.add(x));onChange();draw();box.querySelector('input').focus();}});box.append(inp);};
 if(!document.getElementById(listId))document.body.append(el('datalist',{id:listId}));const dl=document.getElementById(listId);dl.innerHTML='';S.tagRows.forEach(t=>dl.append(el('option',{value:t.name})));draw();return box;}
function kItem(r,v,h,idx,total){const ai=h.origin==='ai';const open=S.kopen.has(h.id)||total<=3||idx===1;const editing=S.kedit.has(h.id);
 const it=el('div',{class:'kitem'+(ai?' ai':'')+(v&&v.sel===h.id?' sel':''),'data-h':h.id});
 const lab=el('select',{title:'Label: F fact, I interpretation, P proposal'},...['','F','I','P'].map(x=>el('option',{value:x,selected:(h.label||'')===x?true:null},x||'-')));lab.addEventListener('change',()=>saveHl(r,h,{label:lab.value},'Label saved.'));
 const where=h.page?`p. ${h.page}`:h.kind==='region'?'region':h.kind==='md'?'text':h.kind==='note'?'note':h.kind;
 it.append(el('div',{class:'khead',onclick:e=>{if(e.target.closest('select,button'))return;if(v)selectHl(r,v,h,true);}},el('span',{class:'n'},'#'+idx),el('span',{},where),el('span',{class:'who'+(ai?' ai':''),title:h.author},ai?`${h.author} via MCP`:h.author),el('span',{class:'sp'}),lab,el('button',{class:'btn sm ico',title:'Delete highlight',onclick:()=>deleteHl(r,h)},'✕')));
 const sg=h.suggestion;if(sg&&sg.action==='delete'&&!sg.resolution)it.append(el('div',{class:'ksug'},el('span',{},`${aiName()} via MCP proposes to delete this highlight${sg.reason?': '+sg.reason:'.'}`),el('div',{class:'acts'},
  el('button',{class:'btn sm',onclick:()=>resolveSugg(r,h,'dismiss')},'Keep it'),el('button',{class:'btn sm danger',onclick:()=>resolveSugg(r,h,'accept')},'Delete highlight'))));
 const sec=(title,content)=>{const s=el('div',{class:'ksec'+(open||editing?'':' closed')});s.append(el('div',{class:'sh',onclick:()=>{s.classList.toggle('closed');S.kopen.add(h.id);}},el('span',{class:'chev'},'▼'),title),el('div',{class:'sc'},content));return s;};
 if(!editing){if(h.verbatim)it.append(sec('Verbatim',el('div',{class:'kq'},h.verbatim)));
  const ana=el('div',{class:'ka',html:md(h.analysis||'')});ana.addEventListener('click',e=>{const a=e.target.closest('a[data-href]');if(a){e.preventDefault();openLink(a.dataset.href,h.file_id||(v&&v.fileId));}});
  const tags=el('div',{class:'ktags'},...h.tags.map(t=>el('span',{class:'tagc'},'#'+t)),...h.suggested_tags.map(t=>el('span',{class:'tagc sug ai',title:`Proposed by ${aiName()} via MCP; click to accept`,onclick:async()=>{try{await POST(`/api/highlights/${enc(h.id)}/accept-tag`,{tag:t});toast(`Tag #${t} accepted.`);}catch(e){fail(e);}await refreshData({hlRefs:[r.id]});}},'+ #'+t)));
  if(!h.tags.length&&!h.suggested_tags.length)tags.append(el('span',{class:'muted',style:'font-size:12px'},'No tags'));
  it.append(sec('Analysis',ana),sec('Tags',tags),el('div',{class:'kfoot'},el('button',{class:'btn sm',title:'Copy the quote with an APA 7 in-text citation',onclick:()=>copyText(`${h.verbatim?'"'+h.verbatim+'" ':''}(${r.authors[0]==='Unknown'?words(r.title,6):r.authors[0].split(',')[0]}, ${r.year||'n.d.'}${h.page?', p. '+h.page:''})`)},'Copy citation'),el('button',{class:'btn sm',onclick:()=>{S.kedit.add(h.id);S.kopen.add(h.id);renderRight();}},'Edit')));return it;}
 /* inline edit: verbatim, analysis, tags with suggestions, Save; the draft survives refreshes */
 let d=S.kdraft.get(h.id);if(!d){d={verbatim:h.verbatim,analysis:h.analysis,tags:new Set(h.tags)};S.kdraft.set(h.id,d);}
 const vb=el('textarea',{class:'kverb','aria-label':'Verbatim text'},d.verbatim);vb.addEventListener('input',()=>{d.verbatim=vb.value;});
 const ta=el('textarea',{'aria-label':'Analysis'},d.analysis);ta.addEventListener('input',()=>{d.analysis=ta.value;});
 const tagBox=tagEditor(d.tags,have=>[...h.suggested_tags.filter(t=>!have.has(t)),...tagSuggest(r,have)],()=>{});
 if(h.kind!=='note'||h.verbatim)it.append(sec('Verbatim',vb));
 it.append(sec('Analysis (Markdown)',el('div',{class:'ka'},ta)),sec('Tags',tagBox),el('div',{class:'kfoot'},el('button',{class:'btn sm',onclick:()=>{S.kedit.delete(h.id);S.kdraft.delete(h.id);renderRight();}},'Cancel'),
  el('button',{class:'btn sm pri',onclick:async()=>{const ok=await saveHl(r,h,{analysis:d.analysis.trim(),verbatim:d.verbatim.trim(),tags:[...d.tags]},(docFile(r)||{}).ext==='pdf'?'Saved to the annotation note and the database.':'Saved to the project database.');if(ok){S.kedit.delete(h.id);S.kdraft.delete(h.id);renderRight();}}},'Save')));return it;}
async function resolveSugg(r,h,resolution){try{await POST(`/api/highlights/${enc(h.id)}/suggestion`,{resolution});toast(resolution==='accept'?'Highlight deleted as proposed.':'Proposal dismissed. The highlight stays.');}catch(e){fail(e);}await refreshData({hlRefs:[r.id]});}

/* ---------- highlight dialog ---------- */
function hlDialog(r,draft){const verb=el('textarea',{id:'hd-verb',style:'min-height:70px'},draft.verbatim||'');
 const ana=el('textarea',{id:'hd-ana',style:'min-height:110px',placeholder:'Analysis in Markdown. Start with the label: (F) fact, (I) interpretation, (P) proposal. A link to a project file opens it in a new tab.'},draft.analysis||'');
 const lab=el('select',{id:'hd-lab',class:'sel',style:'width:100%'},...[['F','F: documented fact'],['I','I: interpretation'],['P','P: proposal']].map(([x,l])=>el('option',{value:x,selected:(draft.label||'I')===x?true:null},l)));
 const tags=new Set(draft.tags||[]);const tagBox=tagEditor(tags,have=>tagSuggest(r,have),()=>{});
 const body=el('div',{});const pdf=(docFile(r)||{}).ext==='pdf';
 if(hintsOn()&&draft.kind!=='note')body.append(el('div',{class:'mcphint'},el('span',{class:'t',html:`<code>highlight_add</code>: ${esc(aiName())} can add this highlight with a drafted analysis; it appears in blue for your review.`}),el('button',{class:'btn sm',onclick:()=>copyPrompt(Prompt.highlight(r,{page:draft.page,verbatim:verb.value}))},'Copy prompt for Claude')));
 body.append(el('div',{class:'fld'},el('label',{for:'hd-verb'},draft.kind==='note'?'Quote (optional)':draft.kind==='region'&&!pdf?'Verbatim text (type what the region shows)':'Verbatim text (prefilled from the text layer; correct it if the layer is wrong)'),verb),
  el('div',{class:'fld'},el('label',{for:'hd-ana'},'Analysis'),ana),el('div',{class:'row2'},el('div',{class:'fld'},el('label',{for:'hd-lab'},'Label'),lab),el('div',{class:'fld'},el('label',{},'Tags'),tagBox)));
 const title=draft.kind==='note'?'New note':draft.kind==='region'?'New region highlight':'New highlight';
 openModal({title,width:640,body,buttons:[{left:(draft.page?`Page ${draft.page} · `:'')+'author You'},{label:'Cancel'},{label:'Save',primary:true,action:async()=>{
  const out={file_id:draft.file_id,page:draft.page??null,kind:draft.kind||'text',quads:draft.quads||[],verbatim:verb.value.trim(),analysis:ana.value.trim(),label:lab.value,tags:[...tags]};if(draft.anchor)out.anchor=draft.anchor;
  if(!out.analysis&&!out.verbatim){toast('Write an analysis or a quote before saving.',{err:true});return false;}
  try{const res=await POST('/api/highlights',out);if(res&&res.id)S.kopen.add(res.id);toast(out.kind==='note'?'Note saved.':pdf?'Highlight written into the PDF annotation layer and indexed.':'Highlight saved to the project database.');}catch(e){fail(e);return false;}
  await refreshData({hlRefs:[r.id]});}}]});}
