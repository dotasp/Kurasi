/* Writes src-tauri/installer/THIRD_PARTY_LICENSES.txt: the licence of every Rust crate linked into
   kurasi.exe on Windows (normal dependencies of the workspace, build and dev dependencies left out),
   with the licence texts each crate ships. build.ps1 copies the file into the installer and the
   portable zip. Run it again after Cargo.lock changes:
     node src-tauri\third_party.mjs
   It reads `cargo metadata`, which fetches any crate missing from the Cargo registry cache, and takes
   the licence files from the crate sources in that cache. installer/third_party_static.txt holds the
   notices of the components outside Cargo.lock (Rust standard library with the crates compiled into
   it, WebView2 SDK loader, Tauri and NSIS parts of the installer with the crates inside
   nsis_tauri_utils.dll); the script appends them as the last section and exits with code 1
   when that file is missing or an entry lacks a field. It also warns about every crate whose source
   folder holds prebuilt .lib, .dll, .a or .so files and lists those crates in the output. */
import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';

const HERE=path.dirname(fileURLToPath(import.meta.url));
const ROOT=path.resolve(HERE,'..');
const OUT=path.join(HERE,'installer','THIRD_PARTY_LICENSES.txt');
const meta=JSON.parse(execFileSync('cargo',['metadata','--format-version','1','--filter-platform','x86_64-pc-windows-msvc'],{cwd:ROOT,maxBuffer:256*1024*1024,encoding:'utf8'}));

/* crates reachable from the workspace members through normal dependencies */
const pkgs=new Map(meta.packages.map(p=>[p.id,p]));const nodes=new Map(meta.resolve.nodes.map(n=>[n.id,n]));
const ws=new Set(meta.workspace_members);const seen=new Set();const stack=[...ws];
while(stack.length){const id=stack.pop();if(seen.has(id))continue;seen.add(id);const n=nodes.get(id);if(!n)continue;
  for(const d of n.deps)if(d.dep_kinds.some(k=>k.kind===null))stack.push(d.pkg);}
const crates=[...seen].filter(id=>!ws.has(id)).map(id=>pkgs.get(id)).sort((a,b)=>a.name.localeCompare(b.name)||a.version.localeCompare(b.version));

/* Components outside Cargo.lock, read before anything is written so a broken file leaves the
   previous output in place. Leading # lines are comments; each entry is a title between two lines
   of 78 hyphens, then the Version, Ships in and Licence lines, then the licence text. */
