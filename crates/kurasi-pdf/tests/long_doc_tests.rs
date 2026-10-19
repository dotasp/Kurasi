//! A long PDF read through the core's document cache with pdfium, many request threads at
//! once. Regression for a 401-page file whose pages from 24 on (the first pages with a
//! full-page picture) did not appear: every request reopened the file and walked its pages,
//! and documents closed outside the pdfium lock while other threads rendered. The sample is
//! synthetic: numbered text pages, then pages under a generated colour picture from page 24.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kurasi_core::pdf::{DocCache, PdfBackend};
use kurasi_pdf::PdfiumBackend;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};

const PAGES: usize = 48;
const FIRST_PICTURE: usize = 24;

/// A4 pages with one line of Helvetica; from `FIRST_PICTURE` on a 48 x 48 RGB picture fills
/// the page under the text.
fn long_pdf(path: &Path) -> PathBuf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let side = 48usize;
    let mut pixels = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            pixels.extend_from_slice(&[(40 + x * 4) as u8, (20 + y * 3) as u8, 150]);
        }
    }
    let picture_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => side as i64, "Height" => side as i64,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        pixels,
    ));
    let text_res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let picture_res = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Im0" => picture_id },
    });
    let mut kids = Vec::new();
    for n in 1..=PAGES {
        let mut ops = Vec::new();
        if n >= FIRST_PICTURE {
            ops.push(Operation::new("q", vec![]));
            ops.push(Operation::new("cm", vec![595.into(), 0.into(), 0.into(), 842.into(), 0.into(), 0.into()]));
            ops.push(Operation::new("Do", vec!["Im0".into()]));
            ops.push(Operation::new("Q", vec![]));
        }
        ops.extend([
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 770.into()]),
            Operation::new("Tj", vec![Object::string_literal(format!("Synthetic page {}", n))]),
            Operation::new("ET", vec![]),
        ]);
        let content_id = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => if n >= FIRST_PICTURE { picture_res } else { text_res },
        });
        kids.push(page_id.into());
    }
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => PAGES as i64 }));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write test pdf");
    path.to_path_buf()
}

/// The colour at the centre of a rendered page.
fn centre(png: &[u8]) -> [u8; 3] {
    let img = image::load_from_memory(png).expect("a PNG").to_rgb8();
    img.get_pixel(img.width() / 2, img.height() / 2).0
}

#[test]
fn every_page_of_a_long_pdf_renders_while_threads_share_and_close_documents() {
    let dir = tempfile::tempdir().unwrap();
    let path = long_pdf(&dir.path().join("long.pdf"));
    let other = long_pdf(&dir.path().join("other.pdf"));
    let cache = Arc::new(DocCache::new(3));
    let workers: Vec<_> = (0..6)
        .map(|t| {
            let (cache, path) = (cache.clone(), path.clone());
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                for i in 0..PAGES {
                    let n = (i * 7 + t * 11) % PAGES + 1;
                    let d = cache.get(&PdfiumBackend, &path).expect("cached open");
                    let pages = d.pages().expect("page list");
                    assert_eq!(pages.len(), PAGES);
                    let png = d.doc().render_png(n, 0.5).expect("render");
                    let words = d.doc().words(n).expect("words");
                    assert_eq!(words[2].text, n.to_string(), "page {n}");
                    seen.push((n, centre(&png)));
                }
                seen
            })
        })
        .collect();
    // Meanwhile other threads open and close documents of their own and push cache entries
    // out, so documents close on one thread while pages render on others.
    let closers: Vec<_> = (0..2)
        .map(|_| {
            let (cache, other) = (cache.clone(), other.clone());
            std::thread::spawn(move || {
                for i in 0..20 {
                    let doc = PdfiumBackend.open(&other).expect("open");
                    assert_eq!(doc.page_count(), PAGES);
                    drop(doc);
                    if i % 4 == 0 {
                        cache.forget(&other);
                    }
                    let d = cache.get(&PdfiumBackend, &other).expect("cached open");
                    let png = d.doc().render_png(FIRST_PICTURE + i, 0.25).expect("render");
                    assert_eq!(&png[..4], b"\x89PNG");
                }
            })
        })
        .collect();
    let mut all = Vec::new();
    for w in workers {
        all.extend(w.join().expect("worker"));
    }
    for c in closers {
        c.join().expect("closer");
    }
    assert_eq!(all.len(), 6 * PAGES);
    for (n, px) in all {
        if n >= FIRST_PICTURE {
            assert!(px[2] > 100 && px[0] < 250, "page {n} shows its picture, centre {px:?}");
        } else {
            assert_eq!(px, [255, 255, 255], "page {n} is a white text page");
        }
    }
    assert!(cache.len() <= 3);
}
