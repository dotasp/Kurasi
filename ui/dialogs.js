'use strict';
/* Dialogs: welcome, projects (the Projects dialog, New project, Remove), scan, settings, MCP server, categories, about, shortcuts, and
   the sample project entries of the Help menu. */

function welcomeDialog(){const st=S.startup||{};const body=el('div',{});
 if(st.folder_missing)body.append(el('div',{class:'warnline'},`The last project, ${st.missing_name||'(unnamed)'}, is unreachable at ${st.missing_folder||'its folder'}. Reconnect the drive that holds it, or open another folder.`));
 body.append(el('p',{},'Kurasi scans a folder, catalogues the files like a shop, and lets you highlight PDFs, Markdown notes and images with an analysis next to each highlight. An MCP server lets an AI agent do the same work under your review.'),
  el('div',{class:'choice'},
   el('button',{onclick:()=>{closeModal(true);startDemo({reset:false,tour:true});}},el('b',{},'Guided tour'),el('span',{},`${STEPS.length} steps on a sample project of public-domain classics: create a project, label new files, highlight a PDF, a Markdown note and an image, and read the Tag graph. When an MCP tool can do a step, the tour says so.`)),
   el('button',{onclick:()=>{closeModal(true);newProjectDialog();}},el('b',{},'Open a folder'),el('span',{},'Create a project from a folder of PDFs, Markdown notes and images. The first scan indexes every file and the highlights already in it.'))));
 const recent=[...S.projects].sort((a,b)=>String(b.last_opened||'').localeCompare(String(a.last_opened||''))).slice(0,5);
 if(recent.length)body.append(el('h4',{},'Recent projects'),el('div',{class:'plist'},...recent.map(p=>el('button',{onclick:()=>{closeModal(true);openProject(p.id);}},el('b',{},p.name||baseName(p.folder)||p.folder),el('span',{class:'pf'},p.folder||''),p.last_opened?el('span',{class:'pm'},'Last opened '+fmtWhen(p.last_opened)):null))),
  S.projects.length>recent.length?el('button',{class:'btn sm',id:'wl-all',onclick:()=>projectsDialog()},`All ${S.projects.length} projects…`):null);
 openModal({title:'Welcome to Kurasi',body,width:600,sticky:true});modalEl._kind='welcome';}

/* ---------- sample project (Help menu and the welcome dialog) ---------- */
async function startDemo({reset=false,tour=false}={}){closeMenus();const prev=S.project&&!S.demo?S.project.id:null;
 const t=toast(reset?'Rebuilding the sample project':'Opening the sample project',{sticky:true});
 try{await POST('/api/demo',{reset});}catch(e){t.remove();fail(e);return false;}t.remove();
 await switchedProject({attachScan:!tour});
 if(tour){const back=prev||(S.previousProjectId&&(!S.project||S.previousProjectId!==S.project.id)?S.previousProjectId:null);S.tourReturn=back||null;startTour();}
 else toast(reset?'The sample project was rebuilt from scratch. Your other projects were left untouched.':'Sample project open. File, Recent switches back to your projects.');return true;}
function resetDemoDialog(){closeMenus();confirmDialog('Reset the sample project','This rebuilds the sample project from scratch: its files, highlights and labels return to the original state. Your other projects are left untouched.','Reset sample',()=>{startDemo({reset:true});},false);}
function restartTour(){closeMenus();const back=S.project&&!S.demo?S.project.name:null;
 confirmDialog('Start the guided tour',`The tour runs on the sample project.${back?` When it ends, Kurasi returns to ${esc(back)}.`:''}`,'Start tour',()=>{startDemo({reset:false,tour:true});},false);}

/* ---------- projects ---------- */
/* the folder picker: the Windows select-folder dialog of tauri-plugin-dialog in the desktop window
   (window.__TAURI__.dialog, capability dialog:allow-open), pywebview's js_api in the old shell. A plain
   browser has neither, and the folder path is typed. Resolves to the path, or null when cancelled. */
function folderPicker(){const t=window.__TAURI__;const d=t&&t.dialog;
 if(d&&typeof d.open==='function')return async()=>{const p=await d.open({directory:true,multiple:false,title:'Choose the project folder'});return Array.isArray(p)?(p[0]||null):(p||null);};
 const py=pyFn('pick_folder');return py?async()=>(await py())||null:null;}
