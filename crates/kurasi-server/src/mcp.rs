//! The stdio MCP server over one project (CONTRACT.md section 7, a port of mcp_server.py).
//!
//! Every tool call writes an `ai_log` row (tool, client name from the `clientInfo` in the
//! request `_meta` or else from `initialize`, purpose, arguments, result summary). Every tool
//! that writes adds change rows with origin `mcp`; a write that changed no catalogue row still
//! adds one so the open app sees what the agent did. The server runs in one of three modes: `read` answers the read
//! tools only, `propose` adds the proposal tools, `write` allows everything.

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use kurasi_core::error::Error as CoreError;
use kurasi_core::media;
use kurasi_core::model::{HighlightFilter, NewHighlight, Origin, Reference, ReferencePatch, Writer};
use kurasi_core::scanner::{ScanMode, ScanReport};
use kurasi_core::Project;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData, Implementation, JsonObject,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServiceExt;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};

// ------------------------------------------------------------------ modes

/// What the server may do: `Read` answers read tools only, `Propose` adds the proposal
/// tools, `Write` allows everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Read,
    Propose,
    Write,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Read => "read",
            Mode::Propose => "propose",
            Mode::Write => "write",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Mode, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Mode::Read),
            "propose" => Ok(Mode::Propose),
            "write" => Ok(Mode::Write),
            other => Err(format!("unknown mode `{}`; use read, propose or write", other)),
        }
    }
}

// ------------------------------------------------------------------ the tool table

/// (name, mode, description), the order of CONTRACT.md section 7.
pub const TOOLS: &[(&str, Mode, &str)] = &[
    (
        "catalog_search",
        Mode::Read,
        "Loose search over titles, authors, tags, publications, register IDs, categories and highlight text. Returns references ranked by relevance.",
    ),
    ("reference_get", Mode::Read, "Metadata, files and highlight count of one reference."),
    (
        "reference_text",
        Mode::Read,
        "Text of one page of a reference (PDF) or the whole Markdown note, plus word boxes in unrotated PDF points.",
    ),
    ("highlights_list", Mode::Read, "Highlights, filtered by reference, tag or author."),
    ("tags_list", Mode::Read, "The tags in use with their reference ids and highlight counts."),
    ("categories_list", Mode::Read, "The category tree with full paths such as Management/Administration."),
    (
        "reference_update_metadata",
        Mode::Propose,
        "Applies metadata fields, authors and categories. The status becomes Suggested (blue dot) until the person reviews and saves. Previous values are kept in the AI-use log.",
    ),
    (
        "reference_categorize",
        Mode::Propose,
        "Adds categories to references, creating missing paths. The status becomes Suggested.",
    ),
    (
        "highlight_propose_delete",
        Mode::Propose,
        "Marks a highlight for deletion with a reason. The person accepts or dismisses it.",
    ),
    ("highlight_suggest_tags", Mode::Propose, "Proposes tags for a highlight. The person accepts each tag."),
    (
        "catalog_scan",
        Mode::Write,
        "Runs an incremental scan of the project folder; full=true rehashes every file.",
    ),
    (
        "highlight_add",
        Mode::Write,
        "Adds a highlight with analysis, label and tags, authored by the AI. PDF: finds the quote on the page (or every page when page is omitted) or uses quads. Markdown: the quote becomes the anchor. Image: pass quads as [[x, y, w, h]] normalised to 0..1.",
    ),
    (
        "export_references",
        Mode::Write,
        "Writes apa-md, bibtex, csl-json or highlights-json under the project's _exports folder.",
    ),
    ("log_append", Mode::Write, "Adds a row to the AI-use log with the purpose and the checks performed."),
];

pub const RESOURCES: &[(&str, &str)] = &[
    ("kurasi://references", "Every reference with metadata, files and highlight counts."),
    ("kurasi://reference/{ref_id}", "One reference with its detail fields."),
    ("kurasi://reference/{ref_id}/highlights", "The highlights of one reference."),
    ("kurasi://tags", "The tags in use."),
];

pub fn tool_mode(name: &str) -> Option<Mode> {
    TOOLS.iter().find(|(n, _, _)| *n == name).map(|(_, m, _)| *m)
}

/// `[{name, description, mode}]` for `GET /api/mcp`.
pub fn tool_list() -> Vec<Value> {
    TOOLS.iter().map(|(n, m, d)| json!({"name": n, "description": d, "mode": m.as_str()})).collect()
}

