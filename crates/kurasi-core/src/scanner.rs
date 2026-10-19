//! Recursive incremental scan, hashing, grouping, cloud-only detection and the folder watcher.
//!
//! Cloud-only placeholders (OneDrive "online-only") are recorded from their directory entry
//! alone: relpath, size and mtime. They are never opened, hashed or thumbnailed, because
//! reading them makes OneDrive download them.

use std::collections::{BTreeMap, HashSet};
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{self, dumps, loads_quads, loads_strs};
use crate::error::{Error, Result};
use crate::model::{FileRow, HlRow, Origin, RefRow, UNCATEGORIZED};
use crate::pdf::{self, AnnotRecord, PdfMetadata};
use crate::util::{capitalize, ext_of, mtime_secs, new_id, now_iso, sha256_file};
use crate::Project;

pub const CLOUD_MASK: u32 = 0x400000 | 0x1000;
const REPARSE_POINT: u32 = 0x400;

/// True for an online-only placeholder. Reads attributes from the directory entry only.
pub fn is_cloud_only(meta: &Metadata) -> bool {
    file_attributes(meta) & CLOUD_MASK != 0
}

#[cfg(windows)]
fn file_attributes(meta: &Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

#[cfg(not(windows))]
fn file_attributes(_meta: &Metadata) -> u32 {
    0
}

// ---------------------------------------------------------------- slugs and roles

fn hl_suffix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)_(highlighted|annotated|reading|hl)$").unwrap())
}

fn excerpt_suffix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)_excerpt(?:-pp[0-9a-z-]+)?$").unwrap())
}

fn translated_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:print\.)?translated-[a-z]{2,3}$").unwrap())
}

/// `(slug, role)` of a file by its name.
///
/// Slug: the name up to the first dot, then a final `_highlighted`, `_annotated`, `_reading` or
/// `_hl` stripped, then a final `_excerpt` or `_excerpt-pp<range>`. Names that start with an
/// underscore (folder working notes such as `_NOTES.md`) get their folder in front, so the
/// notes of two folders stay two references.
pub fn slug_role(relpath: &str) -> (String, String) {
    let rel = relpath.replace('/', "\\");
    let (folder, name) = match rel.rfind('\\') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel.as_str()),
    };
    let (first, rest) = match name.find('.') {
        Some(i) => (&name[..i], &name[i + 1..]),
        None => (name, ""),
    };
    let stripped_hl = hl_suffix().replace(first, "").into_owned();
    let slug = if stripped_hl.is_empty() { first.to_string() } else { stripped_hl };
    let stripped = excerpt_suffix().replace(&slug, "").into_owned();
    let stripped = if stripped.is_empty() { slug.clone() } else { stripped };
    let excerpt = stripped != slug;
    let mut slug = stripped;
    let middle = if rest.contains('.') {
        rest.to_lowercase().rsplit_once('.').map(|(m, _)| m.to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    let role = if middle == "print" {
        "print"
    } else if translated_re().is_match(&middle) {
        "translation"
    } else if middle == "rendered" {
        "rendered"
    } else if excerpt {
        "excerpt"
    } else {
        "primary"
    };
    if slug.starts_with('_') && !folder.is_empty() {
        slug = format!("{}/{}", folder.replace('\\', "/"), slug);
    }
    (slug, role.to_string())
}

/// Starting metadata of a new reference. Every guess keeps status Auto.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    pub title: String,
    pub authors: Vec<String>,
    pub date: String,
    pub year: String,
}

pub fn guesses(slug: &str, meta: Option<&PdfMetadata>) -> Guess {
    static DATE: OnceLock<Regex> = OnceLock::new();
    static YEAR: OnceLock<Regex> = OnceLock::new();
    static SURNAME: OnceLock<Regex> = OnceLock::new();
    let date_re = DATE.get_or_init(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})").unwrap());
    let year_re = YEAR.get_or_init(|| Regex::new(r"_(\d{4})_").unwrap());
    let surname_re = SURNAME.get_or_init(|| Regex::new(r"^([a-z]+)_\d{4}_").unwrap());
    let base = slug.rsplit('/').next().unwrap_or(slug);
    let mut g = Guess {
        title: meta.map(|m| m.title.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| slug.to_string()),
        authors: vec!["Unknown".into()],
        date: String::new(),
        year: String::new(),
    };
    if let Some(a) = meta.map(|m| m.author.trim()).filter(|a| !a.is_empty()) {
        let names: Vec<String> = a.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if !names.is_empty() {
            g.authors = names;
        }
    }
    if let Some(m) = date_re.captures(base) {
        g.date = m[1].to_string();
        g.year = m[1][..4].to_string();
    }
    if g.year.is_empty() {
        if let Some(m) = year_re.captures(base) {
            g.year = m[1].to_string();
        }
    }
    if g.authors == ["Unknown"] {
        if let Some(m) = surname_re.captures(base) {
            g.authors = vec![capitalize(&m[1])];
        }
    }
    g
}

