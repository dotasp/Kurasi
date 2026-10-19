mod common;

use std::fs::Metadata;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use kurasi_core::model::{HighlightFilter, HighlightPatch, NewHighlight, Origin, ReferencePatch, Writer};
use kurasi_core::pdf::{NewAnnot, PdfDoc};
use kurasi_core::scanner::ScanMode;
use kurasi_core::simplepdf::{SimpleDoc, SimplePdfBackend};
use kurasi_core::{sidecar, Project};
use serde_json::json;

#[test]
fn first_scan_groups_and_reads_annotations() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let refs = p.all_references().unwrap();
    let mut slugs: Vec<&str> = refs.iter().map(|r| r.slug.as_str()).collect();
    slugs.sort();
    assert_eq!(slugs, vec!["2026-03-02_news-item", "a/_NOTES", "b/_NOTES", "babbage_1832_machinery", "chart", "reading", "smith_1776_wealth"]);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let mut roles: Vec<&str> = smith.files.iter().map(|f| f.role.as_str()).collect();
    roles.sort();
    assert_eq!(roles, vec!["primary", "print", "translation"]);
    assert_eq!(smith.title, "The wealth of nations");
    assert_eq!(smith.authors, vec!["Smith, A."]);
    assert_eq!(smith.status, "Auto");
    assert_eq!(smith.categories, vec!["Uncategorized"]);
    assert_eq!(smith.year, "1776");
    assert_eq!(smith.hl_count, 3);
    // The translation (en) holds the most highlights, so it opens first.
    let primary = smith.files.iter().find(|f| Some(&f.id) == smith.primary_file_id.as_ref()).unwrap();
    assert_eq!(primary.role, "translation");
    assert_eq!(smith.thumb_url.as_deref(), Some(format!("/api/thumb/{}.png", primary.id).as_str()));
    let mut top = smith.top_tags.clone();
    top.sort();
    assert_eq!(top, vec!["division-of-labour", "efficiency", "quote"]);
    let babbage = ref_by_slug(&p, "babbage_1832_machinery");
    assert_eq!(babbage.files[0].role, "excerpt");
    assert_eq!(babbage.authors, vec!["Babbage"]);
    let news = ref_by_slug(&p, "2026-03-02_news-item");
    assert_eq!(news.date, "2026-03-02");
    assert_eq!(news.files[0].role, "rendered");
    let reading = ref_by_slug(&p, "reading");
    assert_eq!(reading.ext, "md");
    assert!(reading.thumb_url.is_none());
    let hls = p.highlights(&HighlightFilter { ref_id: Some(smith.id.clone()), ..Default::default() }).unwrap();
    let mut authors: Vec<&str> = hls.iter().map(|h| h.author.as_str()).collect();
    authors.sort();
    authors.dedup();
    assert_eq!(authors, vec!["Claude", "You"]);
    assert!(hls.iter().filter(|h| h.author == "Claude").all(|h| h.origin == "ai"));
    let fact = hls.iter().find(|h| h.label.as_deref() == Some("F")).unwrap();
    assert_eq!(fact.verbatim, "division of labour");
    assert_eq!(fact.tags, vec!["division-of-labour", "quote"]);
    // Excluded folders, lock files and other types stay out.
    for r in &refs {
        for f in &r.files {
            assert!(!f.relpath.contains(".git") && !f.relpath.contains("~$") && !f.relpath.ends_with(".txt"), "{}", f.relpath);
        }
    }
}

#[test]
fn incremental_scan_skips_unchanged() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let before: Vec<Option<String>> = p.db().with(|c| {
        let mut stmt = c.prepare("SELECT last_seen FROM file ORDER BY relpath")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }).unwrap();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!((res.new, res.changed, res.moved, res.missing), (0, 0, 0, 0));
    assert_eq!(res.unchanged, 9);
    assert_eq!(res.total, 9);
    assert_eq!(before.len(), 9);
    // Hashes stay untouched: the sha of every file is the same object the first scan wrote.
    let full = p.scan(ScanMode::Full, |_| {}).unwrap();
    assert_eq!(full.unchanged, 0);
    assert_eq!(full.changed, 0);
    assert!(full.full);
}