pub fn resource_list() -> Vec<Value> {
    RESOURCES.iter().map(|(u, d)| json!({"uri": u, "description": d})).collect()
}

// ------------------------------------------------------------------ arguments

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Words to look for; typos are tolerated when the project's loose search is on.
    pub query: String,
    /// Most results to return (1 to 200).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RefArgs {
    /// The reference id (r-...).
    pub ref_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextArgs {
    /// The reference id (r-...).
    pub ref_id: String,
    /// 1-based page number of a PDF (default 1).
    #[serde(default)]
    pub page: Option<i64>,
    /// A file of the reference; default: its primary file.
    #[serde(default)]
    pub file_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HighlightsArgs {
    #[serde(default)]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateMetadataArgs {
    pub ref_id: String,
    /// Metadata fields such as {"title": "...", "year": "1911", "kind": "Book", "pub": "..."}.
    pub fields: Map<String, Value>,
    /// Author names as "Family, G." strings.
    #[serde(default)]
    pub authors: Option<Vec<String>>,
    /// Category paths such as "Management/Administration".
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    /// Why the change is proposed; goes into the AI-use log.
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
    fn into_tags(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => s.split_whitespace().map(str::to_string).collect(),
            OneOrMany::Many(v) => v,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CategorizeArgs {
    /// Reference ids.
    pub ref_ids: OneOrMany,
    /// Category paths such as "Exhibits/Charts"; missing paths are created.
    pub category_paths: OneOrMany,
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeDeleteArgs {
    pub highlight_id: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestTagsArgs {
    pub highlight_id: String,
    /// Tags to propose, as a list or one space-separated string.
    pub tags: OneOrMany,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanArgs {
    /// Rehash every file and re-read every PDF.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HighlightAddArgs {
    pub ref_id: String,
    /// The analysis text, starting with its label such as "(I) ...".
    pub analysis: String,
    /// F, I or P.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub tags: Option<OneOrMany>,
    /// 1-based page of a PDF; omitted: every page is searched for the quote.
    #[serde(default)]
    pub page: Option<i64>,
    /// The exact words on the page (PDF, Markdown) or a caption (image).
    #[serde(default)]
    pub quote: Option<String>,
    /// Rectangles [x0, y0, x1, y1] in PDF points, or [[x, y, w, h]] normalised to 0..1 for an image.
    #[serde(default)]
    pub quads: Option<Vec<Vec<f64>>>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportArgs {
    /// apa-md, bibtex, csl-json or highlights-json.
    pub format: String,
    #[serde(default)]
    pub ref_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LogArgs {
    /// What the AI-assisted step did.
    pub purpose: String,
    /// The checks the person performed on the result.
    pub checks: String,
}

fn schema_of<T: JsonSchema>() -> JsonObject {
    match serde_json::to_value(schemars::schema_for!(T)) {
        Ok(Value::Object(m)) => m,
        _ => JsonObject::new(),
    }
}

/// The tool definitions sent on `tools/list`.
pub fn tool_definitions() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|(name, _, desc)| {
            let schema = match *name {
                "catalog_search" => schema_of::<SearchArgs>(),
                "reference_get" => schema_of::<RefArgs>(),
                "reference_text" => schema_of::<TextArgs>(),
                "highlights_list" => schema_of::<HighlightsArgs>(),
                "tags_list" | "categories_list" => schema_of::<NoArgs>(),
                "reference_update_metadata" => schema_of::<UpdateMetadataArgs>(),
                "reference_categorize" => schema_of::<CategorizeArgs>(),
                "highlight_propose_delete" => schema_of::<ProposeDeleteArgs>(),
                "highlight_suggest_tags" => schema_of::<SuggestTagsArgs>(),
                "catalog_scan" => schema_of::<ScanArgs>(),
                "highlight_add" => schema_of::<HighlightAddArgs>(),
                "export_references" => schema_of::<ExportArgs>(),
                _ => schema_of::<LogArgs>(),
            };
            Tool::new(Cow::Borrowed(*name), Cow::Borrowed(*desc), schema)
        })
        .collect()
}

// ------------------------------------------------------------------ the tool bodies

type CoreResult<T> = std::result::Result<T, CoreError>;

fn brief(r: &Reference) -> Value {
    json!({
        "id": r.id, "title": r.title, "authors": r.authors, "year": r.year, "kind": r.kind, "pub": r.pub_,
        "status": r.status, "categories": r.categories, "hl_count": r.hl_count, "top_tags": r.top_tags,
        "primary_file_id": r.primary_file_id, "usability": r.usability, "lang": r.lang,
    })
}

fn strip(r: &Reference) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("search_text");
    }
    v
}

fn to_value<T: serde::Serialize>(v: T) -> CoreResult<Value> {
    Ok(serde_json::to_value(v)?)
}

fn parse_args<T: serde::de::DeserializeOwned>(tool: &str, args: Map<String, Value>) -> CoreResult<T> {
    serde_json::from_value(Value::Object(args)).map_err(|e| CoreError::bad(format!("The arguments of {} are malformed: {}.", tool, e)))
}

/// One prepared call: its log purpose and arguments, and the work to run.
struct Invocation {
    purpose: String,
    log_args: Value,
    result: CoreResult<(Value, Option<Value>)>,
}

/// The tool bodies, independent of the transport so tests call them directly.
pub struct Tools {
    project: Project,
}

impl Tools {
    pub fn new(project: Project) -> Tools {
        Tools { project }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    fn file_for(&self, ref_id: &str, file_id: Option<&str>) -> CoreResult<(Reference, kurasi_core::model::FileRow)> {
        let r = self.project.reference(ref_id)?;
        let fid = file_id.map(str::to_string).or_else(|| r.primary_file_id.clone()).unwrap_or_default();
        if fid.is_empty() || !r.files.iter().any(|f| f.id == fid) {
            return Err(CoreError::not_found(format!("The file {} does not belong to the reference {}.", fid, ref_id)));
        }
        let row = self.project.file_row(&fid)?;
        Ok((r, row))
    }

    pub fn catalog_search(&self, query: &str, limit: Option<i64>) -> CoreResult<Value> {
        let limit = limit.unwrap_or(20).clamp(1, 200) as usize;
        let loose = self.project.setting("loose").as_bool().unwrap_or(true);
        let hits = self.project.search(query, loose, limit)?;
        let results: Vec<Value> = hits
            .iter()
            .map(|h| {
                let mut b = brief(&h.reference);
                b.as_object_mut().expect("object").insert("score".into(), json!(h.score));
                b
            })
            .collect();
        Ok(json!({"query": query, "results": results}))
    }

    pub fn reference_get(&self, ref_id: &str) -> CoreResult<Value> {
        Ok(strip(&self.project.reference(ref_id)?))
    }

    pub fn reference_text(&self, ref_id: &str, page: Option<i64>, file_id: Option<&str>) -> CoreResult<Value> {
        let (_, f) = self.file_for(ref_id, file_id)?;
        let path = self.project.usable_path(&f)?;
        if f.ext == "pdf" {
            let page = page.unwrap_or(1).max(1) as usize;
            let words = self.project.page_words(&f.id, page)?;
            let text = self.project.page_text(&f.id, page)?;
            let pages = match f.page_count {
                Some(n) if n > 0 => n,
                _ => self.project.file_info(&f.id)?.get("pages").and_then(|v| v.as_i64()).unwrap_or(0),
            };
            let mut out = json!({"ref_id": ref_id, "file_id": f.id, "relpath": f.relpath, "page": page, "pages": pages, "text": text});
            let obj = out.as_object_mut().expect("object");
            for key in ["width", "height", "rotation", "words"] {
                obj.insert(key.into(), words.get(key).cloned().unwrap_or(Value::Null));
            }
            return Ok(out);
        }
        if media::MARKDOWN_EXTS.contains(&f.ext.as_str()) {
            let text = media::read_markdown(&path)?;
            return Ok(json!({"ref_id": ref_id, "file_id": f.id, "relpath": f.relpath, "page": Value::Null, "text": text, "words": []}));
        }
        Err(CoreError::bad("Images have no text layer. Use reference_get for the metadata, or add a region highlight with quads."))
    }

    pub fn highlights_list(&self, ref_id: Option<String>, tag: Option<String>, author: Option<String>) -> CoreResult<Value> {
        let filter = HighlightFilter { ref_id, file_id: None, tag, author };
        Ok(json!({"highlights": self.project.highlights(&filter)?}))
    }

    pub fn tags_list(&self) -> CoreResult<Value> {
        Ok(json!({"tags": self.project.tags()?}))
    }

    pub fn categories_list(&self) -> CoreResult<Value> {
        Ok(json!({"categories": self.project.categories()?}))
    }

    pub fn reference_update_metadata(
        &self,
        ref_id: &str,
        fields: &Map<String, Value>,
        authors: Option<Vec<String>>,
        categories: Option<Vec<String>>,
    ) -> CoreResult<(Value, Value)> {
        let mut body: Map<String, Value> = fields.iter().filter(|(k, _)| k.as_str() != "status").map(|(k, v)| (k.clone(), v.clone())).collect();
        body.insert("status".into(), json!("Suggested"));
        let patch = ReferencePatch { fields: body, authors, categories };
        let (rec, previous) = self.project.update_reference(ref_id, &patch, Origin::Mcp, "Suggested")?;
        Ok((strip(&rec), previous))
    }

    pub fn reference_categorize(&self, ref_ids: Vec<String>, paths: Vec<String>) -> CoreResult<Value> {
        let recs = self.project.categorize(&ref_ids, &paths, Origin::Mcp, "Suggested")?;
        Ok(json!({"references": recs.iter().map(brief).collect::<Vec<_>>()}))
    }

    pub fn highlight_propose_delete(&self, hid: &str, reason: &str) -> CoreResult<Value> {
        to_value(self.project.propose_delete(hid, reason, Origin::Mcp)?)
    }

    pub fn highlight_suggest_tags(&self, hid: &str, tags: Vec<String>) -> CoreResult<Value> {
        to_value(self.project.suggest_tags(hid, &tags, Origin::Mcp)?)
    }

    pub fn catalog_scan(&self, full: bool) -> CoreResult<ScanReport> {
        let mode = if full { ScanMode::Full } else { ScanMode::Incremental };
        self.project.scan_as(mode, Origin::Mcp, |_| {})
    }

    #[allow(clippy::too_many_arguments)]
    pub fn highlight_add(
        &self,
        ref_id: &str,
        analysis: &str,
        label: Option<&str>,
        tags: Vec<String>,
        page: Option<i64>,
        quote: Option<&str>,
        quads: Option<Vec<Vec<f64>>>,
        file_id: Option<&str>,
    ) -> CoreResult<Value> {
        let (_, f) = self.file_for(ref_id, file_id)?;
        let author = match self.project.setting("ai_author") {
            Value::String(s) if !s.trim().is_empty() => s,
            _ => "Claude".to_string(),
        };
        let writer = Writer::ai(&author);
        let base = Path::new(&f.relpath).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let quote = quote.map(str::trim).filter(|q| !q.is_empty());
        let mut body = NewHighlight {
            file_id: f.id.clone(),
            analysis: Some(analysis.to_string()),
            label: label.map(str::to_string),
            tags: Some(tags),
            ..Default::default()
        };
        if f.ext == "pdf" {
            self.project.usable_path(&f)?;
            if let Some(q) = quads.filter(|q| !q.is_empty()) {
                let page = page.filter(|p| *p >= 1).ok_or_else(|| CoreError::bad("Pass the page number together with quads."))?;
                body.page = Some(page);
                body.quads = Some(q);
                body.kind = Some("text".into());
                return to_value(self.project.add_highlight(&body, &writer)?);
            }
            let Some(q) = quote else {
                return Err(CoreError::bad("Pass quote (the exact words on the page) or quads for a PDF highlight."));
            };
            let pages: Vec<usize> = match page {
                Some(p) if p >= 1 => vec![p as usize],
                _ => {
                    let n = match f.page_count {
                        Some(n) if n > 0 => n,
                        _ => self.project.file_info(&f.id)?.get("pages").and_then(|v| v.as_i64()).unwrap_or(0),
                    };
                    (1..=n.max(0) as usize).collect()
                }
            };
            for pg in pages {
                match self.project.add_quote_highlight(&f.id, pg, q, analysis, label, body.tags.as_deref().unwrap_or(&[]), &writer) {
                    Ok(h) => return to_value(h),
                    Err(CoreError::NotFound(msg)) if msg.contains("quote was not found") => continue,
                    Err(e) => return Err(e),
                }
            }
            let place = match page {
                Some(p) => format!("page {}", p),
                None => "any page".to_string(),
            };
            return Err(CoreError::bad(format!(
                "The quote was not found on {} of {}. Check the exact wording with reference_text, or pass quads.",
                place, base
            )));
        }
        if media::MARKDOWN_EXTS.contains(&f.ext.as_str()) {
            let Some(q) = quote else {
                return Err(CoreError::bad("Pass quote: the exact words of the Markdown note to highlight."));
            };
            let text = media::read_markdown(&self.project.usable_path(&f)?)?;
            let target = kurasi_core::util::norm_space(q);
            let mut found: Option<String> = None;
            for hay in [media::markdown_plain(&text), kurasi_core::util::norm_space(&text)] {
                let pos = hay.find(&target).or_else(|| {
                    let lower = hay.to_lowercase();
                    let i = lower.find(&target.to_lowercase())?;
                    if hay.is_char_boundary(i) && hay.is_char_boundary(i + target.len()) {
                        Some(i)
                    } else {
                        None
                    }
                });
                if let Some(i) = pos {
                    found = hay.get(i..i + target.len()).map(str::to_string);
                    if found.is_some() {
                        break;
                    }
                }
            }
            let Some(found) = found else {
                return Err(CoreError::bad(format!("The quote was not found in {}. Copy the words exactly as the note shows them.", base)));
            };
            body.kind = Some("md".into());
            body.anchor = Some(json!({"text": found, "occurrence": 0}));
            body.verbatim = Some(found);
            return to_value(self.project.add_highlight(&body, &writer)?);
        }
        if media::IMAGE_EXTS.contains(&f.ext.as_str()) {
            let Some(q) = quads.filter(|q| !q.is_empty()) else {
                return Err(CoreError::bad("An image highlight needs quads as [[x, y, w, h]] normalised to 0..1."));
            };
            body.kind = Some("region".into());
            body.quads = Some(q);
            body.verbatim = Some(quote.unwrap_or("").to_string());
            return to_value(self.project.add_highlight(&body, &writer)?);
        }
        Err(CoreError::bad("Highlights work on PDF, Markdown and image files."))
    }

    pub fn export_references(&self, format: &str, ref_ids: Option<Vec<String>>) -> CoreResult<Value> {
        let ids = ref_ids.filter(|v| !v.is_empty());
        let path = self.project.export_references(format, ids.as_deref())?;
        Ok(json!({"path": path.to_string_lossy()}))
    }

    pub fn log_append(&self, purpose: &str) -> CoreResult<Value> {
        if purpose.trim().is_empty() {
            return Err(CoreError::bad("Describe the purpose of the AI-assisted step."));
        }
        Ok(json!({"ok": true}))
    }

    // ------------------------------------------------------------ dispatch

    fn prepare(&self, tool: &str, args: Map<String, Value>) -> CoreResult<Invocation> {
        let inv = match tool {
            "catalog_search" => {
                let a: SearchArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: format!("Search: {}", a.query),
                    log_args: json!({"query": a.query, "limit": a.limit.unwrap_or(20)}),
                    result: self.catalog_search(&a.query, a.limit).map(|v| (v, None)),
                }
            }
            "reference_get" => {
                let a: RefArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: format!("Read {}", a.ref_id),
                    log_args: json!({"ref_id": a.ref_id}),
                    result: self.reference_get(&a.ref_id).map(|v| (v, None)),
                }
            }
            "reference_text" => {
                let a: TextArgs = parse_args(tool, args)?;
                let page = a.page.unwrap_or(1);
                Invocation {
                    purpose: format!("Read page {} of {}", page, a.ref_id),
                    log_args: json!({"ref_id": a.ref_id, "page": page, "file_id": a.file_id}),
                    result: self.reference_text(&a.ref_id, Some(page), a.file_id.as_deref()).map(|v| (v, None)),
                }
            }
            "highlights_list" => {
                let a: HighlightsArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: "List highlights".into(),
                    log_args: json!({"ref_id": a.ref_id, "tag": a.tag, "author": a.author}),
                    result: self.highlights_list(a.ref_id, a.tag, a.author).map(|v| (v, None)),
                }
            }
            "tags_list" => Invocation { purpose: "List tags".into(), log_args: json!({}), result: self.tags_list().map(|v| (v, None)) },
            "categories_list" => Invocation {
                purpose: "List categories".into(),
                log_args: json!({}),
                result: self.categories_list().map(|v| (v, None)),
            },
            "reference_update_metadata" => {
                let a: UpdateMetadataArgs = parse_args(tool, args)?;
                let purpose = a.purpose.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| format!("Propose metadata for {}", a.ref_id));
                let log_args = json!({"ref_id": a.ref_id, "fields": a.fields, "authors": a.authors, "categories": a.categories});
                let result = self.reference_update_metadata(&a.ref_id, &a.fields, a.authors, a.categories).map(|(v, prev)| (v, Some(prev)));
                Invocation { purpose, log_args, result }
            }
            "reference_categorize" => {
                let a: CategorizeArgs = parse_args(tool, args)?;
                let ids = a.ref_ids.into_vec();
                let paths = a.category_paths.into_vec();
                let purpose = a.purpose.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "Propose categories".into());
                Invocation {
                    purpose,
                    log_args: json!({"ref_ids": ids, "category_paths": paths}),
                    result: self.reference_categorize(ids, paths).map(|v| (v, None)),
                }
            }
            "highlight_propose_delete" => {
                let a: ProposeDeleteArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: format!("Propose deleting {}: {}", a.highlight_id, a.reason),
                    log_args: json!({"highlight_id": a.highlight_id, "reason": a.reason}),
                    result: self.highlight_propose_delete(&a.highlight_id, &a.reason).map(|v| (v, None)),
                }
            }
            "highlight_suggest_tags" => {
                let a: SuggestTagsArgs = parse_args(tool, args)?;
                let tags = a.tags.into_tags();
                Invocation {
                    purpose: format!("Suggest tags for {}", a.highlight_id),
                    log_args: json!({"highlight_id": a.highlight_id, "tags": tags}),
                    result: self.highlight_suggest_tags(&a.highlight_id, tags).map(|v| (v, None)),
                }
            }
            "catalog_scan" => {
                let a: ScanArgs = parse_args(tool, args)?;
                let full = a.full.unwrap_or(false);
                Invocation {
                    purpose: format!("Scan the project folder{}", if full { " (full)" } else { "" }),
                    log_args: json!({"full": full}),
                    result: self.catalog_scan(full).and_then(to_value).map(|v| (v, None)),
                }
            }
            "highlight_add" => {
                let a: HighlightAddArgs = parse_args(tool, args)?;
                let tags = a.tags.map(OneOrMany::into_tags).unwrap_or_default();
                let purpose = a.purpose.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| format!("Add a highlight to {}", a.ref_id));
                let log_args = json!({
                    "ref_id": a.ref_id, "analysis": a.analysis, "label": a.label, "tags": tags, "page": a.page,
                    "quote": a.quote, "quads": a.quads, "file_id": a.file_id,
                });
                let result = self
                    .highlight_add(&a.ref_id, &a.analysis, a.label.as_deref(), tags, a.page, a.quote.as_deref(), a.quads, a.file_id.as_deref())
                    .map(|v| (v, None));
                Invocation { purpose, log_args, result }
            }
            "export_references" => {
                let a: ExportArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: format!("Export {}", a.format),
                    log_args: json!({"format": a.format, "ref_ids": a.ref_ids}),
                    result: self.export_references(&a.format, a.ref_ids).map(|v| (v, None)),
                }
            }
            "log_append" => {
                let a: LogArgs = parse_args(tool, args)?;
                Invocation {
                    purpose: a.purpose.clone(),
                    log_args: json!({"checks": a.checks}),
                    result: self.log_append(&a.purpose).map(|v| (v, None)),
                }
            }
            other => {
                return Err(CoreError::bad(format!("Unknown tool {}. The tools are: {}.", other, TOOLS.iter().map(|t| t.0).collect::<Vec<_>>().join(", "))));
            }
        };
        Ok(inv)
    }

    /// Runs one tool with mode gating, the `ai_log` row and the change row of a quiet write.
    /// `Err` carries the one-sentence message the client sees as a tool error.
    pub fn run(&self, tool: &str, args: Map<String, Value>, client: &str, mode: Mode) -> std::result::Result<Value, String> {
        let client = if client.trim().is_empty() { "unknown client" } else { client };
        let needed = match tool_mode(tool) {
            Some(m) => m,
            None => {
                let msg = format!("Unknown tool {}. The tools are: {}.", tool, TOOLS.iter().map(|t| t.0).collect::<Vec<_>>().join(", "));
                let _ = self.project.log_ai(tool, client, &format!("Call {}", tool), &Value::Object(args), &format!("Error: {}", msg), None);
                return Err(msg);
            }
        };
        if needed > mode {
            let msg = format!(
                "The Kurasi MCP server runs in {} mode, so {} (a {} tool) is refused. Start it with --mode {} to allow it.",
                mode, tool, needed, needed
            );
            let _ = self.project.log_ai(tool, client, &format!("Call {}", tool), &Value::Object(args), &format!("Error: {}", msg), None);
            return Err(msg);
        }
        let seq_before = self.project.change_seq();
        let inv = match self.prepare(tool, args.clone()) {
            Ok(inv) => inv,
            Err(e) => {
                let msg = e.message();
                let _ = self.project.log_ai(tool, client, &format!("Call {}", tool), &Value::Object(args), &format!("Error: {}", msg), None);
                return Err(msg);
            }
        };
        match inv.result {
            Err(e) => {
                let msg = e.message();
                let _ = self.project.log_ai(tool, client, &inv.purpose, &inv.log_args, &format!("Error: {}", msg), None);
                Err(msg)
            }
            Ok((value, extra)) => {
                let mut log_args = inv.log_args;
                if let Some(prev) = extra {
                    if let Some(obj) = log_args.as_object_mut() {
                        obj.insert("previous".into(), prev);
                    }
                }
                let summary = summary_of(tool, &value);
                if let Err(e) = self.project.log_ai(tool, client, &inv.purpose, &log_args, &summary, None) {
                    tracing::warn!("ai_log row for {} failed: {}", tool, e);
                }
                if needed != Mode::Read && self.project.change_seq() == seq_before {
                    // A write that changed no catalogue row (an export, a log entry, a quiet
                    // scan) still tells the open app what the agent did.
                    let what = format!("{}: {}", inv.purpose, summary);
                    if let Err(e) = self.project.log_change(&what, &[], Origin::Mcp) {
                        tracing::warn!("change row for {} failed: {}", tool, e);
                    }
                }
                Ok(value)
            }
        }
    }

    // ------------------------------------------------------------ resources

    pub fn read_resource(&self, uri: &str) -> CoreResult<String> {
        let value = match uri {
            "kurasi://references" => json!(self.project.all_references()?.iter().map(strip).collect::<Vec<_>>()),
            "kurasi://tags" => json!(self.project.tags()?),
            _ => {
                let rest = uri
                    .strip_prefix("kurasi://reference/")
                    .ok_or_else(|| CoreError::not_found(format!("There is no resource {}.", uri)))?;
                if let Some(ref_id) = rest.strip_suffix("/highlights") {
                    self.project.reference(ref_id)?;
                    json!(self.project.highlights(&HighlightFilter { ref_id: Some(ref_id.to_string()), ..Default::default() })?)
                } else if !rest.is_empty() && !rest.contains('/') {
                    strip(&self.project.reference(rest)?)
                } else {
                    return Err(CoreError::not_found(format!("There is no resource {}.", uri)));
                }
            }
        };
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

fn summary_of(tool: &str, result: &Value) -> String {
    let Some(obj) = result.as_object() else { return "ok".into() };
    let len = |key: &str| obj.get(key).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    match tool {
        "catalog_search" => format!("{} results", len("results")),
        "catalog_scan" => {
            let n = |key: &str| obj.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
            format!(
                "{} new, {} changed, {} moved, {} missing, {} unchanged, {} highlights added, {} removed",
                n("new"),
                n("changed"),
                n("moved"),
                n("missing"),
                n("unchanged"),
                n("highlights_added"),
                n("highlights_removed")
            )
        }
        "highlights_list" => format!("{} highlights", len("highlights")),
        "tags_list" => format!("{} tags", len("tags")),
        "highlight_add" => format!(
            "1 highlight {}, author {}",
            obj.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            obj.get("author").and_then(|v| v.as_str()).unwrap_or("")
        ),
        _ => {
            if let Some(p) = obj.get("path").and_then(|v| v.as_str()) {
                format!("written: {}", p)
            } else if let Some(id) = obj.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                format!("ok: {}", id)
            } else {
                "ok".into()
            }
        }
    }
}

