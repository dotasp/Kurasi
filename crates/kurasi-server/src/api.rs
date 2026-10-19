//! The HTTP API of CONTRACT.md section 6 (a port of api.py): JSON routes under `/api/`, the
//! static UI under `/ui/` and `/`, the Host and Origin check, and the long-poll on
//! `/api/changes`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path as AxPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use kurasi_core::error::Error as CoreError;
use kurasi_core::model::{Changes, HighlightFilter, HighlightPatch, NewHighlight, Origin, Reference, ReferenceFilter, ReferencePatch, Writer};
use kurasi_core::Project;
use serde_json::{json, Map, Value};
use tower::ServiceBuilder;
use tower_http::compression::predicate::{Predicate, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::mcp;
use crate::workspace::Workspace;

pub const MAX_BODY: usize = 8 * 1024 * 1024;
const LONG_POLL_DEFAULT: u64 = 25;
const LONG_POLL_MAX: u64 = 60;

type App = Arc<Workspace>;

// ------------------------------------------------------------------ errors

/// A core error as the API answers it: its status and `{"error": "<one sentence>"}`.
pub struct ApiError(pub CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            tracing::error!("request failed: {}", self.0);
        }
        (status, Json(json!({"error": self.0.message()}))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(CoreError::bad(msg))
}

fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(CoreError::not_found(msg))
}

// ------------------------------------------------------------------ helpers

/// Runs a core call on the blocking pool.
async fn blocking<T, F>(f: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CoreError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(ApiError),
        Err(e) => Err(ApiError(CoreError::Other(format!("The request task failed: {}.", e)))),
    }
}

/// The JSON object of a request body; an empty body is an empty object.
fn body_map(bytes: &Bytes) -> ApiResult<Map<String, Value>> {
    if bytes.len() > MAX_BODY {
        return Err(bad("The request body is too large."));
    }
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(Value::Null) => Ok(Map::new()),
        Ok(_) => Err(bad("The request body must be a JSON object.")),
        Err(_) => Err(bad("The request body is not valid JSON.")),
    }
}

fn parse_into<T: serde::de::DeserializeOwned>(body: Map<String, Value>, what: &str) -> ApiResult<T> {
    serde_json::from_value(Value::Object(body)).map_err(|e| bad(format!("The {} body is malformed: {}.", what, e)))
}

fn str_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn opt_str(v: Option<&Value>) -> Option<String> {
    let s = str_of(v);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn int_of(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        Some(Value::Bool(b)) => Some(*b as i64),
        _ => None,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !matches!(s.trim().to_lowercase().as_str(), "" | "0" | "false" | "no" | "off"),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        Some(Value::Array(items)) => Some(items.iter().map(|x| str_of(Some(x))).filter(|s| !s.is_empty()).collect()),
        Some(Value::String(s)) => Some(s.split_whitespace().map(str::to_string).collect()),
        _ => None,
    }
}

fn raw(data: Vec<u8>, ctype: &str, cache: &str) -> Response {
    let mut resp = data.into_response();
    let headers = resp.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(ctype).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_str(cache).unwrap_or(HeaderValue::from_static("no-store")));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    resp
}

fn ok_json<T: serde::Serialize>(v: T) -> ApiResult<Response> {
    let mut resp = Json(v).into_response();
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(resp)
}

/// Picks the project (and clones it) under the read guard, then runs `f` on the blocking pool
/// while the guard stays held so a switch waits for this request.
macro_rules! with_project {
    ($ws:expr, $p:ident => $body:expr) => {{
        let cur = $ws.read().await;
        let $p: Project = cur.require()?.clone();
        let out = blocking(move || $body).await;
        drop(cur);
        out
    }};
}

// ------------------------------------------------------------------ state and projects

async fn state(State(ws): State<App>) -> ApiResult<Response> {
    ok_json(ws.state().await?)
}

async fn project_create(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    ws.create(&body).await?;
    ok_json(ws.state().await?)
}

async fn project_open(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let pid = str_of(body.get("id"));
    if pid.is_empty() {
        return Err(bad("Send the id of the project to open."));
    }
    ws.open(&pid, true).await?;
    ok_json(ws.state().await?)
}

