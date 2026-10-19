//! The core trait over pdfium: 1-based pages, hashed annotation ids, snapshots and undo,
//! against a PDF written with lopdf. PyMuPDF cross-checks run when the legacy venv exists.

mod common;

use std::collections::BTreeMap;

use common::{close, fitz_dump, make_pdf, PAGE_TEXT};
use kurasi_core::pdf::{find_quote, inspect, words_under, AnnotPatch, NewAnnot, PdfBackend, PdfDoc};
use kurasi_pdf::backend::{id_of_name, NAME_ID_BIT};
use kurasi_pdf::PdfiumBackend;

fn open(path: &std::path::Path) -> Box<dyn PdfDoc> {
    PdfiumBackend.open(path).expect("pdfium opens the file")
}

#[test]
fn pages_are_counted_from_one_and_words_carry_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("p.pdf"), &PAGE_TEXT);
    let doc = open(&path);
    assert_eq!(doc.page_count(), 3);
    let pages = doc.pages().unwrap();
    assert_eq!(pages.len(), 3);
    assert!((pages[0].width - 595.0).abs() < 0.1 && (pages[0].height - 842.0).abs() < 0.1 && pages[0].rotation == 0);
    let words = doc.words(1).unwrap();
    assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "), PAGE_TEXT[0]);
    assert!(words.iter().all(|w| w.line == 0));
    assert_eq!(doc.words(3).unwrap()[0].text, "Measured");
    assert!(doc.words(0).is_err());
    assert!(doc.words(4).is_err());
    let meta = doc.metadata().unwrap();
    assert_eq!((meta.title.as_str(), meta.author.as_str()), ("", ""));
    let png = doc.render_png(1, 1.0).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    assert!((594..=596).contains(&w), "{w}");
    assert_eq!(doc.thumbnail_png(120).unwrap().len() > 100, true);
}

#[test]
fn highlight_life_cycle_through_the_core_trait() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("h.pdf"), &PAGE_TEXT);
    let mut doc = open(&path);
    let words = doc.words(1).unwrap();
    let (quads, verbatim) = find_quote(&words, "division of labour").unwrap();
    assert_eq!(verbatim, "division of labour");

    let added = doc
        .add_highlight(&NewAnnot {
            page: 1,
            quads: quads.clone(),
            author: "Claude".into(),
            contents: "(I) Why.\n\nTags: #quote".into(),
            colour: "#ffd84a".into(),
            kind: "text".into(),
            created: None,
        })
        .unwrap();
    assert!(added.xref >= NAME_ID_BIT, "{}", added.xref);
    assert_eq!(added.page, 1);
    assert_eq!(added.quads, quads);
    doc.save_in_place().unwrap();

    // A fresh open reads the same id from the /NM name.
    let again = open(&path);
    let annots = again.annotations().unwrap();
    assert_eq!(annots.len(), 1);
    let a = &annots[0];
    assert_eq!(a.xref, added.xref);
    assert_eq!((a.page, a.subtype.as_str(), a.author.as_str()), (1, "Highlight", "Claude"));
    assert_eq!(a.contents, "(I) Why.\n\nTags: #quote");
    assert_eq!(a.colour.as_deref(), Some("#ffd84a"));
    for (q, e) in a.quads[0].iter().zip(&quads[0]) {
        assert!((q - e).abs() < 0.06, "{:?} vs {:?}", a.quads, quads);
    }
    let insp = inspect(again.as_ref(), &["claude".into()]).unwrap();
    assert_eq!(insp.page_count, 3);
    assert_eq!(insp.annotations.len(), 1);
    let rec = &insp.annotations[0];
    assert_eq!((rec.pdf_xref, rec.page, rec.kind.as_str(), rec.origin.as_str()), (added.xref, 1, "text", "ai"));
    assert_eq!(rec.verbatim, "division of labour");
    assert_eq!(rec.analysis, "(I) Why.");
    assert_eq!(rec.tags, vec!["quote"]);
    assert_eq!(rec.label.as_deref(), Some("I"));
    assert_eq!(words_under(&again.words(1).unwrap(), &a.quads), "division of labour");

    // Edit by id in a new session.
    let mut editor = open(&path);
    editor
        .update_highlight(added.xref, &AnnotPatch { contents: Some("(F) Edited.".into()), author: Some("You".into()), colour: Some("#3366ee".into()) })
        .unwrap();
    editor.save_in_place().unwrap();
    let a = &open(&path).annotations().unwrap()[0];
    assert_eq!((a.author.as_str(), a.contents.as_str(), a.colour.as_deref()), ("You", "(F) Edited.", Some("#3366ee")));
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.annots.len(), 1);
        let f = &dump.annots[0];
        assert_eq!((f.page, f.author.as_str(), f.contents.as_str()), (0, "You", "(F) Edited."));
        assert!(close(f.colour[0], 0.2, 0.01) && close(f.colour[1], 0.4, 0.01) && close(f.colour[2], 0.933, 0.01), "{:?}", f.colour);
        assert_eq!(id_of_name(&f.name), added.xref);
    }

    // Delete returns a snapshot; the snapshot restores the annotation under the same id.
    let mut deleter = open(&path);
    let snaps = deleter.delete_highlight(added.xref).unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!((snaps[0].xref, snaps[0].page, snaps[0].subtype.as_str()), (added.xref, 1, "Highlight"));
    assert_eq!(snaps[0].contents, "(F) Edited.");
    deleter.save_in_place().unwrap();
    assert!(open(&path).annotations().unwrap().is_empty());
    assert!(open(&path).update_highlight(added.xref, &AnnotPatch::default()).is_err());

    let mut restorer = open(&path);
    let mapping: BTreeMap<i64, i64> = restorer.restore_annotations(&snaps).unwrap();
    assert_eq!(mapping.get(&added.xref), Some(&added.xref));
    restorer.save_in_place().unwrap();
    let back = open(&path).annotations().unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].xref, back[0].author.as_str(), back[0].contents.as_str()), (added.xref, "You", "(F) Edited."));
    assert_eq!(back[0].colour.as_deref(), Some("#3366ee"));
    for (q, e) in back[0].quads[0].iter().zip(&quads[0]) {
        assert!((q - e).abs() < 0.06, "{:?} vs {:?}", back[0].quads, quads);
    }
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.annots.len(), 1);
        assert_eq!((dump.annots[0].kind.as_str(), dump.annots[0].author.as_str()), ("Highlight", "You"));
        assert_eq!(dump.annots[0].quads.len(), 1);
        for i in 0..4 {
            assert!(close(dump.annots[0].quads[0][i], quads[0][i] as f32, 0.06), "{:?} vs {:?}", dump.annots[0].quads, quads);
        }
    }
    // The restored annotation can be edited again by the same id.
    let mut later = open(&path);
    later.update_highlight(added.xref, &AnnotPatch { contents: Some("(P) Again.".into()), ..Default::default() }).unwrap();
    later.save_in_place().unwrap();
    assert_eq!(open(&path).annotations().unwrap()[0].contents, "(P) Again.");
}

