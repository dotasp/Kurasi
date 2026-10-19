//! `<folder>\.kurasi\metadata.json`: export and import.
//!
//! The sidecar holds enough to rebuild the database on another machine: references keyed by
//! slug with their files (relpath, sha256), metadata, authors, categories, and the Markdown and
//! image highlights. PDF highlights live in the PDF files themselves and are read back by a scan.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde_json::{json, Map, Value};

use crate::catalog::{cat_paths_of, set_categories};
use crate::db::{self, dumps, loads, loads_quads, loads_strs};
use crate::error::Result;
use crate::lock::atomic_write_text;
use crate::model::{FileRow, HlRow, RefRow};
use crate::util::{ext_of, new_id, now_iso};
use crate::Project;

pub const VERSION: i64 = 1;
const REF_FIELDS: &[&str] = &[
    "id", "title", "kind", "year", "date", "pub", "vol", "issue", "pages_range", "doi", "url", "lang", "usability",
    "status", "register_id", "created_at", "updated_at", "deleted_at",
];

pub fn sidecar_path(project: &Project) -> PathBuf {
    project.folder().join(".kurasi").join("metadata.json")
}

fn opt(v: &Option<String>) -> Value {
    v.clone().map(Value::String).unwrap_or(Value::Null)
}

/// The sidecar document for the current database.
pub fn build(project: &Project) -> Result<Value> {
    // Read before the connection is taken: `name()` locks the database itself.
    let name = project.name();
    project.db().with(|c| {
        let cat_rows = db::query_cats(c)?;
        let paths = cat_paths_of(&cat_rows);
        let cats: Vec<Value> = cat_rows
            .iter()
            .filter(|r| r.id != 1)
            .map(|r| json!({"path": paths.get(&r.id).cloned().unwrap_or_default(), "sort": r.sort}))
            .collect();
        let mut authors: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let mut stmt = c.prepare("SELECT ref_id,name FROM reference_author ORDER BY ref_id, position")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (rid, name) = row?;
                authors.entry(rid).or_default().push(name);
            }
        }
        let mut rcats: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let mut stmt = c.prepare("SELECT ref_id,category_id FROM reference_category")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (rid, cid) = row?;
                if let Some(p) = paths.get(&cid) {
                    rcats.entry(rid).or_default().push(p.clone());
                }
            }
        }
        let mut files: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut file_by_id: BTreeMap<String, FileRow> = BTreeMap::new();
        for f in db::query_files(c, "SELECT * FROM file ORDER BY relpath", &[])? {
            files.entry(f.ref_id.clone()).or_default().push(json!({
                "id": f.id, "relpath": f.relpath.replace('\\', "/"), "sha256": f.sha256,
                "role": f.role, "view_rotation": f.view_rotation,
            }));
            file_by_id.insert(f.id.clone(), f);
        }
        let mut refs = Map::new();
        let mut slug_of: BTreeMap<String, String> = BTreeMap::new();
        for r in db::query_refs(c, "SELECT * FROM reference ORDER BY slug", &[])? {
            slug_of.insert(r.id.clone(), r.slug.clone());
            let mut entry = Map::new();
            entry.insert("id".into(), json!(r.id));
            for k in REF_FIELDS.iter().skip(1) {
                match *k {
                    "created_at" => entry.insert(k.to_string(), opt(&r.created_at)),
                    "updated_at" => entry.insert(k.to_string(), opt(&r.updated_at)),
                    "deleted_at" => entry.insert(k.to_string(), opt(&r.deleted_at)),
                    other => entry.insert(other.to_string(), json!(r.field(other).unwrap_or(""))),
                };
            }
            entry.insert("authors".into(), json!(authors.get(&r.id).cloned().unwrap_or_else(|| vec!["Unknown".into()])));
            let mut cats_of = rcats.get(&r.id).cloned().unwrap_or_default();
            cats_of.sort();
            entry.insert("categories".into(), json!(cats_of));
            entry.insert("files".into(), Value::Array(files.get(&r.id).cloned().unwrap_or_default()));
            refs.insert(r.slug.clone(), Value::Object(entry));
        }
        let mut hls: Vec<Value> = Vec::new();
        for h in db::query_hls(
            c,
            "SELECT h.* FROM highlight h JOIN file f ON f.id=h.file_id WHERE f.ext<>'pdf' ORDER BY h.created_at, h.id",
            &[],
        )? {
            let f = match file_by_id.get(&h.file_id) {
                Some(f) => f,
                None => continue,
            };
            hls.push(json!({
                "id": h.id, "page": h.page, "kind": h.kind, "verbatim": h.verbatim, "analysis": h.analysis,
                "label": h.label, "author": h.author, "origin": h.origin, "colour": h.colour,
                "created_at": h.created_at, "updated_at": h.updated_at,
                "ref_slug": slug_of.get(&h.ref_id), "file_relpath": f.relpath.replace('\\', "/"),
                "file_sha256": f.sha256, "quads": loads_quads(Some(&h.quads)), "anchor": loads(h.anchor.as_deref()),
                "tags": loads_strs(Some(&h.tags)), "suggestion": loads(h.suggestion.as_deref()),
                "suggested_tags": loads(h.suggested_tags.as_deref()),
            }));
        }
        Ok(json!({
            "kurasi": VERSION, "project": {"name": name}, "categories": cats,
            "references": refs, "highlights": hls,
        }))
    })
}