async function checkFolder(f){try{return await POST('/api/project/check',{folder:f})||{ok:false,error:'Kurasi could not check the folder.'};}catch(e){return {ok:false,error:e.message};}}
function newProjectDialog({folder:start='',focusFolder=false}={}){closeMenus();const tour=S.tour.on;const name=el('input',{type:'text',id:'np-name',value:tour&&S.project?S.project.name:'',placeholder:'For example: Thesis reading'});
 const folder=el('input',{type:'text',id:'np-folder',value:tour&&S.project?S.project.folder:'',placeholder:'For example: D:\\Papers\\Thesis',readonly:tour?true:null});const ferr=el('span',{class:'err',hidden:true});const fok=el('span',{class:'fok',hidden:true});
 /* the name follows the folder name until the person types one */
 let autoName='',seq=0,timer=null;const show=r=>{ferr.hidden=!!r.ok;fok.hidden=!r.ok;if(r.ok)fok.textContent=r.project_id?`This folder is in the project list as ${r.project_name||r.name}. Create and scan opens it.`:'Kurasi can read this folder.';else ferr.textContent=r.error||'Kurasi cannot use this folder.';};
 const validate=async()=>{const f=folder.value.trim();const mine=++seq;if(!f){ferr.hidden=fok.hidden=true;return null;}const r=await checkFolder(f);if(mine!==seq)return null;show(r);
  if(r.ok&&r.name&&(!name.value.trim()||name.value===autoName)){name.value=autoName=r.project_name||r.name;}return r;};
 if(!tour){folder.addEventListener('input',()=>{clearTimeout(timer);timer=setTimeout(validate,350);});folder.addEventListener('change',()=>{clearTimeout(timer);validate();});}
 const pick=tour?null:folderPicker();const browse=pick?el('button',{class:'btn',onclick:async()=>{try{const p=await pick();if(p){folder.value=p;await validate();}}catch(e){fail(e);}}},'Browse\u2026'):null;
 const TYPES=['.pdf','.md','.png','.jpg','.jpeg'];const have=new Set(S.settings.file_types||TYPES);const types=TYPES.map(t=>{const cb=el('input',{type:'checkbox',checked:tour||have.has(t)?true:null,disabled:tour?true:null});cb._t=t;return cb;});
 const auto=el('select',{class:'sel',id:'np-auto',disabled:tour?true:null},...[0,5,15,30,60].map(n=>el('option',{value:n,selected:n===(tour?autoMin():15)?true:null},n?`Every ${n} minutes`:'Off')));
 const body=el('div',{});if(tour)body.append(el('div',{class:'tourhint'},'Tour: the sample project is filled in. Create and scan runs its scan; the scan is recursive and incremental, and nothing is moved or renamed.'));
 body.append(el('div',{class:'fld'},el('label',{for:'np-name'},'Project name'),name),el('div',{class:'fld'},el('label',{for:'np-folder'},'Folder (scanned recursively)'),el('div',{style:'display:flex;gap:6px'},folder,browse),
   !tour&&!pick?el('span',{class:'help',id:'np-typed'},'Type or paste the full path of the folder. In the Kurasi window, Browse opens the Windows folder picker.'):null,ferr,fok),
  el('div',{class:'row2'},el('div',{class:'fld'},el('label',{},'File types to index'),el('div',{style:'display:grid;gap:4px'},...types.map(cb=>el('label',{style:'display:flex;gap:6px;align-items:center;font-size:13px;color:var(--fg)'},cb,cb._t)))),
   el('div',{class:'fld'},el('label',{for:'np-auto'},'Auto-scan'),auto,el('span',{class:'help',style:'margin-top:6px'},'Between scans a folder watcher picks up changes at once.'))));
 openModal({title:'New project',width:600,body,buttons:[{label:'Cancel'},{label:'Create and scan',primary:true,keep:true,action:async()=>{
  if(tour){closeModal(true);runScan({start:true});return;}
  const f=folder.value.trim();if(!f){show({ok:false,error:'Enter the folder to scan, for example D:\\Papers\\Thesis.'});folder.focus();return;}
  const ft=types.filter(c=>c.checked).map(c=>c._t);if(!ft.length){show({ok:false,error:'Tick at least one file type to index.'});return;}
  clearTimeout(timer);const r=await validate();if(!r||!r.ok){folder.focus();return;}
  try{await POST('/api/project',{name:name.value.trim()||r.name||baseName(r.folder),folder:r.folder,file_types:ft,auto_scan_min:+auto.value});}catch(e){show({ok:false,error:e.message});return;}
  closeModal(true);await switchedProject({attachScan:false});runScan({start:false});}}]});
 if(start){folder.value=start;validate();}if(start||focusFolder)setTimeout(()=>folder.focus(),0);
 signal('dialog:new');}

