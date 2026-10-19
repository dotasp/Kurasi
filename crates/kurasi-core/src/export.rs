//! Reference exports under `<folder>\_exports\`: APA 7 Markdown, BibTeX, CSL JSON, highlights JSON.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::lock::atomic_write_text;
use crate::model::{Reference, ReferenceFilter};
use crate::util::now_iso;
use crate::Project;

pub const FORMATS: &[(&str, &str)] = &[
    ("apa-md", "references_apa.md"),
    ("bibtex", "references.bib"),
    ("csl-json", "references.csl.json"),
    ("highlights-json", "highlights.json"),
];

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

fn bib_type(kind: &str) -> &'static str {
    match kind {
        "journal article" | "news article" => "article",
        "book" => "book",
        "report" => "techreport",
        "thesis" => "phdthesis",
        "book chapter" => "incollection",
        "conference paper" => "inproceedings",
        _ => "misc",
    }
}

fn csl_type(kind: &str) -> &'static str {
    match kind {
        "journal article" => "article-journal",
        "book" => "book",
        "report" | "regulatory filing" => "report",
        "news article" => "article-newspaper",
        "statute" => "legislation",
        "press release" => "article",
        "working note" => "manuscript",
        "exhibit" => "figure",
        "book chapter" => "chapter",
        "thesis" => "thesis",
        "conference paper" => "paper-conference",
        "web page" => "webpage",
        _ => "document",
    }
}

pub fn people(r: &Reference) -> Vec<String> {
    r.authors.iter().filter(|a| !a.is_empty() && a.as_str() != "Unknown").cloned().collect()
}

pub fn apa_authors(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{}, & {}", names[0], names[1]),
        n => format!("{}, & {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

pub fn apa_date(r: &Reference) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").unwrap());
    let kind = r.kind.to_lowercase();
    if let Some(m) = re.captures(&r.date) {
        if ["news article", "press release", "web page"].contains(&kind.as_str()) {
            let month: usize = m[2].parse().unwrap_or(1);
            let day: u32 = m[3].parse().unwrap_or(1);
            return format!("{}, {} {}", &m[1], MONTHS[(month.clamp(1, 12)) - 1], day);
        }
    }
    if !r.year.is_empty() {
        return r.year.clone();
    }
    if r.date.len() >= 4 && r.date[..4].chars().all(|c| c.is_ascii_digit()) {
        return r.date[..4].to_string();
    }
    "n.d.".into()
}

/// APA 7 reference text in Markdown (titles of standalone works in italics).
pub fn apa(r: &Reference) -> String {
    let a = apa_authors(&people(r));
    let kind = r.kind.to_lowercase();
    let vol = r.detail("vol");
    let standalone = ["book", "report", "statute", "thesis", ""].contains(&kind.as_str()) && vol.is_empty();
    let shown = if standalone { format!("*{}*", r.title) } else { r.title.clone() };
    let mut s = format!("{}{}({}). {}.", a, if a.is_empty() { "" } else { " " }, apa_date(r), shown);
    if !r.pub_.is_empty() {
        if standalone {
            s.push_str(&format!(" {}.", r.pub_));
        } else {
            s.push_str(&format!(" *{}*", r.pub_));
            if !vol.is_empty() {
                s.push_str(&format!(", *{}*", vol));
                let issue = r.detail("issue");
                if !issue.is_empty() {
                    s.push_str(&format!("({})", issue));
                }
            }
            let pages = r.detail("pages_range");
            if !pages.is_empty() {
                s.push_str(&format!(", {}", pages));
            }
            s.push('.');
        }
    }
    let doi = r.detail("doi");
    let url = r.detail("url");
    if !doi.is_empty() {
        s.push_str(&format!(" https://doi.org/{}", doi));
    } else if !url.is_empty() {
        s.push_str(&format!(" {}", url));
    }
    s.replace("..", ".")
}

fn ascii(text: &str) -> String {
    text.nfkd().filter(|c| c.is_ascii()).collect()
}

pub fn bib_key(r: &Reference, used: &mut HashSet<String>) -> String {
    let names = people(r);
    let mut surname = names
        .first()
        .map(|n| ascii(n.split(',').next().unwrap_or("")).to_lowercase())
        .unwrap_or_else(|| "anon".into());
    surname = surname.chars().filter(|c| c.is_ascii_lowercase()).collect();
    if surname.is_empty() {
        surname = "anon".into();
    }
    static WORDS: OnceLock<Regex> = OnceLock::new();
    let words = WORDS.get_or_init(|| Regex::new(r"[a-z]+").unwrap());
    let title = ascii(&r.title).to_lowercase();
    let word = words.find_iter(&title).map(|m| m.as_str()).find(|w| w.len() > 3).unwrap_or("");
    let base = format!("{}{}{}", surname, if r.year.is_empty() { "nd" } else { &r.year }, word);
    let mut key = base.clone();
    let mut n = 1;
    while used.contains(&key) {
        n += 1;
        key = if n < 28 { format!("{}{}", base, (b'a' + (n - 2) as u8) as char) } else { format!("{}{}", base, n) };
    }
    used.insert(key.clone());
    key
}

fn bib_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('{', "\\{").replace('}', "\\}")
}