/// `POST /api/project/check {folder}`: whether Kurasi can scan that folder.
async fn project_check(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let folder = str_of(body.get("folder"));
    let paths = ws.paths().clone();
    ok_json(blocking(move || Ok(crate::workspace::check_folder(&paths, &folder))).await?)
}

async fn projects(State(ws): State<App>) -> ApiResult<Response> {
    let open = ws.read().await.project.as_ref().map(|p| p.id().to_string());
    ok_json(ws.projects_list(open.as_deref())?)
}

async fn project_entry(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    ok_json(ws.project_entry(&id).await?)
}

/// `DELETE /api/projects/<id>?delete_data=1`
async fn project_remove(State(ws): State<App>, AxPath(id): AxPath<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let with_data = q.get("delete_data").map(|s| matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")).unwrap_or(false);
    ok_json(ws.remove_project(&id, with_data).await?)
}

async fn project_restore(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let rid = str_of(body.get("removal_id"));
    if rid.is_empty() {
        return Err(bad("Send the removal_id to restore."));
    }
    ok_json(ws.restore_project(&rid).await?)
}

async fn project_reset(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    ok_json(ws.reset_project(&id).await?)
}

async fn demo(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    ws.open_demo(truthy(body.get("reset"))).await?;
    ok_json(ws.state().await?)
}

async fn settings_put(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let out = with_project!(ws, p => p.update_settings(&body, Origin::Ui))?;
    ok_json(Value::Object(out))
}

// ------------------------------------------------------------------ references and categories

/// One file of a catalogue list record.
#[derive(serde::Serialize)]
struct ListFile<'a> {
    id: &'a str,
    relpath: &'a str,
    role: &'a str,
    ext: &'a str,
    pages: i64,
    missing: bool,
    cloud_only: bool,
}

/// One reference in the catalogue list of `GET /api/references`: the fields the UI draws its
/// cards, filters, author cards and tag graph from. The full record of `/api/reference/<id>`
/// adds slug, usability, tags, the detail fields, the highlight count per file and search_text.
#[derive(serde::Serialize)]
pub struct ListRef<'a> {
    id: &'a str,
    title: &'a str,
    authors: &'a [String],
    kind: &'a str,
    #[serde(rename = "pub")]
    pub_: &'a str,
    year: &'a str,
    date: &'a str,
    lang: &'a str,
    status: &'a str,
    register_id: &'a str,
    categories: &'a [String],
    ext: &'a str,
    pages: i64,
    hl_count: i64,
    top_tags: &'a [String],
    missing: bool,
    files: Vec<ListFile<'a>>,
    primary_file_id: Option<&'a str>,
    thumb_url: Option<&'a str>,
    created_at: Option<&'a str>,
    updated_at: Option<&'a str>,
}

impl<'a> From<&'a Reference> for ListRef<'a> {
    fn from(r: &'a Reference) -> Self {
        ListRef {
            id: &r.id,
            title: &r.title,
            authors: &r.authors,
            kind: &r.kind,
            pub_: &r.pub_,
            year: &r.year,
            date: &r.date,
            lang: &r.lang,
            status: &r.status,
            register_id: &r.register_id,
            categories: &r.categories,
            ext: &r.ext,
            pages: r.pages,
            hl_count: r.hl_count,
            top_tags: &r.top_tags,
            missing: r.missing,
            files: r
                .files
                .iter()
                .map(|f| ListFile {
                    id: &f.id,
                    relpath: &f.relpath,
                    role: &f.role,
                    ext: &f.ext,
                    pages: f.pages,
                    missing: f.missing,
                    cloud_only: f.cloud_only,
                })
                .collect(),
            primary_file_id: r.primary_file_id.as_deref(),
            thumb_url: r.thumb_url.as_deref(),
            created_at: r.created_at.as_deref(),
            updated_at: r.updated_at.as_deref(),
        }
    }
}

/// The ids of `?ids=a,b,c`; `None` when the parameter is absent.
fn ids_param(q: &HashMap<String, String>) -> Option<Vec<String>> {
    q.get("ids").map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
}

/// A JSON body serialised on the blocking pool (a large list would hold a runtime thread).
fn json_bytes(data: Vec<u8>) -> Response {
    raw(data, "application/json", "no-store")
}