#[test]
fn notes_and_object_number_lookups() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("n.pdf"), &PAGE_TEXT);
    let mut doc = open(&path);
    let note = doc
        .add_highlight(&NewAnnot {
            page: 2,
            quads: vec![vec![18.0, 18.0, 38.0, 38.0]],
            author: "You".into(),
            contents: "A note.".into(),
            colour: "#ffd84a".into(),
            kind: "note".into(),
            created: Some("2026-03-01T12:00:00".into()),
        })
        .unwrap();
    assert_eq!((note.page, note.subtype.as_str()), (2, "Text"));
    doc.save_in_place().unwrap();
    let a = open(&path).annotations().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!((a[0].xref, a[0].page, a[0].subtype.as_str(), a[0].contents.as_str()), (note.xref, 2, "Text", "A note."));
    assert_eq!(a[0].created.as_deref(), Some("2026-03-01T12:00:00"));
    let insp = inspect(open(&path).as_ref(), &[]).unwrap();
    assert_eq!(insp.annotations[0].kind, "note");

    // The Python build stored object numbers: the same annotation answers to its object number.
    let raw = kurasi_pdf::open(&path).unwrap().annotations().unwrap();
    let object = raw[0].object.expect("indirect annotation") as i64;
    let mut by_object = open(&path);
    by_object.update_highlight(object, &AnnotPatch { contents: Some("By object number.".into()), ..Default::default() }).unwrap();
    by_object.save_in_place().unwrap();
    assert_eq!(open(&path).annotations().unwrap()[0].contents, "By object number.");
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!((dump.annots[0].kind.as_str(), dump.annots[0].page), ("Text", 1));
    }
}

#[test]
fn rotate_by_page_number_reports_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("r.pdf"), &PAGE_TEXT);
    let mut doc = open(&path);
    assert_eq!(doc.rotate(Some(2), 90).unwrap(), vec![0, 90, 0]);
    doc.save_in_place().unwrap();
    let mut again = open(&path);
    assert_eq!(again.rotate(None, 90).unwrap(), vec![90, 180, 90]);
    again.save_in_place().unwrap();
    let rots: Vec<i32> = open(&path).pages().unwrap().iter().map(|p| p.rotation).collect();
    assert_eq!(rots, vec![90, 180, 90]);
    assert!(open(&path).rotate(Some(0), 90).is_err());
    assert!(open(&path).rotate(Some(1), 45).is_err());
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.rotation, vec![90, 180, 90]);
    }
}