pub fn bibtex(refs: &[Reference]) -> String {
    let mut used = HashSet::new();
    let mut out = Vec::new();
    for r in refs {
        let etype = bib_type(&r.kind.to_lowercase());
        let mut fields: Vec<(&str, String)> = vec![("author", people(r).join(" and ")), ("title", r.title.clone()), ("year", r.year.clone())];
        match etype {
            "article" => fields.push(("journal", r.pub_.clone())),
            "book" | "incollection" => fields.push(("publisher", r.pub_.clone())),
            "techreport" => fields.push(("institution", r.pub_.clone())),
            _ => fields.push(("howpublished", r.pub_.clone())),
        }
        fields.push(("volume", r.detail("vol").to_string()));
        fields.push(("number", r.detail("issue").to_string()));
        fields.push(("pages", r.detail("pages_range").replace('-', "--")));
        fields.push(("doi", r.detail("doi").to_string()));
        fields.push(("url", r.detail("url").to_string()));
        fields.push(("language", r.lang.clone()));
        let body: Vec<String> = fields.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("  {} = {{{}}}", k, bib_escape(v))).collect();
        out.push(format!("@{}{{{},\n{}\n}}", etype, bib_key(r, &mut used), body.join(",\n")));
    }
    out.join("\n\n") + "\n"
}

fn csl_name(name: &str) -> Value {
    match name.split_once(',') {
        Some((family, given)) => json!({"family": family.trim(), "given": given.trim()}),
        None => json!({"literal": name}),
    }
}

pub fn csl(refs: &[Reference]) -> String {
    static DIGITS: OnceLock<Regex> = OnceLock::new();
    let digits = DIGITS.get_or_init(|| Regex::new(r"\d+").unwrap());
    let mut items = Vec::new();
    for r in refs {
        let mut item = Map::new();
        item.insert("id".into(), json!(r.id));
        item.insert("type".into(), json!(csl_type(&r.kind.to_lowercase())));
        item.insert("title".into(), json!(r.title));
        let names = people(r);
        if !names.is_empty() {
            item.insert("author".into(), Value::Array(names.iter().map(|n| csl_name(n)).collect()));
        }
        let date = if r.date.is_empty() { r.year.clone() } else { r.date.clone() };
        if date.len() >= 4 && date[..4].chars().all(|c| c.is_ascii_digit()) {
            let parts: Vec<i64> = digits.find_iter(&date).take(3).filter_map(|m| m.as_str().parse().ok()).collect();
            if !parts.is_empty() {
                item.insert("issued".into(), json!({"date-parts": [parts]}));
            }
        }
        for (key, value) in [
            ("container-title", r.pub_.as_str()),
            ("volume", r.detail("vol")),
            ("issue", r.detail("issue")),
            ("page", r.detail("pages_range")),
            ("DOI", r.detail("doi")),
            ("URL", r.detail("url")),
            ("language", r.lang.as_str()),
        ] {
            if !value.is_empty() {
                item.insert(key.into(), json!(value));
            }
        }
        let t = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if (t == "book" || t == "report") && item.contains_key("container-title") {
            let pub_ = item.remove("container-title").unwrap();
            item.insert("publisher".into(), pub_);
        }
        items.push(Value::Object(item));
    }
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()) + "\n"
}

