//! The catalogue operations shared by the HTTP API, the MCP server and the CLI: settings,
//! categories, references, files, highlights, vocabularies, changes and the AI log.
//!
//! Every project-file write and every multi-row database write holds the project's
//! `write.lock`. Each write adds a `change` row with its origin in the same transaction.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::ailog;
use crate::change;
use crate::config::{default_settings, THUMB_WIDTH};
use crate::db::{self, dumps, dumps_strs, loads, loads_quads, loads_strs};
use crate::error::{Error, Result};
use crate::lock::atomic_write_bytes;
use crate::media;
use crate::model::*;
use crate::pdf::{self, AnnotPatch, AnnotSnapshot, NewAnnot};
use crate::util::{new_id, norm_space, now_iso, FileStat};
use crate::Project;

pub const CLOUD_MASK: u32 = 0x400000 | 0x1000;
const THUMB_EXTS: &[&str] = &["pdf", "png", "jpg", "jpeg", "webp"];

fn str_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(other) => other.to_string().trim().to_string(),
    }
}

pub fn norm_label(value: &str) -> Result<Option<String>> {
    let v = value.trim().trim_matches(['(', ')']).to_uppercase();
    if v.is_empty() {
        return Ok(None);
    }
    if !LABELS.contains(&v.as_str()) {
        return Err(Error::bad("The label must be F, I or P."));
    }
    Ok(Some(v))
}

pub fn translation_lang(relpath: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"translated-([a-z]{2,3})\b").unwrap());
    let name = Path::new(relpath).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    re.captures(&name).map(|c| c[1].to_string()).unwrap_or_default()
}

pub fn role_rank(f: &FileRow) -> i32 {
    match f.role.as_str() {
        "translation" => {
            if translation_lang(&f.relpath) == "en" {
                0
            } else {
                5
            }
        }
        "primary" | "" => 1,
        "print" => 2,
        "excerpt" => 3,
        "rendered" => 4,
        _ => 6,
    }
}

/// Reads the file attributes only. This never makes OneDrive download the file.
pub fn live_cloud_only(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => crate::scanner::is_cloud_only(&meta),
        Err(_) => false,
    }
}

/// Short name for messages: first author surname and year, else a cut title.
pub fn ref_label(title: &str, slug: &str, year: &str, authors: &[String]) -> String {
    let names: Vec<&String> = authors.iter().filter(|a| !a.is_empty() && a.as_str() != "Unknown").collect();
    if let Some(first) = names.first() {
        let surname = first.split(',').next().unwrap_or("").trim();
        let more = if names.len() > 2 {
            " et al.".to_string()
        } else if names.len() == 2 {
            format!(" and {}", names[1].split(',').next().unwrap_or("").trim())
        } else {
            String::new()
        };
        return format!("{}{} ({})", surname, more, if year.is_empty() { "n.d." } else { year });
    }
    let t = if title.is_empty() { slug } else { title };
    if t.chars().count() <= 60 {
        t.to_string()
    } else {
        let cut: String = t.chars().take(59).collect();
        format!("{}...", cut.trim_end())
    }
}

fn label_of(r: &RefRow, authors: &[String]) -> String {
    ref_label(&r.title, &r.slug, &r.year, authors)
}

impl Project {
    // ------------------------------------------------------------ settings
    pub fn settings(&self) -> Map<String, Value> {
        let mut out = default_settings();
        for (k, v) in self.inner.db.all_settings() {
            out.insert(k, v);
        }
        out.insert("folder".into(), Value::String(self.folder().to_string_lossy().into_owned()));
        out
    }

    pub fn setting(&self, key: &str) -> Value {
        match self.inner.db.get_setting(key) {
            Some(v) => v,
            None => default_settings().remove(key).unwrap_or(Value::Null),
        }
    }