// ------------------------------------------------------------------ the rmcp handler

/// The MCP server over one project.
#[derive(Clone)]
pub struct KurasiMcp {
    tools: Arc<Tools>,
    mode: Mode,
}

impl KurasiMcp {
    pub fn new(project: Project, mode: Mode) -> KurasiMcp {
        KurasiMcp { tools: Arc::new(Tools::new(project)), mode }
    }

    pub fn tools(&self) -> &Arc<Tools> {
        &self.tools
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn instructions(&self) -> String {
        let p = self.tools.project();
        let author = match p.setting("ai_author") {
            Value::String(s) if !s.trim().is_empty() => s,
            _ => "Claude".to_string(),
        };
        format!(
            "Kurasi reference catalogue of the project {} ({}). Mode: {}. Reads are open. Proposals (metadata with status Suggested, delete suggestions, suggested tags) wait for the person's review. Highlights you add carry the author {}. Log each AI-assisted step with log_append.",
            p.name(),
            p.folder().display(),
            self.mode,
            author
        )
    }
}

/// The request `_meta` key that carries the client identity in the 2026-07-28 lifecycle
/// (`server/discover`). That lifecycle has no `initialize`, so `peer_info()` stays empty.
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";

/// The client name for the `ai_log` row. The first non-empty name wins: the `clientInfo` in
/// the `_meta` of this request (protocol 2026-07-28 sends it with every request), then the
/// `clientInfo` of `initialize` (older protocols send it once), then "unknown client". The
/// name is read from the raw `_meta` value, so a `clientInfo` without `version` still counts.
fn client_name(ctx: &RequestContext<RoleServer>) -> String {
    let from_meta = ctx
        .meta
        .get(META_CLIENT_INFO)
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let from_initialize = ctx.peer.peer_info().map(|info| info.client_info.name.clone());
    from_meta
        .into_iter()
        .chain(from_initialize)
        .find(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "unknown client".to_string())
}

fn tool_result(out: std::result::Result<Value, String>) -> CallToolResult {
    match out {
        Ok(value) => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".into());
            let structured = if value.is_object() { Some(value) } else { None };
            let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
            result.structured_content = structured;
            result
        }
        Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
    }
}

