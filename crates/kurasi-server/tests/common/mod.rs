//! Shared fixtures (the same project folder as kurasi-core's tests) and an in-memory HTTP
//! client over the axum router. Everything is written under a temporary folder; the app-data
//! root is `AppPaths::at(<temp>/home)`, so the real `%LOCALAPPDATA%\Kurasi` is never touched.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use http_body_util::BodyExt;
use kurasi_core::model::Reference;
use kurasi_core::pdf::NewAnnot;
use kurasi_core::simplepdf::{SimpleDoc, SimplePdfBackend};
use kurasi_core::{AppPaths, Project};
use kurasi_server::{build_app, ServeOptions, Workspace};
use serde_json::Value;
use tower::ServiceExt;

pub const PAGE_TEXT: [&str; 3] = [
    "The division of labour raises the output of every worker in the shop.",
    "Second paragraph with other words for the reader to find later on.",
    "Measured tasks and a bonus link pay to output in the plant.",
];

pub struct Home {
    pub dir: tempfile::TempDir,
    pub paths: AppPaths,
}

pub fn home() -> Home {
    let dir = tempfile::Builder::new().prefix("kurasi-server-test-").tempdir().expect("temp dir");
    let paths = AppPaths::at(dir.path().join("home"));
    Home { dir, paths }
}

/// A small PDF: one line of text per entry in `pages`. `annots`: (page, needle, author, content).
pub fn make_pdf(path: &Path, pages: Option<&[&str]>, title: &str, author: &str, annots: &[(usize, &str, &str, &str)]) -> PathBuf {
    let mut doc = SimpleDoc::new();
    for text in pages.unwrap_or(&PAGE_TEXT) {
        let p = doc.add_page();
        doc.text(p, 72.0, 84.0, 12.0, text);
    }
    doc.title = title.to_string();
    doc.author = author.to_string();
    for (page, needle, who, content) in annots {
        let (quads, _) = doc.find_quote(*page, needle).unwrap().unwrap_or_else(|| panic!("needle {} not on page {}", needle, page));
        kurasi_core::pdf::PdfDoc::add_highlight(
            &mut doc,
            &NewAnnot {
                page: *page,
                quads,
                author: who.to_string(),
                contents: content.to_string(),
                colour: "#ffd84a".into(),
                kind: "text".into(),
                created: Some("2026-03-01T12:00:00".into()),
            },
        )
        .unwrap();
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    doc.write_to(path).unwrap();
    path.to_path_buf()
}

pub fn make_png(path: &Path, size: (u32, u32), colour: [u8; 3]) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    image::RgbImage::from_pixel(size.0, size.1, image::Rgb(colour)).save(path).unwrap();
    path.to_path_buf()
}

pub fn make_md(path: &Path, text: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text.replace("\r\n", "\n")).unwrap();
    path.to_path_buf()
}

pub const NOTE: &str = "# Notes

Read [the Smith copy](../lib/smith_1776_wealth.pdf) and [chart](../img/chart.png).