#[test]
fn changed_file_rereads_annotations_without_duplicates() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let fid = smith.files.iter().find(|f| f.role == "primary").unwrap().id.clone();
    // An app edit first: its PDF write updates the stored hash, so no outside change is seen.
    let hl = p.highlights(&HighlightFilter { file_id: Some(fid.clone()), ..Default::default() }).unwrap().remove(0);
    p.update_highlight(
        &hl.id,
        &HighlightPatch { analysis: Some("(I) Edited in the app.".into()), tags: Some(vec!["quote".into()]), ..Default::default() },
        Origin::Ui,
    )
    .unwrap();
    assert_eq!(p.scan(ScanMode::Incremental, |_| {}).unwrap().changed, 0);
    // Now another program adds an annotation.
    let path = folder.join("lib").join("smith_1776_wealth.pdf");
    let mut doc = SimpleDoc::open(&path).unwrap();
    let (quads, _) = doc.find_quote(2, "Second paragraph").unwrap().unwrap();
    doc.add_highlight(&NewAnnot {
        page: 2,
        quads,
        author: "Reviewer".into(),
        contents: "(P) From Acrobat.".into(),
        colour: "#ffd84a".into(),
        kind: "text".into(),
        created: None,
    })
    .unwrap();
    doc.save_in_place().unwrap();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.changed, 1);
    assert_eq!(res.highlights_added, 1);
    let hls = p.highlights(&HighlightFilter { file_id: Some(fid), ..Default::default() }).unwrap();
    assert_eq!(hls.len(), 2);
    let edited = hls.iter().find(|x| x.id == hl.id).unwrap();
    assert_eq!(edited.analysis, "(I) Edited in the app.");
    assert_eq!(edited.tags, vec!["quote"]);
    assert!(hls.iter().any(|x| x.author == "Reviewer" && x.label.as_deref() == Some("P")));
}

#[test]
fn moved_file_keeps_row_and_highlights() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let f = smith.files.iter().find(|f| f.role == "primary").unwrap().clone();
    let before: Vec<String> = p.highlights(&HighlightFilter { file_id: Some(f.id.clone()), ..Default::default() }).unwrap().iter().map(|h| h.id.clone()).collect();
    std::fs::create_dir_all(folder.join("moved")).unwrap();
    std::fs::rename(folder.join("lib/smith_1776_wealth.pdf"), folder.join("moved/renamed-copy.pdf")).unwrap();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!((res.moved, res.missing, res.new), (1, 1, 0));
    let row = p.file_row(&f.id).unwrap();
    assert_eq!(row.relpath, "moved\\renamed-copy.pdf");
    assert!(!row.missing);
    assert_eq!(row.ref_id, smith.id);
    let after: Vec<String> = p.highlights(&HighlightFilter { file_id: Some(f.id.clone()), ..Default::default() }).unwrap().iter().map(|h| h.id.clone()).collect();
    assert_eq!(after, before);
}

#[test]
fn removed_file_is_marked_missing_and_restored() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let chart = ref_by_slug(&p, "chart");
    std::fs::remove_file(folder.join("img/chart.png")).unwrap();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.missing, 1);
    let row = p.file_row(&chart.files[0].id).unwrap();
    assert!(row.missing && row.missing_since.is_some());
    let again = ref_by_slug(&p, "chart");
    assert!(again.missing);
    assert!(again.thumb_url.is_none());
    make_png(&folder.join("img/chart.png"), (300, 200), [40, 90, 160]);
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.restored, 1);
    assert!(!p.file_row(&chart.files[0].id).unwrap().missing);
}

#[test]
fn cloud_only_is_never_opened() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = open_project(&h.paths, &folder, "Test project");
    let target = folder.join("lib").join("babbage_1832_machinery_excerpt-pp1-12_highlighted.pdf");
    // The simulated attribute: this one file reports FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.
    let target_key = target.to_string_lossy().to_lowercase();
    p.set_cloud_check(Some(Box::new(move |path: &Path, _meta: &Metadata| path.to_string_lossy().to_lowercase() == target_key)));
    let size_before = std::fs::metadata(&target).unwrap().len();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.cloud_only, 1);
    let r = ref_by_slug(&p, "babbage_1832_machinery");
    let f = &r.files[0];
    assert!(f.cloud_only);
    assert!(r.thumb_url.is_none());
    let row = p.file_row(&f.id).unwrap();
    assert!(row.sha256.is_none(), "a cloud-only file is never hashed");
    assert_eq!(row.size as u64, size_before);
    assert!(row.page_count.is_none(), "a cloud-only PDF is never opened");
    assert!(p.thumb(&f.id).is_err());
    // Once the file is local again the next scan hashes and reads it.
    p.set_cloud_check(None);
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.changed, 1);
    let row = p.file_row(&f.id).unwrap();
    assert!(row.sha256.is_some());
    assert!(!row.cloud_only);
    assert_eq!(row.page_count, Some(3));
}

#[test]
fn partial_scan_of_touched_paths() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    make_md(&folder.join("notes/new-note.md"), "# New\n");
    std::fs::remove_file(folder.join("img/chart.png")).unwrap();
    let res = p.scan(ScanMode::Paths(vec!["notes\\new-note.md".into(), "img\\chart.png".into()]), |_| {}).unwrap();
    assert!(res.partial);
    assert_eq!((res.new, res.missing, res.total), (1, 1, 1));
    ref_by_slug(&p, "new-note");
}