impl ServerHandler for KurasiMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("kurasi", env!("CARGO_PKG_VERSION")))
            .with_instructions(self.instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tool_definitions()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let client = client_name(&context);
        let tools = self.tools.clone();
        let mode = self.mode;
        let name = request.name.to_string();
        let args = request.arguments.unwrap_or_default();
        let out = tokio::task::spawn_blocking(move || tools.run(&name, args, &client, mode))
            .await
            .map_err(|e| ErrorData::internal_error(format!("Kurasi could not complete the tool call: {}", e), None))?;
        Ok(tool_result(out).into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, ErrorData> {
        let items = RESOURCES
            .iter()
            .filter(|(u, _)| !u.contains('{'))
            .map(|(u, d)| {
                Resource::new(*u, u.trim_start_matches("kurasi://").to_string())
                    .with_description(*d)
                    .with_mime_type("application/json")
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(items))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourceTemplatesResult, ErrorData> {
        let items = RESOURCES
            .iter()
            .filter(|(u, _)| u.contains('{'))
            .map(|(u, d)| {
                ResourceTemplate::new(*u, u.trim_start_matches("kurasi://").replace(['{', '}'], "").replace('/', "_"))
                    .with_description(*d)
                    .with_mime_type("application/json")
            })
            .collect();
        Ok(ListResourceTemplatesResult::with_all_items(items))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResponse, ErrorData> {
        let tools = self.tools.clone();
        let uri = request.uri.clone();
        let text = tokio::task::spawn_blocking(move || tools.read_resource(&uri))
            .await
            .map_err(|e| ErrorData::internal_error(format!("Kurasi could not read the resource: {}", e), None))?
            .map_err(|e| match e {
                CoreError::NotFound(_) => ErrorData::resource_not_found(e.message(), None),
                other => ErrorData::internal_error(other.message(), None),
            })?;
        let contents = ResourceContents::text(text, request.uri).with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

/// Serves the project over stdio until the client closes the session.
pub async fn mcp_stdio(project: Project, mode: Mode) -> Result<()> {
    let handler = KurasiMcp::new(project, mode);
    let service = handler
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| Error::Mcp(e.to_string()))?;
    service.waiting().await.map_err(|e| Error::Mcp(e.to_string()))?;
    Ok(())
}