/* the Projects dialog: search, a list sorted by last opened, keyboard control, Open new folder, and per row
   Remove (Reset for the sample project). The query stays for the session. */
const PD={q:''};
function openProjectDialog(){return projectsDialog();}
async function projectsDialog(){closeMenus();let list=null;try{list=await GET('/api/projects');}catch(e){}
 if(!Array.isArray(list))list=[...S.projects].sort((a,b)=>String(b.last_opened||'').localeCompare(String(a.last_opened||'')));
 const openId=S.project&&S.project.id;list=list.map(p=>({...p,open:p.open||p.id===openId}));
 const search=el('input',{type:'text',id:'pd-q',placeholder:'Search by name or folder',autocomplete:'off',value:PD.q,'aria-controls':'pd-list'});
 const count=el('span',{class:'pcount'});const box=el('div',{class:'plist2',id:'pd-list',role:'listbox','aria-label':'Projects'});const none=el('div',{class:'pempty',hidden:true});
 const rows=list.map(p=>{const label=p.name||baseName(p.folder)||p.folder;
  const r=el('div',{class:'prow'+(p.open?' cur':'')+(p.exists===false?' gone':''),role:'option','aria-selected':'false',id:'pr-'+p.id,title:p.folder,
    onclick:e=>{if(e.target.closest('button'))return;go(p);}},
   el('div',{class:'pmain'},el('div',{class:'pn'},el('b',{},label),p.open?el('span',{class:'pill cur'},'Open now'):null,p.demo?el('span',{class:'pill'},'Sample'):null,p.exists===false?el('span',{class:'pill warn'},'Folder unreachable'):null),
    el('span',{class:'pf'},p.folder),p.last_opened?el('span',{class:'pm'},'Last opened '+fmtWhen(p.last_opened)):null),
   p.demo?el('button',{class:'btn sm',title:'Rebuild the sample project from scratch',onclick:()=>resetSampleDialog(p)},'Reset')
    :el('button',{class:'btn sm',title:'Take this project off the list',onclick:()=>removeProjectDialog(p)},'Remove'));
  r._p=p;r._key=(label+'\n'+(p.folder||'')).toLowerCase();return r;});
 box.append(...rows,none);let shown=[],sel=0;
 const mark=()=>{rows.forEach(r=>{r.classList.remove('sel');r.setAttribute('aria-selected','false');});const r=shown[sel];if(r){r.classList.add('sel');r.setAttribute('aria-selected','true');search.setAttribute('aria-activedescendant',r.id);r.scrollIntoView({block:'nearest'});}else search.removeAttribute('aria-activedescendant');};
 const apply=()=>{PD.q=search.value;const toks=PD.q.toLowerCase().split(/\s+/).filter(Boolean);const keep=shown[sel];
  shown=rows.filter(r=>{const ok=toks.every(t=>r._key.includes(t));r.hidden=!ok;return ok;});
  sel=Math.max(0,shown.indexOf(keep));
  count.textContent=toks.length?`${shown.length} of ${plural(rows.length,'project')}`:plural(rows.length,'project');
  none.hidden=!!shown.length;none.textContent=rows.length?`No project matches "${PD.q.trim()}". Search looks at the name and the folder path.`:'No projects yet. Open new folder creates one.';mark();};
 const go=p=>{closeModal(true);if(p.open){toast(`${p.name||baseName(p.folder)} is open.`);return;}openProject(p.id);};
 search.addEventListener('input',()=>{sel=0;shown=[];apply();});
 const body=el('div',{class:'projdlg'},el('div',{class:'psearch'},search,count),box,
  el('p',{class:'help'},'Up and Down move through the list, Enter opens the project, Esc closes. Remove takes a project off the list; its folder and files stay where they are.'));
 const m=openModal({title:'Projects',width:720,body,onClose:()=>{if(!S.project)setTimeout(()=>{if(!modalEl)welcomeDialog();},0);},
  buttons:[{left:el('button',{class:'btn',id:'pd-new',onclick:openNewFolder},'Open new folder\u2026')},{label:'Close'},{label:'Open',primary:true,keep:true,action:()=>{const r=shown[sel];if(r)go(r._p);}}]});
 modalEl._kind='projects';
 m.addEventListener('keydown',e=>{if(e.ctrlKey||e.altKey||e.metaKey)return;const k=e.key;
  if(k==='ArrowDown'||k==='ArrowUp'||k==='PageDown'||k==='PageUp'){e.preventDefault();if(!shown.length)return;const step=k==='ArrowDown'?1:k==='ArrowUp'?-1:k==='PageDown'?8:-8;sel=Math.max(0,Math.min(shown.length-1,sel+step));mark();return;}
  if(k==='Enter'&&e.target.tagName!=='BUTTON'){e.preventDefault();const r=shown[sel];if(r)go(r._p);return;}
  if(e.target===search)return;
  if(k.length===1){e.preventDefault();search.focus();search.value+=k;search.dispatchEvent(new Event('input'));}
  else if(k==='Backspace'){e.preventDefault();search.focus();search.value=search.value.slice(0,-1);search.dispatchEvent(new Event('input'));}});
 apply();setTimeout(()=>{search.focus();search.select();},0);return m;}
