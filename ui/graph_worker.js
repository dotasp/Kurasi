/* Tag graph builder: graph data (tag, author, paper) and a force layout in SVG coordinates.
   Runs as a Web Worker (new Worker("graph_worker.js")). The same file is also loaded as a
   plain script, so the page can call kurasiGraphCompute() itself when a worker cannot start
   (for example a page opened from disk).
   Input:  {key, refs:[{id,title,authors[],tags[]}], spacing (1 = 100%), W, H}
   Output: {key, W, H, nodes:[{id,type,label,lab,ref,deg,r,x,y}], edges:[[i,j],...], ms, iters}
   r is the mark radius in px and lab tells whether the label is a candidate for drawing (the page
   picks the labels that fit at the current zoom). The layout aims at the W x H box less room for
   labels on the right and the view toolbar on top, and may reach past it; the page fits the view.
   Cost grows near n log n: Barnes-Hut repulsion over a quadtree, collision and separation over a
   spatial hash. 15,000 nodes take a few seconds. */
'use strict';
function kurasiGraphCompute(msg){
  const t0=Date.now();
  /* paper labels: three words and 24 letters at most, cut at a word end when one is near */
  const short=(s,nw,nc)=>{const w=String(s).trim().split(/\s+/);let t=w.slice(0,nw).join(' '),cut=w.length>nw;
    if(t.length>nc){const sp=t.lastIndexOf(' ',nc-1);t=sp>=nc/2?t.slice(0,sp):t.slice(0,nc-1);cut=true;}return cut?t.replace(/[\s,.;:!?(-]+$/,'')+'…':t;};
  const isPerson=a=>/,\s/.test(a);
  const spacing=Math.max(0.25,Math.min(4,+msg.spacing||1)),W=msg.W||800,H=msg.H||500;
  const nodes=new Map(),edges=[];
  const add=(id,type,label,ref)=>{if(!nodes.has(id))nodes.set(id,{id,type,label,ref:ref||null,deg:0});return nodes.get(id);};
  for(const r of msg.refs||[]){
    const p=add('p:'+r.id,'paper',short(r.title||'',3,24),r.id);
    for(const a of r.authors||[]){if(!a||a==='Unknown')continue;const n=add('a:'+a,'author',isPerson(a)?a.split(/,\s*/)[0]:a);edges.push([p,n]);}
    for(const t of new Set(r.tags||[])){const n=add('t:'+t,'tag','#'+t);edges.push([p,n]);}
  }
  edges.forEach(([a,b])=>{a.deg++;b.deg++;});
  /* a paper with no tag and no named author has nothing to link to and stays out */
  const N=[...nodes.values()].filter(n=>n.deg>0);
  /* label candidates: every tag and author; a paper when it has three links or the graph is small. lw is
     the label width at the default 12 px (tags are bold) plus the 4 px gap to the mark. */
  N.forEach(n=>{n.r=radius(n);n.lab=n.type!=='paper'||n.deg>=3||N.length<=24;n.lw=n.lab?4+n.label.length*12*(n.type==='paper'?0.5:n.type==='tag'?0.62:0.58):0;});
  const iters=layout(N,edges,W,H,spacing);
  const idx=new Map(N.map((n,i)=>[n,i]));
  return {key:msg.key,W,H,ms:Date.now()-t0,iters,
    nodes:N.map(n=>({id:n.id,type:n.type,label:n.label,lab:n.lab,ref:n.ref,deg:n.deg,r:n.r,x:Math.round(n.x*10)/10,y:Math.round(n.y*10)/10})),
    edges:edges.filter(([a,b])=>idx.has(a)&&idx.has(b)).map(([a,b])=>[idx.get(a),idx.get(b)])};

  /* mark radius in px. Papers keep one size. Tags and authors grow with the number of papers they
     touch on a scale defined up to 10,000 papers: r = 6 + 22 * (min(4, log10(papers)) / 4)^0.75.
     Each tenfold step adds less than the one before, and counts inside one decade still differ:
     1 paper 6, 10 papers 13.8, 100 papers 19.1, 1000 papers 23.7, 3000 papers 25.8,
     7000 papers 27.4, 10,000 papers and more 28. */
  function radius(n){if(n.type==='paper')return 6;return Math.round((6+22*Math.pow(Math.min(4,Math.log10(Math.max(1,n.deg)))/4,0.75))*10)/10;}
  function rng(seed){let h=2166136261;for(const ch of seed){h^=ch.charCodeAt(0);h=Math.imul(h,16777619);}return()=>{h+=0x6D2B79F5;let t=h;t=Math.imul(t^t>>>15,t|1);t^=t+Math.imul(t^t>>>7,t|61);return((t^t>>>14)>>>0)/4294967296;};}
  /* Every force is proportional to the ideal distance k, so the spacing setting scales the picture
     and keeps its shape. k fills the box when the graph is small and never drops below the room two
     average marks need. Repulsion falls with 1/d, so a dense graph spreads to the shape of the box;
     a Barnes-Hut quadtree sums it, treating a far cell as one body at its centre of mass. A link rests
     at k plus both radii plus a share that grows with the busier end, which gives a tag with many
     papers a ring wide enough to hold them; its pull weakens with the degree of the quieter end.
     Marks are kept apart by a collision step in the second half of the rounds and by separation
     passes at the end, both over a spatial hash. A graph of up to 1000 nodes starts on a ring. A
     larger one starts with tags on a sunflower spiral (busiest in the middle), each paper near its
     tags and each author near its papers, and moves further per round. */
  function layout(nodes,edges,W,H,spacing){
    const n=nodes.length;if(!n)return 0;
    /* the area to fill: the box less about 200 px for labels on the right and 90 px for the toolbar on top */
    const UW=Math.max(200,W-200),UH=Math.max(160,H-90),cx=W/2-50,cy=H/2+20;
    const rnd=rng('graph'+n);const asp=Math.max(1,Math.min(2.2,UW/UH));
    let rs=0,rmax=0;for(const v of nodes){rs+=v.r;if(v.r>rmax)rmax=v.r;}const rm=rs/n;
    const k=Math.max(2*rm+28,Math.sqrt(UW*UH/n)*0.9)*spacing,gap=8*spacing;
    const big=n>1000,R0=k*Math.sqrt(n)*(big?0.8:0.5),ax=Math.sqrt(asp),ay=1/Math.sqrt(asp);
    const idx=new Map(nodes.map((v,i)=>[v,i]));
    const X=new Float64Array(n),Y=new Float64Array(n),VX=new Float64Array(n),VY=new Float64Array(n),R=new Float64Array(n);
    nodes.forEach((v,i)=>{R[i]=v.r;});
    const Ed=edges.filter(([a,b])=>idx.has(a)&&idx.has(b));const m=Ed.length;
    const EA=new Int32Array(m),EB=new Int32Array(m),ER=new Float64Array(m),ES=new Float64Array(m);
    Ed.forEach(([a,b],e)=>{EA[e]=idx.get(a);EB[e]=idx.get(b);ER[e]=k*0.8+a.r+b.r+1.5*gap*Math.sqrt(Math.max(a.deg,b.deg));ES[e]=0.07/Math.sqrt(Math.min(a.deg,b.deg));});
    if(!big)nodes.forEach((v,i)=>{const a=i/n*Math.PI*2;const rad=v.type==='paper'?1:v.type==='tag'?0.43:0.71;const s=R0*rad*(0.7+rnd()*0.6);X[i]=cx+Math.cos(a)*s*ax;Y[i]=cy+Math.sin(a)*s*ay;});
    else seed(nodes,EA,EB,X,Y,R0,cx,cy,ax,ay,k,rnd);
    const kr=0.008*k*k,fmax=0.6*k,gx=0.012,gy=gx*asp;
    /* rounds: 260 up to 400 nodes, then fewer down to 60 for a large graph, which moves further per round */
    const iters=big?Math.max(60,Math.round(104*Math.pow(1000/n,0.3))):n>400?Math.floor(260*400/n):260;
    const cap0=big?Math.max(0.12*k,2*R0/iters):0.12*k;
    const bh=quad(n,X,Y,big?1.21:0.81),cell=2*rmax+gap,hs=hash(n);
    for(let it=0;it<iters;it++){const t=1-it/iters;
      bh.build();bh.repel(VX,VY,kr,fmax,rnd);
      if(it>=iters/2)collide(hs,X,Y,R,gap,cell,false);
      for(let e=0;e<m;e++){const a=EA[e],b=EB[e];const dx=X[b]-X[a],dy=Y[b]-Y[a];const d=Math.sqrt(dx*dx+dy*dy)||0.01;const f=(d-ER[e])/d*ES[e];VX[a]+=dx*f;VY[a]+=dy*f;VX[b]-=dx*f;VY[b]-=dy*f;}
      const cap=cap0*t+0.005*k;
      for(let i=0;i<n;i++){let vx=VX[i]+(cx-X[i])*gx,vy=VY[i]+(cy-Y[i])*gy;const sp=Math.sqrt(vx*vx+vy*vy);if(sp>cap){vx*=cap/sp;vy*=cap/sp;}X[i]+=vx;Y[i]+=vy;VX[i]=vx*0.4;VY[i]=vy*0.4;}
    }
    if(n<=300)separateLabels(nodes,hs,X,Y,R,gap/2,250);
    /* separation passes until no pair is closer than both radii plus the gap; after the first pass only the
       marks that moved in the pass before are checked against their neighbours */
    const act=active(n);for(let p=0,most=big?300:80;p<most&&collide(hs,X,Y,R,gap,cell,true,act);p++);
    nodes.forEach((v,i)=>{v.x=X[i];v.y=Y[i];});
    return iters;
  }
  /* start positions of a large graph: tags on a sunflower spiral, busiest first; each paper at the mean
     of its tags (or of its authors' spots when it has no tag) plus a random offset; each author at the
     mean of its papers plus a random offset */
  function seed(nodes,EA,EB,X,Y,R0,cx,cy,ax,ay,k,rnd){const n=nodes.length,m=EA.length;
    const adj=Array.from({length:n},()=>[]);for(let e=0;e<m;e++){adj[EA[e]].push(EB[e]);adj[EB[e]].push(EA[e]);}
    const tags=nodes.map((v,i)=>i).filter(i=>nodes[i].type==='tag').sort((a,b)=>nodes[b].deg-nodes[a].deg);
    const done=new Uint8Array(n),gold=Math.PI*(3-Math.sqrt(5)),Rt=R0*0.6;
    tags.forEach((i,j)=>{const s=Rt*Math.sqrt((j+0.5)/tags.length),a=j*gold;X[i]=cx+Math.cos(a)*s*ax;Y[i]=cy+Math.sin(a)*s*ay;done[i]=1;});
    const jit=(i,s)=>{const a=rnd()*Math.PI*2,d=s*Math.sqrt(rnd());X[i]+=Math.cos(a)*d;Y[i]+=Math.sin(a)*d;};
    const mean=(i,want)=>{let sx=0,sy=0,c=0;for(const j of adj[i])if(done[j]===want){sx+=X[j];sy+=Y[j];c++;}if(!c)return false;X[i]=sx/c;Y[i]=sy/c;return true;};
    const free=i=>{const a=rnd()*Math.PI*2,s=R0*Math.sqrt(rnd());X[i]=cx+Math.cos(a)*s*ax;Y[i]=cy+Math.sin(a)*s*ay;};
    for(let i=0;i<n;i++)if(nodes[i].type==='paper'&&mean(i,1)){jit(i,k*2);done[i]=2;}
    for(let i=0;i<n;i++)if(nodes[i].type==='author'){if(mean(i,2))jit(i,k*1.5);else free(i);done[i]=3;}
    for(let i=0;i<n;i++)if(!done[i]){if(!mean(i,3))free(i);jit(i,k);done[i]=2;}}
  /* Barnes-Hut quadtree with buckets of up to 8 bodies. Every body weighs one. A cell that does not hold
     the body counts as one body at its centre of mass when its side is under 0.9 of the distance
     (1.1 in a large graph). The
     force kr/d is capped at fmax near a body, which only bodies closer than kr/fmax feel. A cell's four
     child slots hold the child ids, or -2 when that quarter is empty. */
  function quad(n,X,Y,TH2){const LEAF=8;let cap=Math.max(64,2*n);
    let C=new Int32Array(cap),S0=new Int32Array(cap),S1=new Int32Array(cap),M=new Float64Array(cap),MX=new Float64Array(cap),MY=new Float64Array(cap),SZ=new Float64Array(cap),BX=new Float64Array(cap),BY=new Float64Array(cap);
    const P=new Int32Array(n),T=new Int32Array(n),PX=new Float64Array(n),PY=new Float64Array(n),Q=new Uint8Array(n),stack=new Int32Array(4096),K4=new Int32Array(4),OF=new Int32Array(4);let cnt=0;
    const grow=()=>{const g=a=>{const b=new a.constructor(a.length*2);b.set(a);return b;};C=g(C);S0=g(S0);S1=g(S1);M=g(M);MX=g(MX);MY=g(MY);SZ=g(SZ);BX=g(BX);BY=g(BY);cap*=2;};
    const node=(s,e,x0,y0,sz,dep)=>{if(cnt+5>cap)grow();const c=cnt++;S0[c]=s;S1[c]=e;SZ[c]=sz;BX[c]=x0;BY[c]=y0;let mx=0,my=0;for(let p=s;p<e;p++){mx+=X[P[p]];my+=Y[P[p]];}M[c]=e-s;MX[c]=mx/(e-s);MY[c]=my/(e-s);
      if(e-s<=LEAF||dep>=30){C[c]=-1;for(let p=s;p<e;p++){PX[p]=X[P[p]];PY[p]=Y[P[p]];}return c;}
      const h=sz/2,xm=x0+h,ym=y0+h;K4.fill(0);for(let p=s;p<e;p++){const i=P[p];const q=(X[i]>=xm?1:0)+(Y[i]>=ym?2:0);Q[i]=q;K4[q]++;}
      OF[0]=s;for(let q=1;q<4;q++)OF[q]=OF[q-1]+K4[q-1];const st=[OF[0],OF[1],OF[2],OF[3],e];for(let p=s;p<e;p++){const i=P[p];T[OF[Q[i]]++]=i;}for(let p=s;p<e;p++)P[p]=T[p];
      const first=cnt;cnt+=4;C[c]=first;
      for(let q=0;q<4;q++){const cs=st[q],ce=st[q+1];C[first+q]=ce>cs?node(cs,ce,q&1?xm:x0,q&2?ym:y0,h,dep+1):-2;}
      return c;};
    return {build(){cnt=0;let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;for(let i=0;i<n;i++){P[i]=i;if(X[i]<x0)x0=X[i];if(X[i]>x1)x1=X[i];if(Y[i]<y0)y0=Y[i];if(Y[i]>y1)y1=Y[i];}node(0,n,x0,y0,Math.max(x1-x0,y1-y0)+1e-6,0);},
      repel(VX,VY,kr,fmax,rnd){const c_=C,s0=S0,s1=S1,m_=M,mx_=MX,my_=MY,sz_=SZ,bx_=BX,by_=BY,dm2=(kr/fmax)*(kr/fmax);
        for(let i=0;i<n;i++){const xi=X[i],yi=Y[i];let fx=0,fy=0,sp=0;stack[sp++]=0;
          while(sp){const c=stack[--sp];
            if(c_[c]===-1){for(let p=s0[c],e=s1[c];p<e;p++){if(P[p]===i)continue;let dx=xi-PX[p],dy=yi-PY[p];let d2=dx*dx+dy*dy;if(d2<0.01){dx=(rnd()-0.5)*0.2;dy=(rnd()-0.5)*0.2;d2=dx*dx+dy*dy;}const f=d2>dm2?kr/d2:fmax/Math.sqrt(d2);fx+=dx*f;fy+=dy*f;}continue;}
            const dx=xi-mx_[c],dy=yi-my_[c],d2=dx*dx+dy*dy,s=sz_[c];
            if(s*s<TH2*d2&&(xi<bx_[c]||xi>bx_[c]+s||yi<by_[c]||yi>by_[c]+s)){const f=m_[c]*(d2>dm2?kr/d2:fmax/Math.sqrt(d2));fx+=dx*f;fy+=dy*f;continue;}
            const f0=c_[c];for(let q=0;q<4;q++){const ch=c_[f0+q];if(ch>=0&&sp<4096)stack[sp++]=ch;}}
          VX[i]+=fx;VY[i]+=fy;}}};}
  /* spatial hash: cells of a given size chained through next[]. pairs() lists every pair of bodies in
     the same or a touching cell once, into PI and PJ; pairs(A) lists only the pairs with a body of the
     active set A. */
  function hash(n){let size=1;while(size<2*n)size*=2;const mask=size-1,head=new Int32Array(size),next=new Int32Array(n),CX=new Int32Array(n),CY=new Int32Array(n);
    const o={PI:new Int32Array(8*n+64),PJ:new Int32Array(8*n+64),
      fill(X,Y,cw,ch,ox){head.fill(-1);for(let i=0;i<n;i++){const a=Math.floor((ox?X[i]+ox[i]:X[i])/cw),b=Math.floor(Y[i]/ch);CX[i]=a;CY[i]=b;const k=(Math.imul(a,73856093)^Math.imul(b,19349663))&mask;next[i]=head[k];head[k]=i;}},
      pairs(A){let np=0;const F=A?A.F:null,L=A?A.L:null,cnt=A?A.len:n;for(let t=0;t<cnt;t++){const i=L?L[t]:t,a=CX[i],b=CY[i];for(let da=-1;da<=1;da++)for(let db=-1;db<=1;db++){const ca=a+da,cb=b+db;
        for(let j=head[(Math.imul(ca,73856093)^Math.imul(cb,19349663))&mask];j>=0;j=next[j]){if((F?j===i||(F[j]&&j<i):j<=i)||CX[j]!==ca||CY[j]!==cb)continue;
          if(np===o.PI.length){const g=x=>{const y=new Int32Array(x.length*2);y.set(x);return y;};o.PI=g(o.PI);o.PJ=g(o.PJ);}o.PI[np]=i;o.PJ[np++]=j;}}}return np;}};
    return o;}
  /* the set of marks a separation pass checks: every mark at first, then the marks the pass before moved */
  function active(n){const A={F:new Uint8Array(n).fill(1),G:new Uint8Array(n),L:new Int32Array(n).map((_,i)=>i),len:n,
      next(){let c=0;for(let i=0;i<n;i++){A.F[i]=A.G[i];if(A.G[i])A.L[c++]=i;A.G[i]=0;}A.len=c;}};return A;}
  /* two marks closer than both radii plus the gap move apart; a separation pass also nudges marks that
     sit on the same spot, notes the marks it moved in the active set A and reports whether anything moved */
  function collide(hs,X,Y,R,gap,cell,pass,A){hs.fill(X,Y,cell,cell);const np=hs.pairs(A),PI=hs.PI,PJ=hs.PJ,G=A?A.G:null;let moved=false;
    for(let p=0;p<np;p++){const i=PI[p],j=PJ[p];const mm=R[i]+R[j]+gap;let dx=X[i]-X[j],dy=Y[i]-Y[j];if(dx>mm||dx<-mm||dy>mm||dy<-mm)continue;
      let d2=dx*dx+dy*dy;if(d2>=(mm-0.01)*(mm-0.01))continue;if(d2<0.01){if(!pass)continue;dx=0.1*(j%2?1:-1);dy=0.1;d2=0.02;}const d=Math.sqrt(d2);const q=(mm-d)/d*0.5;X[i]+=dx*q;Y[i]+=dy*q;X[j]-=dx*q;Y[j]-=dy*q;moved=true;if(G){G[i]=1;G[j]=1;}}
    if(A)A.next();return moved;}
  /* the box of a mark and its label (the mark plus the label to its right, at least one text line
     tall) stays clear of every other box. An overlap is undone along the axis that needs the shorter
     move, which for a wide label is mostly up or down. */
  function separateLabels(nodes,hs,X,Y,R,gap,passes){const n=nodes.length;const BW=new Float64Array(n),BH=new Float64Array(n),BO=new Float64Array(n);let mw=1,mh=1;
    nodes.forEach((v,i)=>{BH[i]=Math.max(R[i],8)+gap;BW[i]=R[i]+v.lw/2+gap;BO[i]=v.lw/2;if(BW[i]>mw)mw=BW[i];if(BH[i]>mh)mh=BH[i];});
    for(let p=0;p<passes;p++){let moved=false;hs.fill(X,Y,2*mw,2*mh,BO);const np=hs.pairs(),PI=hs.PI,PJ=hs.PJ;
      for(let q=0;q<np;q++){const i=PI[q],j=PJ[q];const dx=(X[i]+BO[i])-(X[j]+BO[j]),dy=Y[i]-Y[j],ox=BW[i]+BW[j]-Math.abs(dx),oy=BH[i]+BH[j]-Math.abs(dy);if(ox<=0.01||oy<=0.01)continue;
        if(oy<=ox){const s=(dy<0||(dy===0&&j%2)?-1:1)*oy/2;Y[i]+=s;Y[j]-=s;}else{const s=(dx<0?-1:1)*ox/2;X[i]+=s;X[j]-=s;}moved=true;}
      if(!moved)break;}}
}
if(typeof WorkerGlobalScope!=='undefined'&&typeof self!=='undefined'&&self instanceof WorkerGlobalScope){
  self.onmessage=e=>{let out;try{out=kurasiGraphCompute(e.data);}catch(err){out={key:e.data&&e.data.key,error:String(err&&err.message||err)};}self.postMessage(out);};
}
