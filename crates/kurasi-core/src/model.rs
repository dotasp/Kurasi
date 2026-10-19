//! Serde structs shared by the API, the MCP server and the UI (snake_case JSON as in
//! CONTRACT.md section 6), plus the raw database rows.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const META_FIELDS: &[&str] = &[
    "title", "kind", "year", "date", "pub", "vol", "issue", "pages_range", "doi", "url", "lang", "usability",
    "status", "register_id",
];
pub const DETAIL_FIELDS: &[&str] = &["doi", "url", "vol", "issue", "pages_range"];
pub const USABILITY: &[&str] = &["Full text", "Abstract only", "Excerpt", "Translated", ""];
pub const STATUSES: &[&str] = &["Auto", "Suggested", "Verified"];
pub const LABELS: &[&str] = &["F", "I", "P"];
pub const UNCATEGORIZED: i64 = 1;

// ------------------------------------------------------------------ database rows

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RefRow {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub kind: String,
    pub year: String,
    pub date: String,
    #[serde(rename = "pub")]
    pub pub_: String,
    pub vol: String,
    pub issue: String,
    pub pages_range: String,
    pub doi: String,
    pub url: String,
    pub lang: String,
    pub usability: String,
    pub status: String,
    pub register_id: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl RefRow {
    pub fn field(&self, name: &str) -> Option<&str> {
        Some(match name {
            "title" => &self.title,
            "kind" => &self.kind,
            "year" => &self.year,
            "date" => &self.date,
            "pub" => &self.pub_,
            "vol" => &self.vol,
            "issue" => &self.issue,
            "pages_range" => &self.pages_range,
            "doi" => &self.doi,
            "url" => &self.url,
            "lang" => &self.lang,
            "usability" => &self.usability,
            "status" => &self.status,
            "register_id" => &self.register_id,
            _ => return None,
        })
    }

    pub fn set_field(&mut self, name: &str, value: &str) -> bool {
        let slot = match name {
            "title" => &mut self.title,
            "kind" => &mut self.kind,
            "year" => &mut self.year,
            "date" => &mut self.date,
            "pub" => &mut self.pub_,
            "vol" => &mut self.vol,
            "issue" => &mut self.issue,
            "pages_range" => &mut self.pages_range,
            "doi" => &mut self.doi,
            "url" => &mut self.url,
            "lang" => &mut self.lang,
            "usability" => &mut self.usability,
            "status" => &mut self.status,
            "register_id" => &mut self.register_id,
            _ => return false,
        };
        *slot = value.to_string();
        true
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FileRow {
    pub id: String,
    pub ref_id: String,
    pub relpath: String,
    pub ext: String,
    pub sha256: Option<String>,
    pub size: i64,
    pub mtime: f64,
    pub page_count: Option<i64>,
    pub role: String,
    pub cloud_only: bool,
    pub missing: bool,
    pub missing_since: Option<String>,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
    pub view_rotation: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HlRow {
    pub id: String,
    pub file_id: String,
    pub ref_id: String,
    pub page: Option<i64>,
    pub kind: String,
    pub quads: String,
    pub anchor: Option<String>,
    pub verbatim: String,
    pub analysis: String,
    pub label: Option<String>,
    pub tags: String,
    pub author: String,
    pub origin: String,
    pub colour: Option<String>,
    pub pdf_xref: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub suggestion: Option<String>,
    pub suggested_tags: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryRow {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort: i64,
}

// ------------------------------------------------------------------ API records

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub id: String,
    pub relpath: String,
    pub role: String,
    pub ext: String,
    pub pages: i64,
    pub missing: bool,
    pub cloud_only: bool,
    pub hl_count: i64,
}

/// One catalogue record (`detail` adds doi, url, vol, issue, pages_range). `/api/reference/<id>`
/// returns it whole; the list of `/api/references` sends the fields the UI draws from (see
/// `kurasi-server` `api::ListRef`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub authors: Vec<String>,
    pub kind: String,
    #[serde(rename = "pub")]
    pub pub_: String,
    pub year: String,
    pub date: String,
    pub lang: String,
    pub usability: String,
    pub status: String,
    pub register_id: String,
    pub categories: Vec<String>,
    pub ext: String,
    pub pages: i64,
    pub hl_count: i64,
    pub top_tags: Vec<String>,
    pub tags: Vec<String>,
    pub missing: bool,
    pub files: Vec<FileEntry>,
    pub primary_file_id: Option<String>,
    pub thumb_url: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub search_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_range: Option<String>,
}

impl Reference {
    /// A detail field, empty when the record was built without detail.
    pub fn detail(&self, name: &str) -> &str {
        match name {
            "doi" => self.doi.as_deref().unwrap_or(""),
            "url" => self.url.as_deref().unwrap_or(""),
            "vol" => self.vol.as_deref().unwrap_or(""),
            "issue" => self.issue.as_deref().unwrap_or(""),
            "pages_range" => self.pages_range.as_deref().unwrap_or(""),
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Highlight {
    pub id: String,
    pub file_id: String,
    pub ref_id: String,
    pub page: Option<i64>,
    pub kind: String,
    pub quads: Vec<Vec<f64>>,
    pub anchor: Option<Value>,
    pub verbatim: String,
    pub analysis: String,
    pub label: Option<String>,
    pub tags: Vec<String>,
    pub author: String,
    pub origin: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub suggestion: Option<Value>,
    pub suggested_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Author {
    pub name: String,
    pub ref_ids: Vec<String>,
    pub hl_count: i64,
    pub top_ref_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub name: String,
    pub ref_ids: Vec<String>,
    pub hl_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub parent_id: Option<i64>,
    pub sort: i64,
    #[serde(default)]
    pub children: Vec<Category>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiLogRow {
    pub id: i64,
    pub at: String,
    pub tool: String,
    pub client: String,
    pub purpose: String,
    pub args: Value,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangeRow {
    pub seq: i64,
    pub at: String,
    pub what: String,
    pub ids: Vec<String>,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Changes {
    pub seq: i64,
    pub changes: Vec<ChangeRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Setting {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub score: f64,
    pub reference: Reference,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Counts {
    pub references: i64,
    pub authors: i64,
    pub tags: i64,
    pub highlights: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrashResult {
    pub trash_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Restored {
    pub restored: String,
    pub id: String,
}

/// Filter for `Project::references`.
#[derive(Debug, Clone, Default)]
pub struct ReferenceFilter {
    /// Only these references; an id that is unknown or in the trash is left out.
    pub ids: Option<Vec<String>>,
    /// Adds doi, url, vol, issue and pages_range.
    pub detail: bool,
    /// Leaves `search_text` empty (the catalogue list of `/api/references`).
    pub lean: bool,
}

/// `GET /api/search`: the full-text part of the catalogue score (`Project::text_search`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextSearch {
    pub query: String,
    pub loose: bool,
    pub tokens: Vec<String>,
    pub hits: Vec<TextHit>,
}

/// One reference whose `search_text` scores on at least one token. `scores[i]` belongs to
/// `tokens[i]`: 1 for a substring hit, else 0.9 times the best bigram Dice similarity of a word
/// when it reaches the threshold, else 0. `score` is their sum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextHit {
    pub id: String,
    pub score: f64,
    pub scores: Vec<f64>,
}

/// Filter for `Project::highlights`.
#[derive(Debug, Clone, Default)]
pub struct HighlightFilter {
    pub ref_id: Option<String>,
    pub file_id: Option<String>,
    pub tag: Option<String>,
    pub author: Option<String>,
}

/// Body of `PUT /api/reference/<id>` and of `reference_update_metadata`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReferencePatch {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
}

/// Body of `POST /api/highlights`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewHighlight {
    pub file_id: String,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub quads: Option<Vec<Vec<f64>>>,
    #[serde(default)]
    pub anchor: Option<Value>,
    #[serde(default)]
    pub verbatim: Option<String>,
    #[serde(default)]
    pub analysis: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Body of `PUT /api/highlights/<id>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HighlightPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `Some(None)` clears the label.
    #[serde(default, deserialize_with = "deserialize_label", skip_serializing_if = "Option::is_none")]
    pub label: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbatim: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_tags: Option<Vec<String>>,
}

fn deserialize_label<'de, D>(d: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<String> = Option::deserialize(d)?;
    Ok(Some(v))
}

/// Who writes: the UI (`ui`), the MCP server (`mcp`) or the scanner (`scan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Ui,
    Mcp,
    Scan,
}

impl Origin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Origin::Ui => "ui",
            Origin::Mcp => "mcp",
            Origin::Scan => "scan",
        }
    }
}

/// Who authored a highlight write: the person in the UI or an AI through MCP.
#[derive(Debug, Clone, PartialEq)]
pub struct Writer {
    pub author: String,
    pub origin: String,
    pub change_origin: Origin,
}

impl Writer {
    pub fn human() -> Writer {
        Writer { author: "You".into(), origin: "human".into(), change_origin: Origin::Ui }
    }
    pub fn ai(author: &str) -> Writer {
        Writer { author: author.to_string(), origin: "ai".into(), change_origin: Origin::Mcp }
    }
}
