mod common;

use std::fs::OpenOptions;
use std::time::Instant;

use common::{close, fitz_dump, make_pdf, PAGE_TEXT};
use kurasi_pdf::{open, AnnotKind, Error, HighlightPatch, NewHighlight, REPLACE_DELAY, REPLACE_TRIES};

const YELLOW: [f32; 3] = [1.0, 0.847, 0.29];

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (w, h)
}

fn no_temp_left(dir: &std::path::Path) {
    let left: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".kurasi-tmp"))
        .collect();
    assert!(left.is_empty(), "temp files left: {left:?}");
}

#[test]
fn open_count_render_and_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("a.pdf"), &PAGE_TEXT);
    let doc = open(&path).unwrap();
    assert_eq!(doc.page_count(), 3);
    let info = doc.page_info(0).unwrap();
    assert!(close(info.width, 595.0, 0.1) && close(info.height, 842.0, 0.1) && info.rotation == 0);

    let png = doc.render_png(0, 72).unwrap();
    let (w, h) = png_size(&png);
    assert!((594..=596).contains(&w) && (841..=843).contains(&h), "{w}x{h}");
    let png = doc.render_png(1, 144).unwrap();
    let (w, _) = png_size(&png);
    assert!((1189..=1191).contains(&w), "{w}");

    let thumb = doc.thumbnail_png(120).unwrap();
    assert_eq!(png_size(&thumb), (120, 156));

    assert!(matches!(doc.render_png(3, 72), Err(Error::PageOutOfRange { page: 3, count: 3 })));
}

#[test]
fn words_have_text_and_boxes_in_top_left_points() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("w.pdf"), &PAGE_TEXT);
    let doc = open(&path).unwrap();
    let words = doc.words(0).unwrap();
    let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts.join(" "), PAGE_TEXT[0]);
    let first = &words[0];
    assert!(close(first.x0, 72.0, 1.0), "x0 {}", first.x0);
    // Baseline at 770 from the bottom of an 842 pt page: the box sits around y = 72.
    assert!(first.y0 > 58.0 && first.y1 < 80.0 && first.y1 > first.y0, "{first:?}");
    assert!(first.x1 > first.x0 + 10.0);
    // Words of a later page differ.
    assert_eq!(doc.words(2).unwrap()[0].text, "Measured");

    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.words.len(), words.len());
        for (mine, theirs) in words.iter().zip(dump.words.iter()) {
            assert_eq!(mine.text, theirs.4);
            assert!(close(mine.x0, theirs.0, 1.5), "{mine:?} vs {theirs:?}");
            assert!(close(mine.x1, theirs.2, 1.5), "{mine:?} vs {theirs:?}");
            // Vertical extents differ by font metrics; the boxes must overlap by half their height.
            let overlap = mine.y1.min(theirs.3) - mine.y0.max(theirs.1);
            assert!(overlap >= 0.5 * (theirs.3 - theirs.1), "{mine:?} vs {theirs:?}");
        }
    }
}

