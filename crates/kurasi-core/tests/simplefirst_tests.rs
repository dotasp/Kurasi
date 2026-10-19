//! `SimpleFirst`: Kurasi-made files open with the built-in reader, any other PDF goes to the
//! fallback backend.

mod common;

use std::sync::Arc;

use common::{home, make_pdf};
use kurasi_core::error::Error;
use kurasi_core::pdf::{NoPdf, PdfBackend};
use kurasi_core::simplepdf::SimpleFirst;

#[test]
fn sample_files_open_with_the_built_in_reader() {
    let h = home();
    let path = make_pdf(&h.dir.path().join("sample.pdf"), None, "Sample", "Kurasi", &[]);
    let backend = SimpleFirst::new(Arc::new(NoPdf));
    let doc = backend.open(&path).expect("a Kurasi-made file opens without the fallback");
    assert!(doc.page_count() >= 1);
    assert_eq!(backend.name(), "none");
}

#[test]
fn other_pdfs_go_to_the_fallback() {
    let h = home();
    let path = h.dir.path().join("other.pdf");
    std::fs::write(&path, b"%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n").unwrap();
    let err = SimpleFirst::new(Arc::new(NoPdf)).open(&path).err().expect("the fallback answers");
    match err {
        Error::Unsupported(msg) => assert!(msg.contains("No PDF backend"), "{}", msg),
        other => panic!("unexpected error {:?}", other),
    }
    // A file that is no PDF at all is refused before the fallback is asked.
    let bad = h.dir.path().join("bad.pdf");
    std::fs::write(&bad, b"hello").unwrap();
    assert!(matches!(SimpleFirst::new(Arc::new(NoPdf)).open(&bad), Err(Error::BadRequest(_))));
}