/// `GET /api/references[?ids=a,b,...]`: the catalogue list, or the listed references only (ids
/// that are unknown or in the trash are left out, so the UI drops them).
async fn references(State(ws): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let filter = ReferenceFilter { ids: ids_param(&q), detail: false, lean: true };
    let body = with_project!(ws, p => {
        let refs = p.references(&filter)?;
        let list: Vec<ListRef> = refs.iter().map(ListRef::from).collect();
        Ok(serde_json::to_vec(&list)?)
    })?;
    Ok(json_bytes(body))
}

/// `GET /api/search?q=<query>&loose=1|0`: per-token full-text scores (`Project::text_search`).
/// Without `loose` the project setting applies.
async fn search(State(ws): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let query = q.get("q").cloned().unwrap_or_default();
    let loose = q.get("loose").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).map(|s| !matches!(s.as_str(), "0" | "false" | "no" | "off"));
    let body = with_project!(ws, p => {
        let loose = loose.unwrap_or_else(|| p.setting("loose").as_bool().unwrap_or(true));
        Ok(serde_json::to_vec(&p.text_search(&query, loose)?)?)
    })?;
    Ok(json_bytes(body))
}

async fn reference_get(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.reference(&id))?;
    ok_json(out)
}

async fn reference_put(State(ws): State<App>, AxPath(id): AxPath<String>, body: Bytes) -> ApiResult<Response> {
    let patch: ReferencePatch = parse_into(body_map(&body)?, "reference")?;
    let out = with_project!(ws, p => p.update_reference(&id, &patch, Origin::Ui, "Verified"))?;
    ok_json(out.0)
}

async fn reference_delete(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.delete_reference(&id, Origin::Ui))?;
    ok_json(out)
}

async fn undo(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let tid = match body.get("trash_id") {
        None | Some(Value::Null) => return Err(bad("Send the trash_id to restore.")),
        v => int_of(v).ok_or_else(|| bad("trash_id must be a number."))?,
    };
    let out = with_project!(ws, p => p.undo(tid, Origin::Ui))?;
    ok_json(out)
}

async fn categories(State(ws): State<App>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.categories())?;
    ok_json(out)
}

async fn category_create(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let name = str_of(body.get("name"));
    let parent = match body.get("parent_id") {
        None | Some(Value::Null) => None,
        v => Some(int_of(v).ok_or_else(|| bad("parent_id must be a category id."))?),
    };
    let out = with_project!(ws, p => p.create_category(parent, &name, Origin::Ui))?;
    ok_json(out)
}

async fn category_put(State(ws): State<App>, AxPath(id): AxPath<i64>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let name = body.get("name").map(|v| str_of(Some(v)));
    let parent = match body.get("parent_id") {
        None => None,
        Some(Value::Null) => Some(None),
        v => Some(Some(int_of(v).ok_or_else(|| bad("parent_id must be a category id."))?)),
    };
    let out = with_project!(ws, p => p.update_category(id, name.as_deref(), parent, Origin::Ui))?;
    ok_json(out)
}

async fn category_delete(State(ws): State<App>, AxPath(id): AxPath<i64>) -> ApiResult<Response> {
    let moved = with_project!(ws, p => p.delete_category(id, Origin::Ui))?;
    ok_json(json!({"moved": moved}))
}

async fn authors(State(ws): State<App>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.authors())?;
    ok_json(out)
}

async fn tags(State(ws): State<App>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.tags())?;
    ok_json(out)
}

// ------------------------------------------------------------------ files

async fn file_info(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.file_info(&id))?;
    ok_json(out)
}

fn page_number(name: &str) -> ApiResult<usize> {
    name.strip_suffix(".png")
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| not_found(format!("There is no API route for page {}.", name)))
}

async fn page_png(
    State(ws): State<App>,
    AxPath((id, page)): AxPath<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let n = page_number(&page)?;
    let scale = match q.get("scale") {
        None => 1.5,
        Some(s) => s.trim().parse::<f64>().map_err(|_| bad("scale must be a number such as 1.5."))?,
    };
    let png = with_project!(ws, p => p.render_page(&id, n, scale))?;
    Ok(raw(png, "image/png", "private, max-age=300"))
}

