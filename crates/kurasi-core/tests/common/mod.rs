//! Shared fixtures. Everything is written under a temporary folder; the app-data root is a
//! fresh `AppPaths::at(<temp>/home)`, so the user's real `%LOCALAPPDATA%\Kurasi` is never touched.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kurasi_core::model::Reference;
use kurasi_core::pdf::{compose, NewAnnot};
use kurasi_core::simplepdf::{SimpleDoc, SimplePdfBackend};
use kurasi_core::{AppPaths, Project};

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
    let dir = tempfile::Builder::new().prefix("kurasi-test-").tempdir().expect("temp dir");
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

pub fn compose_note(analysis: &str, tags: &[&str]) -> String {
    compose(analysis, &tags.iter().map(|t| t.to_string()).collect::<Vec<_>>())
}