- The plain sentence to highlight sits here.
- Visit [the web](https://example.org/page).
";

/// A project folder with grouped PDFs, a note, an image and excluded entries.
pub fn folder(root: &Path) -> PathBuf {
    let root = root.join("proj");
    make_pdf(
        &root.join("lib/smith_1776_wealth.pdf"),
        None,
        "The wealth of nations",
        "Smith, A.",
        &[(1, "division of labour", "You", "(F) A fact.\n\nTags: #division-of-labour #quote")],
    );
    make_pdf(&root.join("lib/smith_1776_wealth.print.pdf"), None, "", "", &[]);
    make_pdf(
        &root.join("lib/smith_1776_wealth.print.translated-en.pdf"),
        None,
        "",
        "",
        &[
            (1, "raises the output", "Claude", "(I) An interpretation.\n\nTags: #efficiency"),
            (2, "Second paragraph", "Claude", "(P) A proposal."),
        ],
    );
    make_pdf(&root.join("lib/babbage_1832_machinery_excerpt-pp1-12_highlighted.pdf"), None, "", "", &[]);
    make_pdf(&root.join("press/2026-03-02_news-item.rendered.pdf"), None, "", "", &[]);
    make_md(&root.join("notes/reading.md"), NOTE);
    make_md(&root.join("a/_NOTES.md"), "# A notes\n\nFirst folder.\n");
    make_md(&root.join("b/_NOTES.md"), "# B notes\n\nSecond folder.\n");
    make_png(&root.join("img/chart.png"), (300, 200), [40, 90, 160]);
    make_pdf(&root.join(".git/ignored.pdf"), None, "", "", &[]);
    make_md(&root.join("notes/~$lock.md"), "temp");
    std::fs::write(root.join("notes/skip.txt"), "not indexed").unwrap();
    root
}

pub fn open_project(paths: &AppPaths, folder: &Path, name: &str) -> Project {
    Project::create_with(paths, name, folder, Arc::new(SimplePdfBackend)).expect("project opens")
}

pub fn scanned(paths: &AppPaths, folder: &Path) -> Project {
    let p = open_project(paths, folder, "Test project");
    p.scan(kurasi_core::scanner::ScanMode::Incremental, |_| {}).expect("scan");
    p
}

pub fn ref_by_slug(project: &Project, slug: &str) -> Reference {
    project
        .all_references()
        .unwrap()
        .into_iter()
        .find(|r| r.slug == slug)
        .unwrap_or_else(|| panic!("no reference with slug {}", slug))
}

/// A static UI folder with the files the static tests ask for.
pub fn ui_dir(root: &Path) -> PathBuf {
    let d = root.join("ui");
    std::fs::create_dir_all(d.join("assets")).unwrap();
    std::fs::write(d.join("index.html"), "<!doctype html><title>Kurasi test</title>").unwrap();
    std::fs::write(d.join("app.css"), "body{}").unwrap();
    std::fs::write(d.join("assets/splash.jpg"), [0xff, 0xd8, 0xff]).unwrap();
    d
}

pub type Opened = Arc<Mutex<Vec<(String, String)>>>;

/// The router over a fresh workspace (no watcher, no timer, the simple PDF backend, a
/// recording opener), plus the list of opened targets.
pub fn test_app(paths: &AppPaths, ui: &Path) -> (Router, Arc<Workspace>, Opened) {
    let opened: Opened = Arc::new(Mutex::new(Vec::new()));
    let rec = opened.clone();
    let options = ServeOptions {
        ui_dir: Some(ui.to_path_buf()),
        watch: false,
        auto_scan: false,
        pdf: Some(Arc::new(SimplePdfBackend)),
        opener: Some(Arc::new(move |mode: &str, target: &str| {
            rec.lock().unwrap().push((mode.to_string(), target.to_string()));
            Ok(())
        })),
    };
    let ws = Arc::new(Workspace::new(paths.clone(), options, 7343));
    (build_app(ws.clone()), ws, opened)
}

pub struct Reply {
    pub status: u16,
    pub ctype: String,
    /// The Content-Encoding header, empty when the body is sent as it is.
    pub encoding: String,
    /// The body as sent (compressed when `encoding` names an algorithm).
    pub bytes: Vec<u8>,
    /// The parsed JSON body, after gzip decoding.
    pub json: Value,
}

impl Reply {
    pub fn err(&self) -> String {
        self.json.get("error").and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
}

/// An HTTP client over the router. The default Host header passes the check.
#[derive(Clone)]
pub struct Client {
    pub app: Router,
    pub host: String,
    pub origin: Option<String>,
    pub accept_encoding: Option<String>,
}

impl Client {
    pub fn new(app: Router) -> Client {
        Client { app, host: "127.0.0.1:7343".into(), origin: None, accept_encoding: None }
    }

    pub fn with_host(&self, host: &str) -> Client {
        Client { host: host.to_string(), ..self.clone() }
    }

    pub fn with_origin(&self, origin: &str) -> Client {
        Client { origin: Some(origin.to_string()), ..self.clone() }
    }

    /// A client whose requests carry `Accept-Encoding` as a browser sends it.
    pub fn with_accept_encoding(&self, value: &str) -> Client {
        Client { accept_encoding: Some(value.to_string()), ..self.clone() }
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        let mut builder = Request::builder().method(method).uri(path).header(header::HOST, &self.host);
        if let Some(o) = &self.origin {
            builder = builder.header(header::ORIGIN, o);
        }
        if let Some(e) = &self.accept_encoding {
            builder = builder.header(header::ACCEPT_ENCODING, e);
        }
        let req = match body {
            Some(v) => builder.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.expect("router answers");
        let status = resp.status().as_u16();
        let ctype = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let encoding = resp.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes().to_vec();
        let plain = if encoding == "gzip" {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut out).expect("gzip body");
            out
        } else {
            bytes.clone()
        };
        let json = if ctype.starts_with("application/json") { serde_json::from_slice(&plain).unwrap_or(Value::Null) } else { Value::Null };
        Reply { status, ctype, encoding, bytes, json }
    }

    pub async fn get(&self, path: &str) -> Reply {
        self.request("GET", path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Reply {
        self.request("POST", path, Some(body)).await
    }

    pub async fn put(&self, path: &str, body: Value) -> Reply {
        self.request("PUT", path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Reply {
        self.request("DELETE", path, None).await
    }

    /// Waits for the running scan to finish; returns the status.
    pub async fn wait_scan(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let r = self.get("/api/scan/status").await;
            if r.status == 200 && r.json["running"] == Value::Bool(false) {
                return r.json;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("scan did not finish");
    }

    /// Creates and scans a project over `folder`; returns the state.
    pub async fn open_project(&self, folder: &Path) -> Value {
        let r = self
            .post(
                "/api/project",
                serde_json::json!({"name": "Test", "folder": folder.to_string_lossy(), "file_types": [".pdf", ".md", ".png"], "auto_scan_min": 0}),
            )
            .await;
        assert_eq!(r.status, 200, "{}", r.err());
        self.wait_scan().await;
        r.json
    }

    /// The catalogue list of `/api/references`.
    pub async fn list(&self) -> Vec<Value> {
        let r = self.get("/api/references").await;
        assert_eq!(r.status, 200, "{}", r.err());
        r.json.as_array().unwrap().clone()
    }

    /// The listed references as full records (`/api/reference/<id>`), by slug.
    pub async fn refs(&self) -> std::collections::HashMap<String, Value> {
        let mut out = std::collections::HashMap::new();
        for v in self.list().await {
            let r = self.get(&format!("/api/reference/{}", v["id"].as_str().unwrap())).await;
            assert_eq!(r.status, 200, "{}", r.err());
            out.insert(r.json["slug"].as_str().unwrap().to_string(), r.json);
        }
        out
    }
}
