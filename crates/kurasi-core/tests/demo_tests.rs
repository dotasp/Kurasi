//! The sample project: sterile content, expected shape, and reset.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::home;
use kurasi_core::demo::{self, DEMO_NAME, SAMPLE_LINE};
use kurasi_core::model::{HighlightFilter, Origin};
use kurasi_core::pdf::PdfDoc;
use kurasi_core::simplepdf::SimpleDoc;
use kurasi_core::Project;

/// Optional local list of words the sample must never contain: one entry per line, a line that
/// starts with `#` is a comment. The file is gitignored. Without it the word check is skipped.
fn forbidden_words() -> Option<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("forbidden_words.local.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("cannot read {}: {}", path.display(), e),
    };
    Some(
        text.trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect(),
    )
}

/// Every text the sample exposes: PDF text, annotations and metadata, notes, sidecar, database.
fn all_demo_text(project: &Project, folder: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for entry in walkdir(folder) {
        let rel = entry.strip_prefix(folder).unwrap().to_string_lossy().into_owned();
        parts.push(rel.clone());
        let name = rel.to_lowercase();
        if name.ends_with(".pdf") {
            let doc = SimpleDoc::open(&entry).unwrap();
            let meta = doc.metadata().unwrap();
            parts.push(meta.title);
            parts.push(meta.author);
            for page in 1..=doc.page_count() {
                let words: Vec<String> = doc.words(page).unwrap().into_iter().map(|w| w.text).collect();
                parts.push(words.join(" "));
            }
            for a in doc.annotations().unwrap() {
                parts.push(a.author);
                parts.push(a.contents);
            }
        } else if name.ends_with(".md") || name.ends_with(".json") {
            parts.push(std::fs::read_to_string(&entry).unwrap());
        }
    }
    project
        .db()
        .with(|c| {
            for table in ["setting", "category", "reference", "reference_author", "file", "highlight", "trash", "ai_log", "change"] {
                let mut stmt = c.prepare(&format!("SELECT * FROM {}", table))?;
                let n = stmt.column_count();
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    for i in 0..n {
                        match row.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => {}
                            rusqlite::types::ValueRef::Integer(v) => parts.push(v.to_string()),
                            rusqlite::types::ValueRef::Real(v) => parts.push(v.to_string()),
                            rusqlite::types::ValueRef::Text(t) => parts.push(String::from_utf8_lossy(t).into_owned()),
                            rusqlite::types::ValueRef::Blob(b) => parts.push(String::from_utf8_lossy(b).into_owned()),
                        }
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    parts.join("\n")
}

fn walkdir(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn demo_is_sterile_and_complete() {
    let h = home();
    let info = demo::build(&h.paths).unwrap();
    assert_eq!((info.id.as_str(), info.name.as_str(), info.demo), ("demo", DEMO_NAME, true));
    let folder = std::path::PathBuf::from(&info.folder);
    assert_eq!(folder, h.paths.demo_dir());
    let p = demo::open(&h.paths).unwrap();
    assert!(p.is_demo());
    assert_eq!(p.name(), DEMO_NAME);
    assert_eq!(p.folder(), folder.as_path());

    let text = all_demo_text(&p, &folder);
    match forbidden_words() {
        Some(words) => {
            for word in &words {
                assert!(!text.contains(word.as_str()), "{}", word);
            }
        }
        None => eprintln!("tests/forbidden_words.local.txt is absent, so the forbidden-word check is skipped"),
    }
    assert!(text.contains(SAMPLE_LINE));

    let refs = p.all_references().unwrap();
    assert_eq!(refs.len(), 15);
    let by_slug = |slug: &str| refs.iter().find(|r| r.slug == slug).unwrap_or_else(|| panic!("no {}", slug));
    let weber = by_slug("weber_1905_protestantische-ethik");
    let mut roles: Vec<&str> = weber.files.iter().map(|f| f.role.as_str()).collect();
    roles.sort();
    assert_eq!(roles, vec!["primary", "translation"]);
    assert_eq!(weber.title, "Die protestantische Ethik und der Geist des Kapitalismus");
    for slug in ["scan_0007", "2026-03-02_download"] {
        let r = by_slug(slug);
        assert_eq!((r.status.as_str(), r.categories.clone(), r.authors.clone(), r.title.as_str()), ("Auto", vec!["Uncategorized".to_string()], vec!["Unknown".to_string()], slug));
    }
    assert_eq!(by_slug("2026-03-02_download").date, "2026-03-02");
    assert_eq!(refs.iter().filter(|r| r.status == "Suggested").count(), 2);
    assert_eq!(refs.iter().filter(|r| r.status == "Auto").count(), 2);
    assert_eq!(by_slug("babbage_1832_economy-of-machinery").usability, "Excerpt");
    let cats: BTreeSet<&str> = refs.iter().flat_map(|r| r.categories.iter().map(String::as_str)).collect();
    let want: BTreeSet<&str> = ["Economics", "Management", "Management/Scientific management", "Management/Administration", "Sociology", "Biology", "Notes", "Exhibits", "Uncategorized"].into_iter().collect();
    assert_eq!(cats, want);

    let hls = p.all_highlights().unwrap();
    let pdf_refs: Vec<_> = refs.iter().filter(|r| r.ext == "pdf" && r.status != "Auto").collect();
    assert_eq!(pdf_refs.len(), 9);
    assert!(pdf_refs.iter().all(|r| (2..=4).contains(&r.hl_count)));
    let authors: BTreeSet<&str> = hls.iter().map(|h| h.author.as_str()).collect();
    assert_eq!(authors, ["Claude", "You"].into_iter().collect());
    let labels: BTreeSet<&str> = hls.iter().filter_map(|h| h.label.as_deref()).collect();
    assert_eq!(labels, ["F", "I", "P"].into_iter().collect());
    let with_suggestion: Vec<_> = hls.iter().filter(|h| h.suggestion.is_some()).collect();
    assert_eq!(with_suggestion.len(), 1);
    assert_eq!(with_suggestion[0].author, "Claude");
    assert_eq!(with_suggestion[0].suggestion.as_ref().unwrap()["action"], "delete");
    let tagged: Vec<_> = hls.iter().filter(|h| !h.suggested_tags.is_empty()).collect();
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].author, "Claude");
    let mut md: Vec<&str> = hls.iter().filter(|h| h.kind == "md").map(|h| h.author.as_str()).collect();
    md.sort();
    assert_eq!(md, vec!["Claude", "You"]);
    assert_eq!(hls.iter().filter(|h| h.kind == "region").count(), 1);
    assert_eq!(by_slug("exhibit-1_division-of-labour").hl_count, 1);
    assert!(hls.iter().any(|h| h.analysis.contains("](management/babbage")));
    let tags: BTreeSet<String> = p.tags().unwrap().into_iter().map(|t| t.name).collect();
    let want: BTreeSet<String> = ["division-of-labour", "efficiency", "incentives", "method", "institutions", "evolution", "critique", "to-read", "quote"].into_iter().map(String::from).collect();
    assert_eq!(tags, want);
    // Every PDF highlight is a real annotation in its file.
    assert_eq!(hls.iter().filter(|h| h.kind == "text").count(), 24);
    let taylor = SimpleDoc::open(&folder.join("management").join("taylor_1911_principles-of-scientific-management.pdf")).unwrap();
    assert_eq!(taylor.annotations().unwrap().len(), 4);
    // Note links resolve to sample files.
    let note = by_slug("reading-plan");
    let res = p.resolve("../economics/smith_1776_wealth-of-nations.pdf", note.primary_file_id.as_deref()).unwrap();
    assert_eq!(res["ref_id"], serde_json::json!(by_slug("smith_1776_wealth-of-nations").id));
    let sidecar: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(folder.join(".kurasi").join("metadata.json")).unwrap()).unwrap();
    assert_eq!(sidecar["references"].as_object().unwrap().len(), 15);
    assert!(!p.ai_log().unwrap().is_empty());
    assert_eq!(p.changes_since(0).unwrap().changes.len(), 1);

    // Reset rebuilds from scratch: a change made in the sample is gone afterwards.
    let hid = hls[0].id.clone();
    p.delete_highlight(&hid, Origin::Ui).unwrap();
    assert_eq!(p.all_highlights().unwrap().len(), 26);
    drop(p);
    demo::build(&h.paths).unwrap();
    let p = demo::open(&h.paths).unwrap();
    let again = p.highlights(&HighlightFilter::default()).unwrap();
    assert_eq!(again.len(), 27);
    assert!(again.iter().all(|x| x.id != hid));
    let entries = h.paths.projects().unwrap();
    assert_eq!(entries.iter().filter(|e| e.id == "demo").count(), 1);
    assert!(entries.iter().find(|e| e.id == "demo").unwrap().demo);
}

#[test]
fn page_sentences_are_deterministic() {
    let a = demo::page_sentences("seed", 2, 5);
    let b = demo::page_sentences("seed", 2, 5);
    assert_eq!(a, b);
    assert_eq!(a.len(), 5);
    assert_ne!(a, demo::page_sentences("other", 2, 5));
}
