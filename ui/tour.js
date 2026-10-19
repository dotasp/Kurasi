'use strict';
/* Guided tour: spotlight, coach mark, MCP offers. It runs on the sample project and finds its
   targets in the loaded data (no fixed IDs or titles). Leaving it returns to the project that
   was open before. */

const byHl=list=>[...list].sort((a,b)=>b.hl_count-a.hl_count||a.title.localeCompare(b.title));
const firstMd=()=>byHl(S.refs.filter(r=>r.ext==='md'))[0]||null;
const firstImg=()=>byHl(S.refs.filter(r=>isImgExt(r.ext)))[0]||null;
const topRef=()=>byHl(S.refs.filter(r=>r.ext==='pdf'))[0]||byHl(S.refs)[0]||null;
/* the loose-search example: a title word of 7 or more letters with one inner letter dropped */
function typoQuery(){for(const r of byHl(S.refs)){const w=(r.title.match(/\p{L}{7,}/gu)||[])[0];if(w){const i=Math.floor(w.length/2);return {q:(w.slice(0,i)+w.slice(i+1)).toLowerCase(),word:w,ref:r};}}
 const r=topRef();const w=r?(r.title.match(/\p{L}{3,}/u)||[''])[0]:'';return {q:w.toLowerCase(),word:w,ref:r};}
