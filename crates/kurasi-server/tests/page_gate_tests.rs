//! The PDF page routes run at most `PAGE_PERMITS` at a time, and a request whose client gave
//! up while it waited never renders. Regression for a long PDF whose viewer queued an image
//! and a word request for every page it scrolled past: the server rendered all of them, in
//! order, so the page in view came last. The PDFs are synthetic.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use common::{home, ui_dir, Client};
use kurasi_core::pdf::{Annot, AnnotPatch, AnnotSnapshot, NewAnnot, PageInfo, PdfBackend, PdfDoc, PdfMetadata, Word};
use kurasi_core::simplepdf::{SimpleDoc, SimplePdfBackend};
use kurasi_core::Result;
use kurasi_server::{build_app, ServeOptions, Workspace, PAGE_PERMITS};

/// Renders wait until the test opens the gate; every render that starts is recorded.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
    started: Mutex<Vec<(String, usize)>>,
}

struct Held(Arc<Gate>);

struct HeldDoc {
    name: String,
    inner: Box<dyn PdfDoc>,
    gate: Arc<Gate>,
}

impl PdfBackend for Held {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(Box::new(HeldDoc { name, inner: SimplePdfBackend.open(path)?, gate: self.0.clone() }))
    }
}

impl PdfDoc for HeldDoc {
    fn page_count(&self) -> usize {
        self.inner.page_count()
    }
    fn metadata(&self) -> Result<PdfMetadata> {
        self.inner.metadata()
    }
    fn pages(&self) -> Result<Vec<PageInfo>> {
        self.inner.pages()
    }
    fn render_png(&self, page: usize, scale: f64) -> Result<Vec<u8>> {
        self.gate.started.lock().unwrap().push((self.name.clone(), page));
        let mut open = self.gate.open.lock().unwrap();
        while !*open {
            open = self.gate.cv.wait(open).unwrap();
        }
        drop(open);
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

fn long_pdf(path: &Path, pages: usize) {
    let mut doc = SimpleDoc::new();
    for n in 1..=pages {
        let p = doc.add_page();
        doc.text(p, 72.0, 84.0, 12.0, &format!("Page {} of the synthetic sample", n));
    }
    doc.write_to(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_request_whose_client_left_never_renders() {
    assert_eq!(PAGE_PERMITS, 2);
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = h.dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    long_pdf(&proj.join("alpha.pdf"), 40);
    long_pdf(&proj.join("beta.pdf"), 40);
    let gate = Arc::new(Gate::default());
    let options = ServeOptions {
        ui_dir: Some(ui.clone()),
        watch: false,
        auto_scan: false,
        pdf: Some(Arc::new(Held(gate.clone()))),
        opener: Some(Arc::new(|_: &str, _: &str| Ok(()))),
    };
    let ws = Arc::new(Workspace::new(h.paths.clone(), options, 7343));
    let c = Client::new(build_app(ws.clone()));
    c.open_project(&proj).await;
    let refs = c.list().await;
    let fid = |slug: &str| {
        let r = refs.iter().find(|r| r["files"][0]["relpath"].as_str() == Some(&format!("{slug}.pdf"))).unwrap();
        r["primary_file_id"].as_str().unwrap().to_string()
    };
    let (a, b) = (fid("alpha"), fid("beta"));
    assert_eq!(c.get(&format!("/api/file/{a}/info")).await.json["pages"], 40);

    // Two renders take both permits and wait inside the backend.
    let first = tokio::spawn({
        let c = c.clone();
        let a = a.clone();
        async move { c.get(&format!("/api/file/{a}/page/1.png")).await }
    });
    let second = tokio::spawn({
        let c = c.clone();
        let b = b.clone();
        async move { c.get(&format!("/api/file/{b}/page/1.png")).await }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while gate.started.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "two renders should start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A third request waits for a permit; its client leaves (the viewer scrolled past).
    let gone = tokio::time::timeout(Duration::from_millis(300), c.get(&format!("/api/file/{a}/page/3.png"))).await;
    assert!(gone.is_err(), "the third request waits while both permits are taken");
    // A fourth request stays.
    let kept = tokio::spawn({
        let c = c.clone();
        let b = b.clone();
        async move { c.get(&format!("/api/file/{b}/page/24.png")).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(gate.started.lock().unwrap().len(), 2, "no third render while both permits are held");

    *gate.open.lock().unwrap() = true;
    gate.cv.notify_all();
    for r in [first.await.unwrap(), second.await.unwrap(), kept.await.unwrap()] {
        assert_eq!(r.status, 200, "{}", r.err());
        assert_eq!(&r.bytes[..4], b"\x89PNG");
    }
    let mut started = gate.started.lock().unwrap().clone();
    started.sort();
    assert_eq!(started, vec![("alpha.pdf".to_string(), 1), ("beta.pdf".to_string(), 1), ("beta.pdf".to_string(), 24)]);
}