#[test]
fn add_update_delete_highlight_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("h.pdf"), &PAGE_TEXT);
    let mut doc = open(&path).unwrap();
    assert!(doc.annotations().unwrap().is_empty());

    let words = doc.words(0).unwrap();
    // "division of labour": words 1..=3.
    let quad = [words[1].x0, words[1].y0.min(words[3].y0), words[3].x1, words[1].y1.max(words[3].y1)];
    let name = doc
        .add_highlight(&NewHighlight {
            page: 0,
            quads: vec![quad],
            colour: YELLOW,
            author: "Claude".into(),
            contents: "(I) Why.\n\nTags: #quote".into(),
        })
        .unwrap();
    assert_eq!(name.len(), 36);
    doc.save_in_place().unwrap();
    no_temp_left(dir.path());

    let annots = doc.annotations().unwrap();
    assert_eq!(annots.len(), 1);
    let a = &annots[0];
    assert_eq!(a.name.as_deref(), Some(name.as_str()));
    assert_eq!((a.page, a.index, a.kind), (0, 0, AnnotKind::Highlight));
    assert_eq!(a.author, "Claude");
    assert_eq!(a.contents, "(I) Why.\n\nTags: #quote");
    let c = a.colour.unwrap();
    assert!(close(c[0], 1.0, 0.01) && close(c[1], 0.847, 0.01) && close(c[2], 0.29, 0.01), "{c:?}");
    assert_eq!(a.quads.len(), 1);
    for i in 0..4 {
        assert!(close(a.quads[0][i], quad[i], 0.05), "{:?} vs {quad:?}", a.quads[0]);
    }
    assert!(a.modified.is_some());

    // A fresh open sees the same thing.
    let mut again = open(&path).unwrap();
    assert_eq!(again.annotations().unwrap(), annots);

    // Edit contents, author and colour.
    doc.update_highlight(
        &name,
        &HighlightPatch { contents: Some("(F) Edited.\n\nTags: #x".into()), colour: Some([0.2, 0.4, 0.9]), author: Some("You".into()) },
    )
    .unwrap();
    doc.save_in_place().unwrap();
    let a = &doc.annotations().unwrap()[0];
    assert_eq!((a.author.as_str(), a.contents.as_str()), ("You", "(F) Edited.\n\nTags: #x"));
    let c = a.colour.unwrap();
    assert!(close(c[0], 0.2, 0.01) && close(c[1], 0.4, 0.01) && close(c[2], 0.9, 0.01), "{c:?}");

    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.annots.len(), 1);
        let f = &dump.annots[0];
        assert_eq!((f.page, f.kind.as_str(), f.name.as_str(), f.author.as_str()), (0, "Highlight", name.as_str(), "You"));
        assert_eq!(f.contents, "(F) Edited.\n\nTags: #x");
        assert!(close(f.colour[0], 0.2, 0.01) && close(f.colour[1], 0.4, 0.01) && close(f.colour[2], 0.9, 0.01), "{:?}", f.colour);
        assert_eq!(f.quads.len(), 1);
        for i in 0..4 {
            assert!(close(f.quads[0][i], quad[i], 0.05), "{:?} vs {quad:?}", f.quads[0]);
        }
    }

    // Delete.
    doc.delete_highlight(&name).unwrap();
    doc.save_in_place().unwrap();
    assert!(doc.annotations().unwrap().is_empty());
    assert!(matches!(doc.delete_highlight(&name), Err(Error::AnnotationNotFound(_))));
    if let Some(dump) = fitz_dump(&path) {
        assert!(dump.annots.is_empty());
    }
    no_temp_left(dir.path());
}

#[test]
fn several_changes_before_one_save_keep_their_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("m.pdf"), &PAGE_TEXT);
    let mut doc = open(&path).unwrap();
    let mk = |page: usize, y: f32, who: &str| NewHighlight {
        page,
        quads: vec![[72.0, y, 200.0, y + 12.0]],
        colour: YELLOW,
        author: who.into(),
        contents: format!("note by {who}"),
    };
    let n1 = doc.add_highlight(&mk(0, 60.0, "one")).unwrap();
    let n2 = doc.add_highlight(&mk(0, 100.0, "two")).unwrap();
    let n3 = doc.add_highlight(&mk(2, 60.0, "three")).unwrap();
    // Delete the first before saving: the second moves to index 0 and keeps its name.
    doc.delete_highlight(&n1).unwrap();
    doc.update_highlight(&n2, &HighlightPatch { colour: Some([0.0, 1.0, 0.0]), ..Default::default() }).unwrap();
    doc.save_in_place().unwrap();

    let annots = doc.annotations().unwrap();
    assert_eq!(annots.len(), 2);
    let a2 = annots.iter().find(|a| a.name.as_deref() == Some(n2.as_str())).unwrap();
    assert_eq!((a2.page, a2.index, a2.author.as_str()), (0, 0, "two"));
    assert!(close(a2.colour.unwrap()[1], 1.0, 0.01) && close(a2.colour.unwrap()[0], 0.0, 0.01));
    let a3 = annots.iter().find(|a| a.name.as_deref() == Some(n3.as_str())).unwrap();
    assert_eq!((a3.page, a3.index, a3.author.as_str()), (2, 0, "three"));
    assert!(annots.iter().all(|a| a.name.as_deref() != Some(n1.as_str())));

    // A second session edits an annotation it did not create: the colour goes through the
    // lopdf path and the other fields through pdfium.
    let mut later = open(&path).unwrap();
    later
        .update_highlight(&n3, &HighlightPatch { contents: Some("changed".into()), colour: Some([1.0, 0.0, 0.0]), author: None })
        .unwrap();
    later.save_in_place().unwrap();
    let a3 = later.annotations().unwrap().into_iter().find(|a| a.name.as_deref() == Some(n3.as_str())).unwrap();
    assert_eq!((a3.contents.as_str(), a3.author.as_str()), ("changed", "three"));
    assert!(close(a3.colour.unwrap()[0], 1.0, 0.01) && close(a3.colour.unwrap()[1], 0.0, 0.01));
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.annots.len(), 2);
        let f = dump.annots.iter().find(|a| a.name == n3).unwrap();
        assert_eq!((f.page, f.author.as_str(), f.contents.as_str()), (2, "three", "changed"));
        assert!(close(f.colour[0], 1.0, 0.01) && close(f.colour[1], 0.0, 0.01));
    }
}

