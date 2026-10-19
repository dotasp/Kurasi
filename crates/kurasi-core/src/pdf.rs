//! The PDF backend interface and the text helpers around annotations.
//!
//! `kurasi-core` never links a PDF library. `kurasi-pdf` implements [`PdfBackend`] with
//! pdfium; tests and the sterile sample use the small built-in writer in `simplepdf`.
//! `NoPdf` answers every call with `Error::Unsupported`, so a project without a backend
//! still scans, groups and catalogues its files and leaves PDF annotations unread.
//!
//! Coordinates: words and quads are in unrotated page points with the origin at the top
//! left and y growing downwards, the frame PyMuPDF reports and the contract uses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const MARKUP_KINDS: &[&str] = &["Highlight", "Underline", "Squiggly", "StrikeOut"];
pub const NOTE_KINDS: &[&str] = &["Text", "FreeText"];

/// A word with its box in page points and its block and line numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub text: String,
    pub block: u32,
    pub line: u32,
}

/// Document metadata the scanner guesses from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
}

/// One page's size (unrotated points) and rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
}

/// A raw annotation as the backend reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annot {
    /// Object number (PyMuPDF `xref`), the stable id across saves.
    pub xref: i64,
    /// 1-based page number.
    pub page: usize,
    /// PDF subtype: Highlight, Underline, Squiggly, StrikeOut, Text, FreeText.
    pub subtype: String,
    /// One rectangle per quad, `[x0, y0, x1, y1]` in page points; the annotation rectangle for notes.
    pub quads: Vec<Vec<f64>>,
    /// `/T`, the author.
    pub author: String,
    /// `/Contents`.
    pub contents: String,
    /// `/C` as `#rrggbb`.
    pub colour: Option<String>,
    /// `/CreationDate` and `/M` as ISO timestamps.
    pub created: Option<String>,
    pub modified: Option<String>,
    /// `/IRT` target, 0 when the annotation is no reply.
    pub irt_xref: i64,
}

/// A highlight to add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAnnot {
    pub page: usize,
    pub quads: Vec<Vec<f64>>,
    pub author: String,
    pub contents: String,
    pub colour: String,
    /// `text` or `region` writes a Highlight; `note` writes a Text annotation at the first quad.
    pub kind: String,
    pub created: Option<String>,
}

/// Fields of an annotation to change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotPatch {
    pub contents: Option<String>,
    pub author: Option<String>,
    pub colour: Option<String>,
}

/// Everything needed to recreate one deleted annotation (kept in `trash`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotSnapshot {
    pub xref: i64,
    pub page: usize,
    pub subtype: String,
    pub quads: Vec<Vec<f64>>,
    pub author: String,
    pub contents: String,
    pub colour: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub irt_xref: i64,
    /// Backend-specific extra data (raw dictionary entries) kept opaque.
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// An open document. Writes stay in memory until `save_in_place`. `Send`, so a document opened
/// for one request can wait in a [`DocCache`] for the next.
pub trait PdfDoc: Send {
    fn page_count(&self) -> usize;
    fn metadata(&self) -> Result<PdfMetadata>;
    fn pages(&self) -> Result<Vec<PageInfo>>;
    /// PNG of page `page` (1-based) at `scale` times 72 dpi, without annotations.
    fn render_png(&self, page: usize, scale: f64) -> Result<Vec<u8>>;
    /// PNG of the first page scaled to `width_px` wide.
    fn thumbnail_png(&self, width_px: u32) -> Result<Vec<u8>>;
    fn words(&self, page: usize) -> Result<Vec<Word>>;
    fn annotations(&self) -> Result<Vec<Annot>>;
    /// Returns the annotation as written (xref and quads).
    fn add_highlight(&mut self, h: &NewAnnot) -> Result<Annot>;
    fn update_highlight(&mut self, xref: i64, patch: &AnnotPatch) -> Result<()>;
    /// Deletes the annotation and its replies; returns their snapshots, parent first.
    fn delete_highlight(&mut self, xref: i64) -> Result<Vec<AnnotSnapshot>>;
    /// Recreates snapshots; returns old xref to new xref.
    fn restore_annotations(&mut self, snaps: &[AnnotSnapshot]) -> Result<BTreeMap<i64, i64>>;
    /// Turns one page (`Some(n)`, 1-based) or every page by `delta` degrees; returns every page's rotation.
    fn rotate(&mut self, page: Option<usize>, delta: i32) -> Result<Vec<i32>>;
    /// Saves to `.~kurasi-<name>.tmp` in the same folder, verifies, then renames over the
    /// original with six tries (`lock::replace_retry`).
    fn save_in_place(&mut self) -> Result<()>;
}