// ---------------------------------------------------------------- walking

#[derive(Debug, Clone)]
pub struct Entry {
    pub rel: String,
    pub key: String,
    pub abs: PathBuf,
    pub size: i64,
    pub mtime: f64,
    pub cloud: bool,
    pub ext: String,
}

/// fnmatch on the lower-cased name; `.~kurasi-*` temp files are always excluded.
pub fn excluded(name: &str, patterns: &[String]) -> bool {
    let low = name.to_lowercase();
    if low.starts_with(".~kurasi-") {
        return true;
    }
    patterns.iter().any(|p| fnmatch(&low, &p.to_lowercase()))
}

/// `fnmatch.fnmatchcase` with `*`, `?` and `[...]`.
pub fn fnmatch(name: &str, pattern: &str) -> bool {
    let mut re = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '[' => {
                if let Some(j) = chars[i + 1..].iter().position(|c| *c == ']') {
                    let body: String = chars[i + 1..i + 1 + j].iter().collect();
                    let body = if let Some(rest) = body.strip_prefix('!') { format!("^{}", rest) } else { body };
                    re.push('[');
                    re.push_str(&body.replace('\\', "\\\\"));
                    re.push(']');
                    i += j + 1;
                } else {
                    re.push_str("\\[");
                }
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    Regex::new(&re).map(|r| r.is_match(name)).unwrap_or(false)
}

fn entries(project: &Project, root: &Path, folder: &Path, patterns: &[String], types: &HashSet<String>, recurse: bool, only_name: Option<&str>, errors: &mut Vec<String>) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) => {
                errors.push(format!("{}: {}", crate::util::relpath(root, &d), e));
                continue;
            }
        };
        for e in rd {
            let e = match e {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(only) = only_name {
                if name.to_lowercase() != only {
                    continue;
                }
            }
            if excluded(&name, patterns) {
                continue;
            }
            let meta = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                // Junctions carry the reparse-point attribute; they are never followed.
                if recurse && file_attributes(&meta) & REPARSE_POINT == 0 {
                    stack.push(e.path());
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let ext = ext_of(&name);
            if !types.contains(&format!(".{}", ext)) {
                continue;
            }
            let path = e.path();
            let rel = crate::util::relpath(root, &path);
            out.push(Entry {
                key: rel.to_lowercase(),
                rel,
                cloud: project.is_cloud_only(&path, &meta),
                abs: path,
                size: meta.len() as i64,
                mtime: mtime_secs(&meta),
                ext,
            });
        }
    }
    out
}

fn walk(project: &Project, root: &Path, patterns: &[String], types: &HashSet<String>, errors: &mut Vec<String>) -> BTreeMap<String, Entry> {
    entries(project, root, root, patterns, types, true, None, errors).into_iter().map(|e| (e.key.clone(), e)).collect()
}

/// Entries for touched paths: a file, or every file below a directory.
fn walk_paths(project: &Project, root: &Path, relpaths: &[String], patterns: &[String], types: &HashSet<String>, errors: &mut Vec<String>) -> BTreeMap<String, Entry> {
    let mut seen = BTreeMap::new();
    for rel in relpaths {
        let path = crate::util::normalize(&root.join(rel));
        let rel_of_path = crate::util::relpath(root, &path);
        if rel_of_path.split('\\').any(|p| excluded(p, patterns)) {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path).ok();
        if meta.as_ref().map(|m| m.is_dir() && !m.file_type().is_symlink()).unwrap_or(false) {
            for e in entries(project, root, &path, patterns, types, true, None, errors) {
                seen.insert(e.key.clone(), e);
            }
        } else if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            if parent.is_dir() {
                let only = name.to_string_lossy().to_lowercase();
                for e in entries(project, root, parent, patterns, types, false, Some(&only), errors) {
                    seen.insert(e.key.clone(), e);
                }
            }
        }
    }
    seen
}

