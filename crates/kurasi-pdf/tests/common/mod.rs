//! Shared test helpers: a small PDF written with lopdf, and a PyMuPDF reader run from the
//! legacy venv when it exists.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use serde::Deserialize;

pub const PAGE_TEXT: [&str; 3] = [
    "The division of labour raises the output of every worker in the shop.",
    "Second paragraph with other words for the reader to find later on.",
    "Measured tasks and a bonus link pay to output in the plant.",
];

/// A4 pages, one line of Helvetica 12 pt per entry, baseline at y = 770 from the bottom.
pub fn make_pdf(path: &Path, pages: &[&str]) -> PathBuf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut kids = Vec::new();
    for text in pages {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 770.into()]),
                Operation::new("Tj", vec![Object::string_literal(*text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => resources_id,
        });
        kids.push(page_id.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write test pdf");
    path.to_path_buf()
}

pub fn venv_python() -> Option<PathBuf> {
    let p = PathBuf::from(r"D:\PythonScripts\Kurasi\.venv\Scripts\python.exe");
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
pub struct FitzAnnot {
    pub page: usize,
    pub name: String,
    pub author: String,
    pub contents: String,
    pub colour: Vec<f32>,
    pub quads: Vec<[f32; 4]>,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct FitzDump {
    pub page_count: usize,
    pub rotation: Vec<i32>,
    pub annots: Vec<FitzAnnot>,
    /// Words of the first page: [x0, y0, x1, y1, text].
    pub words: Vec<(f32, f32, f32, f32, String)>,
}

const FITZ_SCRIPT: &str = r#"
import json, sys
import pymupdf as fitz
doc = fitz.open(sys.argv[1])
out = {"page_count": doc.page_count, "rotation": [p.rotation for p in doc], "annots": [], "words": []}
for page in doc:
    for a in page.annots():
        inf = a.info or {}
        col = (a.colors or {}).get("stroke") or (a.colors or {}).get("fill") or []
        v = a.vertices or []
        quads = []
        for i in range(0, len(v) - 3, 4):
            r = fitz.Quad(v[i], v[i + 1], v[i + 2], v[i + 3]).rect
            quads.append([round(r.x0, 2), round(r.y0, 2), round(r.x1, 2), round(r.y1, 2)])
        out["annots"].append({"page": page.number, "name": inf.get("id", ""), "author": inf.get("title", ""),
                              "contents": inf.get("content", ""), "colour": [round(c, 4) for c in col],
                              "quads": quads, "type": a.type[1]})
if doc.page_count:
    flags = fitz.TEXTFLAGS_WORDS & ~fitz.TEXT_PRESERVE_LIGATURES
    out["words"] = [[round(w[0], 2), round(w[1], 2), round(w[2], 2), round(w[3], 2), w[4]]
                    for w in doc[0].get_text("words", flags=flags)]
print(json.dumps(out))
"#;

/// Reads the PDF with PyMuPDF from the legacy venv. `None` when the venv is absent.
pub fn fitz_dump(path: &Path) -> Option<FitzDump> {
    let python = venv_python()?;
    let out = Command::new(python)
        .arg("-I")
        .arg("-c")
        .arg(FITZ_SCRIPT)
        .arg(path)
        .output()
        .expect("run venv python");
    assert!(
        out.status.success(),
        "PyMuPDF check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    Some(serde_json::from_str(text.trim()).expect("parse PyMuPDF json"))
}

pub fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}