async fn page_words(State(ws): State<App>, AxPath((id, n)): AxPath<(String, usize)>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.page_words(&id, n))?;
    ok_json(out)
}

async fn file_raw(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    let (data, ctype) = with_project!(ws, p => p.raw(&id))?;
    Ok(raw(data, ctype, "no-cache"))
}

async fn thumb(State(ws): State<App>, AxPath(name): AxPath<String>) -> ApiResult<Response> {
    let fid = name
        .strip_suffix(".png")
        .map(str::to_string)
        .ok_or_else(|| not_found(format!("There is no API route /api/thumb/{}.", name)))?;
    let png = with_project!(ws, p => p.thumb(&fid))?;
    Ok(raw(png, "image/png", "private, max-age=300"))
}

async fn rotate(State(ws): State<App>, AxPath(id): AxPath<String>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let page = match body.get("page") {
        None | Some(Value::Null) => None,
        v => Some(int_of(v).ok_or_else(|| bad("page must be a page number, or null for every page."))?),
    };
    let delta = match body.get("delta") {
        None | Some(Value::Null) => 90,
        v => int_of(v).ok_or_else(|| bad("delta must be a multiple of 90."))? as i32,
    };
    let out = with_project!(ws, p => p.rotate(&id, page, delta, Origin::Ui))?;
    ok_json(out)
}

// ------------------------------------------------------------------ highlights

async fn highlights(State(ws): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let filter = HighlightFilter {
        ref_id: q.get("ref_id").filter(|s| !s.is_empty()).cloned(),
        file_id: q.get("file_id").filter(|s| !s.is_empty()).cloned(),
        tag: None,
        author: None,
    };
    let out = with_project!(ws, p => p.highlights(&filter))?;
    ok_json(out)
}

async fn highlight_add(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body: NewHighlight = parse_into(body_map(&body)?, "highlight")?;
    let out = with_project!(ws, p => p.add_highlight(&body, &Writer::human()))?;
    ok_json(out)
}

async fn highlight_put(State(ws): State<App>, AxPath(id): AxPath<String>, body: Bytes) -> ApiResult<Response> {
    let patch: HighlightPatch = parse_into(body_map(&body)?, "highlight")?;
    let out = with_project!(ws, p => p.update_highlight(&id, &patch, Origin::Ui))?;
    ok_json(out)
}

async fn highlight_delete(State(ws): State<App>, AxPath(id): AxPath<String>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.delete_highlight(&id, Origin::Ui))?;
    ok_json(out)
}

async fn highlight_suggestion(State(ws): State<App>, AxPath(id): AxPath<String>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let resolution = str_of(body.get("resolution"));
    let out = with_project!(ws, p => p.resolve_suggestion(&id, &resolution, Origin::Ui))?;
    ok_json(out)
}

async fn highlight_accept_tag(State(ws): State<App>, AxPath(id): AxPath<String>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let tag = str_of(body.get("tag"));
    let dismiss = truthy(body.get("dismiss"));
    let out = with_project!(ws, p => p.accept_tag(&id, &tag, dismiss, Origin::Ui))?;
    ok_json(out)
}

// ------------------------------------------------------------------ scan, changes, logs, export

async fn scan_start(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let cur = ws.read().await;
    let project = cur.require()?;
    let runner = cur.runner()?;
    if runner.running() {
        return Err(ApiError(CoreError::conflict("A scan is already running. Its progress shows in the scan dialog.")));
    }
    let started = runner.start(truthy(body.get("full")), Origin::Ui, false);
    let mut snap = runner.snapshot();
    if let Some(obj) = snap.as_object_mut() {
        obj.insert("started".into(), json!(started));
        obj.insert("queued".into(), json!(!started));
        obj.insert("project_id".into(), json!(project.id()));
    }
    ok_json(snap)
}

async fn scan_status(State(ws): State<App>) -> ApiResult<Response> {
    let cur = ws.read().await;
    match &cur.runner {
        Some(r) => ok_json(r.snapshot()),
        None => ok_json(json!({"running": false, "pct": 0, "message": "", "log": [], "result": null, "error": null})),
    }
}