#[test]
fn watcher_picks_up_new_file() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let watcher = kurasi_core::scanner::start_watcher(&p, Duration::from_millis(300)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    make_pdf(&folder.join("lib/watched_2001_file.pdf"), None, "", "", &[]);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut found = false;
    while Instant::now() < deadline {
        if p.all_references().unwrap().iter().any(|r| r.slug == "watched_2001_file") {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    watcher.stop();
    assert!(found, "the watcher did not scan the new file");
    let lines = Arc::new(AtomicUsize::new(0));
    let l2 = lines.clone();
    let res = p.scan(ScanMode::Full, move |pr| {
        if pr.line {
            l2.fetch_add(1, Ordering::SeqCst);
        }
    })
    .unwrap();
    assert!(res.full);
    assert!(lines.load(Ordering::SeqCst) >= 4);
}

#[test]
fn sidecar_written_and_imported() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let chart = ref_by_slug(&p, "chart");
    let note = ref_by_slug(&p, "reading");
    let mut fields = serde_json::Map::new();
    fields.insert("title".into(), json!("Chart of output"));
    p.update_reference(
        &chart.id,
        &ReferencePatch { fields, authors: Some(vec!["Doe, J.".into()]), categories: Some(vec!["Exhibits/Figures".into()]) },
        Origin::Ui,
        "Verified",
    )
    .unwrap();
    let fid = note.files[0].id.clone();
    p.add_highlight(
        &NewHighlight {
            file_id: fid,
            kind: Some("md".into()),
            anchor: Some(json!({"text": "plain sentence", "occurrence": 0})),
            analysis: Some("(F) Note.".into()),
            tags: Some(vec!["quote".into()]),
            ..Default::default()
        },
        &Writer::human(),
    )
    .unwrap();
    let path = sidecar::sidecar_path(&p);
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(data["references"]["chart"]["title"], json!("Chart of output"));
    assert!(data["references"]["chart"]["files"][0]["sha256"].is_string());
    assert!(data["highlights"].as_array().unwrap().iter().any(|h| h["anchor"]["text"] == json!("plain sentence")));
    // Another machine: a fresh database on the same folder rebuilds from the sidecar plus a scan.
    let entry = kurasi_core::ProjectInfo {
        id: "p-othermachine".into(),
        name: "Other".into(),
        folder: folder.to_string_lossy().into_owned(),
        created: None,
        last_opened: None,
        demo: false,
        read_only: None,
    };
    let other = Project::open_entry(&h.paths, &entry, None, None, Arc::new(SimplePdfBackend)).unwrap();
    let r = ref_by_slug(&other, "chart");
    assert_eq!(r.title, "Chart of output");
    assert_eq!(r.categories, vec!["Exhibits/Figures"]);
    assert_eq!(r.status, "Verified");
    other.scan(ScanMode::Incremental, |_| {}).unwrap();
    let smith = ref_by_slug(&other, "smith_1776_wealth");
    assert_eq!(smith.hl_count, 3);
    let md = other.highlights(&HighlightFilter { ref_id: Some(note.id.clone()), ..Default::default() }).unwrap();
    assert!(!md.is_empty());
    assert_eq!(md[0].anchor, Some(json!({"text": "plain sentence", "occurrence": 0})));
}

#[test]
fn read_only_project_writes_no_files() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = open_project(&h.paths, &folder, "Test project");
    p.set_read_only(true).unwrap();
    p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert!(!folder.join(".kurasi").exists());
    assert!(matches!(p.export_references("apa-md", None), Err(kurasi_core::Error::ReadOnly)));
    assert!(matches!(p.export_ai_log(), Err(kurasi_core::Error::ReadOnly)));
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let fid = smith.files.iter().find(|f| f.role == "primary").unwrap().id.clone();
    let err = p
        .add_highlight(
            &NewHighlight { file_id: fid, page: Some(1), quads: Some(vec![vec![72.0, 72.0, 200.0, 90.0]]), analysis: Some("x".into()), ..Default::default() },
            &Writer::human(),
        )
        .unwrap_err();
    assert_eq!(err.status(), 423);
}

#[test]
fn no_pdf_backend_still_catalogues_files() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = Project::create(&h.paths, "NoPdf", &folder).unwrap();
    let res = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(res.new, 9);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    assert_eq!(smith.hl_count, 0, "annotations stay unread without a backend");
    assert_eq!(smith.files.len(), 3);
    let again = p.scan(ScanMode::Incremental, |_| {}).unwrap();
    assert_eq!(again.new, 0);
    assert_eq!(again.total, 9);
}