/* Open new folder: the native picker when there is one, else the typed path of the New project dialog */
async function openNewFolder(){const pick=folderPicker();if(!pick){closeModal(true);newProjectDialog({focusFolder:true});return;}
 let p=null;try{p=await pick();}catch(e){fail(e);return;}if(!p)return;closeModal(true);newProjectDialog({folder:p});}

/* Remove: off the list, and with the box ticked, Kurasi's own data for the project (the database folder in the app
   home and Kurasi's files in .kurasi). The data waits in the trash under the app home, so Undo brings it back. */
async function removeProjectDialog(p){let info=null;try{info=await GET('/api/projects/'+enc(p.id));}catch(e){fail(e);return;}
 const label=p.name||baseName(p.folder);const data=info&&info.data||{};let next='projects';
 const dirs=[data.app?el('div',{class:'rmdir'},el('code',{},data.app.path),el('span',{},'the catalogue database of this project, the whole folder')):null,
  data.sidecar?el('div',{class:'rmdir'},el('code',{},data.sidecar.path),el('span',{},data.sidecar.whole?`${data.sidecar.files.join(', ')}, then the empty folder`:`${data.sidecar.files.join(', ')}; the other files in this folder stay`)):null].filter(Boolean);
 const cb=dirs.length?el('input',{type:'checkbox',id:'rm-data'}):null;const paths=dirs.length?el('div',{class:'rmpaths off',id:'rm-paths'},el('div',{class:'rmhead'},'Kurasi deletes:'),...dirs):null;
 const body=el('div',{},el('p',{html:`Remove <b>${esc(label)}</b> from Kurasi's project list? The folder <code>${esc(p.folder)}</code> stays where it is.`}),
  p.open?el('div',{class:'warnline'},'This project is open now. Removing it closes it and shows the welcome screen.'):null,
  cb?el('div',{class:'rmbox'},el('label',{class:'rmchk',for:'rm-data'},cb,"Also delete Kurasi's data for this project"),paths):null,
  el('p',{class:'muted'},'Your PDFs, Markdown notes, images and every other file of yours stay untouched. Undo in the notice after the removal brings the project back'+(cb?', with its data when it was deleted.':'.')));
 openModal({title:'Remove project',width:640,body,onClose:()=>{setTimeout(()=>{if(modalEl)return;if(next==='projects')projectsDialog();else if(next==='welcome')welcomeDialog();},0);},
  buttons:[{label:'Cancel'},{label:'Remove',danger:true,id:'rm-go',action:async()=>{const withData=!!(cb&&cb.checked);let r;
   try{r=await DEL('/api/projects/'+enc(p.id)+(withData?'?delete_data=1':''));}catch(e){fail(e);return false;}
   if(r&&r.closed){next='welcome';await switchedProject({attachScan:false});}else{try{await loadState();}catch(e){}renderMenubar();}
   undoRemoveToast(p,r||{},withData);}}]});
 if(cb)cb.addEventListener('change',()=>{paths.classList.toggle('off',!cb.checked);$('#rm-go').textContent=cb.checked?'Remove and delete data':'Remove';});}