pub fn highlights_json(project: &Project, refs: &[Reference]) -> Result<String> {
    let mut out = Vec::new();
    for h in project.all_highlights()? {
        let r = match refs.iter().find(|r| r.id == h.ref_id) {
            Some(r) => r,
            None => continue,
        };
        let file = r.files.iter().find(|f| f.id == h.file_id).map(|f| f.relpath.clone());
        out.push(json!({
            "ref_id": r.id, "title": r.title, "citation": apa(r), "file": file, "page": h.page, "kind": h.kind,
            "verbatim": h.verbatim, "analysis": h.analysis, "label": h.label, "tags": h.tags, "author": h.author,
            "origin": h.origin, "created_at": h.created_at, "updated_at": h.updated_at,
        }));
    }
    let doc = json!({"project": project.name(), "exported_at": now_iso(), "highlights": out});
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

/// The references to export, sorted by APA author string (or title).
pub fn select(project: &Project, ref_ids: Option<&[String]>) -> Result<Vec<Reference>> {
    let ids = ref_ids.filter(|ids| !ids.is_empty()).map(|ids| ids.to_vec());
    let mut refs = project.references(&ReferenceFilter { ids, detail: true, ..ReferenceFilter::default() })?;
    refs.sort_by_key(|r| {
        let a = apa_authors(&people(r));
        (if a.is_empty() { r.title.clone() } else { a }).to_lowercase()
    });
    Ok(refs)
}

pub fn file_name(format: &str) -> Result<&'static str> {
    FORMATS.iter().find(|(f, _)| *f == format).map(|(_, n)| *n).ok_or_else(|| {
        Error::bad(format!("format must be one of: {}.", FORMATS.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ")))
    })
}

pub fn render(project: &Project, format: &str, ref_ids: Option<&[String]>) -> Result<String> {
    file_name(format)?;
    let refs = select(project, ref_ids)?;
    Ok(match format {
        "apa-md" => {
            if refs.is_empty() {
                "# References\n".to_string()
            } else {
                let mut parts = vec!["# References".to_string()];
                parts.extend(refs.iter().map(apa));
                parts.join("\n\n") + "\n"
            }
        }
        "bibtex" => bibtex(&refs),
        "csl-json" => csl(&refs),
        _ => highlights_json(project, &refs)?,
    })
}

/// Writes the export and returns its path.
pub fn write(project: &Project, format: &str, ref_ids: Option<&[String]>) -> Result<PathBuf> {
    let text = render(project, format, ref_ids)?;
    let path = project.folder().join("_exports").join(file_name(format)?);
    atomic_write_text(&path, &text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(title: &str, authors: &[&str], year: &str, kind: &str, pub_: &str) -> Reference {
        Reference {
            id: "r-000000000001".into(),
            slug: "x".into(),
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            kind: kind.into(),
            pub_: pub_.into(),
            year: year.into(),
            date: String::new(),
            lang: String::new(),
            usability: String::new(),
            status: "Verified".into(),
            register_id: String::new(),
            categories: vec![],
            ext: "pdf".into(),
            pages: 0,
            hl_count: 0,
            top_tags: vec![],
            tags: vec![],
            missing: false,
            files: vec![],
            primary_file_id: None,
            thumb_url: None,
            created_at: None,
            updated_at: None,
            search_text: String::new(),
            doi: Some(String::new()),
            url: Some(String::new()),
            vol: Some(String::new()),
            issue: Some(String::new()),
            pages_range: Some(String::new()),
        }
    }

    #[test]
    fn apa_book_and_article() {
        let book = r("The wealth of nations", &["Smith, A."], "1776", "Book", "W. Strahan");
        assert_eq!(apa(&book), "Smith, A. (1776). *The wealth of nations*. W. Strahan.");
        let mut art = r("Die protestantische Ethik", &["Weber, M."], "1905", "Journal article", "Archiv");
        art.vol = Some("20-21".into());
        art.pages_range = Some("1-54".into());
        assert_eq!(apa(&art), "Weber, M. (1905). Die protestantische Ethik. *Archiv*, *20-21*, 1-54.");
        let anon = r("scan_0007", &["Unknown"], "", "", "");
        assert_eq!(apa(&anon), "(n.d.). *scan_0007*.");
        assert_eq!(apa_authors(&["A, B.".into(), "C, D.".into(), "E, F.".into()]), "A, B., C, D., & E, F.");
    }

    #[test]
    fn bibtex_keys_and_csl_names() {
        let a = r("The wealth of nations", &["Smith, A."], "1776", "Book", "W. Strahan");
        let b = r("The wealth of cities", &["Smith, A."], "1776", "Book", "");
        let text = bibtex(&[a.clone(), b]);
        assert!(text.contains("@book{smith1776wealth,"));
        assert!(text.contains("@book{smith1776wealtha,"));
        assert!(text.contains("  publisher = {W. Strahan}"));
        let csl_text = csl(&[a]);
        let v: Value = serde_json::from_str(&csl_text).unwrap();
        assert_eq!(v[0]["author"], json!([{"family": "Smith", "given": "A."}]));
        assert_eq!(v[0]["publisher"], json!("W. Strahan"));
        assert_eq!(v[0]["issued"], json!({"date-parts": [[1776]]}));
        assert!(file_name("docx").is_err());
    }
}