    pub fn setting_strs(&self, key: &str) -> Vec<String> {
        match self.setting(key) {
            Value::Array(items) => items.into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    }

    pub fn setting_str(&self, key: &str) -> String {
        self.setting(key).as_str().unwrap_or("").to_string()
    }

    pub fn project_info(&self) -> Value {
        let s = self.settings();
        json!({
            "id": self.id(), "name": s.get("name").cloned().unwrap_or(Value::String(String::new())),
            "folder": self.folder().to_string_lossy(), "read_only": self.read_only(),
            "auto_scan_min": s.get("auto_scan_min").cloned().unwrap_or(json!(15)),
            "last_scan": s.get("last_scan").cloned().unwrap_or(Value::Null),
            "file_types": s.get("file_types").cloned().unwrap_or(json!([])),
            "exclude": s.get("exclude").cloned().unwrap_or(json!([])),
        })
    }

    pub fn set_setting(&self, key: &str, value: Value) -> Result<Map<String, Value>> {
        let mut body = Map::new();
        body.insert(key.to_string(), value);
        self.update_settings(&body, Origin::Ui)
    }

    /// Validates and stores settings; returns every setting afterwards.
    pub fn update_settings(&self, body: &Map<String, Value>, origin: Origin) -> Result<Map<String, Value>> {
        if body.is_empty() {
            return Err(Error::bad("Send the settings to change as a JSON object."));
        }
        let mut clean: Vec<(String, Value)> = Vec::new();
        for (key, value) in body {
            let v = match key.as_str() {
                "folder" => {
                    let given = crate::util::norm_key(Path::new(&str_of(Some(value))));
                    if given != crate::util::norm_key(self.folder()) {
                        return Err(Error::bad("The folder of a project is fixed. Create a new project for another folder."));
                    }
                    continue;
                }
                "read_only" | "loose" | "mcp_hints" => Value::Bool(truthy(value)),
                "auto_scan_min" => {
                    let n = match value {
                        Value::Number(n) => n.as_f64().map(|f| f as i64),
                        Value::String(s) => s.trim().parse::<i64>().ok(),
                        Value::Bool(b) => Some(*b as i64),
                        _ => None,
                    }
                    .ok_or_else(|| Error::bad("auto_scan_min must be a whole number of minutes (0 turns it off)."))?;
                    if !(0..=1440).contains(&n) {
                        return Err(Error::bad("auto_scan_min must be between 0 and 1440 minutes."));
                    }
                    json!(n)
                }
                "file_types" => {
                    let items = value.as_array().filter(|a| !a.is_empty()).ok_or_else(|| {
                        Error::bad("file_types must be a list such as [\".pdf\", \".md\"].")
                    })?;
                    let mut set: Vec<String> = items
                        .iter()
                        .map(|v| str_of(Some(v)))
                        .filter(|s| !s.is_empty())
                        .map(|s| format!(".{}", s.to_lowercase().trim_start_matches('.')))
                        .collect();
                    set.sort();
                    set.dedup();
                    json!(set)
                }
                "exclude" | "ai_authors" => {
                    let items = value.as_array().ok_or_else(|| Error::bad(format!("{} must be a list of names.", key)))?;
                    let mut list: Vec<String> = items.iter().map(|v| str_of(Some(v))).filter(|s| !s.is_empty()).collect();
                    if key == "ai_authors" {
                        list = list.into_iter().map(|s| s.to_lowercase()).collect();
                    }
                    json!(list)
                }
                "name" | "ai_author" => {
                    let s = str_of(Some(value));
                    if s.is_empty() {
                        return Err(Error::bad(format!("{} cannot be empty.", key)));
                    }
                    Value::String(s)
                }
                "hl_colour" => {
                    let s = str_of(Some(value));
                    static RE: OnceLock<Regex> = OnceLock::new();
                    if !RE.get_or_init(|| Regex::new(r"^#[0-9a-fA-F]{6}$").unwrap()).is_match(&s) {
                        return Err(Error::bad("hl_colour must be a colour such as #ffd84a."));
                    }
                    Value::String(s)
                }
                "theme" => {
                    let s = str_of(Some(value));
                    if !["system", "light", "dark"].contains(&s.as_str()) {
                        return Err(Error::bad("theme must be system, light or dark."));
                    }
                    Value::String(s)
                }
                _ => value.clone(),
            };
            clean.push((key.clone(), v));
        }
        {
            let _g = self.write_lock().acquire()?;
            let mut keys: Vec<&str> = clean.iter().map(|(k, _)| k.as_str()).collect();
            keys.sort();
            self.db().tx(|c| {
                for (k, v) in &clean {
                    db::set_setting(c, k, v)?;
                }
                if !clean.is_empty() {
                    self.changes().log(c, &format!("Settings changed: {}", keys.join(", ")), &[], origin.as_str())?;
                }
                Ok(())
            })?;
        }
        // The registry shows the same name as the project list.
        if let Some((_, Value::String(name))) = clean.iter().find(|(k, _)| k == "name") {
            self.paths().register(Some(name), self.folder(), Some(self.id()), self.is_demo())?;
        }
        Ok(self.settings())
    }

    // ------------------------------------------------------------ categories
    pub(crate) fn cat_paths(c: &Connection) -> Result<BTreeMap<i64, String>> {
        let rows = db::query_cats(c)?;
        Ok(cat_paths_of(&rows))
    }

    pub fn categories(&self) -> Result<Vec<Category>> {
        self.db().with(|c| {
            let rows = db::query_cats(c)?;
            let paths = cat_paths_of(&rows);
            let mut nodes: BTreeMap<i64, Category> = rows
                .iter()
                .map(|r| {
                    (
                        r.id,
                        Category {
                            id: r.id,
                            name: r.name.clone(),
                            path: paths.get(&r.id).cloned().unwrap_or_default(),
                            parent_id: r.parent_id,
                            sort: r.sort,
                            children: Vec::new(),
                        },
                    )
                })
                .collect();
            // Attach children deepest first so every subtree is complete when it moves.
            let mut order: Vec<&CategoryRow> = rows.iter().collect();
            order.sort_by_key(|r| std::cmp::Reverse(depth_of(r.id, &rows)));
            let mut roots_ids: Vec<i64> = Vec::new();
            for r in order {
                match r.parent_id.filter(|p| nodes.contains_key(p)) {
                    Some(p) => {
                        if let Some(node) = nodes.remove(&r.id) {
                            nodes.get_mut(&p).unwrap().children.push(node);
                        }
                    }
                    None => roots_ids.push(r.id),
                }
            }
            for n in nodes.values_mut() {
                n.children.sort_by_key(|c| (c.sort, c.id));
            }
            let mut roots: Vec<Category> = Vec::new();
            for r in &rows {
                if roots_ids.contains(&r.id) {
                    if let Some(n) = nodes.remove(&r.id) {
                        roots.push(n);
                    }
                }
            }
            Ok(roots)
        })
    }

    fn check_cat_name(name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::bad("A category needs a name."));
        }
        if name.contains('/') {
            return Err(Error::bad("A category name cannot contain a slash; slashes separate levels."));
        }
        Ok(name.to_string())
    }

    /// The id of a category path such as `Academic/Culture`, creating missing levels. Runs
    /// inside the caller's transaction.
    pub(crate) fn ensure_category_path_in(c: &Connection, path: &str) -> Result<i64> {
        let parts: Vec<&str> = path.split('/').map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Err(Error::bad("A category path cannot be empty."));
        }
        let mut parent: Option<i64> = None;
        for name in parts {
            let rows = db::query_cats(c)?;
            let siblings: Vec<&CategoryRow> = rows.iter().filter(|r| r.parent_id == parent).collect();
            let hit = siblings
                .iter()
                .find(|r| r.name == name)
                .or_else(|| siblings.iter().find(|r| r.name.to_lowercase() == name.to_lowercase()));
            if let Some(h) = hit {
                parent = Some(h.id);
                continue;
            }
            let sort = db::count(c, "SELECT COALESCE(MAX(sort),0) FROM category") + 1;
            c.execute(
                "INSERT INTO category(parent_id,name,sort) VALUES(?,?,?)",
                params![parent, Self::check_cat_name(name)?, sort],
            )?;
            parent = Some(c.last_insert_rowid());
        }
        parent.ok_or_else(|| Error::bad("A category path cannot be empty."))
    }

    pub fn ensure_category_path(&self, path: &str) -> Result<i64> {
        self.db().tx(|c| Self::ensure_category_path_in(c, path))
    }

    fn cat_node(c: &Connection, cid: i64) -> Result<Category> {
        let rows = db::query_cats(c)?;
        let paths = cat_paths_of(&rows);
        let r = rows.iter().find(|r| r.id == cid).ok_or_else(|| Error::not_found("The category does not exist."))?;
        Ok(Category {
            id: r.id,
            name: r.name.clone(),
            path: paths.get(&cid).cloned().unwrap_or_default(),
            parent_id: r.parent_id,
            sort: r.sort,
            children: Vec::new(),
        })
    }

    pub fn create_category(&self, parent_id: Option<i64>, name: &str, origin: Origin) -> Result<Category> {
        let name = Self::check_cat_name(name)?;
        let node = {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                if let Some(p) = parent_id {
                    let exists: Option<i64> = c.query_row("SELECT id FROM category WHERE id=?", [p], |r| r.get(0)).optional()?;
                    if exists.is_none() {
                        return Err(Error::not_found("The parent category does not exist."));
                    }
                }
                let dup: Option<i64> = c
                    .query_row(
                        "SELECT id FROM category WHERE parent_id IS ? AND lower(name)=lower(?)",
                        params![parent_id, name],
                        |r| r.get(0),
                    )
                    .optional()?;
                if dup.is_some() {
                    return Err(Error::conflict(format!("A category named {} already exists at this level.", name)));
                }
                let sort = db::count(c, "SELECT COALESCE(MAX(sort),0) FROM category") + 1;
                c.execute("INSERT INTO category(parent_id,name,sort) VALUES(?,?,?)", params![parent_id, name, sort])?;
                let cid = c.last_insert_rowid();
                self.changes().log(c, &format!("Category added: {}", name), &[cid.to_string()], origin.as_str())?;
                Self::cat_node(c, cid)
            })?
        };
        self.sidecar_write();
        Ok(node)
    }

    pub fn update_category(&self, cid: i64, name: Option<&str>, parent_id: Option<Option<i64>>, origin: Origin) -> Result<Category> {
        if cid == UNCATEGORIZED {
            return Err(Error::bad("Uncategorized is built in and keeps its name and place."));
        }
        let node = {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                let rows = db::query_cats(c)?;
                let row = rows.iter().find(|r| r.id == cid).ok_or_else(|| Error::not_found("The category does not exist."))?;
                let mut fields: Vec<(&str, Value)> = Vec::new();
                let mut shown = row.name.clone();
                if let Some(n) = name {
                    let n = Self::check_cat_name(n)?;
                    shown = n.clone();
                    fields.push(("name", Value::String(n)));
                }
                if let Some(parent) = parent_id {
                    if let Some(p) = parent {
                        if !rows.iter().any(|r| r.id == p) {
                            return Err(Error::not_found("The parent category does not exist."));
                        }
                        let mut walk = Some(p);
                        let mut seen = Vec::new();
                        while let Some(w) = walk {
                            if seen.contains(&w) {
                                break;
                            }
                            if w == cid {
                                return Err(Error::bad("A category cannot move inside itself."));
                            }
                            seen.push(w);
                            walk = rows.iter().find(|r| r.id == w).and_then(|r| r.parent_id);
                        }
                    }
                    fields.push(("parent_id", parent.map(|p| json!(p)).unwrap_or(Value::Null)));
                }
                db::update_fields(c, "category", "id", &cid.to_string(), &fields)?;
                self.changes().log(c, &format!("Category changed: {}", shown), &[cid.to_string()], origin.as_str())?;
                Self::cat_node(c, cid)
            })?
        };
        self.sidecar_write();
        Ok(node)
    }

    /// Deletes a category; its references move to the parent. Returns the number moved.
    pub fn delete_category(&self, cid: i64, origin: Origin) -> Result<usize> {
        if cid == UNCATEGORIZED {
            return Err(Error::bad("Uncategorized is built in and cannot be deleted."));
        }
        let moved = {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                let rows = db::query_cats(c)?;
                let row = rows.iter().find(|r| r.id == cid).ok_or_else(|| Error::not_found("The category does not exist."))?;
                let parent = row.parent_id;
                let name = row.name.clone();
                let mut stmt = c.prepare("SELECT ref_id FROM reference_category WHERE category_id=?")?;
                let refs: Vec<String> = stmt.query_map([cid], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                drop(stmt);
                c.execute("DELETE FROM reference_category WHERE category_id=?", [cid])?;
                for rid in &refs {
                    if let Some(p) = parent {
                        c.execute("INSERT OR IGNORE INTO reference_category(ref_id,category_id) VALUES(?,?)", params![rid, p])?;
                    }
                    fix_uncategorized(c, rid)?;
                }
                c.execute("UPDATE category SET parent_id=? WHERE parent_id=?", params![parent, cid])?;
                c.execute("DELETE FROM category WHERE id=?", [cid])?;
                self.changes().log(c, &format!("Category deleted: {}", name), &refs, origin.as_str())?;
                Ok(refs.len())
            })?
        };
        self.sidecar_write();
        Ok(moved)
    }

    // ------------------------------------------------------------ references
    pub fn authors_of(&self, ref_id: &str) -> Result<Vec<String>> {
        self.db().with(|c| db::authors_of(c, ref_id))
    }

    fn bulk(c: &Connection, ids: Option<&[String]>, lean: bool) -> Result<Vec<Reference>> {
        let (refs, where_clause): (Vec<RefRow>, String) = match ids {
            Some(ids) => {
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                let marks = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let args: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                let refs = db::query_refs(
                    c,
                    &format!("SELECT * FROM reference WHERE deleted_at IS NULL AND id IN ({})", marks),
                    &args,
                )?;
                let quoted: Vec<String> = ids.iter().map(|i| format!("'{}'", i.replace('\'', "''"))).collect();
                (refs, format!(" WHERE ref_id IN ({})", quoted.join(",")))
            }
            None => (db::query_refs(c, "SELECT * FROM reference WHERE deleted_at IS NULL", &[])?, String::new()),
        };
        let mut authors: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = c.prepare(&format!("SELECT ref_id,name FROM reference_author{} ORDER BY ref_id, position", where_clause))?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (rid, name) = row?;
                authors.entry(rid).or_default().push(name);
            }
        }
        let paths = Self::cat_paths(c)?;
        let mut cats: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = c.prepare(&format!("SELECT ref_id,category_id FROM reference_category{}", where_clause))?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (rid, cid) = row?;
                if let Some(p) = paths.get(&cid) {
                    cats.entry(rid).or_default().push(p.clone());
                }
            }
        }
        let mut files: HashMap<String, Vec<FileRow>> = HashMap::new();
        for f in db::query_files(c, &format!("SELECT * FROM file{} ORDER BY relpath", where_clause), &[])? {
            files.entry(f.ref_id.clone()).or_default().push(f);
        }
        let mut hls: HashMap<String, Vec<HlRow>> = HashMap::new();
        for h in db::query_hls(c, &format!("SELECT * FROM highlight{} ORDER BY created_at, id", where_clause), &[])? {
            hls.entry(h.ref_id.clone()).or_default().push(h);
        }
        Ok(refs
            .iter()
            .map(|r| {
                record(
                    r,
                    authors.get(&r.id).cloned().unwrap_or_default(),
                    cats.get(&r.id).cloned().unwrap_or_default(),
                    files.get(&r.id).cloned().unwrap_or_default(),
                    hls.get(&r.id).map(|v| v.as_slice()).unwrap_or(&[]),
                    lean,
                )
            })
            .collect())
    }

    /// Catalogue records. `filter.detail` adds doi, url, vol, issue and pages_range.
    pub fn references(&self, filter: &ReferenceFilter) -> Result<Vec<Reference>> {
        let mut out = self.db().with(|c| Self::bulk(c, filter.ids.as_deref(), filter.lean))?;
        if !filter.detail {
            for r in out.iter_mut() {
                r.doi = None;
                r.url = None;
                r.vol = None;
                r.issue = None;
                r.pages_range = None;
            }
        }
        Ok(out)
    }

    pub fn all_references(&self) -> Result<Vec<Reference>> {
        self.references(&ReferenceFilter::default())
    }

    /// One record with detail fields.
    pub fn reference(&self, ref_id: &str) -> Result<Reference> {
        self.db().with(|c| {
            db::live_ref(c, ref_id)?;
            Self::bulk(c, Some(&[ref_id.to_string()]), false)?
                .into_iter()
                .next()
                .ok_or_else(|| Error::not_found(format!("The reference {} does not exist or is in the trash.", ref_id)))
        })
    }

    pub fn reference_by_slug(&self, slug: &str) -> Result<Option<Reference>> {
        let row = self.db().with(|c| db::ref_by_slug(c, slug))?;
        match row {
            Some(r) if r.deleted_at.is_none() => Ok(Some(self.reference(&r.id)?)),
            _ => Ok(None),
        }
    }

    /// Applies metadata, authors and categories. Status becomes `default_status` unless the
    /// patch sets it. Returns the record and the previous values of what changed.
    pub fn update_reference(&self, ref_id: &str, patch: &ReferencePatch, origin: Origin, default_status: &str) -> Result<(Reference, Value)> {
        let mut body = patch.fields.clone();
        if body.contains_key("pp") && !body.contains_key("pages_range") {
            let v = body.get("pp").cloned().unwrap_or(Value::Null);
            body.insert("pages_range".into(), v);
        }
        let mut fields: Vec<(String, String)> = Vec::new();
        for k in META_FIELDS {
            if let Some(v) = body.get(*k) {
                fields.push((k.to_string(), str_of(Some(v))));
            }
        }
        fn get(fields: &[(String, String)], name: &str) -> Option<String> {
            fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        }
        if let Some(u) = get(&fields, "usability") {
            if !USABILITY.contains(&u.as_str()) {
                return Err(Error::bad("usability must be one of: Full text, Abstract only, Excerpt, Translated, or empty."));
            }
        }
        if let Some(st) = get(&fields, "status") {
            if !STATUSES.contains(&st.as_str()) {
                return Err(Error::bad("status must be Auto, Suggested or Verified."));
            }
        }
        if let Some(t) = get(&fields, "title") {
            if t.is_empty() {
                return Err(Error::bad("A reference needs a title."));
            }
        }
        let previous = {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                let r = db::live_ref(c, ref_id)?;
                static YEAR: OnceLock<Regex> = OnceLock::new();
                let year_re = YEAR.get_or_init(|| Regex::new(r"^\d{4}").unwrap());
                if let Some(d) = get(&fields, "date") {
                    if !d.is_empty() && get(&fields, "year").map(|y| y.is_empty()).unwrap_or(true) && r.year.is_empty() && year_re.is_match(&d) {
                        fields.retain(|(k, _)| k != "year");
                        fields.push(("year".into(), d[..4].to_string()));
                    }
                }
                if get(&fields, "status").is_none() {
                    fields.push(("status".into(), default_status.to_string()));
                }
                let prev_authors = db::authors_of(c, ref_id)?;
                let prev_cats = Self::bulk(c, Some(&[ref_id.to_string()]), true)?.first().map(|x| x.categories.clone()).unwrap_or_default();
                let mut previous = Map::new();
                for (k, _) in &fields {
                    previous.insert(k.clone(), Value::String(r.field(k).unwrap_or("").to_string()));
                }
                previous.insert("authors".into(), json!(prev_authors));
                previous.insert("categories".into(), json!(prev_cats));
                let mut sets: Vec<(&str, Value)> = fields.iter().map(|(k, v)| (k.as_str(), Value::String(v.clone()))).collect();
                let now = now_iso();
                sets.push(("updated_at", Value::String(now)));
                db::update_fields(c, "reference", "id", ref_id, &sets)?;
                if let Some(a) = &patch.authors {
                    db::set_authors(c, ref_id, a)?;
                }
                if let Some(cats) = &patch.categories {
                    set_categories(c, ref_id, cats, false)?;
                }
                let mut after = r.clone();
                for (k, v) in &fields {
                    after.set_field(k, v);
                }
                let status = get(&fields, "status").unwrap_or_else(|| default_status.to_string());
                let verb = if status == "Suggested" { "Proposed metadata for" } else { "Saved" };
                let names = patch.authors.clone().unwrap_or(prev_authors);
                self.changes().log(c, &format!("{} {}", verb, label_of(&after, &names)), &[ref_id.to_string()], origin.as_str())?;
                Ok(Value::Object(previous))
            })?
        };
        self.sidecar_write();
        Ok((self.reference(ref_id)?, previous))
    }

    /// Adds categories (creating missing paths) to references and sets their status.
    pub fn categorize(&self, ref_ids: &[String], paths: &[String], origin: Origin, status: &str) -> Result<Vec<Reference>> {
        if ref_ids.is_empty() || paths.is_empty() {
            return Err(Error::bad("Name at least one reference and one category path."));
        }
        {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                for rid in ref_ids {
                    db::live_ref(c, rid)?;
                }
                for rid in ref_ids {
                    set_categories(c, rid, paths, true)?;
                    c.execute("UPDATE reference SET status=?, updated_at=? WHERE id=?", params![status, now_iso(), rid])?;
                }
                let what = format!(
                    "Proposed categories for {} reference{}: {}",
                    ref_ids.len(),
                    if ref_ids.len() == 1 { "" } else { "s" },
                    paths.join(", ")
                );
                self.changes().log(c, &what, ref_ids, origin.as_str())?;
                Ok(())
            })?;
        }
        self.sidecar_write();
        self.references(&ReferenceFilter { ids: Some(ref_ids.to_vec()), ..ReferenceFilter::default() })
    }

    /// Moves a reference to the trash (files untouched).
    pub fn delete_reference(&self, ref_id: &str, origin: Origin) -> Result<TrashResult> {
        let out = {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                let r = db::live_ref(c, ref_id)?;
                let authors = db::authors_of(c, ref_id)?;
                let at = now_iso();
                c.execute("UPDATE reference SET deleted_at=? WHERE id=?", params![at, ref_id])?;
                c.execute(
                    "INSERT INTO trash(kind,payload,deleted_at) VALUES('reference',?,?)",
                    params![dumps(&json!({"ref_id": ref_id, "title": r.title})), at],
                )?;
                let tid = c.last_insert_rowid();
                self.changes().log(c, &format!("Moved {} to the trash", label_of(&r, &authors)), &[ref_id.to_string()], origin.as_str())?;
                Ok(TrashResult { trash_id: tid })
            })?
        };
        self.sidecar_write();
        Ok(out)
    }

    // ------------------------------------------------------------ files
    pub fn file_row(&self, fid: &str) -> Result<FileRow> {
        self.db().with(|c| db::file_by_id(c, fid))
    }

    /// The absolute path of a file that is present and local.
    pub fn usable_path(&self, f: &FileRow) -> Result<PathBuf> {
        let path = self.abspath(&f.relpath)?;
        if f.missing || !path.exists() {
            return Err(Error::not_found(format!(
                "The file {} is missing from the project folder. Restore it, then scan again.",
                f.relpath
            )));
        }
        if live_cloud_only(&path) {
            return Err(Error::conflict(format!(
                "The file {} is online-only in OneDrive. In Explorer choose Always keep on this device, then scan again.",
                f.relpath
            )));
        }
        Ok(path)
    }

    fn apply_stat(c: &Connection, fid: &str, st: &FileStat) -> Result<()> {
        c.execute(
            "UPDATE file SET sha256=?, size=?, mtime=?, last_seen=? WHERE id=?",
            params![st.sha256, st.size, st.mtime, now_iso(), fid],
        )?;
        Ok(())
    }

    /// `{id, ref_id, relpath, ext, pages, page_sizes, rotation}`.
    pub fn file_info(&self, fid: &str) -> Result<Value> {
        let f = self.file_row(fid)?;
        let path = self.usable_path(&f)?;
        let rot: i32 = f.view_rotation.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut base = json!({"id": f.id, "ref_id": f.ref_id, "relpath": f.relpath, "ext": f.ext});
        let obj = base.as_object_mut().unwrap();
        if f.ext == "pdf" {
            let doc = self.pdf().open(&path)?;
            let pages = doc.pages()?;
            obj.insert("pages".into(), json!(pages.len()));
            obj.insert("page_sizes".into(), json!(pages.iter().map(|p| [round2(p.width), round2(p.height)]).collect::<Vec<_>>()));
            obj.insert("rotation".into(), json!(pages.iter().map(|p| p.rotation).collect::<Vec<_>>()));
        } else if media::IMAGE_EXTS.contains(&f.ext.as_str()) {
            obj.insert("pages".into(), json!(1));
            let [w, h] = media::image_size(&path)?;
            obj.insert("page_sizes".into(), json!([[w as i64, h as i64]]));
            obj.insert("rotation".into(), json!([rot]));
        } else {
            obj.insert("pages".into(), json!(0));
            obj.insert("page_sizes".into(), json!([]));
            obj.insert("rotation".into(), json!([rot]));
        }
        Ok(base)
    }

    fn pdf_file(&self, fid: &str) -> Result<(FileRow, PathBuf)> {
        let f = self.file_row(fid)?;
        if f.ext != "pdf" {
            return Err(Error::bad(format!("The file {} is not a PDF.", f.relpath)));
        }
        let path = self.usable_path(&f)?;
        Ok((f, path))
    }

    /// Page `page_no` (1-based) as PNG, cached under `pages/<sha>/p<n>_r<rot>_s<scale>.png`.
    pub fn render_page(&self, fid: &str, page_no: usize, scale: f64) -> Result<Vec<u8>> {
        let (f, path) = self.pdf_file(fid)?;
        if !scale.is_finite() {
            return Err(Error::bad("scale must be a number such as 1.5."));
        }
        let scale = (scale.clamp(0.25, 6.0) * 100.0).round() / 100.0;
        let doc = self.pdf().open(&path)?;
        let pages = doc.pages()?;
        if page_no == 0 || page_no > pages.len() {
            return Err(Error::not_found(format!("Page {} does not exist; the PDF has {} pages.", page_no, pages.len())));
        }
        let meta = std::fs::metadata(&path)?;
        let fresh = f.sha256.is_some() && meta.len() as i64 == f.size && (crate::util::mtime_secs(&meta) - f.mtime).abs() < 1e-4;
        let cache = if fresh {
            Some(self.paths().pages_dir().join(f.sha256.clone().unwrap_or_default()).join(format!(
                "p{}_r{}_s{}.png",
                page_no,
                pages[page_no - 1].rotation,
                fmt_scale(scale)
            )))
        } else {
            None
        };
        if let Some(c) = &cache {
            if let Ok(bytes) = std::fs::read(c) {
                return Ok(bytes);
            }
        }
        let png = doc.render_png(page_no, scale)?;
        if let Some(c) = &cache {
            if let Some(parent) = c.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = atomic_write_bytes(c, &png) {
                log::warn!("page cache write failed for {}: {}", c.display(), e);
            }
        }
        Ok(png)
    }

    /// `{width, height, rotation, words:[[x0,y0,x1,y1,"text"],...]}` in unrotated points.
    pub fn page_words(&self, fid: &str, page_no: usize) -> Result<Value> {
        let (_, path) = self.pdf_file(fid)?;
        let doc = self.pdf().open(&path)?;
        let pages = doc.pages()?;
        if page_no == 0 || page_no > pages.len() {
            return Err(Error::not_found(format!("Page {} does not exist; the PDF has {} pages.", page_no, pages.len())));
        }
        let p = &pages[page_no - 1];
        let words = doc.words(page_no)?;
        Ok(json!({
            "width": round2(p.width), "height": round2(p.height), "rotation": p.rotation,
            "words": words.iter().map(|w| json!([round2(w.x0), round2(w.y0), round2(w.x1), round2(w.y1), w.text])).collect::<Vec<_>>(),
        }))
    }

    /// Page text of a PDF page, words joined by spaces.
    pub fn page_text(&self, fid: &str, page_no: usize) -> Result<String> {
        let (_, path) = self.pdf_file(fid)?;
        let doc = self.pdf().open(&path)?;
        let words = doc.words(page_no)?;
        Ok(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
    }

    /// File bytes with their content type (Markdown, images).
    pub fn raw(&self, fid: &str) -> Result<(Vec<u8>, &'static str)> {
        let f = self.file_row(fid)?;
        let path = self.usable_path(&f)?;
        Ok((std::fs::read(path)?, media::content_type(&f.ext)))
    }

    /// First page or image, 120 px wide; cached by SHA-256.
    pub fn thumb(&self, fid: &str) -> Result<Vec<u8>> {
        let f = self.file_row(fid)?;
        if !THUMB_EXTS.contains(&f.ext.as_str()) {
            return Err(Error::not_found("Markdown files have no thumbnail image; the catalogue draws one."));
        }
        if f.cloud_only {
            return Err(Error::not_found("Online-only files get no thumbnail, so OneDrive keeps them online."));
        }
        let path = self.usable_path(&f)?;
        let cache = f.sha256.as_ref().map(|s| self.paths().thumbs_dir().join(format!("{}.png", s)));
        if let Some(c) = &cache {
            if let Ok(bytes) = std::fs::read(c) {
                return Ok(bytes);
            }
        }
        let png = if f.ext == "pdf" {
            let doc = self.pdf().open(&path)?;
            doc.thumbnail_png(THUMB_WIDTH)?
        } else {
            media::image_thumbnail_png(&path, THUMB_WIDTH)?
        };
        if let Some(c) = &cache {
            if let Err(e) = atomic_write_bytes(c, &png) {
                log::warn!("thumbnail cache write failed: {}", e);
            }
        }
        Ok(png)
    }

    /// Rotates one page (`Some(n)`) or every page of a PDF in the file, or the view of an
    /// image or Markdown file. Returns `file_info`.
    pub fn rotate(&self, fid: &str, page: Option<i64>, delta: i32, origin: Origin) -> Result<Value> {
        let f = self.file_row(fid)?;
        if delta % 90 != 0 {
            return Err(Error::bad("delta must be a multiple of 90."));
        }
        let base = Path::new(&f.relpath).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if f.ext == "pdf" {
            self.require_writable()?;
            let path = self.usable_path(&f)?;
            let page_no = match page {
                Some(n) if n >= 1 => Some(n as usize),
                Some(_) => return Err(Error::bad("page must be a page number, or null for every page.")),
                None => None,
            };
            let _g = self.write_lock().acquire()?;
            let mut doc = self.pdf().open(&path)?;
            doc.rotate(page_no, delta)?;
            doc.save_in_place()?;
            let st = crate::util::file_stat(&path)?;
            self.db().tx(|c| {
                Self::apply_stat(c, fid, &st)?;
                let what = format!("Rotated {} in {}", page.map(|p| format!("page {}", p)).unwrap_or_else(|| "every page".into()), base);
                self.changes().log(c, &what, &[f.ref_id.clone(), fid.to_string()], origin.as_str())?;
                Ok(())
            })?;
        } else {
            let rot = (f.view_rotation.as_deref().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) + delta).rem_euclid(360);
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                c.execute("UPDATE file SET view_rotation=? WHERE id=?", params![rot.to_string(), fid])?;
                self.changes().log(c, &format!("Rotated the view of {}", base), &[f.ref_id.clone(), fid.to_string()], origin.as_str())?;
                Ok(())
            })?;
        }
        self.file_info(fid)
    }

    // ------------------------------------------------------------ highlights
    pub fn highlights(&self, filter: &HighlightFilter) -> Result<Vec<Highlight>> {
        let rows = self.db().with(|c| {
            let mut sql = String::from(
                "SELECT h.* FROM highlight h JOIN reference r ON r.id=h.ref_id WHERE r.deleted_at IS NULL",
            );
            let mut args: Vec<String> = Vec::new();
            if let Some(rid) = &filter.ref_id {
                sql.push_str(" AND h.ref_id=?");
                args.push(rid.clone());
            }
            if let Some(fid) = &filter.file_id {
                sql.push_str(" AND h.file_id=?");
                args.push(fid.clone());
            }
            sql.push_str(" ORDER BY h.ref_id, h.page, h.created_at, h.id");
            let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            db::query_hls(c, &sql, &refs)
        })?;
        let mut out: Vec<Highlight> = rows.iter().map(hl_record).collect();
        if let Some(tag) = &filter.tag {
            let t = tag.trim_start_matches('#').to_lowercase();
            out.retain(|h| h.tags.iter().any(|x| x.to_lowercase() == t));
        }
        if let Some(a) = &filter.author {
            let a = a.to_lowercase();
            out.retain(|h| h.author.to_lowercase() == a);
        }
        Ok(out)
    }

    pub fn all_highlights(&self) -> Result<Vec<Highlight>> {
        self.highlights(&HighlightFilter::default())
    }

    pub fn highlight(&self, hid: &str) -> Result<Highlight> {
        self.db().with(|c| Ok(hl_record(&db::hl_by_id(c, hid)?)))
    }

    fn quads_pdf(quads: Option<&Vec<Vec<f64>>>) -> Result<Vec<Vec<f64>>> {
        let quads = quads.filter(|q| !q.is_empty()).ok_or_else(|| Error::bad("quads must be a list of [x0, y0, x1, y1] boxes in page points."))?;
        let mut out = Vec::new();
        for q in quads {
            if q.len() != 4 {
                return Err(Error::bad("Each quad must be [x0, y0, x1, y1]."));
            }
            if q.iter().any(|v| !v.is_finite()) {
                return Err(Error::bad("Quad values must be numbers."));
            }
            out.push(vec![q[0].min(q[2]), q[1].min(q[3]), q[0].max(q[2]), q[1].max(q[3])]);
        }
        Ok(out)
    }

    fn quads_image(quads: Option<&Vec<Vec<f64>>>) -> Result<Vec<Vec<f64>>> {
        let quads = quads.filter(|q| !q.is_empty()).ok_or_else(|| Error::bad("An image highlight needs quads as [[x, y, w, h]] normalised to 0..1."))?;
        let mut out = Vec::new();
        for q in quads {
            if q.len() != 4 {
                return Err(Error::bad("Each image quad must be [x, y, w, h]."));
            }
            if q.iter().any(|v| !v.is_finite()) {
                return Err(Error::bad("Quad values must be numbers."));
            }
            let (x, y) = (q[0].clamp(0.0, 1.0), q[1].clamp(0.0, 1.0));
            let (w, h) = (q[2].max(0.0).min(1.0 - x), q[3].max(0.0).min(1.0 - y));
            let r5 = |v: f64| (v * 100000.0).round() / 100000.0;
            out.push(vec![r5(x), r5(y), r5(w), r5(h)]);
        }
        Ok(out)
    }

    /// Adds a highlight. PDFs get a real annotation; Markdown and images go to the database
    /// and the sidecar.
    pub fn add_highlight(&self, body: &NewHighlight, writer: &Writer) -> Result<Highlight> {
        let f = self.file_row(&body.file_id)?;
        let tags = pdf::clean_tags(body.tags.as_deref().unwrap_or(&[]));
        let analysis = body.analysis.clone().unwrap_or_default().trim().to_string();
        let label = match body.label.as_deref().filter(|l| !l.trim().is_empty()) {
            Some(l) => norm_label(l)?,
            None => pdf::parse_label(&analysis),
        };
        let mut verbatim = norm_space(body.verbatim.as_deref().unwrap_or(""));
        let colour = self.setting_str("hl_colour");
        let now = now_iso();
        let (r, authors) = self.db().with(|c| {
            let r = db::live_ref(c, &f.ref_id)?;
            let a = db::authors_of(c, &r.id)?;
            Ok((r, a))
        })?;
        let what = format!("Added a highlight to {}", label_of(&r, &authors));
        let mut row = HlRow {
            id: new_id("h"),
            file_id: f.id.clone(),
            ref_id: f.ref_id.clone(),
            page: None,
            kind: String::new(),
            quads: "[]".into(),
            anchor: None,
            verbatim: verbatim.clone(),
            analysis,
            label,
            tags: dumps_strs(&tags),
            author: writer.author.clone(),
            origin: writer.origin.clone(),
            colour: Some(colour.clone()),
            pdf_xref: None,
            created_at: Some(now.clone()),
            updated_at: Some(now),
            suggestion: None,
            suggested_tags: None,
        };
        let kind = body.kind.clone().filter(|k| !k.is_empty());
        if f.ext == "pdf" {
            self.require_writable()?;
            let path = self.usable_path(&f)?;
            let mut kind = kind.unwrap_or_else(|| "text".into());
            if !["text", "region", "note"].contains(&kind.as_str()) {
                return Err(Error::bad("kind must be text, region or note for a PDF."));
            }
            let page = if kind == "note" && body.page.is_none() {
                1usize
            } else {
                match body.page {
                    Some(p) if p >= 1 => p as usize,
                    _ => return Err(Error::bad("A PDF highlight needs the page number (1-based).")),
                }
            };
            let quads = if kind == "note" && body.quads.as_ref().map(|q| q.is_empty()).unwrap_or(true) {
                vec![vec![18.0, 18.0, 38.0, 38.0]]
            } else {
                Self::quads_pdf(body.quads.as_ref())?
            };
            let _g = self.write_lock().acquire()?;
            let mut doc = self.pdf().open(&path)?;
            if kind != "note" && verbatim.is_empty() {
                verbatim = pdf::words_under(&doc.words(page)?, &quads);
                if verbatim.is_empty() {
                    kind = "region".into();
                }
            }
            let added = doc.add_highlight(&NewAnnot {
                page,
                quads,
                author: writer.author.clone(),
                contents: pdf::compose(&row.analysis, &tags),
                colour,
                kind: kind.clone(),
                created: None,
            })?;
            doc.save_in_place()?;
            let st = crate::util::file_stat(&path)?;
            row.page = Some(page as i64);
            row.kind = kind;
            row.quads = dumps(&json!(added.quads));
            row.pdf_xref = Some(added.xref);
            row.verbatim = verbatim;
            self.db().tx(|c| {
                Self::apply_stat(c, &f.id, &st)?;
                db::insert_highlight(c, &row)?;
                self.changes().log(c, &what, &[row.id.clone(), f.ref_id.clone()], writer.change_origin.as_str())?;
                Ok(())
            })?;
        } else if media::MARKDOWN_EXTS.contains(&f.ext.as_str()) {
            let kind = kind.unwrap_or_else(|| "md".into());
            if !["md", "note"].contains(&kind.as_str()) {
                return Err(Error::bad("kind must be md or note for a Markdown file."));
            }
            let mut anchor = body.anchor.clone().filter(|a| !a.is_null());
            if anchor.is_none() && kind == "md" {
                anchor = Some(json!({"text": verbatim, "occurrence": 0}));
            }
            if let Some(a) = anchor.as_ref() {
                let text = a.get("text").and_then(|t| t.as_str()).map(str::trim).unwrap_or("");
                if !a.is_object() || text.is_empty() {
                    return Err(Error::bad("A Markdown highlight needs anchor {text, occurrence} or verbatim text."));
                }
                let occ = match a.get("occurrence") {
                    None | Some(Value::Null) => 0,
                    Some(Value::Number(n)) => n.as_i64().ok_or_else(|| Error::bad("anchor.occurrence must be a whole number counted from 0."))?,
                    Some(Value::String(s)) if s.trim().is_empty() => 0,
                    Some(Value::String(s)) => s.trim().parse::<i64>().map_err(|_| Error::bad("anchor.occurrence must be a whole number counted from 0."))?,
                    Some(_) => return Err(Error::bad("anchor.occurrence must be a whole number counted from 0.")),
                }
                .max(0);
                let full = a.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                if verbatim.is_empty() {
                    verbatim = norm_space(&full);
                }
                anchor = Some(json!({"text": full, "occurrence": occ}));
            }
            row.kind = kind;
            row.anchor = anchor.map(|a| dumps(&a));
            row.verbatim = verbatim;
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                db::insert_highlight(c, &row)?;
                self.changes().log(c, &what, &[row.id.clone(), f.ref_id.clone()], writer.change_origin.as_str())?;
                Ok(())
            })?;
            drop(_g);
            self.sidecar_write();
        } else if media::IMAGE_EXTS.contains(&f.ext.as_str()) {
            let kind = kind.unwrap_or_else(|| "region".into());
            if !["region", "note"].contains(&kind.as_str()) {
                return Err(Error::bad("kind must be region or note for an image."));
            }
            let quads = if kind == "note" && body.quads.as_ref().map(|q| q.is_empty()).unwrap_or(true) {
                Vec::new()
            } else {
                Self::quads_image(body.quads.as_ref())?
            };
            row.kind = kind;
            row.quads = dumps(&json!(quads));
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                db::insert_highlight(c, &row)?;
                self.changes().log(c, &what, &[row.id.clone(), f.ref_id.clone()], writer.change_origin.as_str())?;
                Ok(())
            })?;
            drop(_g);
            self.sidecar_write();
        } else {
            return Err(Error::bad("Highlights work on PDF, Markdown and image files."));
        }
        self.highlight(&row.id)
    }

    /// Finds `quote` on a PDF page and adds a text highlight over it (MCP `highlight_add`).
    pub fn add_quote_highlight(&self, file_id: &str, page: usize, quote: &str, analysis: &str, label: Option<&str>, tags: &[String], writer: &Writer) -> Result<Highlight> {
        let (_, path) = self.pdf_file(file_id)?;
        let doc = self.pdf().open(&path)?;
        let words = doc.words(page)?;
        drop(doc);
        let (quads, verbatim) = pdf::find_quote(&words, quote).ok_or_else(|| {
            Error::not_found(format!(
                "The quote was not found on page {}. Check the page number and the wording, or pass quads.",
                page
            ))
        })?;
        self.add_highlight(
            &NewHighlight {
                file_id: file_id.to_string(),
                page: Some(page as i64),
                kind: Some("text".into()),
                quads: Some(quads),
                anchor: None,
                verbatim: Some(verbatim),
                analysis: Some(analysis.to_string()),
                label: label.map(str::to_string),
                tags: Some(tags.to_vec()),
            },
            writer,
        )
    }

    fn write_pdf_content(&self, h: &HlRow, f: &FileRow, analysis: &str, tags: &[String]) -> Result<FileStat> {
        self.require_writable()?;
        let path = self.usable_path(f)?;
        let xref = h.pdf_xref.ok_or_else(|| Error::not_found("The highlight has no annotation in the PDF."))?;
        let mut doc = self.pdf().open(&path)?;
        doc.update_highlight(xref, &AnnotPatch { contents: Some(pdf::compose(analysis, tags)), author: None, colour: None })?;
        doc.save_in_place()?;
        crate::util::file_stat(&path)
    }

    pub fn update_highlight(&self, hid: &str, patch: &HighlightPatch, origin: Origin) -> Result<Highlight> {
        let (h, f) = self.db().with(|c| {
            let h = db::hl_by_id(c, hid)?;
            let f = db::file_by_id(c, &h.file_id)?;
            Ok((h, f))
        })?;
        let mut fields: Vec<(&str, Value)> = Vec::new();
        let mut new_analysis = h.analysis.clone();
        if let Some(a) = &patch.analysis {
            new_analysis = a.trim().to_string();
            fields.push(("analysis", Value::String(new_analysis.clone())));
        }
        let mut new_tags = loads_strs(Some(&h.tags));
        if let Some(t) = &patch.tags {
            new_tags = pdf::clean_tags(t);
            fields.push(("tags", Value::String(dumps_strs(&new_tags))));
        }
        if let Some(label) = &patch.label {
            let l = match label {
                Some(v) => norm_label(v)?,
                None => None,
            };
            fields.push(("label", l.map(Value::String).unwrap_or(Value::Null)));
        } else if patch.analysis.is_some() && h.label.is_none() {
            fields.push(("label", pdf::parse_label(&new_analysis).map(Value::String).unwrap_or(Value::Null)));
        }
        if let Some(v) = &patch.verbatim {
            fields.push(("verbatim", Value::String(norm_space(v))));
        }
        if let Some(st) = &patch.suggested_tags {
            fields.push(("suggested_tags", Value::String(dumps_strs(&pdf::clean_tags(st)))));
        }
        if fields.is_empty() {
            return Ok(hl_record(&h));
        }
        fields.push(("updated_at", Value::String(now_iso())));
        let content_changed = new_analysis != h.analysis || new_tags != loads_strs(Some(&h.tags));
        let (r, authors) = self.db().with(|c| {
            let r = db::live_ref(c, &h.ref_id)?;
            let a = db::authors_of(c, &r.id)?;
            Ok((r, a))
        })?;
        let what = format!("Edited a highlight in {}", label_of(&r, &authors));
        {
            let _g = self.write_lock().acquire()?;
            let st = if f.ext == "pdf" && h.pdf_xref.is_some() && content_changed {
                Some(self.write_pdf_content(&h, &f, &new_analysis, &new_tags)?)
            } else {
                None
            };
            self.db().tx(|c| {
                if let Some(st) = &st {
                    Self::apply_stat(c, &f.id, st)?;
                }
                db::update_fields(c, "highlight", "id", hid, &fields)?;
                self.changes().log(c, &what, &[hid.to_string(), h.ref_id.clone()], origin.as_str())?;
                Ok(())
            })?;
        }
        if f.ext != "pdf" {
            self.sidecar_write();
        }
        self.highlight(hid)
    }

    /// Deletes a highlight (and its PDF annotation) into the trash.
    pub fn delete_highlight(&self, hid: &str, origin: Origin) -> Result<TrashResult> {
        let (h, f, r, authors) = self.db().with(|c| {
            let h = db::hl_by_id(c, hid)?;
            let f = db::file_by_id(c, &h.file_id)?;
            let r = db::live_ref(c, &h.ref_id)?;
            let a = db::authors_of(c, &r.id)?;
            Ok((h, f, r, a))
        })?;
        let what = format!("Deleted a highlight from {}", label_of(&r, &authors));
        let mut payload = json!({"highlight": serde_json::to_value(&h)?, "relpath": f.relpath, "snapshots": []});
        let out = {
            let _g = self.write_lock().acquire()?;
            let mut sha_before: Option<String> = None;
            let mut sha_after: Option<String> = None;
            let mut st: Option<FileStat> = None;
            if f.ext == "pdf" {
                if let Some(xref) = h.pdf_xref {
                    self.require_writable()?;
                    let path = self.usable_path(&f)?;
                    let mut doc = self.pdf().open(&path)?;
                    match doc.delete_highlight(xref) {
                        Ok(snaps) => {
                            doc.save_in_place()?;
                            let stat = crate::util::file_stat(&path)?;
                            payload["snapshots"] = serde_json::to_value(&snaps)?;
                            sha_before = f.sha256.clone();
                            sha_after = Some(stat.sha256.clone());
                            st = Some(stat);
                        }
                        Err(Error::NotFound(_)) => {
                            log::info!("annotation {} already gone from {}", xref, f.relpath);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            self.db().tx(|c| {
                if let Some(st) = &st {
                    Self::apply_stat(c, &f.id, st)?;
                }
                c.execute(
                    "INSERT INTO trash(kind,payload,sha_before,sha_after,deleted_at) VALUES('highlight',?,?,?,?)",
                    params![dumps(&payload), sha_before, sha_after, now_iso()],
                )?;
                let tid = c.last_insert_rowid();
                c.execute("DELETE FROM highlight WHERE id=?", [hid])?;
                self.changes().log(c, &what, &[hid.to_string(), h.ref_id.clone()], origin.as_str())?;
                Ok(TrashResult { trash_id: tid })
            })?
        };
        if f.ext != "pdf" {
            self.sidecar_write();
        }
        Ok(out)
    }

    /// Restores a reference or a highlight from the trash.
    pub fn undelete(&self, trash_id: i64, origin: Origin) -> Result<Restored> {
        let (kind, payload) = self.db().with(|c| {
            c.query_row("SELECT kind,payload FROM trash WHERE id=?", [trash_id], |r| {
                Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?))
            })
            .optional()?
            .ok_or_else(|| Error::not_found("Nothing to undo: the trash entry is gone."))
        })?;
        let payload = loads(payload.as_deref()).unwrap_or(json!({}));
        if kind == "reference" {
            let rid = payload.get("ref_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = payload.get("title").and_then(|v| v.as_str()).filter(|t| !t.is_empty()).unwrap_or("a reference").to_string();
            {
                let _g = self.write_lock().acquire()?;
                self.db().tx(|c| {
                    c.execute("UPDATE reference SET deleted_at=NULL WHERE id=?", [&rid])?;
                    c.execute("DELETE FROM trash WHERE id=?", [trash_id])?;
                    self.changes().log(c, &format!("Restored {}", title), &[rid.clone()], origin.as_str())?;
                    Ok(())
                })?;
            }
            self.sidecar_write();
            return Ok(Restored { restored: "reference".into(), id: rid });
        }
        if kind != "highlight" {
            return Err(Error::bad("This trash entry cannot be restored."));
        }
        let mut h: HlRow = serde_json::from_value(payload.get("highlight").cloned().unwrap_or(Value::Null))
            .map_err(|_| Error::bad("This trash entry cannot be restored."))?;
        let f = self.db().with(|c| db::file_by_id(c, &h.file_id))
            .map_err(|_| Error::not_found("The file of this highlight is no longer in the project."))?;
        let snaps: Vec<AnnotSnapshot> = serde_json::from_value(payload.get("snapshots").cloned().unwrap_or(json!([]))).unwrap_or_default();
        let restored_id = {
            let _g = self.write_lock().acquire()?;
            let mut st: Option<FileStat> = None;
            if !snaps.is_empty() {
                self.require_writable()?;
                let path = self.usable_path(&f)?;
                let mut doc = self.pdf().open(&path)?;
                let mapping = doc.restore_annotations(&snaps)?;
                doc.save_in_place()?;
                st = Some(crate::util::file_stat(&path)?);
                let old = h.pdf_xref.unwrap_or(snaps[0].xref);
                h.pdf_xref = mapping.get(&old).or_else(|| mapping.get(&snaps[0].xref)).copied();
            }
            self.db().tx(|c| {
                if let Some(st) = &st {
                    Self::apply_stat(c, &f.id, st)?;
                }
                let exists: Option<String> = c.query_row("SELECT id FROM highlight WHERE id=?", [&h.id], |r| r.get(0)).optional()?;
                if exists.is_some() {
                    h.id = new_id("h");
                }
                db::insert_highlight(c, &h)?;
                c.execute("DELETE FROM trash WHERE id=?", [trash_id])?;
                self.changes().log(c, "Restored a highlight", &[h.id.clone(), h.ref_id.clone()], origin.as_str())?;
                Ok(h.id.clone())
            })?
        };
        if f.ext != "pdf" {
            self.sidecar_write();
        }
        Ok(Restored { restored: "highlight".into(), id: restored_id })
    }

    /// Alias of `undelete` with the API's name.
    pub fn undo(&self, trash_id: i64, origin: Origin) -> Result<Restored> {
        self.undelete(trash_id, origin)
    }

    pub fn propose_delete(&self, hid: &str, reason: &str, origin: Origin) -> Result<Highlight> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::bad("Give a reason for the proposed deletion."));
        }
        let h = self.db().with(|c| db::hl_by_id(c, hid))?;
        let sug = json!({"action": "delete", "reason": reason, "at": now_iso(), "resolution": Value::Null});
        {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                c.execute("UPDATE highlight SET suggestion=? WHERE id=?", params![dumps(&sug), hid])?;
                self.changes().log(c, &format!("Proposed deleting a highlight: {}", reason), &[hid.to_string(), h.ref_id.clone()], origin.as_str())?;
                Ok(())
            })?;
        }
        if self.file_row(&h.file_id)?.ext != "pdf" {
            self.sidecar_write();
        }
        self.highlight(hid)
    }

    pub fn suggest_tags(&self, hid: &str, tags: &[String], origin: Origin) -> Result<Highlight> {
        let h = self.db().with(|c| db::hl_by_id(c, hid))?;
        let have = loads_strs(Some(&h.tags));
        let new: Vec<String> = pdf::clean_tags(tags).into_iter().filter(|t| !have.contains(t)).collect();
        if new.is_empty() {
            return Err(Error::bad("Name at least one tag the highlight does not have yet."));
        }
        let mut merged = loads_strs(h.suggested_tags.as_deref());
        merged.extend(new.iter().cloned());
        let merged = pdf::clean_tags(&merged);
        {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                c.execute("UPDATE highlight SET suggested_tags=? WHERE id=?", params![dumps_strs(&merged), hid])?;
                let what = format!("Suggested tags {}", new.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "));
                self.changes().log(c, &what, &[hid.to_string(), h.ref_id.clone()], origin.as_str())?;
                Ok(())
            })?;
        }
        if self.file_row(&h.file_id)?.ext != "pdf" {
            self.sidecar_write();
        }
        self.highlight(hid)
    }

    /// Accepts or dismisses an open suggestion. Accepting a delete suggestion deletes the highlight.
    pub fn resolve_suggestion(&self, hid: &str, resolution: &str, origin: Origin) -> Result<Value> {
        let h = self.db().with(|c| db::hl_by_id(c, hid))?;
        let mut sug = match loads(h.suggestion.as_deref()) {
            Some(Value::Object(m)) if m.get("resolution").map(|r| r.is_null()).unwrap_or(true) => m,
            _ => return Err(Error::bad("This highlight has no open suggestion.")),
        };
        if resolution == "accept" {
            sug.insert("resolution".into(), json!("accepted"));
            self.db().with(|c| {
                c.execute("UPDATE highlight SET suggestion=? WHERE id=?", params![dumps(&Value::Object(sug.clone())), hid])?;
                Ok(())
            })?;
            if sug.get("action").and_then(|a| a.as_str()) == Some("delete") {
                let out = self.delete_highlight(hid, origin)?;
                return Ok(json!({"trash_id": out.trash_id, "deleted": true}));
            }
            return Ok(json!({"deleted": false, "highlight": self.highlight(hid)?}));
        }
        if resolution != "dismiss" {
            return Err(Error::bad("resolution must be accept or dismiss."));
        }
        sug.insert("resolution".into(), json!("dismissed"));
        {
            let _g = self.write_lock().acquire()?;
            self.db().tx(|c| {
                c.execute("UPDATE highlight SET suggestion=? WHERE id=?", params![dumps(&Value::Object(sug.clone())), hid])?;
                self.changes().log(c, "Dismissed a suggestion", &[hid.to_string(), h.ref_id.clone()], origin.as_str())?;
                Ok(())
            })?;
        }
        Ok(json!({"deleted": false, "highlight": self.highlight(hid)?}))
    }

    /// Moves a suggested tag into the tags (or drops it when `dismiss`).
    pub fn accept_tag(&self, hid: &str, tag: &str, dismiss: bool, origin: Origin) -> Result<Highlight> {
        let h = self.db().with(|c| db::hl_by_id(c, hid))?;
        let tag = pdf::clean_tags(&[tag]).into_iter().next().ok_or_else(|| Error::bad("Name the tag to accept."))?;
        let suggested: Vec<String> = loads_strs(h.suggested_tags.as_deref()).into_iter().filter(|t| *t != tag).collect();
        let mut patch = HighlightPatch { suggested_tags: Some(suggested), ..Default::default() };
        if !dismiss {
            let mut tags = loads_strs(Some(&h.tags));
            tags.push(tag);
            patch.tags = Some(pdf::clean_tags(&tags));
        }
        self.update_highlight(hid, &patch, origin)
    }

    // ------------------------------------------------------------ vocabularies
    pub fn authors(&self) -> Result<Vec<Author>> {
        let mut out: BTreeMap<String, (Author, i64)> = BTreeMap::new();
        for r in self.references(&ReferenceFilter { lean: true, ..ReferenceFilter::default() })? {
            for a in &r.authors {
                let entry = out.entry(a.clone()).or_insert_with(|| {
                    (Author { name: a.clone(), ref_ids: Vec::new(), hl_count: 0, top_ref_id: None }, -1)
                });
                entry.0.ref_ids.push(r.id.clone());
                entry.0.hl_count += r.hl_count;
                if r.hl_count > entry.1 {
                    entry.1 = r.hl_count;
                    entry.0.top_ref_id = Some(r.id.clone());
                }
            }
        }
        let mut rows: Vec<Author> = out.into_values().map(|(a, _)| a).collect();
        rows.sort_by(|a, b| {
            (a.name == "Unknown").cmp(&(b.name == "Unknown"))
                .then_with(|| b.hl_count.cmp(&a.hl_count))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(rows)
    }

    pub fn tags(&self) -> Result<Vec<Tag>> {
        let mut out: BTreeMap<String, Tag> = BTreeMap::new();
        for h in self.all_highlights()? {
            for t in &h.tags {
                let e = out.entry(t.clone()).or_insert_with(|| Tag { name: t.clone(), ref_ids: Vec::new(), hl_count: 0 });
                if !e.ref_ids.contains(&h.ref_id) {
                    e.ref_ids.push(h.ref_id.clone());
                }
                e.hl_count += 1;
            }
        }
        let mut rows: Vec<Tag> = out.into_values().collect();
        rows.sort_by(|a, b| b.ref_ids.len().cmp(&a.ref_ids.len()).then_with(|| a.name.cmp(&b.name)));
        Ok(rows)
    }

    pub fn counts(&self) -> Result<Counts> {
        let refs = self.references(&ReferenceFilter { lean: true, ..ReferenceFilter::default() })?;
        let mut authors = std::collections::HashSet::new();
        let mut tags = std::collections::HashSet::new();
        let mut highlights = 0;
        for r in &refs {
            for a in &r.authors {
                if a != "Unknown" {
                    authors.insert(a.clone());
                }
            }
            for t in &r.tags {
                tags.insert(t.clone());
            }
            highlights += r.hl_count;
        }
        Ok(Counts { references: refs.len() as i64, authors: authors.len() as i64, tags: tags.len() as i64, highlights })
    }

    // ------------------------------------------------------------ search
    /// Loose (or strict) search over the catalogue, best first.
    pub fn search(&self, query: &str, loose: bool, limit: usize) -> Result<Vec<SearchHit>> {
        let refs = self.all_references()?;
        Ok(crate::search::rank(&refs, query, loose, limit)
            .into_iter()
            .map(|(score, reference)| SearchHit { score, reference })
            .collect())
    }

    /// Full-text part of the UI search: `search_text` scored per token with the UI rules
    /// (`search::FullText`). The index is built on the first call and then follows the change
    /// feed: a change that names references re-reads their text, settings, AI-log and category
    /// changes leave it as it is, and anything else (a scan, a change without reference ids, a
    /// feed that jumped back or moved more than 500 rows) rebuilds it.
    pub fn text_search(&self, query: &str, loose: bool) -> Result<TextSearch> {
        let tokens = crate::search::ui_tokens(query);
        let mut out = TextSearch { query: query.to_string(), loose, tokens: tokens.clone(), hits: Vec::new() };
        if tokens.is_empty() {
            return Ok(out);
        }
        let mut guard = self.inner.text_index.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh_text_index(&mut guard)?;
        let index = guard.as_ref().expect("index built");
        let mut hits: Vec<TextHit> = index
            .search(&tokens, loose)
            .into_iter()
            .map(|(id, scores)| TextHit { id, score: scores.iter().sum(), scores })
            .collect();
        drop(guard);
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
        out.hits = hits;
        Ok(out)
    }

    fn refresh_text_index(&self, slot: &mut Option<crate::search::FullText>) -> Result<()> {
        self.db().with(|c| {
            let seq = db::change_seq(c);
            let mut rebuild = true;
            if let Some(index) = slot.as_mut() {
                if index.seq() == seq {
                    return Ok(());
                }
                if index.seq() < seq {
                    let ch = change::changes_since(c, index.seq(), 500)?;
                    let mut dirty: Vec<String> = Vec::new();
                    let mut ok = ch.changes.len() < 500;
                    for row in &ch.changes {
                        match text_change(row) {
                            Some(ids) => dirty.extend(ids),
                            None => ok = false,
                        }
                        if !ok {
                            break;
                        }
                    }
                    if ok {
                        dirty.sort();
                        dirty.dedup();
                        let texts = texts_of(c, Some(&dirty))?;
                        for id in &dirty {
                            index.remove(id);
                        }
                        for (id, text) in &texts {
                            index.set(id, text);
                        }
                        index.set_seq(ch.changes.last().map(|r| r.seq).unwrap_or(seq).max(seq));
                        rebuild = false;
                    }
                }
            }
            if rebuild {
                let mut index = crate::search::FullText::new(seq);
                for (id, text) in texts_of(c, None)? {
                    index.set(&id, &text);
                }
                *slot = Some(index);
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ changes and AI log
    pub fn changes_since(&self, since: i64) -> Result<Changes> {
        self.db().with(|c| change::changes_since(c, since, 500))
    }

    pub fn log_ai(&self, tool: &str, client: &str, purpose: &str, args: &Value, result: &str, at: Option<&str>) -> Result<i64> {
        self.db().with(|c| ailog::log_ai(c, tool, client, purpose, args, result, at))
    }

    /// `log_append` as the MCP tool does it: an ai_log row plus a change row.
    pub fn log_append(&self, client: &str, purpose: &str, checks: &str) -> Result<i64> {
        let _g = self.write_lock().acquire()?;
        self.db().tx(|c| {
            let id = ailog::log_ai(c, "log_append", client, purpose, &json!({"checks": checks}), "logged", None)?;
            self.changes().log(c, &format!("AI log entry: {}", purpose.trim()), &[], Origin::Mcp.as_str())?;
            Ok(id)
        })
    }

    /// Writes one change row on its own and tells the subscribers. A write that changed no
    /// catalogue row (an export, a quiet scan, a log entry made by an agent) uses it so the
    /// open app still sees what happened.
    pub fn log_change(&self, what: &str, ids: &[String], origin: Origin) -> Result<i64> {
        self.db().with(|c| self.changes().log(c, what, ids, origin.as_str()))
    }

    pub fn ai_log(&self) -> Result<Vec<AiLogRow>> {
        self.db().with(ailog::ai_log)
    }

    /// Writes `<folder>\.kurasi\AI_USE_LOG.md`; returns its path.
    pub fn export_ai_log(&self) -> Result<PathBuf> {
        self.require_writable()?;
        let rows = self.ai_log()?;
        let text = ailog::render_markdown(&self.name(), &rows);
        let path = self.folder().join(".kurasi").join("AI_USE_LOG.md");
        let _g = self.write_lock().acquire()?;
        atomic_write_bytes(&path, text.as_bytes())?;
        Ok(path)
    }

    /// Writes a reference export under `<folder>\_exports\`; returns its path.
    pub fn export_references(&self, format: &str, ref_ids: Option<&[String]>) -> Result<PathBuf> {
        self.require_writable()?;
        let _g = self.write_lock().acquire()?;
        crate::export::write(self, format, ref_ids)
    }

    // ------------------------------------------------------------ resolve
    /// Resolves a link from a Markdown file to a project file: `{file_id, ref_id}`, or
    /// `{external: true}` (with `relpath` when the target exists outside the catalogue).
    pub fn resolve(&self, href: &str, from_fid: Option<&str>) -> Result<Value> {
        let href = href.trim();
        static SCHEME: OnceLock<Regex> = OnceLock::new();
        static DRIVE: OnceLock<Regex> = OnceLock::new();
        let scheme = SCHEME.get_or_init(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*:").unwrap());
        let drive = DRIVE.get_or_init(|| Regex::new(r"(?i)^[a-z]:[\\/]").unwrap());
        if href.is_empty() || (scheme.is_match(href) && !drive.is_match(href)) {
            return Ok(json!({"external": true}));
        }
        let target = href.split('#').next().unwrap_or("").split('?').next().unwrap_or("");
        let target = percent_decode(target).replace('/', "\\");
        if target.is_empty() {
            return Ok(json!({"external": true}));
        }
        let mut bases: Vec<PathBuf> = Vec::new();
        if let Some(fid) = from_fid {
            if let Ok(f) = self.file_row(fid) {
                if let Ok(p) = self.abspath(&f.relpath) {
                    if let Some(parent) = p.parent() {
                        bases.push(parent.to_path_buf());
                    }
                }
            }
        }
        bases.push(self.folder().to_path_buf());
        let mut target = target;
        if target.starts_with('\\') {
            bases = vec![self.folder().to_path_buf()];
            target = target.trim_start_matches('\\').to_string();
        }
        let root = crate::util::norm_key(self.folder()) + "\\";
        for base in bases {
            let path = crate::util::normalize(&base.join(&target));
            let key = crate::util::norm_key(&path);
            if !key.starts_with(&root) {
                continue;
            }
            let rel = self.relpath(&path);
            let hit = self.db().with(|c| {
                let f = db::file_by_relpath(c, &rel)?;
                Ok(match f {
                    Some(f) => {
                        let r = db::ref_by_id(c, &f.ref_id)?;
                        match r {
                            Some(r) if r.deleted_at.is_none() => Some((f.id, f.ref_id)),
                            _ => None,
                        }
                    }
                    None => None,
                })
            })?;
            if let Some((fid, rid)) = hit {
                return Ok(json!({"file_id": fid, "ref_id": rid}));
            }
            if path.exists() {
                return Ok(json!({"external": true, "relpath": rel}));
            }
        }
        Ok(json!({"external": true}))
    }

    // ------------------------------------------------------------ sidecar
    /// Writes the sidecar after a change. A read-only project skips it: its files stay
    /// unchanged and the database keeps the change. Failures are logged.
    pub fn sidecar_write(&self) -> Option<PathBuf> {
        if self.read_only() || self.sidecar_deferred() {
            return None;
        }
        let result = (|| -> Result<PathBuf> {
            let _g = self.write_lock().acquire()?;
            crate::sidecar::write(self)
        })();
        match result {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("sidecar write failed for {}: {}", self.folder().display(), e);
                None
            }
        }
    }
}

// ------------------------------------------------------------------ free helpers

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn fmt_scale(scale: f64) -> String {
    let s = format!("{}", scale);
    if s.contains('.') { s } else { format!("{}.0", s) }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn depth_of(id: i64, rows: &[CategoryRow]) -> usize {
    let mut depth = 0;
    let mut cur = rows.iter().find(|r| r.id == id).and_then(|r| r.parent_id);
    while let Some(p) = cur {
        depth += 1;
        if depth > 50 {
            break;
        }
        cur = rows.iter().find(|r| r.id == p).and_then(|r| r.parent_id);
    }
    depth
}

pub(crate) fn cat_paths_of(rows: &[CategoryRow]) -> BTreeMap<i64, String> {
    let by_id: BTreeMap<i64, &CategoryRow> = rows.iter().map(|r| (r.id, r)).collect();
    let mut paths: BTreeMap<i64, String> = BTreeMap::new();
    fn path(cid: i64, by_id: &BTreeMap<i64, &CategoryRow>, paths: &mut BTreeMap<i64, String>, depth: usize) -> String {
        if let Some(p) = paths.get(&cid) {
            return p.clone();
        }
        let r = by_id[&cid];
        let p = match r.parent_id {
            Some(parent) if by_id.contains_key(&parent) && depth <= 50 => format!("{}/{}", path(parent, by_id, paths, depth + 1), r.name),
            _ => r.name.clone(),
        };
        paths.insert(cid, p.clone());
        p
    }
    for cid in by_id.keys() {
        path(*cid, &by_id, &mut paths, 0);
    }
    paths
}

/// A reference with no category gets Uncategorized; one with others loses it.
pub(crate) fn fix_uncategorized(c: &Connection, ref_id: &str) -> Result<()> {
    let mut stmt = c.prepare("SELECT category_id FROM reference_category WHERE ref_id=?")?;
    let ids: Vec<i64> = stmt.query_map([ref_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    if ids.is_empty() {
        c.execute("INSERT INTO reference_category(ref_id,category_id) VALUES(?,?)", params![ref_id, UNCATEGORIZED])?;
    } else if ids.len() > 1 && ids.contains(&UNCATEGORIZED) {
        c.execute("DELETE FROM reference_category WHERE ref_id=? AND category_id=?", params![ref_id, UNCATEGORIZED])?;
    }
    Ok(())
}

/// Sets (or with `add`, extends) the categories of a reference from paths, creating missing ones.
pub(crate) fn set_categories(c: &Connection, ref_id: &str, paths: &[String], add: bool) -> Result<()> {
    let mut ids = Vec::new();
    for p in paths {
        ids.push(Project::ensure_category_path_in(c, p)?);
    }
    if !add {
        c.execute("DELETE FROM reference_category WHERE ref_id=?", [ref_id])?;
    }
    for cid in ids {
        c.execute("INSERT OR IGNORE INTO reference_category(ref_id,category_id) VALUES(?,?)", params![ref_id, cid])?;
    }
    fix_uncategorized(c, ref_id)
}

pub fn choose_primary<'a>(files: &'a [FileRow], hl_by_file: &HashMap<String, i64>) -> Option<&'a FileRow> {
    files.iter().min_by(|a, b| {
        let ka = (a.missing as i64, a.cloud_only as i64, -hl_by_file.get(&a.id).copied().unwrap_or(0), role_rank(a), a.relpath.to_lowercase());
        let kb = (b.missing as i64, b.cloud_only as i64, -hl_by_file.get(&b.id).copied().unwrap_or(0), role_rank(b), b.relpath.to_lowercase());
        ka.cmp(&kb)
    })
}

/// `search_text` of a reference: "verbatim analysis" of each highlight (oldest first), trimmed,
/// empty ones skipped, one per line, 20,000 characters at most.
pub(crate) fn search_text_of<'a>(hls: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (verbatim, analysis) in hls {
        let p = format!("{} {}", verbatim, analysis).trim().to_string();
        if !p.is_empty() {
            parts.push(p);
        }
    }
    parts.join("\n").chars().take(20000).collect()
}

/// The texts of live references with highlights, as `search_text_of` builds them; `ids` limits
/// the read to those references.
fn texts_of(c: &Connection, ids: Option<&[String]>) -> Result<Vec<(String, String)>> {
    let mut sql = String::from(
        "SELECT h.ref_id, h.verbatim, h.analysis FROM highlight h JOIN reference r ON r.id = h.ref_id WHERE r.deleted_at IS NULL",
    );
    let args: Vec<&dyn rusqlite::ToSql> = match ids {
        Some(ids) => {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            sql.push_str(&format!(" AND h.ref_id IN ({})", ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")));
            ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect()
        }
        None => Vec::new(),
    };
    sql.push_str(" ORDER BY h.ref_id, h.created_at, h.id");
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(args.as_slice(), |r| {
        Ok((
            r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        ))
    })?;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur: Option<(String, Vec<(String, String)>)> = None;
    for row in rows {
        let (rid, verbatim, analysis) = row?;
        match &mut cur {
            Some((id, parts)) if *id == rid => parts.push((verbatim, analysis)),
            _ => {
                if let Some((id, parts)) = cur.take() {
                    out.push((id, search_text_of(parts.iter().map(|(v, a)| (v.as_str(), a.as_str())))));
                }
                cur = Some((rid, vec![(verbatim, analysis)]));
            }
        }
    }
    if let Some((id, parts)) = cur.take() {
        out.push((id, search_text_of(parts.iter().map(|(v, a)| (v.as_str(), a.as_str())))));
    }
    Ok(out)
}

/// What a change row means for the full-text index: `None` asks for a rebuild, `Some(ids)` names
/// the references whose text may have changed (none for settings, the AI log and categories).
fn text_change(c: &ChangeRow) -> Option<Vec<String>> {
    let w = c.what.as_str();
    if w.starts_with("Settings changed") || w.starts_with("AI log entry") || w.starts_with("Category ") {
        return Some(Vec::new());
    }
    let refs: Vec<String> = c.ids.iter().filter(|i| i.starts_with("r-")).cloned().collect();
    if refs.is_empty() || w.starts_with("Scan") {
        None
    } else {
        Some(refs)
    }
}

pub(crate) fn record(r: &RefRow, authors: Vec<String>, mut cats: Vec<String>, files: Vec<FileRow>, hls: &[HlRow], lean: bool) -> Reference {
    let mut hl_by_file: HashMap<String, i64> = HashMap::new();
    for h in hls {
        *hl_by_file.entry(h.file_id.clone()).or_insert(0) += 1;
    }
    let primary = choose_primary(&files, &hl_by_file).cloned();
    let mut tag_counts: BTreeMap<String, i64> = BTreeMap::new();
    for h in hls {
        for t in loads_strs(Some(&h.tags)) {
            *tag_counts.entry(t).or_insert(0) += 1;
        }
    }
    let mut top: Vec<(&String, &i64)> = tag_counts.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let search_text = if lean { String::new() } else { search_text_of(hls.iter().map(|h| (h.verbatim.as_str(), h.analysis.as_str()))) };
    let ext = primary.as_ref().map(|p| p.ext.clone()).unwrap_or_default();
    let thumb = primary
        .as_ref()
        .filter(|p| THUMB_EXTS.contains(&p.ext.as_str()) && !p.missing && !p.cloud_only)
        .map(|p| format!("/api/thumb/{}.png", p.id));
    cats.sort();
    Reference {
        id: r.id.clone(),
        slug: r.slug.clone(),
        title: if r.title.is_empty() { r.slug.clone() } else { r.title.clone() },
        authors: if authors.is_empty() { vec!["Unknown".into()] } else { authors },
        kind: r.kind.clone(),
        pub_: r.pub_.clone(),
        year: r.year.clone(),
        date: r.date.clone(),
        lang: r.lang.clone(),
        usability: r.usability.clone(),
        status: if r.status.is_empty() { "Auto".into() } else { r.status.clone() },
        register_id: r.register_id.clone(),
        categories: if cats.is_empty() { vec!["Uncategorized".into()] } else { cats },
        ext,
        pages: primary.as_ref().and_then(|p| p.page_count).unwrap_or(0),
        hl_count: hls.len() as i64,
        top_tags: top.iter().take(3).map(|(t, _)| (*t).clone()).collect(),
        tags: tag_counts.keys().cloned().collect(),
        missing: if files.is_empty() { true } else { files.iter().all(|f| f.missing) },
        files: files
            .iter()
            .map(|f| FileEntry {
                id: f.id.clone(),
                relpath: f.relpath.clone(),
                role: f.role.clone(),
                ext: f.ext.clone(),
                pages: f.page_count.unwrap_or(0),
                missing: f.missing,
                cloud_only: f.cloud_only,
                hl_count: hl_by_file.get(&f.id).copied().unwrap_or(0),
            })
            .collect(),
        primary_file_id: primary.as_ref().map(|p| p.id.clone()),
        thumb_url: thumb,
        created_at: r.created_at.clone(),
        updated_at: r.updated_at.clone(),
        search_text,
        doi: Some(r.doi.clone()),
        url: Some(r.url.clone()),
        vol: Some(r.vol.clone()),
        issue: Some(r.issue.clone()),
        pages_range: Some(r.pages_range.clone()),
    }
}

pub fn hl_record(row: &HlRow) -> Highlight {
    Highlight {
        id: row.id.clone(),
        file_id: row.file_id.clone(),
        ref_id: row.ref_id.clone(),
        page: row.page,
        kind: row.kind.clone(),
        quads: loads_quads(Some(&row.quads)),
        anchor: loads(row.anchor.as_deref()),
        verbatim: row.verbatim.clone(),
        analysis: row.analysis.clone(),
        label: row.label.clone(),
        tags: loads_strs(Some(&row.tags)),
        author: row.author.clone(),
        origin: row.origin.clone(),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
        suggestion: loads(row.suggestion.as_deref()),
        suggested_tags: loads_strs(row.suggested_tags.as_deref()),
    }
}

/// Deletes the oldest page renders until the cache is under the limit (500 MB).
pub fn trim_page_cache(pages_dir: &Path, limit: u64) -> usize {
    let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut total = 0u64;
    fn walk(dir: &Path, out: &mut Vec<(std::time::SystemTime, u64, PathBuf)>, total: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out, total);
                } else if let Ok(m) = e.metadata() {
                    *total += m.len();
                    out.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), p));
                }
            }
        }
    }
    walk(pages_dir, &mut entries, &mut total);
    if total <= limit {
        return 0;
    }
    entries.sort();
    let mut removed = 0;
    for (_, size, p) in entries {
        if std::fs::remove_file(&p).is_ok() {
            total -= size;
            removed += 1;
        }
        if total <= limit {
            break;
        }
    }
    removed
}