#[test]
fn highlight_needs_a_quad_and_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("q.pdf"), &PAGE_TEXT);
    let mut doc = open(&path).unwrap();
    let bad = NewHighlight { page: 0, quads: vec![], colour: YELLOW, author: "x".into(), contents: "".into() };
    assert!(matches!(doc.add_highlight(&bad), Err(Error::NoQuads)));
    let off = NewHighlight { page: 9, quads: vec![[0.0, 0.0, 1.0, 1.0]], colour: YELLOW, author: "x".into(), contents: "".into() };
    assert!(matches!(doc.add_highlight(&off), Err(Error::PageOutOfRange { page: 9, count: 3 })));
}

#[test]
fn rotate_one_page_and_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("r.pdf"), &PAGE_TEXT);
    let mut doc = open(&path).unwrap();
    doc.rotate(&[1], 90).unwrap();
    doc.save_in_place().unwrap();
    let rots: Vec<i32> = (0..3).map(|i| doc.page_info(i).unwrap().rotation).collect();
    assert_eq!(rots, [0, 90, 0]);
    doc.rotate(&[], 90).unwrap();
    doc.save_in_place().unwrap();
    let rots: Vec<i32> = (0..3).map(|i| doc.page_info(i).unwrap().rotation).collect();
    assert_eq!(rots, [90, 180, 90]);
    // Word boxes stay in unrotated points; rendering turns the page.
    let info = doc.page_info(0).unwrap();
    assert!(close(info.width, 595.0, 0.1));
    assert!(close(doc.words(0).unwrap()[0].x0, 72.0, 1.0));
    let (w, h) = png_size(&doc.render_png(0, 72).unwrap());
    assert!(w > h, "rotated page renders landscape: {w}x{h}");
    assert!(matches!(doc.rotate(&[0], 45), Err(Error::BadRotation)));
    let reopened = open(&path).unwrap();
    assert_eq!(reopened.page_info(1).unwrap().rotation, 180);
    if let Some(dump) = fitz_dump(&path) {
        assert_eq!(dump.rotation, vec![90, 180, 90]);
    }
}

#[cfg(windows)]
#[test]
fn locked_file_gets_six_tries_then_a_clear_error() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("locked.pdf"), &PAGE_TEXT);
    let before = std::fs::read(&path).unwrap();
    let mut doc = open(&path).unwrap();
    doc.add_highlight(&NewHighlight {
        page: 0,
        quads: vec![[72.0, 60.0, 200.0, 75.0]],
        colour: YELLOW,
        author: "You".into(),
        contents: "x".into(),
    })
    .unwrap();
    // A second handle that shares nothing, as a viewer holding the file does.
    let holder = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
    let started = Instant::now();
    let err = doc.save_in_place().unwrap_err();
    let elapsed = started.elapsed();
    assert!(matches!(err, Error::FileBusy), "{err:?}");
    assert_eq!(err.to_string(), "The file is open in another program. Close it and try again.");
    let waits = REPLACE_DELAY * (REPLACE_TRIES - 1);
    assert!(elapsed >= waits, "retried for {elapsed:?}, expected at least {waits:?}");
    assert!(elapsed < waits * 4, "took {elapsed:?}");
    drop(holder);
    assert_eq!(std::fs::read(&path).unwrap(), before, "original unchanged");
    no_temp_left(dir.path());
    // Once the holder is gone the same save goes through.
    doc.save_in_place().unwrap();
    assert_eq!(doc.annotations().unwrap().len(), 1);
}

#[cfg(windows)]
#[test]
fn cloud_only_placeholder_is_never_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_pdf(&dir.path().join("cloud.pdf"), &PAGE_TEXT);
    let status = std::process::Command::new("attrib").arg("+O").arg(&path).status().unwrap();
    assert!(status.success());
    assert!(kurasi_pdf::is_cloud_only(&path));
    match open(&path) {
        Err(Error::CloudOnly(p)) => assert_eq!(p, path),
        other => panic!("expected CloudOnly, got {:?}", other.map(|_| ())),
    }
    assert!(open(&path).unwrap_err().to_string().contains("online-only"));
    let _ = std::process::Command::new("attrib").arg("-O").arg(&path).status();
    assert!(!kurasi_pdf::is_cloud_only(&path));
    assert_eq!(open(&path).unwrap().page_count(), 3);
}

#[test]
fn bad_input_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("junk.pdf");
    std::fs::write(&path, b"this is no pdf").unwrap();
    assert!(matches!(open(&path), Err(Error::BadPdf(_))));
    assert!(matches!(open(&dir.path().join("missing.pdf")), Err(Error::Io(_))));
}