/// `GET /api/changes?since=<seq>&timeout=<s>`: answers at once when the feed moved past
/// `since`, else waits (25 s by default) for the next change in this process or any other.
async fn changes(State(ws): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let since = match q.get("since").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => 0,
        Some(s) => s.parse::<i64>().map_err(|_| bad("since must be a change number."))?,
    };
    let timeout = match q.get("timeout").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => LONG_POLL_DEFAULT,
        Some(s) => s.parse::<u64>().map_err(|_| bad("timeout must be a number of seconds."))?.min(LONG_POLL_MAX),
    };
    let (project, mut rx, generation) = {
        let cur = ws.read().await;
        match &cur.project {
            None => return ok_json(Changes { seq: 0, changes: Vec::new() }),
            Some(p) => (p.clone(), p.subscribe(), ws.generation()),
        }
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        // A close of the project (a switch or a removal) answers at once, so this request lets
        // go of the project and its database.
        let closing = ws.closing().notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        let ch = project.changes_since(since)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !ch.changes.is_empty() || ch.seq != since || remaining.is_zero() || ws.generation() != generation {
            return ok_json(ch);
        }
        // The broadcast covers this process; the one-second tick catches writes by the
        // MCP process, which has its own feed.
        tokio::select! {
            _ = &mut closing => return ok_json(project.changes_since(since)?),
            r = tokio::time::timeout(remaining.min(Duration::from_secs(1)), rx.recv()) => match r {
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return ok_json(project.changes_since(since)?),
                _ => continue,
            },
        }
    }
}

async fn ai_log(State(ws): State<App>) -> ApiResult<Response> {
    let out = with_project!(ws, p => p.ai_log())?;
    ok_json(out)
}

async fn ai_log_export(State(ws): State<App>) -> ApiResult<Response> {
    let path = with_project!(ws, p => p.export_ai_log())?;
    ok_json(json!({"path": path.to_string_lossy()}))
}

async fn export(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let format = str_of(body.get("format"));
    let ids = strings(body.get("ref_ids")).filter(|v| !v.is_empty());
    let path = with_project!(ws, p => p.export_references(&format, ids.as_deref()))?;
    ok_json(json!({"path": path.to_string_lossy()}))
}

async fn open_target(State(ws): State<App>, body: Bytes) -> ApiResult<Response> {
    let body = body_map(&body)?;
    let mode = opt_str(body.get("mode")).unwrap_or_else(|| "default".into());
    if mode != "default" && mode != "reveal" {
        return Err(bad("mode must be default or reveal."));
    }
    let url = str_of(body.get("url"));
    if !url.is_empty() {
        let lower = url.to_lowercase();
        if !(lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("mailto:")) {
            return Err(bad("Only http, https and mailto links open from here."));
        }
        (ws.opener())("default", &url)?;
        return ok_json(json!({"ok": true, "opened": url}));
    }
    let cur = ws.read().await;
    let p = cur.require()?.clone();
    let path = if let Some(fid) = opt_str(body.get("file_id")) {
        let f = p.file_row(&fid)?;
        p.abspath(&f.relpath)?
    } else if let Some(rel) = opt_str(body.get("relpath")) {
        p.abspath(&rel)?
    } else {
        return Err(bad("Send file_id, relpath or url."));
    };
    if !path.exists() {
        return Err(not_found(format!("The file {} does not exist.", p.relpath(&path))));
    }
    let target = path.to_string_lossy().into_owned();
    (ws.opener())(&mode, &target)?;
    ok_json(json!({"ok": true, "opened": target}))
}

async fn resolve(State(ws): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Response> {
    let href = q.get("href").cloned().unwrap_or_default();
    let from = q.get("from").filter(|s| !s.is_empty()).cloned();
    let out = with_project!(ws, p => p.resolve(&href, from.as_deref()))?;
    ok_json(out)
}

async fn mcp_info(State(ws): State<App>) -> ApiResult<Response> {
    let cur = ws.read().await;
    let folder = cur.project.as_ref().map(|p| p.folder().to_path_buf());
    let command = match &folder {
        Some(f) => ws.paths().mcp_command(f),
        None => ws.paths().mcp_command(Path::new("<folder>")),
    };
    ok_json(json!({"command": command, "tools": mcp::tool_list(), "resources": mcp::resource_list()}))
}

async fn api_not_found(req: Request) -> Response {
    ApiError(CoreError::not_found(format!("There is no API route {}.", req.uri().path()))).into_response()
}

// ------------------------------------------------------------------ host check

async fn host_check(State(port): State<u16>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("").to_lowercase();
    let ok_host = host == format!("127.0.0.1:{}", port) || host == format!("localhost:{}", port);
    let ok_origin = match req.headers().get(header::ORIGIN).and_then(|h| h.to_str().ok()) {
        None => true,
        Some(o) => {
            let o = o.to_lowercase();
            o == format!("http://127.0.0.1:{}", port) || o == format!("http://localhost:{}", port)
        }
    };
    if !(ok_host && ok_origin) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "This server only answers the Kurasi window on this computer."})),
        )
            .into_response();
    }
    next.run(req).await
}

