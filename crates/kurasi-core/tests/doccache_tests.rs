//! A long PDF read page after page. Regression for a 401-page file whose viewer stalled from
//! page 24 on: every page image and word request opened the whole file again and walked all of
//! its pages for their sizes, so each request cost about 0.6 s with pdfium, and the requests a
//! scroll queued finished tens of seconds later. The read routes now share one open document
//! per file version and walk its pages once.
//!
//! The PDFs are synthetic (numbered pages of filler words) and written by the built-in writer.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::home;
use kurasi_core::model::Origin;
use kurasi_core::pdf::{Annot, AnnotPatch, AnnotSnapshot, DocCache, NewAnnot, PageInfo, PdfBackend, PdfDoc, PdfMetadata, Word};
use kurasi_core::scanner::ScanMode;
use kurasi_core::simplepdf::{SimpleDoc, SimplePdfBackend};
use kurasi_core::{Project, Result};
use serde_json::json;

/// Counts how often documents are opened and how often a document walks all its pages.
#[derive(Default)]
struct Counts {
    opens: AtomicUsize,
    walks: AtomicUsize,
    renders: AtomicUsize,
}

impl Counts {
    fn get(&self) -> (usize, usize) {
        (self.opens.load(Ordering::SeqCst), self.walks.load(Ordering::SeqCst))
    }
    fn reset(&self) {
        self.opens.store(0, Ordering::SeqCst);
        self.walks.store(0, Ordering::SeqCst);
        self.renders.store(0, Ordering::SeqCst);
    }
}

struct Counting(Arc<Counts>);

struct CountingDoc {
    inner: Box<dyn PdfDoc>,
    counts: Arc<Counts>,
}

impl PdfBackend for Counting {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingDoc { inner: SimplePdfBackend.open(path)?, counts: self.0.clone() }))
    }
}

impl PdfDoc for CountingDoc {
    fn page_count(&self) -> usize {
        self.inner.page_count()
    }
    fn metadata(&self) -> Result<PdfMetadata> {
        self.inner.metadata()
    }
    fn pages(&self) -> Result<Vec<PageInfo>> {
        self.counts.walks.fetch_add(1, Ordering::SeqCst);
        self.inner.pages()
    }
    fn render_png(&self, page: usize, scale: f64) -> Result<Vec<u8>> {
        self.counts.renders.fetch_add(1, Ordering::SeqCst);
        self.inner.render_png(page, scale)
    }
    fn thumbnail_png(&self, width_px: u32) -> Result<Vec<u8>> {
        self.inner.thumbnail_png(width_px)
    }
    fn words(&self, page: usize) -> Result<Vec<Word>> {
        self.inner.words(page)
    }
    fn annotations(&self) -> Result<Vec<Annot>> {
        self.inner.annotations()
    }
    fn add_highlight(&mut self, h: &NewAnnot) -> Result<Annot> {
        self.inner.add_highlight(h)
    }
    fn update_highlight(&mut self, xref: i64, patch: &AnnotPatch) -> Result<()> {
        self.inner.update_highlight(xref, patch)
    }
    fn delete_highlight(&mut self, xref: i64) -> Result<Vec<AnnotSnapshot>> {
        self.inner.delete_highlight(xref)
    }
    fn restore_annotations(&mut self, snaps: &[AnnotSnapshot]) -> Result<BTreeMap<i64, i64>> {
        self.inner.restore_annotations(snaps)
    }
    fn rotate(&mut self, page: Option<usize>, delta: i32) -> Result<Vec<i32>> {
        self.inner.rotate(page, delta)
    }
    fn save_in_place(&mut self) -> Result<()> {
        self.inner.save_in_place()
    }
}

/// `pages` numbered pages of filler words.
fn long_pdf(path: &Path, pages: usize) -> PathBuf {
    let mut doc = SimpleDoc::new();
    for n in 1..=pages {
        let p = doc.add_page();
        doc.text(p, 72.0, 84.0, 12.0, &format!("Page {} of the synthetic sample", n));
        doc.text(p, 72.0, 110.0, 11.0, "Filler words for the long document test.");
    }
    doc.title = "Synthetic long document".into();
    doc.write_to(path).unwrap();
    path.to_path_buf()
}

/// Page renders in the render cache (temp files of a writer left out).
fn cached_pages(dir: &Path) -> usize {
    let mut n = 0;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            n += cached_pages(&p);
        } else if p.extension().is_some_and(|x| x == "png") {
            n += 1;
        }
    }
    n
}

fn project_with(h: &common::Home, folder: &Path, counts: &Arc<Counts>) -> (Project, String) {
    let p = Project::create_with(&h.paths, "Long", folder, Arc::new(Counting(counts.clone()))).unwrap();
    p.scan(ScanMode::Incremental, |_| {}).unwrap();
    let r = p.all_references().unwrap().into_iter().find(|r| r.slug == "long").expect("the long PDF is catalogued");
    (p, r.primary_file_id.unwrap())
}