function undoRemoveToast(p,r,withData){const label=p.name||baseName(p.folder);const was=!!r.closed;
 toast(withData?`${label} and Kurasi's data for it are removed.`:`${label} is removed from the project list.`,{ms:15000,action:{label:'Undo',fn:async()=>{
  try{await POST('/api/projects/restore',{removal_id:r.removal_id});}catch(e){fail(e);return;}
  if(was){try{await POST('/api/project/open',{id:p.id});}catch(e){fail(e);}if(modalEl&&modalEl._kind==='welcome')closeModal(true);await switchedProject();}
  else{try{await loadState();}catch(e){}renderMenubar();if(modalEl&&modalEl._kind==='projects')projectsDialog();}
  toast(withData?`${label} is back in the list, with its data.`:`${label} is back in the list.`);}}});}
function resetSampleDialog(p){openModal({title:'Reset the sample project',width:560,onClose:()=>setTimeout(()=>{if(!modalEl)projectsDialog();},0),
 body:'<p>This rebuilds the sample project from scratch: its files, highlights and labels return to the original state. Your other projects are left untouched.</p>',
 buttons:[{label:'Cancel'},{label:'Reset sample',primary:true,action:async()=>{const t=toast('Rebuilding the sample project',{sticky:true});let r=null;
  try{r=await POST('/api/projects/'+enc(p.id)+'/reset');}catch(e){t.remove();fail(e);return false;}t.remove();
  if(r&&r.open)await switchedProject({attachScan:false});else{try{await loadState();}catch(e){}}
  toast('The sample project was rebuilt from scratch. Your other projects were left untouched.');}}]});}

/* ---------- scan dialog: real progress from /api/scan/status ---------- */
let scanOpen=false;
function scanSummary(res){if(res==null)return '';if(typeof res==='string')return res;if(typeof res!=='object')return String(res);
 const parts=Object.entries(res).filter(([k,v])=>typeof v==='number'||typeof v==='string').map(([k,v])=>`${v} ${k.replace(/_/g,' ')}`);return parts.join(' \u00B7 ');}
async function runScan({start=true}={}){closeMenus();if(!S.project||scanOpen)return;scanOpen=true;
 const log=el('div',{class:'scanlog'});const bar=el('div',{class:'bar'},el('i'));const msg=el('div',{class:'muted'},`Scanning ${S.project.folder} (recursive)`);
 let closed=false;openModal({title:'Scan folder',width:620,body:el('div',{},msg,bar,log),sticky:true,onClose:()=>{closed=true;},buttons:[{label:'Run in background',title:'Close this dialog; the scan goes on',action:()=>{closed=true;}}]});const mine=modalEl;
 const draw=st=>{const pct=Math.max(0,Math.min(100,+st.pct||0));bar.firstChild.style.width=pct+'%';if(st.message)msg.textContent=String(st.message);
  const lines=Array.isArray(st.log)?st.log:[];log.innerHTML='';lines.slice(-200).forEach(x=>{const s=typeof x==='string'?x:(x&&(x.msg||x.message||x.text))||JSON.stringify(x);log.append(el('div',{class:/error|fail|cannot/i.test(s)?'er':''},s));});log.scrollTop=log.scrollHeight;};
 if(start){try{await POST('/api/scan',{full:false});}catch(e){if(e.status!==409){log.append(el('div',{class:'er'},e.message));scanOpen=false;return;}}}
 const t0=Date.now();let st=null,seen=false;
 while(!closed){try{st=await GET('/api/scan/status')||{};}catch(e){log.append(el('div',{class:'er'},e.message));break;}draw(st);if(st.running)seen=true;
  if(!st.running&&(seen||!start||Date.now()-t0>1500))break;await sleep(400);}
 scanOpen=false;if(closed){toast('The scan goes on in the background. The catalogue updates when it finishes.');return;}
 if(st&&!st.running){bar.firstChild.style.width='100%';const sum=scanSummary(st.result);if(sum)log.append(el('div',{class:'ok'},sum));log.scrollTop=log.scrollHeight;
  try{await loadState();}catch(e){}await refreshData({cats:true});
  setTimeout(()=>{if(modalEl===mine)closeModal(true);toast(sum?`Scan finished: ${sum}.`:'Scan finished.');signal('scan:done');if(S.tour.on&&!modalEl)tourShow(S.tour.i);},700);}}
