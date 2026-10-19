# kurasi-core public interface (Rust)

Binding for every crate in the workspace. `crates/kurasi-core` implements it; `kurasi-pdf`,
`kurasi-server` and `src-tauri` call it. Change this file first, then the code.

```rust
// crates/kurasi-core/src/lib.rs
pub mod config;   // AppPaths, projects.json, portable mode
pub mod db;       // Db: rusqlite connection pool, schema, migrations
pub mod model;    // serde structs shared by api, mcp and ui (snake_case JSON as in CONTRACT.md)
pub mod scanner;  // scan, incremental scan, watcher
pub mod search;   // bigram Dice, loose and strict; FullText, the index behind text_search
pub mod sidecar;  // <folder>/.kurasi/metadata.json
pub mod lock;     // write.lock
pub mod media;    // images: thumbnails, rotation, region highlights
pub mod demo;     // sterile sample project
pub mod export;   // APA references, AI_USE_LOG.md
pub mod change;   // change feed, ChangeSeq, subscribe()
pub mod ailog;    // ai_log rows
pub mod removal;  // removing a project from projects.json: data folders, safety checks, trash, restore

pub use config::{AppPaths, ProjectInfo};
pub use db::Db;

/// One opened project. Cheap to clone (Arc inside). Thread safe.
#[derive(Clone)]
pub struct Project { /* Arc<Inner> */ }

impl Project {
    /// Opens or creates <appdata>/projects/<id>/kurasi.sqlite. Never writes into `folder`
    /// unless a write method is called and `read_only` is false.
    pub fn open(paths: &AppPaths, id: &str) -> Result<Project>;
    pub fn create(paths: &AppPaths, name: &str, folder: &Path) -> Result<Project>;
    pub fn info(&self) -> ProjectInfo;             // id, name, folder, read_only, created
    pub fn set_read_only(&self, on: bool) -> Result<()>;
    pub fn db(&self) -> &Db;
    pub fn folder(&self) -> &Path;
    pub fn change_seq(&self) -> i64;
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<i64>;   // fires on every change row
    pub fn scan(&self, mode: scanner::ScanMode, progress: impl FnMut(scanner::Progress)) -> Result<scanner::ScanReport>;
    pub fn start_watcher(&self) -> Result<scanner::WatcherHandle>;        // debounce 2 s, incremental scan
}

pub struct AppPaths { pub root: PathBuf /* %LOCALAPPDATA%\Kurasi or <exe dir>\data when portable.txt exists */ }
impl AppPaths {
    pub fn detect() -> AppPaths;                 // honours env KURASI_HOME first, then portable.txt, then LOCALAPPDATA
    pub fn projects(&self) -> Result<Vec<ProjectInfo>>;
    pub fn last_opened(&self) -> Option<String>;
    pub fn set_last_opened(&self, id: &str) -> Result<()>;
    pub fn resolve(&self, id_or_folder: &str) -> Option<ProjectInfo>;
    /// `claude mcp add kurasi -s user -- "<exe>" mcp --project "<folder>" --mode write`
    pub fn mcp_command(&self, folder: &Path) -> String;
}
```

Removing a project (`crates/kurasi-core/src/removal.rs`, CONTRACT.md sections 2 and 6). Removal
takes the entry out of `projects.json`. With `with_data` it also moves Kurasi's data for the
project into `<root>\trash\projects\<removal id>\`: the folder `<root>\projects\<id>\` (database
and write.lock) and the files Kurasi wrote into `<folder>\.kurasi\` (`metadata.json`,
`AI_USE_LOG.md`, its own temporary files); `.kurasi` itself goes only when nothing else is left in
it. A read-only project keeps its `.kurasi`. Every removal writes `removed.json` there, so
`restore_project` brings the entry back, and the data when it was moved. `purge_removed` deletes
trash entries older than `TRASH_DAYS`; the server calls it at start. User files are never moved.

```rust
pub const TRASH_DAYS: u64 = 7;
/// One folder "Also delete Kurasi's data" would remove: its path and the entries in it that go.
/// `whole` is true when the folder itself goes as well.
pub struct DataDir { pub path: String, pub files: Vec<String>, pub whole: bool }
/// The app-data folder and the sidecar folder of one project; a folder that is absent, fails a
/// safety check, or (sidecar) belongs to a read-only project is None.
pub struct DataTargets { pub app: Option<DataDir>, pub sidecar: Option<DataDir> }
/// What `remove_project` did. `moved` lists the original paths now in the trash.
pub struct Removal { pub removal_id: String, pub entry: ProjectInfo, pub with_data: bool, pub moved: Vec<String> }
pub enum TargetKind { AppData, Sidecar }