const STATIC=path.join(HERE,'installer','third_party_static.txt');const SEP='-'.repeat(78);
let statics;try{statics=fs.readFileSync(STATIC,'utf8');}catch(e){console.error(`${STATIC} is missing: ${e.message}`);process.exit(1);}
const rustc=execFileSync('rustc',['--version'],{cwd:ROOT,encoding:'utf8'}).trim();
statics=statics.replace(/\r\n/g,'\n').replace(/^(#[^\n]*\n)+/,'').replace(/\{\{rustc\}\}/g,rustc).trim();
const parts=statics.split(new RegExp(`^${SEP}\\n(.+)\\n${SEP}\\n`,'m'));const staticTitles=[];const staticBad=[];
if(parts[0].trim())staticBad.push('text before the first entry');
for(let i=1;i<parts.length;i+=2){staticTitles.push(parts[i]);const miss=['Version','Ships in','Licence'].filter(f=>!new RegExp(`^${f}: `,'m').test(parts[i+1]));if(miss.length)staticBad.push(`${parts[i]}: no ${miss.join(', ')} line`);}
if(!staticTitles.length)staticBad.push('no entry');
if(staticBad.length){console.error(`${STATIC}: ${staticBad.join('; ')}`);process.exit(1);}

/* Crates of Cargo.lock (those cargo resolves for the Windows target) whose source folder holds
   prebuilt binaries. A crate named with its version in third_party_static.txt counts as covered. */
const BIN=/\.(lib|dll|a|so)(\.\d+)*$/i;const prebuilt=[];
for(const p of meta.packages){if(ws.has(p.id))continue;const dir=path.dirname(p.manifest_path);let files=[];
  try{files=fs.readdirSync(dir,{recursive:true,withFileTypes:true}).filter(e=>e.isFile()&&BIN.test(e.name)).map(e=>path.relative(dir,path.join(e.parentPath??e.path,e.name))).sort();}catch(e){}
  if(files.length)prebuilt.push({p,files,linked:seen.has(p.id),covered:statics.includes(`${p.name} ${p.version}`)});}
prebuilt.sort((a,b)=>a.p.name.localeCompare(b.p.name)||a.p.version.localeCompare(b.p.version));
const preLine=({p,files,linked,covered})=>`${p.name} ${p.version}, ${linked?'dependency of kurasi.exe':'build or dev dependency'}: `+
  files.slice(0,2).join(', ')+(files.length>2?` and ${files.length-2} more`:'')+(covered?'; entry under "Components outside Cargo.lock"':'');
for(const x of prebuilt)console.log(`WARNING: prebuilt binaries in ${preLine(x)}`);

/* Kurasi takes each crate under the first licence of this list that the crate offers; every part
   of an AND expression is kept */
const PREFER=['MIT','Apache-2.0','BSD-3-Clause','BSD-2-Clause','ISC','Zlib','0BSD','MIT-0','Unlicense','BSL-1.0','CC0-1.0','Unicode-3.0','MPL-2.0'];
function chosen(expr){if(!expr)return [];const parts=[];let depth=0,cur='';const s=expr.replace(/\//g,' OR ');
  for(let i=0;i<s.length;i++){const c=s[i];if(c==='(')depth++;if(c===')')depth--;if(depth===0&&s.startsWith(' AND ',i)){parts.push(cur);cur='';i+=4;continue;}cur+=c;}parts.push(cur);
  return parts.map(p=>{const alts=p.replace(/[()]/g,'').split(/\s+OR\s+/).map(x=>x.trim()).filter(Boolean);return alts.slice().sort((a,b)=>rank(a)-rank(b))[0];});}
const rank=id=>{const i=PREFER.indexOf(id);return i<0?PREFER.length:i;};
const FILE_HINT={'MIT':/mit/i,'Apache-2.0':/apache/i,'BSD-3-Clause':/bsd/i,'BSD-2-Clause':/bsd/i,'Zlib':/zlib/i,'Unlicense':/unlicense/i,'Unicode-3.0':/unicode/i,'MPL-2.0':/mpl/i,'ISC':/isc/i,'BSL-1.0':/bsl|boost/i,'CC0-1.0':/cc0/i,'0BSD':/0bsd/i,'MIT-0':/mit/i};
const isLicFile=f=>/^(licen[cs]e|copying|unlicense)/i.test(f);
/* the files named after the chosen licences; when one licence has no file of its own name, every
   other licence file of the crate is taken too (extra texts do no harm) */
function filesFor(p,ids){const dir=path.dirname(p.manifest_path);let names=[];try{names=fs.readdirSync(dir).filter(f=>isLicFile(f)&&fs.statSync(path.join(dir,f)).isFile());}catch(e){}
  if(p.license_file){const f=path.resolve(dir,p.license_file);if(fs.existsSync(f))return {texts:[f],missing:[]};}
  const texts=new Set(),unnamed=[];
  for(const id of ids){const hit=names.filter(f=>FILE_HINT[id]&&FILE_HINT[id].test(f));if(hit.length)hit.forEach(f=>texts.add(f));else unnamed.push(id);}
  if(unnamed.length){const rest=names.filter(f=>!ids.some(id=>FILE_HINT[id]&&FILE_HINT[id].test(f)));rest.forEach(f=>texts.add(f));}
  return {texts:[...texts].map(f=>path.join(dir,f)),missing:names.length?[]:ids};}
const noticeOf=p=>{const dir=path.dirname(p.manifest_path);try{return fs.readdirSync(dir).filter(f=>/^(notice|copyright)/i.test(f)).map(f=>path.join(dir,f));}catch(e){return [];}};

/* identical texts are printed once, with every crate that ships them */
const groups=new Map();const rows=[];const mpl=[];const gaps=[];
for(const p of crates){const ids=chosen(p.license);const {texts,missing}=filesFor(p,ids);const notices=noticeOf(p);
  rows.push(`${p.name} ${p.version}`.padEnd(44)+(p.license||'see licence file'));
  if(ids.includes('MPL-2.0'))mpl.push(p);
  for(const f of [...texts,...notices]){const t=fs.readFileSync(f,'utf8').replace(/\r\n/g,'\n').trim();const h=crypto.createHash('sha256').update(t).digest('hex');
    const g=groups.get(h)||{t,crates:[]};g.crates.push(`${p.name} ${p.version}`);groups.set(h,g);}
  if(missing.length)gaps.push({p,missing});}
/* Texts for crates that ship no licence file. Apache-2.0 and MPL-2.0 carry no copyright line, so
   the text another crate ships verbatim serves. MIT and BSD-3-Clause are the standard texts with
   the authors from the crate's Cargo.toml as copyright holders. */
const shipped=re=>{let best=null;for(const g of groups.values())if(re.test(g.t)&&(!best||g.t.length<best.length))best=g.t;return best;};
const holders=p=>`the ${p.name} authors${p.authors&&p.authors.length?' ('+p.authors.map(a=>a.replace(/\s*<[^>]*>/,'')).join(', ')+')':''}`;
const TEMPLATE={
 'MIT':p=>`MIT License\n\nCopyright (c) ${holders(p)}\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the "Software"), to deal\nin the Software without restriction, including without limitation the rights\nto use, copy, modify, merge, publish, distribute, sublicense, and/or sell\ncopies of the Software, and to permit persons to whom the Software is\nfurnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\nOUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\nSOFTWARE.`,
 'BSD-3-Clause':p=>`Copyright (c) ${holders(p)}\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice, this\n   list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must reproduce the above copyright notice,\n   this list of conditions and the following disclaimer in the documentation\n   and/or other materials provided with the distribution.\n\n3. Neither the name of the copyright holder nor the names of its\n   contributors may be used to endorse or promote products derived from\n   this software without specific prior written permission.\n\nTHIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"\nAND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE\nIMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE\nDISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE\nFOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL\nDAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR\nSERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER\nCAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,\nOR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE\nOF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`,
 'Apache-2.0':()=>shipped(/Apache License\s+Version 2\.0, January 2004/),
 'MPL-2.0':()=>shipped(/Mozilla Public License Version 2\.0/)};
const line='='.repeat(78);
let out=`Third-party licences for Kurasi ${meta.packages.find(p=>ws.has(p.id)&&p.name==='kurasi').version}\n\n`+
`Kurasi itself is under the MIT License (LICENSE.txt next to this file). kurasi.exe links the ${crates.length} Rust crates\n`+
`listed below. Each is used under the licence named in the second column; where a crate offers a\n`+
`choice, Kurasi uses the first of: ${PREFER.slice(0,4).join(', ')}. The texts follow the list.\n\n`+
`pdfium.dll is a separate file. Its licences are in the folder licenses\\pdfium.\n`+
`The installer carries Microsoft's WebView2 Evergreen Bootstrapper (MicrosoftEdgeWebview2Setup.exe).\n`+
`When the WebView2 Runtime is missing, the bootstrapper installs it under Microsoft's terms.\n`+
`The last section, "Components outside Cargo.lock", lists the ${staticTitles.length} components of kurasi.exe and\n`+
`of the installer that are absent from Cargo.lock, with their licence texts.\n\n`+line+`\nCrates\n`+line+`\n`+rows.join('\n')+'\n';
if(mpl.length)out+=`\n${line}\nSource of the MPL-2.0 crates\n${line}\nThese crates are used unmodified. Their source code is available from crates.io:\n`+mpl.map(p=>`  ${p.name} ${p.version}: https://crates.io/crates/${p.name}/${p.version}`).join('\n')+'\n';
if(prebuilt.length)out+=`\n${line}\nCrates with prebuilt binaries in their source\n${line}\nThe source folders of these crates hold prebuilt .lib, .dll, .a or .so files:\n`+prebuilt.map(x=>'  '+preLine(x)).join('\n')+'\n';
out+=`\n${line}\nLicence texts\n${line}\n`;
for(const g of groups.values())out+=`\n${'-'.repeat(78)}\nUsed by: ${g.crates.join(', ')}\n${'-'.repeat(78)}\n\n${g.t}\n`;
const unfilled=[];
if(gaps.length){out+=`\n${line}\nCrates that ship no licence file: the standard text of their licence\n${line}\n`;
  for(const {p,missing} of gaps)for(const id of missing){const t=TEMPLATE[id]&&TEMPLATE[id](p);if(!t)unfilled.push(`${p.name} ${p.version} (${id})`);
    out+=`\n${'-'.repeat(78)}\n${p.name} ${p.version}, ${id}. Source: https://crates.io/crates/${p.name}/${p.version}\n${'-'.repeat(78)}\n\n${t||`See https://spdx.org/licenses/${id}.html`}\n`;}}
out+=`\n${line}\nComponents outside Cargo.lock\n${line}\nThese components ship with Kurasi and are absent from Cargo.lock. Each entry names the version,\n`+
`the shipped files that contain the component, the licence and its full text.\n\n${statics}\n`;
fs.writeFileSync(OUT,out.replace(/\n/g,'\r\n'));
console.log(`${OUT}: ${crates.length} crates, ${groups.size} distinct texts, ${gaps.length} crates without a licence file, ${mpl.length} MPL-2.0, ${staticTitles.length} components outside Cargo.lock, ${prebuilt.length} crates with prebuilt binaries, ${Math.round(out.length/1024)} KB`);
if(unfilled.length){console.error('No licence text for: '+unfilled.join(', '));process.exitCode=1;}