/// Writes the sidecar when its content changed. Returns the path.
pub fn write(project: &Project) -> Result<PathBuf> {
    let path = sidecar_path(project);
    let text = serde_json::to_string_pretty(&build(project)?)? + "\n";
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if existing == text {
            return Ok(path);
        }
    }
    atomic_write_text(&path, &text)?;
    Ok(path)
}

pub fn read(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let data: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    if data.get("kurasi").map(|v| !v.is_null() && v != &json!(0) && v != &json!(false)).unwrap_or(false) {
        Some(data)
    } else {
        None
    }
}

/// Opening a project whose database has no references imports the sidecar.
pub fn import_if_empty(project: &Project) -> Result<bool> {
    let count = project.db().with(|c| Ok(db::count(c, "SELECT COUNT(*) FROM reference")))?;
    if count > 0 {
        return Ok(false);
    }
    let data = match read(&sidecar_path(project)) {
        Some(d) => d,
        None => return Ok(false),
    };
    import_data(project, &data)?;
    Ok(true)
}

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn opt_s(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(x)) => Some(x.clone()),
        _ => None,
    }
}

pub fn import_data(project: &Project, data: &Value) -> Result<()> {
    let now = now_iso();
    let _g = project.write_lock().acquire()?;
    project.db().tx(|c| {
        let mut cats: Vec<&Value> = data.get("categories").and_then(|v| v.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
        cats.sort_by_key(|cat| cat.get("sort").and_then(|v| v.as_i64()).unwrap_or(0));
        for cat in cats {
            if let Some(p) = cat.get("path").and_then(|v| v.as_str()).filter(|p| !p.is_empty()) {
                Project::ensure_category_path_in(c, p)?;
            }
        }
        let mut file_ids: BTreeMap<String, (String, String)> = BTreeMap::new();
        if let Some(refs) = data.get("references").and_then(|v| v.as_object()) {
            for (slug, r) in refs {
                let mut rid = s(r.get("id"));
                if rid.is_empty() || db::ref_by_id(c, &rid)?.is_some() {
                    rid = new_id("r");
                }
                let mut row = RefRow { id: rid.clone(), slug: slug.clone(), ..Default::default() };
                for k in REF_FIELDS.iter().skip(1) {
                    match *k {
                        "created_at" | "updated_at" | "deleted_at" => {}
                        other => {
                            row.set_field(other, &s(r.get(other)));
                        }
                    }
                }
                row.deleted_at = opt_s(r.get("deleted_at"));
                row.created_at = Some(opt_s(r.get("created_at")).unwrap_or_else(|| now.clone()));
                row.updated_at = Some(opt_s(r.get("updated_at")).unwrap_or_else(|| now.clone()));
                if row.status.is_empty() {
                    row.status = "Auto".into();
                }
                db::insert_reference(c, &row)?;
                let authors: Vec<String> = r
                    .get("authors")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let authors = if authors.is_empty() { vec!["Unknown".to_string()] } else { authors };
                db::set_authors(c, &rid, &authors)?;
                let cats: Vec<String> = r
                    .get("categories")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                set_categories(c, &rid, &cats, false)?;
                for f in r.get("files").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
                    let rel = s(f.get("relpath")).replace('/', "\\");
                    if rel.is_empty() || db::file_by_relpath(c, &rel)?.is_some() {
                        continue;
                    }
                    let mut fid = s(f.get("id"));
                    if fid.is_empty() || c.query_row("SELECT id FROM file WHERE id=?", [&fid], |r| r.get::<_, String>(0)).is_ok() {
                        fid = new_id("f");
                    }
                    let name = rel.rsplit('\\').next().unwrap_or(&rel);
                    // size -1 and page_count NULL make the next scan hash the file and read its annotations.
                    db::insert_file(
                        c,
                        &FileRow {
                            id: fid.clone(),
                            ref_id: rid.clone(),
                            relpath: rel.clone(),
                            ext: ext_of(name),
                            sha256: opt_s(f.get("sha256")),
                            size: -1,
                            mtime: 0.0,
                            page_count: None,
                            role: opt_s(f.get("role")).filter(|r| !r.is_empty()).unwrap_or_else(|| "primary".into()),
                            cloud_only: false,
                            missing: false,
                            missing_since: None,
                            first_seen: Some(now.clone()),
                            last_seen: None,
                            view_rotation: opt_s(f.get("view_rotation")),
                        },
                    )?;
                    file_ids.insert(rel.to_lowercase(), (fid, rid.clone()));
                }
            }
        }
        for h in data.get("highlights").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let key = s(h.get("file_relpath")).replace('/', "\\").to_lowercase();
            let (fid, rid) = match file_ids.get(&key) {
                Some(x) => x.clone(),
                None => continue,
            };
            let mut hid = s(h.get("id"));
            if hid.is_empty() || c.query_row("SELECT id FROM highlight WHERE id=?", [&hid], |r| r.get::<_, String>(0)).is_ok() {
                hid = new_id("h");
            }
            let nonempty = |v: Option<&Value>| match v {
                Some(Value::Null) | None => None,
                Some(Value::Array(a)) if a.is_empty() => None,
                Some(Value::Object(o)) if o.is_empty() => None,
                Some(x) => Some(dumps(x)),
            };
            db::insert_highlight(
                c,
                &HlRow {
                    id: hid,
                    file_id: fid,
                    ref_id: rid,
                    page: h.get("page").and_then(|v| v.as_i64()),
                    kind: s(h.get("kind")),
                    quads: dumps(h.get("quads").filter(|v| v.is_array()).unwrap_or(&json!([]))),
                    anchor: nonempty(h.get("anchor")),
                    verbatim: s(h.get("verbatim")),
                    analysis: s(h.get("analysis")),
                    label: opt_s(h.get("label")),
                    tags: dumps(h.get("tags").filter(|v| v.is_array()).unwrap_or(&json!([]))),
                    author: s(h.get("author")),
                    origin: s(h.get("origin")),
                    colour: opt_s(h.get("colour")),
                    pdf_xref: None,
                    created_at: opt_s(h.get("created_at")),
                    updated_at: opt_s(h.get("updated_at")),
                    suggestion: nonempty(h.get("suggestion")),
                    suggested_tags: nonempty(h.get("suggested_tags")),
                },
            )?;
        }
        c.execute("INSERT INTO change(at,what,ids,origin) VALUES(?,?,?,?)", params![now, "Imported .kurasi\\metadata.json", "[]", "scan"])?;
        Ok(())
    })
}