impl AppPaths {
    /// `<root>\trash\projects`
    pub fn removed_dir(&self) -> PathBuf;
    pub fn data_targets(&self, entry: &ProjectInfo) -> DataTargets;
    /// 400 for the sample project, 404 for an unknown id, 409 when a data file is held by
    /// another program (nothing changes then).
    pub fn remove_project(&self, id: &str, with_data: bool) -> Result<Removal>;
    /// 404 for an unknown removal id, 409 when the id, the folder or a data path is taken again.
    pub fn restore_project(&self, removal_id: &str) -> Result<ProjectInfo>;
    pub fn purge_removed(&self, older_than: std::time::Duration) -> Result<usize>;
}

/// The safety gate of every move: `target` must be `<root>\projects\<entry.id>` holding a
/// Kurasi database (SQLite header, Kurasi tables) and nothing else Kurasi did not create, or
/// `<entry.folder>\.kurasi` holding a valid `metadata.json`. Drive roots, the project folder,
/// any folder that contains the project folder or the app root, links and junctions are refused.
pub fn check_target(paths: &AppPaths, entry: &ProjectInfo, target: &Path) -> Result<TargetKind>;
```

`model` exposes the structs the HTTP API returns, named after CONTRACT.md section 6:
`Reference`, `Author`, `Category`, `FileRow`, `Highlight`, `Tag`, `AiLogRow`, `ChangeRow`,
`Setting`, `SearchHit`, `TextSearch`, `TextHit`, `ScanReport`. Every id is a `String` in the
Python format (`r-…`, `h-…`, `f-…`, `p-…`).

Catalog operations live on `Project` as plain methods (one per API route and MCP tool):
`references(filter)`, `reference(id)`, `update_reference(id, patch)`, `categorize(ids, categories, status)`,
`highlights(filter)`, `highlight(id)`, `add_highlight(new)`, `update_highlight(id, patch)`,
`delete_highlight(id, by)`, `undelete(trash_id)`, `tags()`, `suggest_tags(highlight_id, tags, by)`,
`categories()`, `settings()`, `set_setting(k, v)`, `ai_log(filter)`, `log_append(row)`,
`changes_since(seq)`, `search(query, loose: bool, limit)`, `text_search(query, loose: bool)`,
`export_references(format)`, `export_ai_log()`.

```rust
// crates/kurasi-core/src/model.rs
pub struct ReferenceFilter {
    pub ids: Option<Vec<String>>,   // only these references; an id that is unknown or in the trash is left out
    pub detail: bool,               // adds doi, url, vol, issue, pages_range
    pub lean: bool,                 // leaves search_text empty (the catalogue list of /api/references)
}
/// GET /api/search: the full-text part of the catalogue score.
pub struct TextSearch { pub query: String, pub loose: bool, pub tokens: Vec<String>, pub hits: Vec<TextHit> }
/// `scores[i]` belongs to `tokens[i]`: 1 for a substring hit, else 0.9 times the best bigram Dice
/// similarity of a word when it reaches the threshold, else 0. `score` is their sum.
pub struct TextHit { pub id: String, pub score: f64, pub scores: Vec<f64> }

// crates/kurasi-core/src/catalog.rs
impl Project {
    /// Matches `search_text` (highlight verbatim and analysis, 20,000 characters at most) with
    /// the token and threshold rules of the UI search (ui/app.js `score`). Lists the references
    /// where at least one token scores, best first. The index behind it is built on the first
    /// call, kept per open project and updated from the change feed.
    pub fn text_search(&self, query: &str, loose: bool) -> Result<TextSearch>;
}
```

`search` holds both matchers. `score`, `rank` and `field_sim` keep the rules of `search.py`
(MCP `catalog_search`). `FullText` and `ui_tokens` follow the UI: the query is lower-cased and
split at JavaScript whitespace (`\s`), words are runs of `\p{L}`, `\p{N}` and the hyphen, and
bigrams are pairs of UTF-16 code units.

PDF reads (`crates/kurasi-core/src/pdf.rs`). `PdfDoc` is `Send`, so an open document can wait
between two requests. Each `Project` keeps a `DocCache` of three documents for its read routes
(`file_info`, `render_page`, `page_words`, `page_text`). A viewer that asks for one page after
another opens the file once and reads the size and rotation of every page once. Opening a large
PDF and walking its pages costs about half a second, so a cache miss on every page request
made a 400-page file take tens of seconds per page while the viewer queued requests.

```rust
pub trait PdfDoc: Send { /* page_count, metadata, pages, render_png, words, annotations, writes */ }