// ---------------------------------------------------------------- annotations

fn close(a: &[Vec<f64>], b: &[Vec<f64>], tol: f64) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).all(|(qa, qb)| qa.len() == qb.len() && qa.iter().zip(qb).all(|(x, y)| (x - y).abs() <= tol))
}

/// Brings the database highlights of one PDF in line with its annotations.
///
/// A highlight already in the database is matched by `pdf_xref`, else by page and quads
/// within 2 points, so edits made in the app are never duplicated. A database highlight
/// whose annotation is gone from the PDF is removed. Returns `(added, updated, removed)`.
pub fn merge_annotations(c: &Connection, f: &FileRow, records: &[AnnotRecord]) -> Result<(i64, i64, i64)> {
    let file_id = f.id.clone();
    let existing = db::query_hls(c, "SELECT * FROM highlight WHERE file_id=? AND kind IN ('text','region','note')", &[&file_id])?;
    let quads_of: Vec<Vec<Vec<f64>>> = existing.iter().map(|h| loads_quads(Some(&h.quads))).collect();
    let mut used: HashSet<usize> = HashSet::new();
    let mut plan: Vec<(usize, &AnnotRecord)> = Vec::new();
    let passes: [&dyn Fn(&HlRow, &[Vec<f64>], &AnnotRecord) -> bool; 3] = [
        &|h, q, r| h.pdf_xref == Some(r.pdf_xref) && h.page == Some(r.page) && close(q, &r.quads, 2.0),
        &|h, q, r| h.page == Some(r.page) && close(q, &r.quads, 2.0),
        &|h, _q, r| h.pdf_xref == Some(r.pdf_xref) && h.page == Some(r.page),
    ];
    let mut pending: Vec<&AnnotRecord> = records.iter().collect();
    for test in passes.iter() {
        let mut left = Vec::new();
        for r in pending {
            let hit = existing.iter().enumerate().find(|(i, h)| !used.contains(i) && test(h, &quads_of[*i], r)).map(|(i, _)| i);
            match hit {
                Some(i) => {
                    used.insert(i);
                    plan.push((i, r));
                }
                None => left.push(r),
            }
        }
        pending = left;
    }
    let (mut added, mut updated) = (0, 0);
    let now = now_iso();
    for (i, r) in plan {
        let h = &existing[i];
        let mut upd: Vec<(&str, Value)> = Vec::new();
        if h.pdf_xref != Some(r.pdf_xref) {
            upd.push(("pdf_xref", json!(r.pdf_xref)));
        }
        if h.page != Some(r.page) {
            upd.push(("page", json!(r.page)));
        }
        if quads_of[i] != r.quads {
            upd.push(("quads", Value::String(dumps(&json!(r.quads)))));
        }
        if h.verbatim.is_empty() && !r.verbatim.is_empty() {
            upd.push(("verbatim", Value::String(r.verbatim.clone())));
        }
        if h.kind != r.kind && !(h.kind == "region" && r.kind == "text") {
            upd.push(("kind", Value::String(r.kind.clone())));
        }
        let same_note = pdf::compose(&h.analysis, &loads_strs(Some(&h.tags))) == pdf::compose(&r.analysis, &r.tags);
        if !same_note || h.author != r.author {
            upd.push(("analysis", Value::String(r.analysis.clone())));
            upd.push(("tags", Value::String(db::dumps_strs(&r.tags))));
            upd.push(("author", Value::String(r.author.clone())));
            upd.push(("origin", Value::String(r.origin.clone())));
            upd.push(("updated_at", Value::String(r.modified.clone().unwrap_or_else(|| now.clone()))));
            if !same_note {
                upd.push(("label", r.label.clone().map(Value::String).unwrap_or(Value::Null)));
            }
        }
        if !upd.is_empty() {
            db::update_fields(c, "highlight", "id", &h.id, &upd)?;
            updated += 1;
        }
    }
    for r in pending {
        let created = r.created.clone().unwrap_or_else(|| now.clone());
        db::insert_highlight(
            c,
            &HlRow {
                id: new_id("h"),
                file_id: f.id.clone(),
                ref_id: f.ref_id.clone(),
                page: Some(r.page),
                kind: r.kind.clone(),
                quads: dumps(&json!(r.quads)),
                anchor: None,
                verbatim: r.verbatim.clone(),
                analysis: r.analysis.clone(),
                label: r.label.clone(),
                tags: db::dumps_strs(&r.tags),
                author: r.author.clone(),
                origin: r.origin.clone(),
                colour: r.colour.clone(),
                pdf_xref: Some(r.pdf_xref),
                created_at: Some(created.clone()),
                updated_at: Some(r.modified.clone().unwrap_or(created)),
                suggestion: None,
                suggested_tags: None,
            },
        )?;
        added += 1;
    }
    let mut removed = 0;
    for (i, h) in existing.iter().enumerate() {
        if !used.contains(&i) {
            c.execute("DELETE FROM highlight WHERE id=?", [&h.id])?;
            removed += 1;
        }
    }
    Ok((added, updated, removed))
}

