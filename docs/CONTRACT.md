# Kurasi build contract, v1 (17 Oct 2026)

Kurasi is a personal reference-curation desktop tool for Windows 11. It scans a project
folder, shows the files as a shop-style catalogue, lets a person highlight PDFs, Markdown
and images with an analysis per highlight, and exposes the same data to an AI agent
through an MCP server. This contract is binding for every agent that builds a part.

## 1. Layout and stack

The stack and the crate layout are set by `CONTRACT_RUST.md` sections 2 and 3. The repository
holds:

```
D:\PythonScripts\Kurasi\
  Cargo.toml     Rust workspace, version 0.4.0
  crates\        kurasi-core, kurasi-pdf, kurasi-server
  src-tauri\     kurasi.exe (CLI: app | mcp | scan | demo | export | serve), splash, window,
                 NSIS installer, build.ps1
  ui\            index.html, app.css, app.js and the other interface files; dev\ for test scripts
  docs\          CONTRACT.md, CONTRACT_RUST.md, CORE_API.md
  release\       build.ps1 output: Kurasi-<version>-Setup.exe, Kurasi-<version>-portable.zip,
                 SHA256SUMS.txt (gitignored)
  README.md
  CHANGELOG.md
```

- The Python build (package `kurasi\`, pytest suite `tests\`, `Kurasi.cmd`, `requirements.txt`,
  `pytest.ini`) was removed from the working tree on 18 Oct 2026 and stays recoverable from git
  tag `python-v0.1`.
- Python 3.14 in `D:\PythonScripts\Kurasi\.venv` serves the development helpers only:
  `ui\dev\pdfcheck.py` and the PyMuPDF cross-checks of the Rust tests (package `pymupdf`), and
  `ui\dev\mcp_client.py` (package `mcp`). Create it with
  `py -3.14 -m venv .venv`.
- Window: Tauri 2 on Edge WebView2 (`CONTRACT_RUST.md` section 4). The interface keeps its
  HTML menubar (the guided tour points at it) and uses the native title bar of the window.
- API port 7343, or the next free port. Bind 127.0.0.1 only.
- No git commits.

## 2. Storage

- App data: `%LOCALAPPDATA%\Kurasi\`
  - `projects.json`: `[{id, name, folder, created, last_opened}]`
  - `projects\<id>\kurasi.sqlite` (WAL mode), `projects\<id>\write.lock`
  - `thumbs\<sha256>.png`, `pages\<sha256>\p<n>_r<rot>_s<scale>.png` (render cache, trim to 500 MB oldest first)
  - `demo\` (demo project files), `logs\kurasi.log`
  - `trash\projects\rm-<12 hex>\`: one folder per project removal with its data (section 6,
    `DELETE /api/projects/<id>?delete_data=1`). `removed.json` in it names the registry entry and
    where each file came from; `app\` holds the database folder and `sidecar\` Kurasi's files from
    `.kurasi`. Undo moves them back. The server deletes removals older than 7 days at start.
- Sidecar in the project folder: `<folder>\.kurasi\metadata.json`, written atomically after every
  change to reference metadata, categories, tags, or Markdown and image highlights. It holds
  enough to rebuild the database on another machine: references keyed by slug with their files
  (relpath, sha256), metadata, categories, and the Markdown and image highlights. PDF highlights
  live in the PDF files themselves. Opening a project whose database is empty imports the sidecar.
- Project `read_only` flag: every write to a file inside the project folder (PDF annotations,
  rotation, sidecar, exports) raises `ReadOnlyProject` and the API answers 423. Database writes
  still happen.

## 3. Database schema (SQLite)

```
setting(key TEXT PRIMARY KEY, value TEXT)          -- JSON values
category(id INTEGER PK, parent_id INTEGER NULL, name TEXT, sort INTEGER)   -- id 1 = "Uncategorized", seeded
reference(id TEXT PK, slug TEXT UNIQUE, title, kind, year, date, pub, vol, issue, pages_range,
          doi, url, lang, usability, status, register_id, created_at, updated_at, deleted_at NULL)
reference_author(ref_id, position INTEGER, name)   -- "Surname, I." or an organisation; "Unknown" when none
reference_category(ref_id, category_id)
file(id TEXT PK, ref_id, relpath UNIQUE, ext, sha256, size, mtime, page_count, role,
     cloud_only INTEGER, missing INTEGER, missing_since, first_seen, last_seen, view_rotation TEXT)
highlight(id TEXT PK, file_id, ref_id, page INTEGER NULL, kind, quads TEXT, anchor TEXT NULL,
          verbatim, analysis, label NULL, tags TEXT, author, origin, colour, pdf_xref INTEGER NULL,
          created_at, updated_at, suggestion TEXT NULL, suggested_tags TEXT NULL)
trash(id INTEGER PK, kind, payload TEXT, sha_before NULL, sha_after NULL, deleted_at)
ai_log(id INTEGER PK, at, tool, client, purpose, args TEXT, result)
change(seq INTEGER PRIMARY KEY AUTOINCREMENT, at, what, ids TEXT, origin)
```

- IDs: `r-`, `f-`, `h-` followed by 12 lowercase hex characters.
- `reference.status`: `Auto` (read from the file at scan), `Suggested` (written by an AI through
  MCP, waits for review), `Verified` (saved by the person). UI dots: Auto amber, Suggested blue,
  Verified green.
- `reference.usability`: `Full text`, `Abstract only`, `Excerpt`, `Translated`, or empty.
- `file.role`: `primary`, `print`, `translation`, `excerpt`, `rendered`.
- `highlight.kind`: `text` (PDF words), `region` (PDF or image rectangle), `md` (Markdown text), `note`.
- `highlight.quads`: PDF: list of `[x0,y0,x1,y1]` in unrotated page points. Image: list of
  `[x,y,w,h]` normalised 0..1 on the unrotated image. Markdown: `[]`.
- `highlight.anchor` (Markdown only): `{"text": verbatim, "occurrence": n}` with n counted from 0
  among exact matches in the rendered text.
- `highlight.origin`: `human` or `ai`. `author`: `You` for UI writes, the AI author name for MCP
  writes (setting `ai_author`, default `Claude`), or the annotation title read from the PDF.
- `highlight.suggestion`: `{"action":"delete","reason":..., "at":..., "resolution":null|"accepted"|"dismissed"}`.
- `change` gets one row per write from any process. `origin` is `ui`, `mcp` or `scan`.

Settings keys: `name, folder, file_types (default [".pdf",".md",".png",".jpg",".jpeg"]),
exclude (default [".git",".kurasi",".venv","node_modules","__pycache__","~$*"]),
auto_scan_min (default 15), read_only (false), ai_authors (["claude"]), ai_author ("Claude"),
hl_colour ("#ffd84a"), loose (true), last_scan, mcp_hints (true), theme ("system")`.

## 4. Scan rules

1. Walk with `os.scandir` recursively. Skip entries matching `exclude` (fnmatch on the name).
   Do not follow junctions or symbolic links.
2. Index files whose extension is in `file_types`.
3. Cloud-only placeholder: `st_file_attributes & (0x400000 | 0x1000)` set
   (RECALL_ON_DATA_ACCESS or OFFLINE). Record relpath, size, mtime, `cloud_only=1`. Never open,
   hash or thumbnail it; opening it would make OneDrive download it.
4. Incremental: a known relpath with unchanged size and mtime is skipped. Everything else is
   hashed (SHA-256, 1 MB chunks).
5. Moved: a new path whose sha256 equals a file marked missing takes over that file row
   (relpath updated, `missing=0`). Its highlights and reference stay.
6. Changed: a known path with a new hash keeps its row; annotations are re-read.
7. Removed: `missing=1`, `missing_since` set. Nothing is deleted.
8. Grouping into one reference by slug. Slug = file name up to the first `.`, then strip a final
   `_highlighted`, `_annotated`, `_reading` or `_hl`, then a final `_excerpt` or `_excerpt-pp<range>`.
   Every file with that slug anywhere in the project joins the same reference.
   Role from the part after the first dot: empty or the plain extension: `primary`;
   `print`: `print`; `print.translated-xx` or `translated-xx`: `translation`; `rendered`:
   `rendered`; an excerpt suffix stripped in the slug step: `excerpt`.
   The file opened for a reference: most highlights; ties go translation (en), primary, print,
   excerpt, rendered.
9. A new reference starts as: title = slug, author `Unknown`, category Uncategorized,
   status `Auto`. Guesses that keep status `Auto`: PDF metadata title and author when non-empty;
   date from a slug starting `YYYY-MM-DD`; year from `_(\d{4})_`; first author surname from a slug
   matching `^([a-z]+)_\d{4}_` (capitalised).
10. PDF annotations are read on every scan of a new or changed PDF:
    Highlight, Underline, Squiggly, StrikeOut: kind `text` (or `region` when no words fall inside);
    Text and FreeText: kind `note`.
    author = `info["title"]` or `Unknown`; origin `ai` when author lower-cased is in `ai_authors`.
    analysis = `info["content"]` with a final line `Tags: #a #b` removed and parsed into tags.
    label = the first `(F)`, `(I)` or `(P)` at the start of a paragraph of the analysis.
    verbatim = words from `page.get_text("words")` whose box overlaps a quad by at least half
    its height and 30% of its width, joined by spaces.
    Replies (`irt_xref`) are appended to the parent analysis as `Reply from NAME: ...`.
    A highlight already in the database is matched by `pdf_xref`, else by page and quads within
    2 points, so edits made in the app are never duplicated.
11. Watcher: `watchdog` observer on the folder, debounced 2 s, rescans only the touched paths.
    The auto-scan timer runs a full incremental scan every `auto_scan_min` minutes (0 = off).
    File → Scan folder now runs it by hand.
12. Scan progress is reported as `{running, pct, message, log[]}`.

## 5. Writing to PDF files

- Add: `page.add_highlight_annot(quads)`, `set_info(title=author, content=compose(analysis, tags))`,
  `set_colors(stroke=hl_colour)`, `update()`. A region with no words uses one quad covering the rect.
- `compose(analysis, tags)` = analysis stripped of trailing space, then `\n\nTags: #a #b` when there are tags.
- Edit: update the content of the annotation found by `pdf_xref`.
- Delete: snapshot the annotation (type, page, quads, colours, info, xref) into `trash`, delete it,
  verify it is gone.
- Rotate: `page.set_rotation((current + 90) % 360)` on one page or every page.
- Save protocol: open, apply, save to a temp file in the same folder
  (`.~kurasi-<name>.tmp`), reopen it to verify, then `os.replace` with 6 tries and a delay
  doubling from 0.1 s (OneDrive and Acrobat hold files). On success update `file.sha256`,
  `size`, `mtime` so the next scan does not see an outside change. On failure the API answers
  409 with "The file is open in another program. Close it and try again."
- Every project-file write and every multi-row database write holds `write.lock`
  (`msvcrt.locking`). The app and the MCP server both use it.
- Display colour is chosen by the UI from `origin` (human: yellow token, ai: blue token). The colour
  stored in the PDF is `hl_colour` for both, so other PDF readers show ordinary highlights.

## 6. HTTP API (JSON, 127.0.0.1 only)

Reject a request whose Host header is not `127.0.0.1:<port>` or `localhost:<port>`, or whose
Origin header is present and differs. Errors: `{"error": "<one plain sentence>"}` with 400, 404,
409 or 423.

Compression: a response of 1 KB or more whose type is JSON, `text/*`, JavaScript or SVG is sent
gzip-compressed when the request carries `Accept-Encoding: gzip` (WebView2 and Chrome do), with
`Vary: Accept-Encoding`. Page images, thumbnails, PDF bytes and other binary bodies are sent as
they are.

```
GET    /                                 ui/index.html (and /ui/... static files)
GET    /api/state                        {project:{id,name,folder,read_only,auto_scan_min,last_scan,file_types,exclude},
                                          projects:[...], settings:{...}, counts:{references,authors,tags,highlights},
                                          change_seq, demo:bool, version} (version from Cargo.toml, shown in About)
GET    /api/references                   [{id,title,authors[],kind,pub,year,date,lang,status,register_id,
                                           categories:["Academic/HCM dimension/Employee engagement",...],
                                           ext,pages,hl_count,top_tags[],missing,
                                           files:[{id,relpath,role,ext,pages,missing,cloud_only}],
                                           primary_file_id,thumb_url,created_at,updated_at}]
                                          the catalogue list holds the fields the UI draws from
GET    /api/references?ids=<id>,<id>     the same list for those references only; an id that is unknown or in the
                                          trash is left out (the UI's partial refresh, section 8)
GET    /api/reference/<id>               the full record: the list fields plus slug, usability, tags[], hl_count per
                                          file, doi, url, vol, issue, pages_range and search_text (highlight verbatim +
                                          analysis per highlight, one per line, max 20000 chars)
GET    /api/search?q=<query>&loose=1|0   {query,loose,tokens[],hits:[{id,score,scores[]}]}  full-text search over
                                          search_text with the UI rules (section 8): scores[i] belongs to tokens[i],
                                          score is their sum, hits sorted by score then id; without loose the
                                          loose setting applies
PUT    /api/reference/<id>               body: any metadata fields, authors[], categories[] (paths); status becomes
                                          "Verified" unless the body sets it
DELETE /api/reference/<id>               moves it to trash (files untouched) → {trash_id}
POST   /api/undo                         {trash_id} → restores a reference or a highlight
GET    /api/categories                   [{id,name,path,children:[...]}]
POST   /api/categories                   {parent_id,name}      PUT /api/categories/<id> {name,parent_id}
DELETE /api/categories/<id>              references move to the parent
GET    /api/authors                      [{name, ref_ids[], hl_count, top_ref_id}]
GET    /api/tags                         [{name, ref_ids[], hl_count}]
GET    /api/file/<fid>/info              {ext, pages, page_sizes:[[w,h],...] (points, unrotated), rotation:[deg per page]}
GET    /api/file/<fid>/page/<n>.png?scale=1.5   page n (1-based) rendered with annots=False, page rotation applied
GET    /api/file/<fid>/page/<n>/words    {width,height,rotation,words:[[x0,y0,x1,y1,"text"],...]}  unrotated points
GET    /api/file/<fid>/raw               file bytes with the right content type (Markdown, images)
GET    /api/thumb/<fid>.png              first page or image, 120 px wide; 404 for Markdown (UI draws a CSS thumb)
GET    /api/highlights?ref_id=&file_id=  [{id,file_id,ref_id,page,kind,quads,anchor,verbatim,analysis,label,tags[],
                                           author,origin,created_at,updated_at,suggestion,suggested_tags[]}]
POST   /api/highlights                   {file_id,page,kind,quads|anchor,verbatim,analysis,label,tags[]} author "You"
PUT    /api/highlights/<id>              {analysis,label,tags[],verbatim}
DELETE /api/highlights/<id>              → {trash_id}
POST   /api/highlights/<id>/suggestion   {resolution:"accept"|"dismiss"}   accept on a delete suggestion deletes
POST   /api/highlights/<id>/accept-tag   {tag}
POST   /api/file/<fid>/rotate            {page: n | null for all, delta: 90}  PDF: written to the file; image or md: view_rotation
POST   /api/scan                         {full:bool} → starts; GET /api/scan/status → {running,pct,message,log[],result}
POST   /api/project                      {name,folder,file_types,auto_scan_min} → creates, opens, starts a scan
POST   /api/project/open                 {id}
POST   /api/project/check                {folder} → {ok:true, folder, name, project_id?, project_name?} when the path
                                          is a readable directory (name = the folder name; project_id when the
                                          folder is in the list already), else {ok:false, folder, error}
GET    /api/projects                     [{id,name,folder,created,last_opened,demo,exists,open}] sorted by
                                          last_opened, most recent first (also the projects of /api/state)
GET    /api/projects/<id>                the list record plus data:{app:{path,files[],whole}|null,
                                          sidecar:{path,files[],whole}|null}: what "Also delete Kurasi's data"
                                          would remove. app is the database folder in the app home; sidecar is
                                          the project's .kurasi with Kurasi's files only (metadata.json,
                                          AI_USE_LOG.md); whole means the folder holds nothing else
DELETE /api/projects/<id>[?delete_data=1]  takes the project off the list → {removal_id, project, with_data,
                                          moved[], closed}. closed is true when it was the open project, which
                                          is closed first. With delete_data the folders of data go to
                                          trash\projects\<removal_id>\ (section 2) after the safety gate:
                                          Kurasi refuses (409) any target outside
                                          <app home>\projects\p-<12 hex> or <folder>\.kurasi, a drive root, a
                                          target that holds the project folder or the app home, a link, a
                                          database folder with foreign files or without Kurasi's schema, and a
                                          .kurasi without a valid metadata.json. A read-only project keeps its
                                          .kurasi. _exports\ and every other file of the person stay. A held
                                          database (an MCP session) answers 409 and changes nothing. The sample
                                          project answers 400.
POST   /api/projects/restore             {removal_id} → {project}: Undo. Puts the entry back at its place in
                                          the list and moves its data back; the project stays closed
POST   /api/projects/<id>/reset          sample project only → {project, open}: rebuilds it from scratch and
                                          reopens it when it was open
POST   /api/demo                         {reset:bool} → builds or resets the demo project and opens it
PUT    /api/settings                     {key: value, ...}
GET    /api/changes?since=<seq>          {seq, changes:[{seq,what,ids,origin}]}
GET    /api/ai-log                       rows newest first
POST   /api/ai-log/export                writes <folder>\.kurasi\AI_USE_LOG.md as | Date | Tool | Purpose | Checks performed by the author | → {path}
POST   /api/export                       {format:"apa-md"|"bibtex"|"csl-json"|"highlights-json", ref_ids[]} → {path} under <folder>\_exports\
POST   /api/open                         {file_id | relpath | url, mode:"default"|"reveal"}  os.startfile or explorer /select,
GET    /api/resolve?href=<link>&from=<fid>  {file_id, ref_id} when the link resolves to a project file, else {external:true}
GET    /api/mcp                          {command, tools:[{name,description,mode}], resources:[...]}
```

Folder picker: inside the desktop window the page calls `window.__TAURI__.dialog.open({directory:
true})` of tauri-plugin-dialog, the Windows select-folder dialog (capability `dialog:allow-open`,
`CONTRACT_RUST.md` section 2). The chosen path then goes through `POST /api/project/check`. A
plain browser has no picker, and the folder path is typed or pasted.

## 7. MCP server

Start: `kurasi.exe mcp --project "<folder>" [--mode write|propose|read]` over stdio. The mode
is `write` by default: the agent may use every tool below. `propose` and `read` are stricter
options. Under `propose` the agent may only suggest changes (the propose and read rows), and
under `read` it may only look. A tool above the mode is refused with a message that names the
mode. `highlight_add` stays a write tool. Built with `rmcp` (`CONTRACT_RUST.md` section 2).
Tool names use underscores.

The mode is set once, at registration, by the `--mode` argument of the registered command.
Tools → MCP server shows the line to copy, for user scope and write mode:
`claude mcp add kurasi -s user -- "<exe folder>\kurasi.exe" mcp --project "<folder>" --mode write`
To register a stricter server, replace `write` with `propose` or `read` in that line, after
removing the old registration with `claude mcp remove kurasi -s user`.

| Tool | Mode | Does |
| --- | --- | --- |
| `catalog_search(query, limit=20)` | read | loose search over metadata, tags and highlights |
| `reference_get(ref_id)` | read | metadata, files, highlight count |
| `reference_text(ref_id, page, file_id=None)` | read | page text plus word boxes |
| `highlights_list(ref_id=None, tag=None, author=None)` | read | highlights |
| `tags_list()`, `categories_list()` | read | vocabularies |
| `reference_update_metadata(ref_id, fields, authors=None, categories=None, purpose="")` | propose | applies, status `Suggested`; previous values kept in the ai_log args |
| `reference_categorize(ref_ids, category_paths, purpose="")` | propose | adds categories (creates missing paths), status `Suggested` |
| `highlight_propose_delete(highlight_id, reason)` | propose | sets `suggestion` |
| `highlight_suggest_tags(highlight_id, tags)` | propose | sets `suggested_tags` |
| `catalog_scan(full=False)` | write | runs a scan |
| `highlight_add(ref_id, analysis, label, tags, page=None, quote=None, quads=None, file_id=None, purpose="")` | write | PDF: finds `quote` on the page (whitespace-normalised, case-sensitive first, then case-insensitive) and builds quads from its words, or uses `quads`; fails with a clear message when the quote is not found. Markdown: `quote` becomes the anchor. Author = `ai_author`. |
| `export_references(format, ref_ids=None)` | write | same as the export route |
| `log_append(purpose, checks)` | write | adds a row to the AI-use log |

Every tool call writes an `ai_log` row (tool, client name, purpose, arguments, result summary)
and, when it writes, a `change` row with origin `mcp`. The client name comes from
`_meta["io.modelcontextprotocol/clientInfo"]` of the request (protocol 2026-07-28, opened with
`server/discover`), else from the `clientInfo` of `initialize` (older protocols), else it is
`unknown client`.
Resources: `kurasi://references`, `kurasi://reference/{ref_id}`,
`kurasi://reference/{ref_id}/highlights`, `kurasi://tags`.

## 8. UI

Parts: tokens and both themes, menubar and menus, search and sort row, Reference / Authors / Tag
views, closable document tabs, filter pane (max 15%) with Reference, Author and Tag trees,
end-truncated labels (75 letters), dense reference cards, author cards, tag graph, edit pane
(30%), knowledge pane (30%) with collapsible verbatim, analysis, tags, label, Save, Copy
citation, the highlight dialog, context menus, toasts, Undo, guided tour and its coach marks,
status bar.

Rules:
- Reference, Authors and Tag sit in a segmented switch at the start of the tab strip (the active
  view filled with the accent colour). Open documents follow it as closable file tabs with a
  type badge, so the two kinds of tab look different.
- The Reference and Authors views scroll vertically and have no pages. The grid renders about
  two screens of cards and adds the next chunk when the scroll comes within 400 px of the end.
  A redraw of the same list (same view, sort, search and filters, for example after a live
  refresh) keeps the cards shown and the scroll position; another list starts at the top. The
  count of references or authors stands at the right of the row above the grid.
- Search (rules revised 18 Oct 2026): the query splits at whitespace into lowercase tokens. A
  field scores 1 for a token it contains; otherwise it scores 0.9 times the best bigram Dice
  coefficient between the token and its words (split at every character other than a letter, a
  digit or a hyphen) when that reaches 0.42 (loose, the default) or 0.72 (strict). Field
  weights: title 3, authors 2, tags 2, publication 1, register ID 1, highlight text 1,
  categories 1. A reference scores the sum over the tokens of its best weighted field and drops
  out when one token scores 0. The page scores the metadata fields itself and shows those
  matches at once. The highlight text stays on the server: `/api/search` scores it with the same
  rules, the page asks 90 ms after the query settles, and the catalogue draws again with the
  higher of the two scores per token. A newer query aborts the request in flight and an answer
  for an older query is dropped. The ranking equals that of scoring every field in the page.
- Search chips (added 18 Oct 2026): Enter in the search box, or the + button beside it ("Add as
  filter"), keeps the text as a chip in the filter bar and empties the box. Blank text adds
  nothing. A chip equal to one already there, letter case aside, is left out and the box still
  empties. Each chip has a remove control; Clear all filters removes the chips too. A reference
  must match every chip and the text still in the box, each part scored with the rules above at
  the current looseness. Ranking rule: a reference scores the sum of its part scores, which
  equals the score of one query holding the words of every part; equal scores keep the order
  of the list from the server. The page asks `/api/search` once per part text, with the same
  90 ms settle. A text no longer wanted (an older query, a removed chip, another project) has
  its request aborted and its answer dropped. An answer stays cached per project and per text
  until a data change asks again, so a chip made from the box reuses the answer the box had.
  The chips count in the filter signature, so the tag graph and the kept scroll place follow
  them.
- Filter pane finder and order (added 18 Oct 2026): a small box at the top of the filter pane
  narrows the Author and Tag entries of the pane. Every word typed must sit inside the name or
  come near one of its words (bigram Dice of 0.42 or more; names split at every character other
  than a letter or a digit). Ticked entries stay in sight. The finder leaves the catalogue and
  the active filters alone, Enter in it does nothing, and a small clear control empties it.
  Pane entries run A to Z: categories by name at every level with Uncategorized first, authors
  by surname within People and by name within Organisations with Unknown last, tags by name.
  Every text sort in the pane and the catalogue uses `localeCompare` with no options, which V8
  runs on a fast path; names it holds equal fall back to code order, and ISO timestamps sort in
  code order. Pane rows carry `content-visibility: auto`, so rows out of sight skip layout and
  paint.
- Data comes from the API. No SAMPLE, POOL, MD or MCP_LOG constants remain in the shipped UI.
- Scan dialog shows the real progress from `/api/scan/status`.
- PDF viewer: continuous scroll over every page; each page an `<img>` from `/page/<n>.png` at
  scale = zoom x devicePixelRatio, lazy-loaded with IntersectionObserver; word boxes from
  `/words` placed as transparent spans for drag selection; highlight overlays drawn from quads,
  coloured by origin; rotation from the server. Ctrl + wheel zoom, Fit width, Hand pan,
  right-click page menu for rotate one page or all pages.
- Markdown: rendered with the renderer in `ui\viewer.js` or a vendored, pinned `marked`; Raw mode;
  selection with the Highlight tool creates an `md` highlight; links go through `/api/resolve`
  (project file: new tab; anything else: `/api/open` with the default app).
- Image: one image per tab, region highlights, zoom, pan, view rotation.
- "Ask Claude" buttons become "Copy prompt for Claude": they copy a ready prompt that names the
  MCP tool and the reference ID. The app cannot start an agent itself.
- Live refresh: poll `/api/changes` every 2 s; refetch what changed; a change with origin `mcp`
  shows a toast "Claude via MCP: <what>". When every change of a batch names reference IDs, the
  page fetches `/api/references?ids=` for those references (at most 200), puts the answers in
  place and drops the references the answer leaves out. A scan, an import, an undo or restore,
  a project switch, the sample project build and a change without reference IDs fetch the whole
  list. Tags (`/api/tags`) and the highlights of open documents are fetched after every change.
  Writes made in the page follow the same rules: Save in the edit pane and Delete reference
  refresh their reference, highlight writes refresh the reference they belong to, Undo and
  category writes fetch the whole list.
- First run with no project: welcome dialog with "Guided tour" (POST /api/demo, then the tour) and
  "Open a folder" (New project dialog). Its recent list shows five projects; with more, the
  button "All N projects…" opens the Projects dialog. Help → Guided tour always runs on the demo project.
  The Help menu holds the onboarding entries: "Guided tour", "Open sample project" (no tour),
  "Reset sample project" (POST /api/demo {reset:true}). Leaving the tour returns to the project
  that was open before it.
- The tour is data-agnostic: no fixed reference IDs or titles. Steps find their targets in the
  loaded data (the first Markdown reference, the first image reference, the first author card).
  The loose-search step builds its typo from the data: a word of 7 or more letters from the title
  of the most highlighted reference, with one inner letter dropped.
- New project dialog: Browse opens the Windows folder picker (section 6) in the desktop window;
  in a plain browser the folder path is typed or pasted. Every path, picked or typed, goes
  through `POST /api/project/check` (350 ms after typing stops, and on Create): the dialog says
  why a path fails (missing, a file, unreadable) and fills the project name from the folder
  name until the person types a name.
- Projects dialog (File → Projects…, Ctrl+O; replaces Open project): a search box over name and
  folder path (case-insensitive, every word must match), and a scrolling list sorted by last
  opened, most recent first, built for hundreds of projects. Each row shows the name, the path
  and when it was last opened, with pills for the open project, the sample and a folder that
  cannot be reached. Up and Down (Page Up and Page Down by eight) move the selection, Enter
  opens, Esc closes, and typing anywhere in the dialog goes to the search box. The footer holds
  "Open new folder…": the folder picker, then the New project dialog with the path and name
  filled in (the typed path in a plain browser).
- Remove (per row): a confirmation with one checkbox, unticked by default, "Also delete Kurasi's
  data for this project". Ticked, it lists the exact folders that would go (from
  `GET /api/projects/<id>`). The person's own files are never touched. Removing the open project
  closes it and shows the welcome dialog. A notice with Undo stays 15 s and calls
  `POST /api/projects/restore`; Undo of the open project opens it again. The sample project row
  shows Reset instead (`POST /api/projects/<id>/reset`).

## 9. Demo project (`crates\kurasi-core\src\demo.rs`): sterile sample (revised 17 Oct 2026)

The sample carries no data from any real project of the user. It is built from public-domain
classics of work and organisation with real bibliographic metadata, and generated placeholder
body text whose sentences explain Kurasi features (the highlights quote those sentences). Page 1
of every PDF carries the line "Sample file generated by Kurasi. The body text is placeholder."

Folder `%LOCALAPPDATA%\Kurasi\demo\`, project name "Sample: Classics of work and organisation".
Rebuilt from scratch on reset; it never touches any other project.

| Path | Reference |
| --- | --- |
| `economics/smith_1776_wealth-of-nations.pdf` | Smith, A. (1776). *An inquiry into the nature and causes of the wealth of nations*. W. Strahan and T. Cadell. |
| `economics/mill_1848_principles-of-political-economy.pdf` | Mill, J. S. (1848). *Principles of political economy*. John W. Parker. |
| `economics/veblen_1899_theory-of-the-leisure-class.pdf` | Veblen, T. (1899). *The theory of the leisure class*. Macmillan. |
| `management/taylor_1911_principles-of-scientific-management.pdf` | Taylor, F. W. (1911). *The principles of scientific management*. Harper & Brothers. |
| `management/gilbreth_1911_motion-study.pdf` | Gilbreth, F. B. (1911). *Motion study*. D. Van Nostrand. |
| `management/follett_1918_the-new-state.pdf` | Follett, M. P. (1918). *The new state*. Longmans, Green. |
| `management/babbage_1832_economy-of-machinery_excerpt-pp1-12_highlighted.pdf` | Babbage, C. (1832). *On the economy of machinery and manufactures*. Charles Knight. Usability Excerpt. |
| `sociology/weber_1905_protestantische-ethik.pdf` and `sociology/weber_1905_protestantische-ethik.print.translated-en.pdf` | Weber, M. (1905). Die protestantische Ethik und der Geist des Kapitalismus. *Archiv für Sozialwissenschaft und Sozialpolitik, 20-21*. German title page plus an English translated copy: one reference, two files (slug grouping). |
| `biology/darwin_1859_origin-of-species.pdf` | Darwin, C. (1859). *On the origin of species by means of natural selection*. John Murray. |
| `inbox/scan_0007.pdf` | Title page of Owen, R. (1813). *A new view of society*. No PDF metadata: stays Uncategorized, author Unknown, status Auto. |
| `inbox/2026-03-02_download.pdf` | Title page of Marshall, A. (1890). *Principles of economics*. Macmillan. No PDF metadata: Uncategorized, Auto (date guessed from the name). |
| `notes/reading-plan.md`, `notes/open-questions.md` | Headings, a list, a table, links to two sample PDFs and one exhibit, one external https link. |
| `exhibits/exhibit-1_division-of-labour.png`, `exhibits/exhibit-2_reading-timeline.png` | Drawn by the sample builder. Exhibit 1 is labelled "Illustrative numbers". Exhibit 2 is a timeline of the publication years above. |

- Categories: Economics, Management (Scientific management, Administration), Sociology, Biology,
  Notes, Exhibits, plus Uncategorized.
- Tags: division-of-labour, efficiency, incentives, method, institutions, evolution, critique,
  to-read, quote.
- Highlights: 2 to 4 on each categorised PDF, mixed authors `You` and `Claude`, labels F, I and P,
  short neutral analyses, one analysis with a link to a project file. One `Claude` highlight carries
  a delete suggestion and one carries suggested tags, so the review flows have something to show.
  One Markdown highlight each by `You` and `Claude`; one region highlight on exhibit 1.
- Status: most references Verified, two Suggested (blue dot), the two inbox files Auto.
- PDF highlights are real annotations written through `simplepdf` and read back by a real scan,
  so the tour exercises the same code as real projects.

## 10. Testing rules (binding)

- Tests and smoke runs write only inside the repo folder, `%TEMP%` and `%LOCALAPPDATA%\Kurasi`.
- Nothing is ever written under `D:\OneDrive\`.
- The integration stage may run one read-only scan of
  `D:\OneDrive\Study\Essay\10_reading` with the project opened
  `read_only=True`. It must skip cloud-only files and must not hydrate them.
- `cargo test --workspace` covers the scanner (move, change, removal, grouping, cloud-only via a
  patched attribute), the PDF backend (add, edit, delete, rotate round trips; verbatim
  extraction; Tags line parse and compose), every API route on a temporary project, and every
  MCP tool. When `.venv` is present, PyMuPDF reads the written PDFs back as a cross-check.

## 11. Writing rules for every text an agent writes

UI copy, README, comments, error messages: never the em dash or the en dash character. No
"X, not Y" or "not X but Y" constructions; state the affirmative claim. A plain factual negation
is fine ("The file is open in another program."). Button labels say what happens; errors say
what went wrong and how to fix it.

## 12. Startup, splash screen and last project (added 17 Oct 2026)

- Image: `ui\assets\splash.jpg` (1920 x 1200, 16:10, already in place). Shown with
  `object-fit: cover`. A thin bar along the bottom carries "Kurasi" and one status line
  ("Opening <project name>", "Loading references", "Building tag graph").
- Desktop window (pywebview, owned by `app.py`): at start create a frameless, centred,
  always-on-top splash window of 960 x 600 that loads `ui/splash.html`, and create the main
  window hidden. The main window loads the UI. The UI reports progress through the js_api
  method `splash_status(text)` (the backend forwards it to the splash window with
  `evaluate_js`) and calls `app_ready()` when it is ready. The backend then waits until at
  least 2.0 s have passed since process start, shows the main window maximised and destroys
  the splash. Safety: 30 s after start the main window is shown anyway, and the UI shows a toast
  naming what is still loading.
- Ready means: the last opened project is open, `/api/state` and `/api/references` are loaded,
  the first chunk of catalogue cards is rendered, and the tag graph layout for the whole project is
  computed and cached, or 1.5 s have passed while it is computed. A layout that takes longer
  finishes in the background, and the Tag tab draws it when it arrives.
- Tag graph builder: the UI computes graph data and the force layout at startup (in a Web
  Worker, `ui/graph_worker.js`), caches the result keyed by a fingerprint of the references,
  authors and tags it draws (a filtered graph by `change_seq`) and the graph settings, and the
  Tag tab draws from the cache. Later changes recompute it in the background; a change that
  leaves the fingerprint as it was keeps the cached layout.
- Scale (added 18 Oct 2026): 10,000 files per project is the recommended maximum. It is a soft
  cap: Kurasi does not enforce it, and above it Kurasi stays usable and gets slower. The figure
  follows the libraries Kurasi is made for: a PhD dissertation library holds about 1,000 to
  6,000 files and a master's thesis library under 1,500. The tag graph is built for about
  15,000 nodes, the papers, authors and tags of a 10,000-file library (CONTRACT_RUST.md
  section 5). The catalogue list carries no highlight text and travels gzip-compressed
  (section 6): on the sample project it went from 15.9 KB to 9.7 KB, 2.6 KB on the wire. For a
  10,000-file library with about 500 characters of highlight text per reference the estimate is
  12.8 MB before and 6.5 MB after, about 1.7 MB on the wire. The highlight text is searched on
  the server (`/api/search`), and a change that names its references fetches those references
  only (section 8).
- Last project: on start the backend opens the project with the latest `last_opened` in
  `projects.json` whose folder exists, updates `last_opened`, and starts an incremental scan in
  the background. The splash does not wait for the scan. If that folder is unreachable, the UI
  opens the welcome dialog with one line naming the project and its path. With no project at
  all: splash for 2 s, then the welcome dialog.
- `GET /api/state` gains `startup: {opened_last: bool, folder_missing: bool, missing_name, missing_folder}`.
- Edge `--app` fallback: `index.html` shows the same splash as a full-window overlay under the
  same rules (2 s minimum, the ready condition, the 30 s safety).