// ------------------------------------------------------------------ the router

/// The axum application over a workspace. Tests drive it with `tower::ServiceExt::oneshot`.
pub fn build_app(ws: Arc<Workspace>) -> Router {
    let port = ws.port();
    let static_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .service(ServeDir::new(ws.ui_dir()).append_index_html_on_directories(true));
    Router::new()
        .route("/api/state", get(state))
        .route("/api/project", post(project_create))
        .route("/api/project/open", post(project_open))
        .route("/api/project/check", post(project_check))
        .route("/api/projects", get(projects))
        .route("/api/projects/restore", post(project_restore))
        .route("/api/projects/{id}", get(project_entry).delete(project_remove))
        .route("/api/projects/{id}/reset", post(project_reset))
        .route("/api/demo", post(demo))
        .route("/api/settings", put(settings_put))
        .route("/api/references", get(references))
        .route("/api/search", get(search))
        .route("/api/reference/{id}", get(reference_get).put(reference_put).delete(reference_delete))
        .route("/api/undo", post(undo))
        .route("/api/categories", get(categories).post(category_create))
        .route("/api/categories/{id}", put(category_put).delete(category_delete))
        .route("/api/authors", get(authors))
        .route("/api/tags", get(tags))
        .route("/api/file/{id}/info", get(file_info))
        .route("/api/file/{id}/page/{page}", get(page_png))
        .route("/api/file/{id}/page/{n}/words", get(page_words))
        .route("/api/file/{id}/raw", get(file_raw))
        .route("/api/file/{id}/rotate", post(rotate))
        .route("/api/thumb/{name}", get(thumb))
        .route("/api/highlights", get(highlights).post(highlight_add))
        .route("/api/highlights/{id}", put(highlight_put).delete(highlight_delete))
        .route("/api/highlights/{id}/suggestion", post(highlight_suggestion))
        .route("/api/highlights/{id}/accept-tag", post(highlight_accept_tag))
        .route("/api/scan", post(scan_start))
        .route("/api/scan/status", get(scan_status))
        .route("/api/changes", get(changes))
        .route("/api/ai-log", get(ai_log))
        .route("/api/ai-log/export", post(ai_log_export))
        .route("/api/export", post(export))
        .route("/api/open", post(open_target))
        .route("/api/resolve", get(resolve))
        .route("/api/mcp", get(mcp_info))
        .route("/api/{*rest}", any(api_not_found))
        .nest_service("/ui", static_files.clone())
        .fallback_service(static_files)
        .layer(middleware::from_fn_with_state(port, host_check))
        .layer(compression())
        .with_state(ws)
}

/// Text bodies the compression layer takes: JSON, `text/*` (HTML, CSS, Markdown), JavaScript
/// and SVG. Page images, thumbnails and file bytes of other types pass as they are.
fn text_body(_: StatusCode, _: http::Version, headers: &http::HeaderMap, _: &http::Extensions) -> bool {
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    ct.starts_with("application/json")
        || ct.starts_with("text/")
        || ct.starts_with("application/javascript")
        || ct.starts_with("image/svg+xml")
}

/// gzip at the fastest level for text bodies of 1 KB or more when the request accepts it. The
/// server answers on 127.0.0.1, so a level that costs little time per megabyte fits best.
pub fn compression() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .quality(CompressionLevel::Fastest)
        .compress_when(SizeAbove::new(1024).and(text_body as fn(StatusCode, http::Version, &http::HeaderMap, &http::Extensions) -> bool))
}