// ---------------------------------------------------------------- the scan

/// What to scan.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanMode {
    /// Known files with unchanged size and mtime are skipped.
    Incremental,
    /// Every local file is hashed and every PDF re-read.
    Full,
    /// Only these relative paths (files or folders), as the watcher reports them.
    Paths(Vec<String>),
}

/// One progress step: percent, message, and whether it is a log line (or a transient status).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub pct: u8,
    pub message: String,
    pub line: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub new: i64,
    pub changed: i64,
    pub moved: i64,
    pub missing: i64,
    pub restored: i64,
    pub unchanged: i64,
    pub cloud_only: i64,
    pub new_references: i64,
    pub highlights_added: i64,
    pub highlights_removed: i64,
    pub errors: Vec<String>,
    pub total: i64,
    pub full: bool,
    pub partial: bool,
    pub duration_s: f64,
    pub summary: String,
}

struct Ctx<'a> {
    project: &'a Project,
    ai: Vec<String>,
    full: bool,
    affected: HashSet<String>,
    res: ScanReport,
}

fn find_or_create_reference(c: &Connection, slug: &str, meta: Option<&PdfMetadata>, now: &str) -> Result<(String, bool)> {
    if let Some(row) = db::ref_by_slug(c, slug)? {
        if let Some(m) = meta {
            if row.status == "Auto" && row.title == row.slug && !m.title.trim().is_empty() {
                let g = guesses(slug, meta);
                c.execute("UPDATE reference SET title=?, updated_at=? WHERE id=?", params![g.title, now, row.id])?;
                if g.authors != ["Unknown"] && db::authors_of(c, &row.id)? == ["Unknown"] {
                    db::set_authors(c, &row.id, &g.authors)?;
                }
            }
        }
        return Ok((row.id, false));
    }
    let g = guesses(slug, meta);
    let rid = new_id("r");
    db::insert_reference(
        c,
        &RefRow {
            id: rid.clone(),
            slug: slug.to_string(),
            title: g.title,
            year: g.year,
            date: g.date,
            status: "Auto".into(),
            created_at: Some(now.to_string()),
            updated_at: Some(now.to_string()),
            ..Default::default()
        },
    )?;
    db::set_authors(c, &rid, &g.authors)?;
    c.execute("INSERT INTO reference_category(ref_id,category_id) VALUES(?,?)", params![rid, UNCATEGORIZED])?;
    Ok((rid, true))
}

fn file_row(e: &Entry, rid: &str, role: &str, sha: Option<String>, page_count: Option<i64>, now: &str, cloud: bool) -> FileRow {
    FileRow {
        id: new_id("f"),
        ref_id: rid.to_string(),
        relpath: e.rel.clone(),
        ext: e.ext.clone(),
        sha256: sha,
        size: e.size,
        mtime: e.mtime,
        page_count,
        role: role.to_string(),
        cloud_only: cloud,
        missing: false,
        missing_since: None,
        first_seen: Some(now.to_string()),
        last_seen: Some(now.to_string()),
        view_rotation: None,
    }
}