const uncatRefs=()=>S.refs.filter(r=>r.categories.includes('Uncategorized'));
function expandFirstBranch(){const n=S.cats.find(c=>c.c&&c.c.length);if(!n)return;S.exp.add(n.p);const k=n.c.find(c=>c.c&&c.c.length);if(k)S.exp.add(k.p);}
function resetFilters(){S.f={cats:new Set(),authors:new Set(),tags:new Set(),refs:new Set()};}
let tourTypo=null;
const STEPS=[
 {id:'file',target:()=>$('#mb-file'),title:'Create a project',body:'A project is a name and a folder. Open the File menu to start one; references, authors and tags come from scanning the folder. Click File, or press Next.',next:()=>openMenuByName('mb-file'),advanceOn:'menu:file'},
 {id:'new',target:()=>$('#mi-new')||$('#mb-file'),title:'New project',body:'Choose New project. The dialog asks for a name, the folder and the file types to index, then runs the first scan. In the tour the sample project is filled in.',next:()=>newProjectDialog(),advanceOn:'scan:done'},
 {id:'uncat',target:()=>$('#main'),title:'New files start in Uncategorized',before:()=>{resetFilters();if(uncatRefs().length)S.f.cats=new Set(['Uncategorized']);S.tab='reference';},
  body:()=>{const n=uncatRefs().length;return `Each file has a path, a hash, created and modified times, a first-page thumbnail and the highlights it already contained. A new file has the author Unknown and its file name as its title until someone labels it. ${n?`${plural(n,'file')} in the sample still ${n===1?'waits':'wait'} here, shown with the amber dot.`:'Every file in the sample is labelled already.'}`;},
  mcp:{tool:'reference_categorize',text:()=>`${aiName()} can read each first page and propose author, title, date, publication and category. Copy the prompt into Claude Code; proposals carry a blue dot until you Save them.`,prompt:()=>Prompt.categorize(uncatRefs())},advanceOn:'mcp:categorized',nextLabel:'Skip, I will label them'},
 {id:'filter',target:()=>$('#filter'),title:'Filter pane, at most 15% of the window',body:'Reference, Author and Tag are collapsible trees. Ticking a parent ticks its children; a partial selection shows as a dash. Labels are cut at 75 letters with an ellipsis at the end; hover to read the full term. Entries run from A to Z, and the box at the top finds an author or a tag from a near spelling. Right-click a category to add, rename or delete one.',before:()=>expandFirstBranch()},
 {id:'search',target:()=>$('#searchbox'),also:()=>$('#qadd'),title:'Search loosely',before:()=>{resetFilters();tourTypo=typoQuery();setQuery(tourTypo.q);},
  body:()=>{const t=tourTypo||typoQuery();return t.word?`Relevance tolerates typos and word order, like a shop search. The box now holds "${t.q}", the word "${t.word}" with a letter missing, and "${words(t.ref.title,8)}" still comes up. Press Enter or + to keep the words as a filter chip, then search within the result. The tolerance is set in Tools, Settings.`:'Relevance tolerates typos and word order, like a shop search. Press Enter or + to keep the words as a filter chip, then search within the result. The tolerance is set in Tools, Settings.';}},
 {id:'sort',target:()=>$('#sortbox'),title:'Sort',body:'Relevance, date added or alphabetical. With an empty search, relevance ranks by highlight count: the more markers, the more the paper mattered to you.'},
 {id:'tabs',target:()=>$('#tabstrip'),title:'Reference, Authors, Tag',body:'Three views over the same filtered set. Authors show a magnified name in place of a photo, with their most highlighted paper. Tag is a graph of tags, authors and papers; it comes later in the tour. Document tabs open to the right of these and can be closed.',before:()=>{setQuery('');switchTab('authors');}},
 {id:'author',target:()=>$('.card.author'),title:'Click an author',body:'An author card takes you back to Reference with that author ticked on the left. Authors are kept as long as they have references; delete the references to remove them.',next:()=>{const a=$('.card.author');if(a)a.click();else switchTab('reference');},advanceOn:'author:clicked'},
 {id:'card',target:()=>$('.card.ref'),title:'Reference cards',body:'A small first-page thumbnail on the left; on the right up to three lines of title, the authors, type and publication, the document date, page count, highlight count and the three most used tags. Right-click for Edit or Delete. Left-click opens the document. Next opens Edit for you.',
  next:()=>{const l=sorted(visibleRefs());const first=l.find(r=>r.ext==='pdf')||l[0];if(first)openEdit(first.id);},advanceOn:'edit:open'},
 {id:'edit',target:()=>$('#right'),title:'Edit pane, 30%',body:'APA fields, dates, categories, register ID and the files. Save writes to the project database; the file is never moved or renamed. Next opens the document.',
  mcp:{tool:'reference_update_metadata',text:()=>`${aiName()} can fill these fields from the first page or a DOI lookup; you review and Save.`,prompt:()=>{const r=S.right&&refById(S.right.id);return r?Prompt.metadata(r):'';}},
  next:()=>{const id=S.right&&S.right.id;closeRight();if(id)openDoc(id);else switchTab('reference');},advanceOn:'doc:open'},
 {id:'viewer',target:()=>$('.vtb'),title:'Document tab',body:'Choose Highlight, then drag across words for a text highlight, or from an empty spot for a region. A dialog asks for the verbatim text (prefilled from the text layer) and your analysis. Right-click a page to rotate it; Ctrl and the wheel zoom; the Hand tool pans.',
  mcp:{tool:'highlight_add',text:()=>`${aiName()} can add highlights with analysis through MCP. They are drawn in blue and carry the author "${aiName()}".`},before:()=>{const v=V.get(S.tab);if(v)setTool(v,'hl');}},
 {id:'knowledge',target:()=>$('#right'),title:'Knowledge pane, 30%',body:'Every highlight on this document: verbatim text, analysis in Markdown, a label (F, I, P), tags with suggestions, and Save when you edit. Proposals from the AI (a delete or extra tags) wait here for your answer. A link to a project file opens it in a new tab; any other link opens with the Windows default app.'},
 {id:'kfilter',target:()=>$('#filter'),title:'The filter pane follows the document',body:()=>`Only the tags and highlight authors present in this document are listed. Unticking hides items in the knowledge pane.${firstMd()?' Next opens a Markdown note.':firstImg()?' Next opens an image.':''}`,
  next:()=>{const r=firstMd()||firstImg();if(r)openDoc(r.id);else switchTab('tags');},advanceOn:'doc:open'},
 {id:'markdown',skip:()=>!firstMd(),target:()=>$('.vtb'),title:'Markdown files are rendered',body:()=>`Notes render with headings, lists and tables. With Highlight on, selecting text with the mouse makes a highlight; the verbatim text is the selection itself. A link to a project file opens that file in a new tab; Raw Markdown shows the source.${firstImg()?' Next opens an image.':''}`,
  before:()=>{const r=firstMd();if(r&&S.tab!==r.id)openDoc(r.id);const v=V.get(S.tab);if(v)setTool(v,'hl');},mcp:{tool:'highlight_add',text:()=>'For Markdown the agent passes the quote; the app finds and marks it in the rendered note.'},
  next:()=>{const r=firstImg();if(r)openDoc(r.id);else switchTab('tags');},advanceOn:'doc:open'},
 {id:'image',skip:()=>!firstImg(),target:()=>$('.vtb'),title:'Images are annotated by region',body:'PNG and JPG files have no text layer, so a highlight is a dragged rectangle stored as fractions of the image; it survives zoom and rotation. The verbatim field is typed by hand. The knowledge pane lists it like any other highlight. Next opens the Tag graph.',
  before:()=>{const r=firstImg();if(r&&S.tab!==r.id)openDoc(r.id);const v=V.get(S.tab);if(v)setTool(v,'hl');},next:()=>switchTab('tags')},
 {id:'graph',target:()=>$('#main'),title:'Tag graph: tag, author, paper',before:()=>{if(S.tab!=='tags')switchTab('tags');},body:'Every tag (circle), author (diamond) and paper (square) in the current filter, linked where a paper carries the tag or the author. A bigger circle or diamond touches more papers. Hover a node to trace its links; click one and Reference opens filtered to it. Node spacing in the toolbar spreads the nodes apart.',
  mcp:{tool:'highlight_suggest_tags',text:()=>'Tags the AI proposes appear as dashed chips in the knowledge pane; the graph shows a tag once you accept it.'},next:()=>{const t=$('.gbox .node.tag .mark');if(t)t.dispatchEvent(new MouseEvent('click',{bubbles:true}));else{switchTab('reference');tourNext();}},advanceOn:'graph:clicked'},
 {id:'tools',target:()=>$('#mb-tools'),title:'The menu bar holds the rest',body:'Tools, MCP server shows the registration command, the tools and the call log that becomes your AI-use disclosure. File, Auto-scan sets the interval; File, Scan folder now runs it by hand. Help opens the sample project, resets it, or runs this tour again.',last:true}
];
const stepText=x=>typeof x==='function'?x():x;
function startTour(){closeMenus();resetFilters();S.q='';$('#q').value='';$('#qclr').hidden=true;S.tab='reference';S.right=null;STEPS.forEach(s=>s._done=false);S.tour={on:true,i:0};renderAll();tourShow(0);}
function tourShow(i,dir=1){if(!S.tour.on)return;let st=STEPS[i];while(st&&st.skip&&st.skip()){i+=dir;st=STEPS[i];}if(!st){if(i<0){i=0;st=STEPS[0];}else{tourEnd();return;}}S.tour.i=i;renderStatus();
 /* setup actions never count as the step's own advance signal */
 if(st.before&&!st._done){st._done=true;S.tour.mute=true;try{st.before();}finally{S.tour.mute=false;}renderAll();}
 if(modalEl){$('#tour').hidden=true;return;}
 const coach=$('#coach');coach.innerHTML='';coach.append(el('h3',{},st.title),el('p',{},stepText(st.body)));
 if(st.mcp){const box=el('div',{class:'mcpbox'},el('b',{html:`MCP available: <code>${esc(st.mcp.tool)}</code>`}),el('span',{},stepText(st.mcp.text)));if(st.mcp.prompt)box.append(el('div',{class:'acts'},el('button',{class:'btn sm pri',onclick:()=>{const p=st.mcp.prompt();if(p)copyPrompt(p);}},'Copy prompt for Claude')));coach.append(box);}
 const shown=STEPS.filter(s=>!(s.skip&&s.skip()));const pos=shown.indexOf(st)+1;
 const foot=el('div',{class:'cfoot'},el('span',{class:'step'},`${pos} of ${shown.length}`),el('button',{class:'btn sm',onclick:tourEnd},'Skip tour'),
  el('button',{class:'btn sm',disabled:i===0?true:null,onclick:()=>{STEPS.forEach((s,k)=>{if(k>=i-1)s._done=false;});tourShow(i-1,-1);}},'Back'),
  el('button',{class:'btn sm pri',onclick:()=>{if(st.last){tourEnd();return;}if(st.next){st.next();if(!st.advanceOn)tourNext();}else tourNext();}},st.last?'Finish':st.nextLabel||'Next'));
 coach.append(foot);$('#tour').hidden=false;requestAnimationFrame(positionTour);}