/// Opens documents. `kurasi-pdf` implements it with pdfium.
pub trait PdfBackend: Send + Sync {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>>;
    fn name(&self) -> &'static str {
        "pdf"
    }
}

/// The backend used when no PDF library is available.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoPdf;

impl PdfBackend for NoPdf {
    fn open(&self, _path: &Path) -> Result<Box<dyn PdfDoc>> {
        Err(Error::unsupported("No PDF backend is available in this build, so PDF pages and annotations cannot be read."))
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

// ------------------------------------------------------------------ open documents

/// One document in a [`DocCache`]: the document behind a mutex and its page list once read.
pub struct OpenDoc {
    path: PathBuf,
    size: u64,
    mtime: f64,
    doc: Mutex<Box<dyn PdfDoc>>,
    pages: Mutex<Option<Arc<Vec<PageInfo>>>>,
}

impl OpenDoc {
    /// The document. Hold the guard for one call or a short run of calls.
    pub fn doc(&self) -> MutexGuard<'_, Box<dyn PdfDoc>> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Size and rotation of every page, read on first use and kept with the entry. Walking
    /// the pages loads each one, about a millisecond a page with pdfium.
    pub fn pages(&self) -> Result<Arc<Vec<PageInfo>>> {
        let mut slot = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let pages = Arc::new(self.doc().pages()?);
        *slot = Some(pages.clone());
        Ok(pages)
    }

    fn matches(&self, path: &Path, size: u64, mtime: f64) -> bool {
        self.path == path && self.size == size && self.mtime == mtime
    }
}

/// Documents kept open for reading, so a viewer that asks for one page after another opens
/// the file once. An entry is keyed by the path, size and modification time of the file: a
/// file another program changed opens again, and Kurasi's own writes call [`DocCache::forget`]
/// after the save. At most `capacity` documents stay open; the one used longest ago closes
/// first. A cached document holds no handle on the file (both backends read it into memory).
pub struct DocCache {
    capacity: usize,
    slots: Mutex<Vec<Arc<OpenDoc>>>,
    /// Taken while a missing document opens, so two requests that miss at once open it once.
    opening: Mutex<()>,
}

impl DocCache {
    pub fn new(capacity: usize) -> DocCache {
        DocCache { capacity: capacity.max(1), slots: Mutex::new(Vec::new()), opening: Mutex::new(()) }
    }

    fn slots(&self) -> MutexGuard<'_, Vec<Arc<OpenDoc>>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The entry for the file as it is on disk now, moved to the front. Entries of an older
    /// version of the same file are taken out and returned in `stale`.
    fn lookup(&self, path: &Path, size: u64, mtime: f64, stale: &mut Vec<Arc<OpenDoc>>) -> Option<Arc<OpenDoc>> {
        let mut slots = self.slots();
        if let Some(i) = slots.iter().position(|d| d.matches(path, size, mtime)) {
            let d = slots.remove(i);
            slots.insert(0, d.clone());
            return Some(d);
        }
        let (old, keep): (Vec<_>, Vec<_>) = slots.drain(..).partition(|d| d.path == path);
        *slots = keep;
        stale.extend(old);
        None
    }

    /// The open document for `path`, opened with `backend` when the cache has no entry for
    /// the file as it is on disk now.
    pub fn get(&self, backend: &dyn PdfBackend, path: &Path) -> Result<Arc<OpenDoc>> {
        let meta = std::fs::metadata(path)?;
        let (size, mtime) = (meta.len(), crate::util::mtime_secs(&meta));
        // Documents leave the cache through `closed` and close after every lock is released.
        let mut closed: Vec<Arc<OpenDoc>> = Vec::new();
        if let Some(d) = self.lookup(path, size, mtime, &mut closed) {
            return Ok(d);
        }
        let _opening = self.opening.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = self.lookup(path, size, mtime, &mut closed) {
            return Ok(d);
        }
        // Opened outside the slot lock, so pages of other files keep coming from the cache.
        let doc = backend.open(path)?;
        let entry = Arc::new(OpenDoc { path: path.to_path_buf(), size, mtime, doc: Mutex::new(doc), pages: Mutex::new(None) });
        {
            let mut slots = self.slots();
            slots.insert(0, entry.clone());
            while slots.len() > self.capacity {
                closed.extend(slots.pop());
            }
        }
        drop(_opening);
        drop(closed);
        Ok(entry)
    }

    /// Drops the entry of `path`, after Kurasi saved the file.
    pub fn forget(&self, path: &Path) {
        let gone: Vec<Arc<OpenDoc>> = {
            let mut slots = self.slots();
            let (gone, keep): (Vec<_>, Vec<_>) = slots.drain(..).partition(|d| d.path == path);
            *slots = keep;
            gone
        };
        drop(gone);
    }

    /// Number of open documents.
    pub fn len(&self) -> usize {
        self.slots().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ------------------------------------------------------------------ notes, tags, labels

fn tags_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^[ \t]*Tags:[ \t]*((?:#\S+[ \t]*)*)$").unwrap())
}

fn label_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^[ \t]*\((F|I|P)\)").unwrap())
}

pub fn clean_tags<S: AsRef<str>>(tags: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.as_ref().trim().trim_start_matches('#').trim().trim_end_matches([',', ';']).to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// `(analysis, tags)`: a final line `Tags: #a #b` is removed and parsed into tags.
pub fn split_tags(content: &str) -> (String, Vec<String>) {
    let text = content.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_end();
    let lines: Vec<&str> = text.split('\n').collect();
    let last = match lines.last() {
        Some(l) => *l,
        None => return (text.to_string(), Vec::new()),
    };
    match tags_line_re().captures(last) {
        Some(caps) => {
            let raw: Vec<&str> = caps.get(1).map(|m| m.as_str()).unwrap_or("").split([' ', '\t', ',', '\n']).collect();
            let tags = clean_tags(&raw);
            let head = lines[..lines.len() - 1].join("\n");
            (head.trim_end().to_string(), tags)
        }
        None => (text.to_string(), Vec::new()),
    }
}

/// analysis stripped of trailing space, then a blank line and `Tags: #a #b` when tagged.
pub fn compose(analysis: &str, tags: &[String]) -> String {
    let mut text = analysis.trim_end().to_string();
    let tags = clean_tags(tags);
    if !tags.is_empty() {
        text.push_str("\n\nTags: ");
        text.push_str(&tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "));
    }
    text
}

/// The first `(F)`, `(I)` or `(P)` at the start of a paragraph.
pub fn parse_label(analysis: &str) -> Option<String> {
    label_re().captures(analysis).map(|c| c[1].to_string())
}

pub fn colour_hex(rgb: &[f64]) -> Option<String> {
    if rgb.len() < 3 {
        return None;
    }
    let ch = |v: f64| ((v * 255.0).round().clamp(0.0, 255.0)) as u8;
    Some(format!("#{:02x}{:02x}{:02x}", ch(rgb[0]), ch(rgb[1]), ch(rgb[2])))
}

pub fn hex_rgb(value: &str) -> (f64, f64, f64) {
    let v = value.trim().trim_start_matches('#');
    let v = if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) { v } else { "ffd84a" };
    let ch = |i: usize| u8::from_str_radix(&v[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
    (ch(0), ch(2), ch(4))
}

/// `D:YYYYMMDDHHmmSS...` to `YYYY-MM-DDTHH:MM:SS`.
pub fn pdf_date(raw: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^D?:?(\d{4})(\d{2})?(\d{2})?(\d{2})?(\d{2})?(\d{2})?").unwrap());
    let caps = re.captures(raw.trim())?;
    let num = |i: usize, default: u32| caps.get(i).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(default);
    let (y, mo, d, h, mi, s) = (num(1, 0), num(2, 1), num(3, 1), num(4, 0), num(5, 0), num(6, 0));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, mi, s))
}

/// ISO timestamp to `D:YYYYMMDDHHmmSS`.
pub fn to_pdf_date(iso: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})").unwrap());
    let caps = re.captures(iso.trim())?;
    Some(format!("D:{}{}{}{}{}{}", &caps[1], &caps[2], &caps[3], &caps[4], &caps[5], &caps[6]))
}

// ------------------------------------------------------------------ words and quads

/// Words whose box overlaps a quad by at least half its height and 30% of its width.
pub fn words_under(words: &[Word], rects: &[Vec<f64>]) -> String {
    let mut hits: Vec<&str> = Vec::new();
    for w in words {
        let (ww, wh) = (w.x1 - w.x0, w.y1 - w.y0);
        if ww <= 0.0 || wh <= 0.0 {
            continue;
        }
        for r in rects {
            if r.len() < 4 {
                continue;
            }
            let (x0, y0) = (w.x0.max(r[0]), w.y0.max(r[1]));
            let (x1, y1) = (w.x1.min(r[2]), w.y1.min(r[3]));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            if (y1 - y0) >= 0.5 * wh && (x1 - x0) >= 0.3 * ww {
                hits.push(&w.text);
                break;
            }
        }
    }
    crate::util::norm_space(&hits.join(" "))
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn rect_list(rects: &[[f64; 4]]) -> Vec<Vec<f64>> {
    rects.iter().map(|r| vec![round2(r[0]), round2(r[1]), round2(r[2]), round2(r[3])]).collect()
}

/// Quads (one per text line) and verbatim of a quote among the words of one page, or `None`.
/// Whitespace is normalised. A case-sensitive match is tried first, then a case-insensitive one.
pub fn find_quote(words: &[Word], quote: &str) -> Option<(Vec<Vec<f64>>, String)> {
    let target = crate::util::norm_space(quote);
    if target.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut starts = Vec::with_capacity(words.len());
    for w in words {
        starts.push(text.len());
        text.push_str(&w.text);
        text.push(' ');
    }
    let idx = match text.find(&target) {
        Some(i) => i,
        None => {
            let lower = text.to_lowercase();
            let tl = target.to_lowercase();
            // Byte offsets of the lower-cased text line up with the original only for ASCII,
            // so map back through char counts.
            let pos = lower.find(&tl)?;
            let chars_before = lower[..pos].chars().count();
            text.char_indices().nth(chars_before).map(|(i, _)| i)?
        }
    };
    let end = idx + target.len();
    let mut sel: Vec<usize> = Vec::new();
    for (i, s) in starts.iter().enumerate() {
        let wl = words[i].text.len();
        if *s < end && s + wl > idx {
            sel.push(i);
        }
    }
    if sel.is_empty() {
        return None;
    }
    let mut lines: Vec<((u32, u32), [f64; 4])> = Vec::new();
    for &i in &sel {
        let w = &words[i];
        let key = (w.block, w.line);
        match lines.iter_mut().find(|(k, _)| *k == key) {
            Some((_, r)) => {
                r[0] = r[0].min(w.x0);
                r[1] = r[1].min(w.y0);
                r[2] = r[2].max(w.x1);
                r[3] = r[3].max(w.y1);
            }
            None => lines.push((key, [w.x0, w.y0, w.x1, w.y1])),
        }
    }
    let rects: Vec<[f64; 4]> = lines.into_iter().map(|(_, r)| r).collect();
    let verbatim = sel.iter().map(|&i| words[i].text.as_str()).collect::<Vec<_>>().join(" ");
    Some((rect_list(&rects), verbatim))
}

/// One highlight or note as the scanner stores it, built from the raw annotations of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotRecord {
    pub page: i64,
    pub kind: String,
    pub quads: Vec<Vec<f64>>,
    pub verbatim: String,
    pub analysis: String,
    pub tags: Vec<String>,
    pub label: Option<String>,
    pub author: String,
    pub origin: String,
    pub colour: Option<String>,
    pub pdf_xref: i64,
    pub created: Option<String>,
    pub modified: Option<String>,
}

/// What a scan reads from one PDF.
#[derive(Debug, Clone, Default)]
pub struct Inspection {
    pub page_count: i64,
    pub metadata: PdfMetadata,
    pub annotations: Vec<AnnotRecord>,
}

/// Page count, metadata and every highlight or note annotation of a PDF.
pub fn inspect(doc: &dyn PdfDoc, ai_authors: &[String]) -> Result<Inspection> {
    let ai: Vec<String> = ai_authors.iter().map(|a| a.to_lowercase()).collect();
    let metadata = doc.metadata().unwrap_or_default();
    let raw = doc.annotations()?;
    let mut records: Vec<AnnotRecord> = Vec::new();
    let mut page_words: BTreeMap<usize, Vec<Word>> = BTreeMap::new();
    let xrefs: Vec<i64> = raw.iter().map(|a| a.xref).collect();
    // Replies to another annotation are folded into the parent's analysis.
    let mut by_xref: BTreeMap<i64, usize> = BTreeMap::new();
    let mut replies: Vec<&Annot> = Vec::new();
    for a in &raw {
        let is_markup = MARKUP_KINDS.contains(&a.subtype.as_str());
        let is_note = NOTE_KINDS.contains(&a.subtype.as_str());
        if !is_markup && !is_note {
            continue;
        }
        if is_note && a.irt_xref != 0 && xrefs.contains(&a.irt_xref) {
            replies.push(a);
            continue;
        }
        let (verbatim, kind) = if is_markup {
            let words = match page_words.get(&a.page) {
                Some(w) => w,
                None => {
                    let w = doc.words(a.page).unwrap_or_default();
                    page_words.insert(a.page, w);
                    &page_words[&a.page]
                }
            };
            let v = words_under(words, &a.quads);
            let kind = if v.is_empty() { "region" } else { "text" };
            (v, kind.to_string())
        } else {
            (String::new(), "note".to_string())
        };
        let contents = a.contents.replace("\r\n", "\n").replace('\r', "\n");
        let (analysis, tags) = split_tags(&contents);
        let author = crate::util::norm_space(&a.author);
        let author = if author.is_empty() { "Unknown".to_string() } else { author };
        let origin = if ai.contains(&author.to_lowercase()) { "ai" } else { "human" };
        by_xref.insert(a.xref, records.len());
        records.push(AnnotRecord {
            page: a.page as i64,
            kind,
            quads: a.quads.iter().map(|q| q.iter().map(|v| round2(*v)).collect()).collect(),
            verbatim,
            analysis,
            tags,
            label: None,
            author,
            origin: origin.to_string(),
            colour: a.colour.clone(),
            pdf_xref: a.xref,
            created: a.created.clone(),
            modified: a.modified.clone(),
        });
    }
    let mut pending = replies;
    loop {
        let mut left = Vec::new();
        for r in &pending {
            match by_xref.get(&r.irt_xref).copied() {
                Some(idx) => {
                    by_xref.insert(r.xref, idx);
                    let body = crate::util::norm_space(&r.contents);
                    if !body.is_empty() {
                        let who = crate::util::norm_space(&r.author);
                        let line = format!("Reply from {}: {}", if who.is_empty() { "Unknown" } else { &who }, body);
                        let parent = &mut records[idx];
                        if !parent.analysis.contains(&line) {
                            if parent.analysis.is_empty() {
                                parent.analysis = line;
                            } else {
                                parent.analysis = format!("{}\n{}", parent.analysis, line);
                            }
                        }
                    }
                }
                None => left.push(*r),
            }
        }
        if left.len() == pending.len() {
            break;
        }
        pending = left;
    }
    for rec in records.iter_mut() {
        rec.label = parse_label(&rec.analysis);
    }
    Ok(Inspection { page_count: doc.page_count() as i64, metadata, annotations: records })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_line_parse_and_compose() {
        let (analysis, tags) = split_tags("(F) A fact.\n\nTags: #a #b-c #a");
        assert_eq!(analysis, "(F) A fact.");
        assert_eq!(tags, vec!["a", "b-c"]);
        assert_eq!(compose(&analysis, &tags), "(F) A fact.\n\nTags: #a #b-c");
        assert_eq!(compose("text  \n", &[]), "text");
        assert_eq!(split_tags("Tags: words without hashes"), ("Tags: words without hashes".to_string(), vec![]));
        assert_eq!(split_tags("Body\r\nTags: #x\r\n"), ("Body".to_string(), vec!["x".to_string()]));
        let tagged = compose("Body\n\nMore", &["#t".to_string(), "u".to_string()]);
        assert_eq!(split_tags(&tagged), ("Body\n\nMore".to_string(), vec!["t".to_string(), "u".to_string()]));
    }

    #[test]
    fn label_is_first_paragraph_marker() {
        assert_eq!(parse_label("Claude: source line.\n\n(F) The fact.\n\n(I) Reading."), Some("F".into()));
        assert_eq!(parse_label("Text that mentions (P) inside."), None);
        assert_eq!(parse_label("  (P) Proposal"), Some("P".into()));
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(pdf_date("D:20260301120000+00'00'"), Some("2026-03-01T12:00:00".into()));
        assert_eq!(pdf_date("D:2026"), Some("2026-01-01T00:00:00".into()));
        assert_eq!(pdf_date("garbage"), None);
        assert_eq!(to_pdf_date("2026-03-01T12:00:00"), Some("D:20260301120000".into()));
        assert_eq!(colour_hex(&[1.0, 0.847, 0.29]), Some("#ffd84a".into()));
        assert_eq!(hex_rgb("#ffd84a"), (1.0, 216.0 / 255.0, 74.0 / 255.0));
    }

    #[test]
    fn find_quote_normalises_space_and_case() {
        let words: Vec<Word> = ["The", "division", "of", "labour", "raises", "output"]
            .iter()
            .enumerate()
            .map(|(i, t)| Word { x0: i as f64 * 50.0, y0: 100.0, x1: i as f64 * 50.0 + 40.0, y1: 112.0, text: t.to_string(), block: 0, line: 0 })
            .collect();
        let (quads, verbatim) = find_quote(&words, "division of  labour raises").unwrap();
        assert_eq!(verbatim, "division of labour raises");
        assert_eq!(quads, vec![vec![50.0, 100.0, 240.0, 112.0]]);
        let (_, v2) = find_quote(&words, "DIVISION OF").unwrap();
        assert_eq!(v2, "division of");
        assert!(find_quote(&words, "absent words").is_none());
        assert_eq!(words_under(&words, &quads), "division of labour raises");
    }
}