fn inspect_pdf(project: &Project, path: &Path, ai: &[String]) -> Result<Option<pdf::Inspection>> {
    match project.pdf().open(path) {
        Ok(doc) => Ok(Some(pdf::inspect(doc.as_ref(), ai)?)),
        Err(Error::Unsupported(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn process(ctx: &mut Ctx, e: &Entry, row: Option<&FileRow>) -> Result<()> {
    let project = ctx.project;
    let now = now_iso();
    if e.cloud {
        ctx.res.cloud_only += 1;
        let _g = project.write_lock().acquire()?;
        let mut touched: Vec<String> = Vec::new();
        project.db().tx(|c| {
            match row {
                Some(row) => {
                    c.execute(
                        "UPDATE file SET cloud_only=1, size=?, mtime=?, missing=0, missing_since=NULL, last_seen=?, relpath=? WHERE id=?",
                        params![e.size, e.mtime, now, e.rel, row.id],
                    )?;
                    if row.missing {
                        ctx.res.restored += 1;
                    }
                    touched.push(row.ref_id.clone());
                }
                None => {
                    let (slug, role) = slug_role(&e.rel);
                    let (rid, created) = find_or_create_reference(c, &slug, None, &now)?;
                    db::insert_file(c, &file_row(e, &rid, &role, None, None, &now, true))?;
                    ctx.res.new += 1;
                    ctx.res.new_references += created as i64;
                    touched.push(rid);
                }
            }
            Ok(())
        })?;
        ctx.affected.extend(touched);
        return Ok(());
    }

    let sha = sha256_file(&e.abs)?;
    if let Some(row) = row {
        let changed = row.sha256.as_deref() != Some(sha.as_str());
        let read = e.ext == "pdf" && (changed || ctx.full || row.page_count.is_none());
        let info = if read { inspect_pdf(project, &e.abs, &ctx.ai)? } else { None };
        let _g = project.write_lock().acquire()?;
        let (a, r) = project.db().tx(|c| {
            let mut fields: Vec<(&str, Value)> = vec![
                ("sha256", json!(sha)),
                ("size", json!(e.size)),
                ("mtime", json!(e.mtime)),
                ("last_seen", json!(now)),
                ("cloud_only", json!(0)),
                ("missing", json!(0)),
                ("missing_since", Value::Null),
                ("relpath", json!(e.rel)),
            ];
            if let Some(i) = &info {
                fields.push(("page_count", json!(i.page_count)));
            }
            db::update_fields(c, "file", "id", &row.id, &fields)?;
            let mut out = (0, 0);
            if let Some(i) = &info {
                let mut updated = row.clone();
                updated.relpath = e.rel.clone();
                let (a, _, r) = merge_annotations(c, &updated, &i.annotations)?;
                out = (a, r);
            }
            Ok(out)
        })?;
        if row.missing {
            ctx.res.restored += 1;
        }
        if changed {
            ctx.res.changed += 1;
        }
        ctx.res.highlights_added += a;
        ctx.res.highlights_removed += r;
        ctx.affected.insert(row.ref_id.clone());
        return Ok(());
    }

    let moved = project.db().with(|c| {
        Ok(c.query_row(
            "SELECT * FROM file WHERE missing=1 AND sha256=? ORDER BY missing_since DESC LIMIT 1",
            [&sha],
            db::file_from_row,
        )
        .optional()?)
    })?;
    if let Some(moved) = moved {
        let (_, role) = slug_role(&e.rel);
        let _g = project.write_lock().acquire()?;
        project.db().tx(|c| {
            db::update_fields(
                c,
                "file",
                "id",
                &moved.id,
                &[
                    ("relpath", json!(e.rel)),
                    ("ext", json!(e.ext)),
                    ("role", json!(role)),
                    ("size", json!(e.size)),
                    ("mtime", json!(e.mtime)),
                    ("missing", json!(0)),
                    ("missing_since", Value::Null),
                    ("cloud_only", json!(0)),
                    ("last_seen", json!(now)),
                ],
            )
        })?;
        ctx.affected.insert(moved.ref_id.clone());
        ctx.res.moved += 1;
        return Ok(());
    }

    let (slug, role) = slug_role(&e.rel);
    let info = if e.ext == "pdf" { inspect_pdf(project, &e.abs, &ctx.ai)? } else { None };
    let _g = project.write_lock().acquire()?;
    let (rid, created, added) = project.db().tx(|c| {
        let (rid, created) = find_or_create_reference(c, &slug, info.as_ref().map(|i| &i.metadata), &now)?;
        let page_count = match (&info, e.ext.as_str()) {
            (Some(i), _) => Some(i.page_count),
            (None, "pdf") => None,
            _ => Some(0),
        };
        let frow = file_row(e, &rid, &role, Some(sha.clone()), page_count, &now, false);
        db::insert_file(c, &frow)?;
        let mut added = 0;
        if let Some(i) = &info {
            let (a, _, _) = merge_annotations(c, &frow, &i.annotations)?;
            added = a;
        }
        Ok((rid, created, added))
    })?;
    ctx.affected.insert(rid);
    ctx.res.new += 1;
    ctx.res.new_references += created as i64;
    ctx.res.highlights_added += added;
    Ok(())
}

/// Runs one scan. Scans of one project run one at a time.
pub fn scan_project(project: &Project, mode: ScanMode, mut progress: impl FnMut(Progress), origin: Origin) -> Result<ScanReport> {
    let _scan = project.scan_guard();
    let t0 = Instant::now();
    let mut rep = |pct: u8, message: String, line: bool| progress(Progress { pct, message, line });
    let types: HashSet<String> = project
        .setting_strs("file_types")
        .iter()
        .map(|t| format!(".{}", t.to_lowercase().trim_start_matches('.')))
        .collect();
    let patterns = project.setting_strs("exclude");
    let ai: Vec<String> = project.setting_strs("ai_authors").iter().map(|a| a.to_lowercase()).collect();
    let root = project.folder().to_path_buf();
    if !root.is_dir() {
        return Err(Error::not_found(format!(
            "The project folder {} is unreachable. Reconnect the drive or open another project.",
            root.display()
        )));
    }
    let (full, only) = match &mode {
        ScanMode::Full => (true, None),
        ScanMode::Incremental => (false, None),
        ScanMode::Paths(p) => (false, Some(p.clone())),
    };
    let mut errors: Vec<String> = Vec::new();
    let mut ctx = Ctx {
        project,
        ai,
        full,
        affected: HashSet::new(),
        res: ScanReport { full, partial: only.is_some(), ..Default::default() },
    };

    rep(5, format!("Enumerating {}, skipping online-only placeholders from opening", root.display()), true);
    let (seen, prefixes): (BTreeMap<String, Entry>, Option<Vec<String>>) = match &only {
        None => (walk(project, &root, &patterns, &types, &mut errors), None),
        Some(paths) => (
            walk_paths(project, &root, paths, &patterns, &types, &mut errors),
            Some(paths.iter().map(|p| crate::util::normalize(Path::new(p)).to_string_lossy().to_lowercase()).collect()),
        ),
    };
    ctx.res.total = seen.len() as i64;
    let mut type_list: Vec<&String> = types.iter().collect();
    type_list.sort();
    rep(30, format!("{} files match {}", seen.len(), type_list.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")), true);

    let known: BTreeMap<String, FileRow> = project
        .db()
        .with(|c| db::query_files(c, "SELECT * FROM file", &[]))?
        .into_iter()
        .map(|r| (r.relpath.to_lowercase(), r))
        .collect();
    let in_scope = |key: &str| match &prefixes {
        None => true,
        Some(p) => p.iter().any(|pre| key == pre || key.starts_with(&format!("{}\\", pre))),
    };

    let gone: Vec<&FileRow> = known.iter().filter(|(k, r)| !r.missing && !seen.contains_key(*k) && in_scope(k)).map(|(_, r)| r).collect();
    if !gone.is_empty() {
        let now = now_iso();
        let _g = project.write_lock().acquire()?;
        project.db().tx(|c| {
            for r in &gone {
                c.execute("UPDATE file SET missing=1, missing_since=? WHERE id=?", params![now, r.id])?;
            }
            Ok(())
        })?;
        for r in &gone {
            ctx.affected.insert(r.ref_id.clone());
        }
        ctx.res.missing = gone.len() as i64;
    }

    rep(48, "Comparing size and modified time with the database".into(), true);
    let mut todo: Vec<(&Entry, Option<&FileRow>)> = Vec::new();
    let mut quiet: Vec<String> = Vec::new();
    for (key, e) in &seen {
        let row = known.get(key);
        let unchanged = match row {
            Some(r) => {
                !full
                    && !r.missing
                    && r.size == e.size
                    && (r.mtime - e.mtime).abs() < 1e-4
                    && r.cloud_only == e.cloud
                    && (e.cloud || r.sha256.is_some())
                    && (e.ext != "pdf" || e.cloud || r.page_count.is_some())
                    && r.relpath == e.rel
            }
            None => false,
        };
        if unchanged {
            quiet.push(row.unwrap().id.clone());
        } else {
            todo.push((e, row));
        }
    }
    ctx.res.unchanged = quiet.len() as i64;
    if !quiet.is_empty() {
        let now = now_iso();
        project.db().tx(|c| {
            for fid in &quiet {
                c.execute("UPDATE file SET last_seen=? WHERE id=?", params![now, fid])?;
            }
            Ok(())
        })?;
    }

    rep(
        62,
        if todo.is_empty() { "No new or changed files".to_string() } else { format!("Hashing {} new or changed files (SHA-256)", todo.len()) },
        true,
    );
    let total_todo = todo.len();
    for (i, (e, row)) in todo.into_iter().enumerate() {
        if i % 5 == 0 {
            rep(62 + (30 * i / total_todo.max(1)) as u8, format!("Reading {}", e.rel), false);
        }
        if let Err(exc) = process(&mut ctx, e, row) {
            log::warn!("scan failed on {}: {}", e.rel, exc);
            errors.push(format!("{}: {}", e.rel, exc.message()));
        }
    }

    let res = &mut ctx.res;
    let summary = format!("{} new, {} changed, {} moved, {} missing", res.new, res.changed, res.moved, res.missing);
    {
        let _g = project.write_lock().acquire()?;
        let mut ids: Vec<String> = ctx.affected.iter().filter(|a| !a.is_empty()).cloned().collect();
        ids.sort();
        ids.truncate(500);
        let wrote = res.new > 0 || res.changed > 0 || res.moved > 0 || res.missing > 0 || res.restored > 0;
        project.db().tx(|c| {
            if only.is_none() {
                db::set_setting(c, "last_scan", &Value::String(now_iso()))?;
            }
            if wrote {
                project.changes().log(c, &format!("Scan: {}", summary), &ids, origin.as_str())?;
            }
            Ok(())
        })?;
    }
    if res.new > 0 || res.changed > 0 || res.moved > 0 || res.restored > 0 {
        project.sidecar_write();
    }
    res.duration_s = (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    res.summary = summary.clone();
    res.errors = errors.clone();
    for line in errors.iter().take(20) {
        rep(95, format!("Skipped {}", line), true);
    }
    rep(
        100,
        format!(
            "Done: {}, {} new reference{}, {} online-only left untouched",
            summary,
            res.new_references,
            if res.new_references == 1 { "" } else { "s" },
            res.cloud_only
        ),
        true,
    );
    Ok(ctx.res)
}

// ---------------------------------------------------------------- watcher

/// Stops the watcher when dropped.
pub struct WatcherHandle {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _debouncer: Option<Box<dyn std::any::Any + Send>>,
}

impl WatcherHandle {
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        self._debouncer = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum WatchMsg {
    Paths(Vec<PathBuf>),
}

/// Watches the project folder; after `debounce` of quiet the touched paths are rescanned
/// with `ScanMode::Paths`. Excluded names are ignored.
pub fn start_watcher(project: &Project, debounce: Duration) -> Result<WatcherHandle> {
    use notify::RecursiveMode;
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};

    let (tx, rx) = mpsc::channel::<WatchMsg>();
    let tx_events = tx.clone();
    let mut debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
        if let Ok(events) = result {
            let mut paths: Vec<PathBuf> = Vec::new();
            for ev in events {
                if ev.kind.is_access() {
                    continue;
                }
                for p in &ev.paths {
                    if !paths.contains(p) {
                        paths.push(p.clone());
                    }
                }
            }
            if !paths.is_empty() {
                let _ = tx_events.send(WatchMsg::Paths(paths));
            }
        }
    })
    .map_err(|e| Error::Other(format!("The folder watcher could not start: {}", e)))?;
    debouncer
        .watch(project.folder(), RecursiveMode::Recursive)
        .map_err(|e| Error::Other(format!("The folder watcher could not start: {}", e)))?;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let worker_project = project.clone();
    let thread = std::thread::Builder::new()
        .name("kurasi-watch".into())
        .spawn(move || loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(WatchMsg::Paths(paths)) => {
                    let root = worker_project.folder().to_path_buf();
                    let patterns = worker_project.setting_strs("exclude");
                    let mut rels: Vec<String> = Vec::new();
                    for p in paths {
                        let rel = match p.strip_prefix(&root) {
                            Ok(r) => r.to_string_lossy().into_owned(),
                            Err(_) => continue,
                        };
                        if rel.is_empty() || rel.split('\\').any(|part| excluded(part, &patterns)) {
                            continue;
                        }
                        if !rels.contains(&rel) {
                            rels.push(rel);
                        }
                    }
                    if rels.is_empty() {
                        continue;
                    }
                    rels.sort();
                    if let Err(e) = scan_project(&worker_project, ScanMode::Paths(rels), |_| {}, Origin::Scan) {
                        log::warn!("watcher scan failed: {}", e);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        })
        .map_err(|e| Error::Other(format!("The folder watcher could not start: {}", e)))?;
    let _ = tx.send(WatchMsg::Paths(Vec::new()));
    Ok(WatcherHandle { stop: Some(stop_tx), thread: Some(thread), _debouncer: Some(Box::new(debouncer)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_role() {
        let cases = [
            ("lib/smith_1776_wealth.pdf", "smith_1776_wealth", "primary"),
            ("lib/smith_1776_wealth.print.pdf", "smith_1776_wealth", "print"),
            ("lib/smith_1776_wealth.print.translated-en.pdf", "smith_1776_wealth", "translation"),
            ("lib/smith_1776_wealth.translated-de.pdf", "smith_1776_wealth", "translation"),
            ("press/2026-10-01_x_y-kr.rendered.pdf", "2026-10-01_x_y-kr", "rendered"),
            ("a/herzberg_1959_motivation_excerpt-pp113-119_highlighted.pdf", "herzberg_1959_motivation", "excerpt"),
            ("a/cameron_2006_culture_excerpt_highlighted.pdf", "cameron_2006_culture", "excerpt"),
            ("a/curtis_2009_cmm_annotated.pdf", "curtis_2009_cmm", "primary"),
            ("a/curtis_2009_cmm_hl.print.pdf", "curtis_2009_cmm", "print"),
            ("notes/_NOTES.md", "notes/_NOTES", "primary"),
            ("img/chart.png", "chart", "primary"),
        ];
        for (rel, slug, role) in cases {
            assert_eq!(slug_role(rel), (slug.to_string(), role.to_string()), "{}", rel);
        }
    }

    #[test]
    fn guesses_from_slug_and_metadata() {
        let g = guesses("smith_1776_wealth", None);
        assert_eq!(g, Guess { title: "smith_1776_wealth".into(), authors: vec!["Smith".into()], date: "".into(), year: "1776".into() });
        let g = guesses("2026-03-02_download", None);
        assert_eq!((g.date.as_str(), g.year.as_str(), g.authors.clone()), ("2026-03-02", "2026", vec!["Unknown".to_string()]));
        let g = guesses("scan_0007", Some(&PdfMetadata { title: "".into(), author: "".into() }));
        assert_eq!(g, Guess { title: "scan_0007".into(), authors: vec!["Unknown".into()], date: "".into(), year: "".into() });
        let g = guesses("x_2001_y", Some(&PdfMetadata { title: "Real title".into(), author: "Hofstede, G.; Other, A.".into() }));
        assert_eq!(g.title, "Real title");
        assert_eq!(g.authors, vec!["Hofstede, G.", "Other, A."]);
    }

    #[test]
    fn exclude_patterns() {
        let patterns: Vec<String> = [".git", ".kurasi", "~$*", "node_modules"].iter().map(|s| s.to_string()).collect();
        assert!(excluded(".git", &patterns));
        assert!(excluded("~$lock.md", &patterns));
        assert!(excluded(".~kurasi-x.tmp", &patterns));
        assert!(!excluded("notes.md", &patterns));
        assert!(fnmatch("abc.pdf", "*.pdf"));
        assert!(fnmatch("a1", "a[0-9]"));
        assert!(!fnmatch("ab", "a[0-9]"));
    }
}
