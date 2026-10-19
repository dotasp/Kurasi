//! Opens the database the Python build made (`python -m kurasi demo` at tag python-v0.1) and
//! checks that the Rust build reads the same catalogue from it unchanged.

mod common;

use common::home;
use kurasi_core::model::HighlightFilter;
use kurasi_core::{Project, ProjectInfo};

const FIXTURE: &[u8] = include_bytes!("fixtures/python_demo.sqlite");

#[test]
fn python_made_database_opens_with_the_same_counts() {
    let h = home();
    let folder = h.dir.path().join("demo-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let dir = h.paths.project_dir("demo");
    std::fs::write(dir.join("kurasi.sqlite"), FIXTURE).unwrap();
    let entry = ProjectInfo {
        id: "demo".into(),
        name: "Sample".into(),
        folder: folder.to_string_lossy().into_owned(),
        created: None,
        last_opened: None,
        demo: true,
        read_only: None,
    };
    let p = Project::open_entry(&h.paths, &entry, None, None, std::sync::Arc::new(kurasi_core::NoPdf)).unwrap();
    let (refs, hls, files, version) = p
        .db()
        .with(|c| {
            Ok((
                kurasi_core::db::count(c, "SELECT COUNT(*) FROM reference"),
                kurasi_core::db::count(c, "SELECT COUNT(*) FROM highlight"),
                kurasi_core::db::count(c, "SELECT COUNT(*) FROM file"),
                kurasi_core::db::count(c, "PRAGMA user_version"),
            ))
        })
        .unwrap();
    assert_eq!((refs, hls, files, version), (15, 27, 16, 1));
    let records = p.all_references().unwrap();
    assert_eq!(records.len(), 15);
    assert_eq!(p.all_highlights().unwrap().len(), 27);
    assert_eq!(p.counts().unwrap().references, 15);
    assert_eq!(p.counts().unwrap().highlights, 27);
    let weber = records.iter().find(|r| r.slug == "weber_1905_protestantische-ethik").unwrap();
    assert_eq!(weber.files.len(), 2);
    assert_eq!(weber.title, "Die protestantische Ethik und der Geist des Kapitalismus");
    assert_eq!(p.name(), "Sample: Classics of work and organisation");
    let text = p.highlights(&HighlightFilter::default()).unwrap().iter().filter(|h| h.kind == "text").count();
    assert_eq!(text, 24);
    let mut cats: Vec<String> = records.iter().flat_map(|r| r.categories.clone()).collect();
    cats.sort();
    cats.dedup();
    assert!(cats.contains(&"Management/Scientific management".to_string()));
    assert_eq!(p.tags().unwrap().len(), 9);
    assert!(!p.ai_log().unwrap().is_empty());
    // Opening the Python database left the schema and the rows untouched.
    let after = p.db().with(|c| Ok(kurasi_core::db::count(c, "SELECT COUNT(*) FROM change"))).unwrap();
    assert_eq!(after, 1);
}