function positionTour(){if(!S.tour.on||$('#tour').hidden)return;const st=STEPS[S.tour.i];let t=st&&st.target();if(!t||t.hidden||!t.isConnected)t=$('#app');let r=t.getBoundingClientRect();const pad=6;const vw=innerWidth,vh=innerHeight;
 /* a step can take a second element into the ring (the + beside the search box), so the coach card sits clear of it */
 const a=st&&st.also&&st.also();if(a&&a.isConnected&&!a.hidden){const q=a.getBoundingClientRect();r={left:Math.min(r.left,q.left),top:Math.min(r.top,q.top),right:Math.max(r.right,q.right),bottom:Math.max(r.bottom,q.bottom)};}
 const top=Math.max(0,r.top-pad),left=Math.max(0,r.left-pad),bottom=Math.min(vh,r.bottom+pad),right=Math.min(vw,r.right+pad);
 const set=(id,s)=>Object.assign($(id).style,s);set('#tb0',{left:0,top:0,width:vw+'px',height:top+'px'});set('#tb1',{left:0,top:bottom+'px',width:vw+'px',height:Math.max(0,vh-bottom)+'px'});set('#tb2',{left:0,top:top+'px',width:left+'px',height:(bottom-top)+'px'});set('#tb3',{left:right+'px',top:top+'px',width:Math.max(0,vw-right)+'px',height:(bottom-top)+'px'});
 set('#tring',{left:left+'px',top:top+'px',width:(right-left)+'px',height:(bottom-top)+'px'});
 const c=$('#coach');const cw=c.offsetWidth,ch=c.offsetHeight;let x,y;
 if(right+12+cw<=vw){x=right+12;y=Math.min(Math.max(8,top),vh-ch-8);}else if(left-12-cw>=0){x=left-12-cw;y=Math.min(Math.max(8,top),vh-ch-8);}else if(bottom+12+ch<=vh){x=Math.min(Math.max(8,left),vw-cw-8);y=bottom+12;}else if(top-12-ch>=0){x=Math.min(Math.max(8,left),vw-cw-8);y=top-12-ch;}else{x=Math.max(8,right-cw-16);y=top+16;}
 c.style.left=x+'px';c.style.top=y+'px';}
function tourNext(){const i=S.tour.i+1;if(i>=STEPS.length){tourEnd();return;}tourShow(i,1);}
async function tourEnd(){if(!S.tour.on)return;S.tour.on=false;$('#tour').hidden=true;STEPS.forEach(s=>s._done=false);closeMenus();resetFilters();setQuery('');
 const back=S.tourReturn;S.tourReturn=null;const p=back&&S.projects.find(x=>x.id===back);
 if(back){await openProject(back);toast(`Tour finished. Back in ${p&&p.name?p.name:'your project'}.`);}
 else{S.tab='reference';renderAll();toast('Tour finished. Help, Guided tour runs it again.');}}
function tourPos(){const shown=STEPS.filter(s=>!(s.skip&&s.skip()));return {n:Math.max(1,shown.indexOf(STEPS[S.tour.i])+1),of:shown.length};}
function signal(name){if(!S.tour.on||S.tour.mute)return;const st=STEPS[S.tour.i];if(st&&st.advanceOn===name)setTimeout(tourNext,60);}
