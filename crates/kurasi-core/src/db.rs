//! SQLite schema, migrations and data access helpers.
//!
//! One connection per project and process, shared by threads behind a mutex. WAL mode lets
//! the app and the MCP server read while the other writes. The schema is the one of the
//! Python build (CONTRACT.md section 3), so a `kurasi.sqlite` made by either build opens in
//! the other unchanged.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row, ToSql};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{CategoryRow, FileRow, HlRow, RefRow};
use crate::util::now_iso;

pub const SCHEMA_VERSION: i64 = 1;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS setting(key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS category(
  id INTEGER PRIMARY KEY, parent_id INTEGER NULL, name TEXT NOT NULL, sort INTEGER DEFAULT 0);
CREATE TABLE IF NOT EXISTS reference(
  id TEXT PRIMARY KEY, slug TEXT UNIQUE COLLATE NOCASE, title TEXT DEFAULT '',
  kind TEXT DEFAULT '', year TEXT DEFAULT '', date TEXT DEFAULT '', pub TEXT DEFAULT '',
  vol TEXT DEFAULT '', issue TEXT DEFAULT '', pages_range TEXT DEFAULT '', doi TEXT DEFAULT '',
  url TEXT DEFAULT '', lang TEXT DEFAULT '', usability TEXT DEFAULT '', status TEXT DEFAULT 'Auto',
  register_id TEXT DEFAULT '', created_at TEXT, updated_at TEXT, deleted_at TEXT NULL);
CREATE TABLE IF NOT EXISTS reference_author(ref_id TEXT, position INTEGER, name TEXT);
CREATE TABLE IF NOT EXISTS reference_category(
  ref_id TEXT, category_id INTEGER, PRIMARY KEY(ref_id, category_id));
CREATE TABLE IF NOT EXISTS file(
  id TEXT PRIMARY KEY, ref_id TEXT, relpath TEXT UNIQUE COLLATE NOCASE, ext TEXT, sha256 TEXT,
  size INTEGER, mtime REAL, page_count INTEGER, role TEXT, cloud_only INTEGER DEFAULT 0,
  missing INTEGER DEFAULT 0, missing_since TEXT, first_seen TEXT, last_seen TEXT,
  view_rotation TEXT);
CREATE TABLE IF NOT EXISTS highlight(
  id TEXT PRIMARY KEY, file_id TEXT, ref_id TEXT, page INTEGER NULL, kind TEXT, quads TEXT,
  anchor TEXT NULL, verbatim TEXT DEFAULT '', analysis TEXT DEFAULT '', label TEXT NULL,
  tags TEXT DEFAULT '[]', author TEXT, origin TEXT, colour TEXT, pdf_xref INTEGER NULL,
  created_at TEXT, updated_at TEXT, suggestion TEXT NULL, suggested_tags TEXT NULL);
CREATE TABLE IF NOT EXISTS trash(
  id INTEGER PRIMARY KEY, kind TEXT, payload TEXT, sha_before TEXT NULL, sha_after TEXT NULL,
  deleted_at TEXT);
CREATE TABLE IF NOT EXISTS ai_log(
  id INTEGER PRIMARY KEY, at TEXT, tool TEXT, client TEXT, purpose TEXT, args TEXT, result TEXT);
CREATE TABLE IF NOT EXISTS change(
  seq INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT, what TEXT, ids TEXT, origin TEXT);
CREATE INDEX IF NOT EXISTS ix_author_ref ON reference_author(ref_id);
CREATE INDEX IF NOT EXISTS ix_refcat_cat ON reference_category(category_id);
CREATE INDEX IF NOT EXISTS ix_file_ref ON file(ref_id);
CREATE INDEX IF NOT EXISTS ix_file_sha ON file(sha256);
CREATE INDEX IF NOT EXISTS ix_hl_file ON highlight(file_id);
CREATE INDEX IF NOT EXISTS ix_hl_ref ON highlight(ref_id);
";

/// `json.dumps(value, ensure_ascii=False)`.
pub fn dumps(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

pub fn dumps_strs(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".into())
}

/// `loads(text, default)`: empty or broken text gives `None`.
pub fn loads(text: Option<&str>) -> Option<Value> {
    let t = text?;
    if t.is_empty() {
        return None;
    }
    serde_json::from_str(t).ok()
}

pub fn loads_strs(text: Option<&str>) -> Vec<String> {
    match loads(text) {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                other if !other.is_null() => Some(other.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn loads_quads(text: Option<&str>) -> Vec<Vec<f64>> {
    match loads(text) {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|q| match q {
                Value::Array(nums) => Some(nums.into_iter().filter_map(|n| n.as_f64()).collect::<Vec<f64>>()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub struct Db {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Db { path: path.to_path_buf(), conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> Result<()> {
        self.with(|c| {
            let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version < 1 {
                // Every statement is idempotent, so two processes starting at once are safe.
                c.execute_batch(SCHEMA)?;
                c.execute_batch("BEGIN IMMEDIATE")?;
                let seeded: Option<i64> =
                    c.query_row("SELECT id FROM category WHERE id=1", [], |r| r.get(0)).optional()?;
                if seeded.is_none() {
                    c.execute("INSERT INTO category(id,parent_id,name,sort) VALUES(1,NULL,'Uncategorized',0)", [])?;
                }
                c.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                c.execute_batch("COMMIT")?;
            }
            Ok(())
        })
    }

    /// Runs `f` with the connection. Reads and single-statement writes go here.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    /// Runs `f` inside `BEGIN IMMEDIATE ... COMMIT`; an error rolls back.
    pub fn tx<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        guard.execute_batch("BEGIN IMMEDIATE")?;
        match f(&guard) {
            Ok(v) => {
                guard.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = guard.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn change_seq(&self) -> i64 {
        self.with(|c| Ok(change_seq(c))).unwrap_or(0)
    }

    pub fn get_setting(&self, key: &str) -> Option<Value> {
        self.with(|c| Ok(get_setting(c, key))).ok().flatten()
    }

    pub fn all_settings(&self) -> serde_json::Map<String, Value> {
        self.with(|c| all_settings(c)).unwrap_or_default()
    }
}

// ------------------------------------------------------------------ connection helpers

pub fn change_seq(c: &Connection) -> i64 {
    c.query_row("SELECT COALESCE(MAX(seq),0) FROM change", [], |r| r.get(0)).unwrap_or(0)
}

pub fn log_change(c: &Connection, what: &str, ids: &[String], origin: &str) -> Result<i64> {
    c.execute(
        "INSERT INTO change(at,what,ids,origin) VALUES(?,?,?,?)",
        params![now_iso(), what, dumps_strs(ids), origin],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn get_setting(c: &Connection, key: &str) -> Option<Value> {
    let text: Option<String> =
        c.query_row("SELECT value FROM setting WHERE key=?", [key], |r| r.get(0)).optional().ok().flatten();
    loads(text.as_deref())
}

pub fn set_setting(c: &Connection, key: &str, value: &Value) -> Result<()> {
    c.execute(
        "INSERT INTO setting(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, dumps(value)],
    )?;
    Ok(())
}

pub fn all_settings(c: &Connection) -> Result<serde_json::Map<String, Value>> {
    let mut stmt = c.prepare("SELECT key,value FROM setting")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
    let mut out = serde_json::Map::new();
    for row in rows {
        let (k, v) = row?;
        out.insert(k, loads(v.as_deref()).unwrap_or(Value::Null));
    }
    Ok(out)
}

pub fn ref_from_row(r: &Row) -> rusqlite::Result<RefRow> {
    Ok(RefRow {
        id: r.get("id")?,
        slug: r.get::<_, Option<String>>("slug")?.unwrap_or_default(),
        title: r.get::<_, Option<String>>("title")?.unwrap_or_default(),
        kind: r.get::<_, Option<String>>("kind")?.unwrap_or_default(),
        year: r.get::<_, Option<String>>("year")?.unwrap_or_default(),
        date: r.get::<_, Option<String>>("date")?.unwrap_or_default(),
        pub_: r.get::<_, Option<String>>("pub")?.unwrap_or_default(),
        vol: r.get::<_, Option<String>>("vol")?.unwrap_or_default(),
        issue: r.get::<_, Option<String>>("issue")?.unwrap_or_default(),
        pages_range: r.get::<_, Option<String>>("pages_range")?.unwrap_or_default(),
        doi: r.get::<_, Option<String>>("doi")?.unwrap_or_default(),
        url: r.get::<_, Option<String>>("url")?.unwrap_or_default(),
        lang: r.get::<_, Option<String>>("lang")?.unwrap_or_default(),
        usability: r.get::<_, Option<String>>("usability")?.unwrap_or_default(),
        status: r.get::<_, Option<String>>("status")?.unwrap_or_else(|| "Auto".into()),
        register_id: r.get::<_, Option<String>>("register_id")?.unwrap_or_default(),
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        deleted_at: r.get("deleted_at")?,
    })
}

pub fn file_from_row(r: &Row) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get("id")?,
        ref_id: r.get::<_, Option<String>>("ref_id")?.unwrap_or_default(),
        relpath: r.get::<_, Option<String>>("relpath")?.unwrap_or_default(),
        ext: r.get::<_, Option<String>>("ext")?.unwrap_or_default(),
        sha256: r.get("sha256")?,
        size: r.get::<_, Option<i64>>("size")?.unwrap_or(0),
        mtime: r.get::<_, Option<f64>>("mtime")?.unwrap_or(0.0),
        page_count: r.get("page_count")?,
        role: r.get::<_, Option<String>>("role")?.unwrap_or_else(|| "primary".into()),
        cloud_only: r.get::<_, Option<i64>>("cloud_only")?.unwrap_or(0) != 0,
        missing: r.get::<_, Option<i64>>("missing")?.unwrap_or(0) != 0,
        missing_since: r.get("missing_since")?,
        first_seen: r.get("first_seen")?,
        last_seen: r.get("last_seen")?,
        view_rotation: r.get("view_rotation")?,
    })
}

pub fn hl_from_row(r: &Row) -> rusqlite::Result<HlRow> {
    Ok(HlRow {
        id: r.get("id")?,
        file_id: r.get::<_, Option<String>>("file_id")?.unwrap_or_default(),
        ref_id: r.get::<_, Option<String>>("ref_id")?.unwrap_or_default(),
        page: r.get("page")?,
        kind: r.get::<_, Option<String>>("kind")?.unwrap_or_default(),
        quads: r.get::<_, Option<String>>("quads")?.unwrap_or_else(|| "[]".into()),
        anchor: r.get("anchor")?,
        verbatim: r.get::<_, Option<String>>("verbatim")?.unwrap_or_default(),
        analysis: r.get::<_, Option<String>>("analysis")?.unwrap_or_default(),
        label: r.get("label")?,
        tags: r.get::<_, Option<String>>("tags")?.unwrap_or_else(|| "[]".into()),
        author: r.get::<_, Option<String>>("author")?.unwrap_or_default(),
        origin: r.get::<_, Option<String>>("origin")?.unwrap_or_default(),
        colour: r.get("colour")?,
        pdf_xref: r.get("pdf_xref")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        suggestion: r.get("suggestion")?,
        suggested_tags: r.get("suggested_tags")?,
    })
}

pub fn cat_from_row(r: &Row) -> rusqlite::Result<CategoryRow> {
    Ok(CategoryRow {
        id: r.get("id")?,
        parent_id: r.get("parent_id")?,
        name: r.get::<_, Option<String>>("name")?.unwrap_or_default(),
        sort: r.get::<_, Option<i64>>("sort")?.unwrap_or(0),
    })
}

pub fn query_refs(c: &Connection, sql: &str, args: &[&dyn ToSql]) -> Result<Vec<RefRow>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt.query_map(args, ref_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn query_files(c: &Connection, sql: &str, args: &[&dyn ToSql]) -> Result<Vec<FileRow>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt.query_map(args, file_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn query_hls(c: &Connection, sql: &str, args: &[&dyn ToSql]) -> Result<Vec<HlRow>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt.query_map(args, hl_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn query_cats(c: &Connection) -> Result<Vec<CategoryRow>> {
    let mut stmt = c.prepare("SELECT * FROM category ORDER BY sort, id")?;
    let rows = stmt.query_map([], cat_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn ref_by_id(c: &Connection, id: &str) -> Result<Option<RefRow>> {
    Ok(c.query_row("SELECT * FROM reference WHERE id=?", [id], ref_from_row).optional()?)
}

pub fn live_ref(c: &Connection, id: &str) -> Result<RefRow> {
    c.query_row("SELECT * FROM reference WHERE id=? AND deleted_at IS NULL", [id], ref_from_row)
        .optional()?
        .ok_or_else(|| Error::not_found(format!("The reference {} does not exist or is in the trash.", id)))
}

pub fn ref_by_slug(c: &Connection, slug: &str) -> Result<Option<RefRow>> {
    Ok(c.query_row("SELECT * FROM reference WHERE slug=? COLLATE NOCASE", [slug], ref_from_row).optional()?)
}

pub fn file_by_id(c: &Connection, id: &str) -> Result<FileRow> {
    c.query_row("SELECT * FROM file WHERE id=?", [id], file_from_row)
        .optional()?
        .ok_or_else(|| Error::not_found(format!("The file {} is not in this project.", id)))
}

pub fn file_by_relpath(c: &Connection, relpath: &str) -> Result<Option<FileRow>> {
    Ok(c.query_row("SELECT * FROM file WHERE relpath=? COLLATE NOCASE", [relpath], file_from_row).optional()?)
}

pub fn hl_by_id(c: &Connection, id: &str) -> Result<HlRow> {
    c.query_row("SELECT * FROM highlight WHERE id=?", [id], hl_from_row)
        .optional()?
        .ok_or_else(|| Error::not_found(format!("The highlight {} does not exist. Refresh the view.", id)))
}

pub fn authors_of(c: &Connection, ref_id: &str) -> Result<Vec<String>> {
    let mut stmt = c.prepare("SELECT name FROM reference_author WHERE ref_id=? ORDER BY position")?;
    let rows = stmt.query_map([ref_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn set_authors(c: &Connection, ref_id: &str, names: &[String]) -> Result<()> {
    let mut clean: Vec<String> = names.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    if clean.is_empty() {
        clean.push("Unknown".into());
    }
    c.execute("DELETE FROM reference_author WHERE ref_id=?", [ref_id])?;
    for (i, n) in clean.iter().enumerate() {
        c.execute("INSERT INTO reference_author(ref_id,position,name) VALUES(?,?,?)", params![ref_id, i as i64, n])?;
    }
    Ok(())
}

pub fn insert_file(c: &Connection, f: &FileRow) -> Result<()> {
    c.execute(
        "INSERT INTO file(id,ref_id,relpath,ext,sha256,size,mtime,page_count,role,cloud_only,missing,missing_since,\
         first_seen,last_seen,view_rotation) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            f.id, f.ref_id, f.relpath, f.ext, f.sha256, f.size, f.mtime, f.page_count, f.role,
            f.cloud_only as i64, f.missing as i64, f.missing_since, f.first_seen, f.last_seen, f.view_rotation
        ],
    )?;
    Ok(())
}

pub fn insert_highlight(c: &Connection, h: &HlRow) -> Result<()> {
    c.execute(
        "INSERT INTO highlight(id,file_id,ref_id,page,kind,quads,anchor,verbatim,analysis,label,tags,author,origin,\
         colour,pdf_xref,created_at,updated_at,suggestion,suggested_tags) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            h.id, h.file_id, h.ref_id, h.page, h.kind, h.quads, h.anchor, h.verbatim, h.analysis, h.label, h.tags,
            h.author, h.origin, h.colour, h.pdf_xref, h.created_at, h.updated_at, h.suggestion, h.suggested_tags
        ],
    )?;
    Ok(())
}

pub fn insert_reference(c: &Connection, r: &RefRow) -> Result<()> {
    c.execute(
        "INSERT INTO reference(id,slug,title,kind,year,date,pub,vol,issue,pages_range,doi,url,lang,usability,status,\
         register_id,created_at,updated_at,deleted_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            r.id, r.slug, r.title, r.kind, r.year, r.date, r.pub_, r.vol, r.issue, r.pages_range, r.doi, r.url,
            r.lang, r.usability, r.status, r.register_id, r.created_at, r.updated_at, r.deleted_at
        ],
    )?;
    Ok(())
}

/// `UPDATE <table> SET a=?,b=? WHERE <key>=?` from a list of (column, value) pairs.
pub fn update_fields(c: &Connection, table: &str, key: &str, key_value: &str, fields: &[(&str, Value)]) -> Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let sets: Vec<String> = fields.iter().map(|(k, _)| format!("{}=?", k)).collect();
    let sql = format!("UPDATE {} SET {} WHERE {}=?", table, sets.join(","), key);
    let mut args: Vec<Box<dyn ToSql>> = fields.iter().map(|(_, v)| json_to_sql(v)).collect();
    args.push(Box::new(key_value.to_string()));
    let refs: Vec<&dyn ToSql> = args.iter().map(|b| b.as_ref()).collect();
    c.execute(&sql, refs.as_slice())?;
    Ok(())
}

/// A JSON scalar as a SQLite value: strings, numbers, booleans and null map directly;
/// arrays and objects are stored as JSON text.
pub fn json_to_sql(v: &Value) -> Box<dyn ToSql> {
    match v {
        Value::Null => Box::new(rusqlite::types::Null),
        Value::Bool(b) => Box::new(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else {
                Box::new(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => Box::new(s.clone()),
        other => Box::new(dumps(other)),
    }
}

pub fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}