function customIntervalDialog(){closeMenus();const inp=el('input',{type:'number',min:1,max:1440,value:autoMin()||10,id:'ci'});
 openModal({title:'Auto-scan interval',body:el('div',{class:'fld'},el('label',{for:'ci'},'Minutes between full scans (1 to 1440)'),inp),buttons:[{label:'Cancel'},{label:'Set interval',primary:true,action:()=>setAutoScan(Math.max(1,Math.min(1440,Math.round(+inp.value)||10)))}]});}

/* ---------- settings ---------- */
function settingsDialog(){closeMenus();if(!S.project)return;const st=S.settings;const chk=(on,label,help)=>{const cb=el('input',{type:'checkbox',checked:on?true:null});return [cb,el('label',{style:'display:flex;gap:8px;align-items:flex-start;margin:8px 0'},cb,el('span',{},label,help?el('span',{class:'help',style:'display:block'},help):null))];};
 const [hints,hintsL]=chk(st.mcp_hints!==false,'Notify when an MCP tool can do the current step');const [pane,paneL]=chk(S.ui.filterPane,'Show the filter pane');
 const [ro,roL]=chk(isRO(),'Read-only project','Kurasi writes nothing into the project folder (PDF annotations, rotation, sidecar, exports). The database still records your work.');
 const loose=el('input',{type:'range',min:30,max:80,value:st.loose===false?72:42,style:'width:100%'});
 const ai=el('input',{type:'text',value:st.ai_author||'Claude'});const ft=el('input',{type:'text',value:(st.file_types||S.project.file_types||['.pdf','.md','.png','.jpg','.jpeg']).join(' ')});
 const ex=el('input',{type:'text',value:(st.exclude||S.project.exclude||[]).join(' ')});const col=el('input',{type:'color',value:/^#[0-9a-f]{6}$/i.test(st.hl_colour||'')?st.hl_colour:'#ffd84a',style:'height:30px;padding:2px'});
 const ro_=(label,val)=>el('div',{class:'fld'},el('label',{},label),el('input',{type:'text',readonly:true,value:val}));
 const body=el('div',{},ro_('Project folder',S.project.folder||''),ro_('Database',`%LOCALAPPDATA%\\Kurasi\\projects\\${S.project.id}\\kurasi.sqlite`),
  hintsL,paneL,roL,el('div',{class:'fld'},el('label',{},'Relevance threshold (left: loose, right: strict)'),loose),
  el('div',{class:'row2'},el('div',{class:'fld'},el('label',{},'File types to index (separated by spaces)'),ft),el('div',{class:'fld'},el('label',{},'Skip names matching (separated by spaces)'),ex)),
  el('div',{class:'row2'},el('div',{class:'fld'},el('label',{},'AI author name for MCP writes'),ai),el('div',{class:'fld'},el('label',{},'Highlight colour stored in PDFs'),col)),
  el('p',{class:'muted',style:'font-size:12px'},'The colour in the PDF is the same for you and the AI, so other PDF readers show ordinary highlights. Kurasi colours them by author on screen.'));
 openModal({title:'Settings',width:600,body,buttons:[{label:'Cancel'},{label:'Save settings',primary:true,action:async()=>{
  const split=s=>s.split(/[\s,;]+/).map(x=>x.trim()).filter(Boolean);const types=split(ft.value).map(t=>t.startsWith('.')?t.toLowerCase():'.'+t.toLowerCase());
  const out={mcp_hints:hints.checked,loose:+loose.value<57,read_only:ro.checked,ai_author:ai.value.trim()||'Claude',file_types:types.length?types:['.pdf'],exclude:split(ex.value),hl_colour:col.value};
  const typesChanged=JSON.stringify(out.file_types)!==JSON.stringify(st.file_types||S.project.file_types||[]);
  try{await PUT('/api/settings',out);}catch(e){fail(e);return false;}S.ui.filterPane=pane.checked;ls.set('kurasi.filterPane',pane.checked?'1':'0');
  try{await loadState();}catch(e){}SI.reset();renderAll();toast(typesChanged?'Settings saved. Scan the folder to index the new file types.':'Settings saved.');}}]});}

/* ---------- MCP server ---------- */
async function mcpDialog(){closeMenus();const body=el('div',{},el('p',{class:'muted'},'Loading the MCP server description'));
 openModal({title:'MCP server',width:760,body,buttons:[{label:'Copy registration command',keep:true,action:()=>{if(info&&info.command)copyText(info.command);}},{label:'Export log (Markdown)',keep:true,disabled:!S.project,action:async()=>{try{const r=await POST('/api/ai-log/export');toast(`Written: ${r&&r.path||'the .kurasi folder'}.`,{ms:9000});}catch(e){fail(e);}}},{label:'Close',primary:true}]});
 let info=null,rows=[];try{[info,rows]=await Promise.all([GET('/api/mcp'),S.project?GET('/api/ai-log').catch(()=>[]):Promise.resolve([])]);}catch(e){body.innerHTML='';body.append(el('p',{},e.message));return;}
 body.innerHTML='';const tools=Array.isArray(info&&info.tools)?info.tools:[];const res=Array.isArray(info&&info.resources)?info.resources:[];rows=Array.isArray(rows)?rows:[];
 put(body,el('p',{},`The MCP server runs over stdio: Claude Code starts it when it needs it. Register it once for this project with the command below, in write mode, where the agent may use every tool. For a stricter server, change write at the end to propose (suggestions only) or read. Reads are open; proposals wait for your review; writes carry the agent name (${aiName()}).`),
  el('div',{class:'cmdline'},el('code',{},info&&info.command||'(no project open)'),el('button',{class:'btn sm',disabled:info&&info.command?null:true,onclick:()=>copyText(info.command)},'Copy')),
  res.length?el('p',{class:'muted',style:'font-size:12px'},'Resources: '+res.map(x=>typeof x==='string'?x:x.uri||x.name||'').filter(Boolean).join(', ')+'.'):null,
  el('h4',{},'Tools'),el('table',{class:'tools'},el('tr',{},el('th',{},'Tool'),el('th',{},'Does'),el('th',{},'Mode')),...tools.map(t=>el('tr',{},el('td',{},el('code',{},t.name)),el('td',{},t.description||''),el('td',{},t.mode||'')))),
  el('h4',{},'Recent calls (the AI-use log)'),rows.length?el('table',{class:'tools'},el('tr',{},el('th',{},'When'),el('th',{},'Tool'),el('th',{},'Client'),el('th',{},'Purpose'),el('th',{},'Result')),...rows.slice(0,100).map(e=>el('tr',{},el('td',{style:'white-space:nowrap'},fmtWhen(e.at)),el('td',{},el('code',{},e.tool||'')),el('td',{},e.client||''),el('td',{},e.purpose||''),el('td',{},typeof e.result==='string'?e.result:JSON.stringify(e.result??''))))):el('p',{class:'muted'},'No MCP calls in this project yet.'));}

/* ---------- categories ---------- */
function categoryDialog({node=null,parent=null,onCreated=null}={}){closeMenus();const renaming=!!node;
 const name=el('input',{type:'text',id:'cat-name',value:node?node.n:'',placeholder:'For example: Methods'});const err=el('span',{class:'err',hidden:true});
 const below=node?new Set(subtree(node,node.p,true)):new Set();const curParent=node?(node.parent?node.parent.id:null):(parent?parent.id:null);
 const sel=el('select',{id:'cat-parent'},el('option',{value:''},'(top level)'),...S.catPaths.filter(p=>p!=='Uncategorized'&&!below.has(p)).map(p=>{const n=S.catByPath.get(p);return el('option',{value:String(n.id),selected:n.id===curParent?true:null},p);}));
 openModal({title:renaming?'Rename category':'New category',width:520,body:el('div',{},el('div',{class:'fld'},el('label',{for:'cat-name'},'Name'),name,err),el('div',{class:'fld'},el('label',{for:'cat-parent'},'Inside'),sel)),
  buttons:[{label:'Cancel'},{label:renaming?'Rename':'Create category',primary:true,keep:true,action:async()=>{const n=name.value.trim();
   if(!n||n.includes('/')){err.textContent=n?'A name cannot contain a slash; choose the parent under Inside.':'Enter a name for the category.';err.hidden=false;return;}
   const pid=sel.value?(isNaN(+sel.value)?sel.value:+sel.value):null;
   try{if(renaming)await PUT('/api/categories/'+enc(node.id),{name:n,parent_id:pid});else await POST('/api/categories',{parent_id:pid,name:n});}catch(e){err.textContent=e.message;err.hidden=false;return;}
   closeModal(true);const ppath=pid!=null?([...S.catByPath.values()].find(c=>String(c.id)===String(pid))||{}).p:'';const path=(ppath?ppath+'/':'')+n;if(ppath)S.exp.add(ppath);
   await refreshData({cats:true,hlRefs:[]});toast(renaming?'Category renamed.':`Category ${path} created.`);if(onCreated)onCreated(path);}}]});}
function deleteCategory(n){const to=n.parent?n.parent.p:'Uncategorized';confirmDialog('Delete category',`Delete the category <b>${esc(n.p)}</b>${n.c?' and the categories inside it':''}? Its references move to ${esc(to)}. No file is touched.`,'Delete',async()=>{
 try{await DEL('/api/categories/'+enc(n.id));toast('Category deleted.');}catch(e){fail(e);}await refreshData({cats:true,hlRefs:[]});});}

/* ---------- about and shortcuts ---------- */
function aboutDialog(){closeMenus();openModal({title:'About Kurasi',width:560,body:`<p><b>Kurasi</b>${S.version?`, version ${esc(S.version)}`:''}. Personal reference curation: scan a folder, catalogue its files, highlight PDFs, Markdown notes and images, and keep an analysis next to each highlight.</p><p>PDF highlights are written into the PDF files as ordinary annotations, so other PDF readers show them. Highlights on Markdown notes and images live in the project database and in <code>.kurasi\\metadata.json</code> inside the project folder.</p><p>An MCP server gives an AI agent the same catalogue under your review: its proposals carry a blue dot until you save them, and every call lands in the AI-use log.</p><p class="muted">Built with Rust, Tauri 2, pdfium and SQLite. The window opens at a size that fits the screen.</p><p class="muted">Kurasi is MIT licensed and provided as is, without warranty. <code>LICENSE.txt</code> and <code>THIRD_PARTY_LICENSES.txt</code> sit in the install folder.</p>`,buttons:[{label:'Close',primary:true}]});}
function shortcutsDialog(){closeMenus();openModal({title:'Keyboard shortcuts',body:`<table class="tools"><tr><td><kbd>Ctrl</kbd> <kbd>N</kbd></td><td>New project</td></tr><tr><td><kbd>Ctrl</kbd> <kbd>O</kbd></td><td>Projects: search, open, remove</td></tr><tr><td><kbd>F5</kbd></td><td>Scan folder now</td></tr><tr><td><kbd>Ctrl</kbd> <kbd>F</kbd></td><td>Search</td></tr><tr><td><kbd>Ctrl</kbd> + wheel</td><td>Zoom the document</td></tr><tr><td>Wheel, drag, arrows</td><td>Tag graph: zoom around the pointer, pan the canvas, pan 40 px per arrow press</td></tr><tr><td><kbd>Ctrl</kbd> <kbd>+</kbd> <kbd>Ctrl</kbd> <kbd>-</kbd> <kbd>Ctrl</kbd> <kbd>0</kbd></td><td>Tag graph: zoom in, zoom out, fit all nodes</td></tr><tr><td><kbd>Ctrl</kbd> <kbd>Z</kbd></td><td>Undo the last delete</td></tr><tr><td><kbd>Esc</kbd></td><td>Close menu or dialog</td></tr><tr><td>Middle click</td><td>Close a document tab</td></tr><tr><td>Right-click</td><td>Card: Edit, Delete. Page: Rotate. Highlight: Edit, Delete. Category: New, Rename, Delete</td></tr></table>`,buttons:[{label:'Close',primary:true}]});}