/// Documents kept open for reading. An entry is keyed by the path, size and modification time
/// of the file, so a file another program changed opens again. Kurasi's own writes call
/// `forget` after the save. At most `capacity` documents stay open; the one used longest ago
/// closes first.
pub struct DocCache { /* Mutex<Vec<Arc<OpenDoc>>> */ }
impl DocCache {
    pub fn new(capacity: usize) -> DocCache;
    pub fn get(&self, backend: &dyn PdfBackend, path: &Path) -> Result<Arc<OpenDoc>>;
    pub fn forget(&self, path: &Path);
    pub fn len(&self) -> usize;
}
/// One cached document: the document behind a Mutex and its page list once read.
pub struct OpenDoc { /* … */ }
impl OpenDoc {
    pub fn doc(&self) -> MutexGuard<'_, Box<dyn PdfDoc>>;
    /// Size and rotation of every page, read on first use and kept with the entry.
    pub fn pages(&self) -> Result<Arc<Vec<PageInfo>>>;
}
```

Writes (highlights, rotation, restore) still open a document of their own, save it, and then
call `forget` for the path.

PDF work is in `kurasi-pdf` (`crates/kurasi-pdf`):

```rust
pub struct Pdfium;                          // loads pdfium.dll from the exe folder, then resources/
pub fn open(path: &Path) -> Result<Doc>;
/// Every call holds the process-wide pdfium lock, and so does dropping a Doc
/// (FPDF_CloseDocument), so a document closed on one thread never runs beside pdfium work on another.
impl Doc {
    pub fn page_count(&self) -> usize;
    pub fn page_info(&self, page: usize) -> Result<PageInfo>;               // unrotated size and /Rotate
    pub fn page_infos(&self) -> Result<Vec<PageInfo>>;                      // every page, one hold of the lock
    pub fn render_png(&self, page: usize, dpi: u32) -> Result<Vec<u8>>;
    pub fn thumbnail_png(&self, width_px: u32) -> Result<Vec<u8>>;            // first page, portrait box
    pub fn words(&self, page: usize) -> Result<Vec<Word>>;                    // text + bbox in PDF points
    pub fn annotations(&self) -> Result<Vec<Annot>>;                          // highlights with author, contents, colour, quads, page
    pub fn add_highlight(&mut self, h: &NewHighlight) -> Result<String>;      // returns annotation name (/NM)
    pub fn update_highlight(&mut self, name: &str, patch: &HighlightPatch) -> Result<()>;
    pub fn delete_highlight(&mut self, name: &str) -> Result<()>;
    pub fn rotate(&mut self, pages: &[usize], degrees: i32) -> Result<()>;
    pub fn save_in_place(&mut self) -> Result<()>;                            // temp file + rename, six retries
}
```

`kurasi-core` depends on `kurasi-pdf` for scanning and highlight writes. `kurasi-server`
depends on both and exposes:

```rust
/// axum, CONTRACT.md section 6. `project` is None when projects.json has no usable
/// last_opened: the UI then shows the welcome dialog and opens or creates a project through
/// the /api/projects routes, which swap the current project under a RwLock (close the old
/// Db only after in-flight requests finish, as the Python build does).
/// JSON and text bodies of 1 KB or more are gzip-compressed (tower-http) when the request
/// accepts gzip; page images, thumbnails and other binary bodies are sent as they are.
/// The PDF page routes (`/info`, `/page/<n>.png`, `/page/<n>/words`) wait for one of two
/// permits (`PAGE_PERMITS`) before they start pdfium work, in arrival order. A request whose
/// client has gone while it waits (the viewer aborts pages that left the view) is dropped
/// before it renders.
pub async fn serve(paths: AppPaths, project: Option<Project>, bind: Option<u16>) -> Result<(std::net::SocketAddr, ServerHandle)>;
/// Sterile sample project (CONTRACT.md section 9), created under `paths`; idempotent reset.
/// Lives in kurasi-core: `kurasi_core::demo::build(&AppPaths) -> Result<ProjectInfo>`.
pub async fn mcp_stdio(project: Project, mode: Mode) -> Result<()>;         // rmcp, CONTRACT.md section 7
pub enum Mode { Read, Propose, Write }   // `kurasi.exe mcp` defaults to Write

impl Workspace {
    /// `GET /api/projects`: every entry, last opened first, with `exists` and `open`.
    pub fn projects_list(&self, open_id: Option<&str>) -> CoreResult<Vec<Value>>;
    /// `DELETE /api/projects/<id>`: closes the project first when it is open (watcher, scan
    /// runner, timer and the long-polls of `/api/changes` let go of it), then
    /// `AppPaths::remove_project`.
    pub async fn remove_project(&self, id: &str, with_data: bool) -> CoreResult<Value>;
    /// `POST /api/projects/restore`: `AppPaths::restore_project`; the project stays closed.
    pub async fn restore_project(&self, removal_id: &str) -> CoreResult<Value>;
    /// `POST /api/projects/<id>/reset`: rebuilds the sample project; reopens it when it was open.
    pub async fn reset_project(&self, id: &str) -> CoreResult<Value>;
}
```

`src-tauri` depends on `kurasi-server` and `kurasi-core` only.