#[test]
fn a_long_pdf_opens_once_and_walks_its_pages_once() {
    let h = home();
    let folder = h.dir.path().join("proj");
    std::fs::create_dir_all(&folder).unwrap();
    let path = long_pdf(&folder.join("long.pdf"), 40);
    let counts = Arc::new(Counts::default());
    let (p, fid) = project_with(&h, &folder, &counts);
    counts.reset();

    let info = p.file_info(&fid).unwrap();
    assert_eq!(info["pages"], json!(40));
    for n in 1..=40 {
        let png = p.render_page(&fid, n, 0.25).unwrap();
        assert_eq!(&png[..4], b"\x89PNG", "page {n}");
        let words = p.page_words(&fid, n).unwrap();
        assert_eq!(words["words"][1][4], json!(n.to_string()), "page {n}");
    }
    assert!(p.page_text(&fid, 24).unwrap().starts_with("Page 24 of"));
    assert_eq!(p.render_page(&fid, 41, 0.25).unwrap_err().status(), 404);
    assert_eq!(counts.get(), (1, 1), "one open and one page walk for the whole run");
    assert_eq!(counts.renders.load(Ordering::SeqCst), 40);

    // A second pass is served from the render cache and the open document. Renders reach the
    // cache on a writer thread, so wait for them first.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while cached_pages(&h.paths.pages_dir()) < 40 {
        assert!(std::time::Instant::now() < deadline, "the render cache should fill");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for n in [1, 23, 24, 25, 40] {
        p.render_page(&fid, n, 0.25).unwrap();
    }
    assert_eq!(counts.get(), (1, 1));
    assert_eq!(counts.renders.load(Ordering::SeqCst), 40);

    // Kurasi's own write drops the entry: the rotation shows at once.
    let rotated = p.rotate(&fid, Some(24), 90, Origin::Ui).unwrap();
    assert_eq!(rotated["rotation"][23], json!(90));
    assert_eq!(p.file_info(&fid).unwrap()["rotation"][23], json!(90));
    assert_eq!(p.page_words(&fid, 24).unwrap()["rotation"], json!(90));
    assert_eq!(counts.get(), (3, 2), "the write opens its own document, the next read opens the new file once");

    // A change by another program (another size) opens the file again.
    long_pdf(&path, 41);
    assert_eq!(p.file_info(&fid).unwrap()["pages"], json!(41));
    assert!(p.page_text(&fid, 41).unwrap().starts_with("Page 41 of"));
    assert_eq!(counts.get(), (4, 3));
}

#[test]
fn the_cache_keeps_three_documents_and_forgets_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let counts = Arc::new(Counts::default());
    let backend = Counting(counts.clone());
    let files: Vec<PathBuf> = (0..4).map(|i| long_pdf(&dir.path().join(format!("f{i}.pdf")), 30 + i)).collect();
    let cache = DocCache::new(3);
    for f in &files {
        let d = cache.get(&backend, f).unwrap();
        let walked = d.pages().unwrap().len();
        assert_eq!(d.doc().page_count(), walked);
    }
    assert_eq!(cache.len(), 3);
    assert_eq!(counts.get().0, 4);
    // The three most recent stay open; the first one closed and opens again.
    cache.get(&backend, &files[3]).unwrap();
    cache.get(&backend, &files[1]).unwrap();
    assert_eq!(counts.get().0, 4);
    cache.get(&backend, &files[0]).unwrap();
    assert_eq!(counts.get().0, 5);
    assert_eq!(cache.len(), 3);
    cache.forget(&files[0]);
    assert_eq!(cache.len(), 2);
    cache.get(&backend, &files[0]).unwrap();
    assert_eq!(counts.get().0, 6);
}

#[test]
fn concurrent_page_requests_share_one_open_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = long_pdf(&dir.path().join("long.pdf"), 40);
    let counts = Arc::new(Counts::default());
    let backend = Arc::new(Counting(counts.clone()));
    let cache = Arc::new(DocCache::new(3));
    let threads: Vec<_> = (0..6)
        .map(|t| {
            let (cache, backend, path) = (cache.clone(), backend.clone(), path.clone());
            std::thread::spawn(move || {
                for i in 0..40 {
                    let n = (i * 7 + t * 5) % 40 + 1;
                    let d = cache.get(backend.as_ref(), &path).unwrap();
                    assert_eq!(d.pages().unwrap().len(), 40);
                    let png = d.doc().render_png(n, 0.25).unwrap();
                    assert_eq!(&png[..4], b"\x89PNG");
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(counts.get(), (1, 1), "requests that miss at once still open the file once");
}
