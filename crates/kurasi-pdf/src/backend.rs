//! `kurasi_core::pdf::PdfBackend` over [`Doc`]: the pdfium backend of the Rust build.
//!
//! Two translations happen here.
//!
//! Pages: the core counts pages from 1 (the number a reader sees), this crate from 0.
//!
//! Annotation ids: the core addresses an annotation by an `i64` it stores as `pdf_xref`,
//! this crate by the `/NM` name. The id of an annotation is:
//!
//! 1. a 53-bit FNV-1a hash of its `/NM` (bit 52 set, so it sits in `[2^52, 2^53)`, above any
//!    object number and within JavaScript's exact integer range) when the file carries a
//!    name. This crate writes a UUID name into every annotation it creates, so the id is
//!    known before the save and stays the same across saves by pdfium, MuPDF or Acrobat,
//!    which may renumber objects;
//! 2. else the object number of the annotation dictionary (the PyMuPDF `xref`), as the
//!    Python build stored it, when the dictionary is an indirect object;
//! 3. else `-(page * 1_000_000 + index)` for a dictionary written directly into `/Annots`.
//!
//! Lookups accept both the hash and the object number, so a row the Python build made
//! (object number) can still be edited or deleted; the next scan rewrites its `pdf_xref`
//! to the hash through the page-and-quads match in `scanner::merge_annotations`.
//!
//! An annotation without `/NM` cannot be changed in place: this crate locates annotations
//! by name. The adapter answers with a plain `NotFound` that names the limitation.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use kurasi_core::error::{Error as CoreError, Result as CoreResult};
use kurasi_core::pdf::{
    colour_hex, hex_rgb, rect_list, to_pdf_date, Annot as CoreAnnot, AnnotPatch, AnnotSnapshot, NewAnnot, PageInfo as CorePageInfo,
    PdfBackend, PdfDoc, PdfMetadata, Word as CoreWord,
};
use serde_json::json;

use crate::error::Error;
use crate::types::{Annot, HighlightPatch, NewHighlight, Rect, RestoreAnnot};
use crate::Doc;

/// Bit 52: set on every hashed id so it never collides with an object number.
pub const NAME_ID_BIT: i64 = 1 << 52;

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The core id of an annotation named `name`.
pub fn id_of_name(name: &str) -> i64 {
    ((fnv1a(name) & ((1u64 << 52) - 1)) as i64) | NAME_ID_BIT
}

/// The core id of an annotation as read from the file (rules 1 to 3 of the module notes).
pub fn id_of(a: &Annot) -> i64 {
    match (&a.name, a.object) {
        (Some(n), _) => id_of_name(n),
        (None, Some(o)) => o as i64,
        (None, None) => -((a.page as i64 + 1) * 1_000_000 + a.index as i64),
    }
}

fn core_err(e: Error) -> CoreError {
    match e {
        Error::LibraryNotFound(_) | Error::Library(_) => CoreError::unsupported(e.to_string()),
        Error::CloudOnly(_) | Error::NotVerified => CoreError::conflict(e.to_string()),
        Error::PageOutOfRange { .. } | Error::AnnotationNotFound(_) => CoreError::not_found(e.to_string()),
        Error::NoQuads | Error::BadRotation => CoreError::bad(e.to_string()),
        Error::FileBusy => CoreError::FileBusy,
        Error::Io(io) => CoreError::Io(io),
        Error::BadPdf(_) | Error::Encrypted | Error::Structure(_) | Error::Pdfium(_) | Error::Image(_) => CoreError::Other(e.to_string()),
    }
}

fn page_index(page: usize) -> CoreResult<usize> {
    page.checked_sub(1).ok_or_else(|| CoreError::not_found("Page 0 does not exist; pages are counted from 1."))
}

fn to_rects(quads: &[Vec<f64>]) -> Vec<Rect> {
    quads
        .iter()
        .filter(|q| q.len() == 4)
        .map(|q| [q[0].min(q[2]) as f32, q[1].min(q[3]) as f32, q[0].max(q[2]) as f32, q[1].max(q[3]) as f32])
        .collect()
}

fn from_rects(rects: &[Rect]) -> Vec<Vec<f64>> {
    let v: Vec<[f64; 4]> = rects.iter().map(|r| [r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]).collect();
    rect_list(&v)
}

fn rgb_of(hex: &str) -> [f32; 3] {
    let (r, g, b) = hex_rgb(hex);
    [r as f32, g as f32, b as f32]
}

fn hex_of(rgb: Option<[f32; 3]>) -> Option<String> {
    rgb.and_then(|c| colour_hex(&[c[0] as f64, c[1] as f64, c[2] as f64]))
}

fn irt_id(a: &Annot) -> i64 {
    match &a.in_reply_to {
        Some(n) if !n.starts_with('#') => id_of_name(n),
        _ => 0,
    }
}

fn core_annot(a: &Annot) -> CoreAnnot {
    CoreAnnot {
        xref: id_of(a),
        page: a.page + 1,
        subtype: a.kind.subtype().to_string(),
        quads: from_rects(&a.quads),
        author: a.author.clone(),
        contents: a.contents.clone(),
        colour: hex_of(a.colour),
        created: a.created.clone(),
        modified: a.modified.clone(),
        irt_xref: irt_id(a),
    }
}

fn snapshot(a: &Annot) -> AnnotSnapshot {
    AnnotSnapshot {
        xref: id_of(a),
        page: a.page + 1,
        subtype: a.kind.subtype().to_string(),
        quads: from_rects(&a.quads),
        author: a.author.clone(),
        contents: a.contents.clone(),
        colour: hex_of(a.colour),
        created: a.created.clone(),
        modified: a.modified.clone(),
        irt_xref: irt_id(a),
        extra: json!({"name": a.name, "index": a.index, "object": a.object}),
    }
}

/// The pdfium backend. `open` loads `pdfium.dll` on first use (see [`crate::Pdfium`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfiumBackend;

impl PdfBackend for PdfiumBackend {
    fn open(&self, path: &Path) -> CoreResult<Box<dyn PdfDoc>> {
        Ok(Box::new(PdfiumDoc::open(path)?))
    }
    fn name(&self) -> &'static str {
        "pdfium"
    }
}

/// One open document behind the core trait.
pub struct PdfiumDoc {
    doc: Mutex<Doc>,
    /// Ids of annotations written in this session, before the saved file lists them.
    fresh: Mutex<BTreeMap<i64, String>>,
}

impl PdfiumDoc {
    pub fn open(path: &Path) -> CoreResult<PdfiumDoc> {
        let doc = crate::open(path).map_err(core_err)?;
        Ok(PdfiumDoc { doc: Mutex::new(doc), fresh: Mutex::new(BTreeMap::new()) })
    }

    fn doc(&self) -> std::sync::MutexGuard<'_, Doc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn remember(&self, name: &str) -> i64 {
        let id = id_of_name(name);
        self.fresh.lock().unwrap_or_else(|e| e.into_inner()).insert(id, name.to_string());
        id
    }

    /// The `/NM` behind a core id: a name written in this session, else the annotation of the
    /// saved file whose hash or object number equals `xref`.
    fn name_for(&self, doc: &mut Doc, xref: i64) -> CoreResult<String> {
        if let Some(n) = self.fresh.lock().unwrap_or_else(|e| e.into_inner()).get(&xref) {
            return Ok(n.clone());
        }
        let annots = doc.annotations().map_err(core_err)?;
        for a in &annots {
            if id_of(a) == xref || a.object.map(|o| o as i64) == Some(xref) {
                return a.name.clone().ok_or_else(|| {
                    CoreError::not_found(
                        "The annotation has no name (/NM) in the PDF, so Kurasi cannot change it in place. Edit it in a PDF viewer, then scan the folder.",
                    )
                });
            }
        }
        Err(CoreError::not_found("The annotation is gone from the PDF. Scan the folder to refresh the highlights."))
    }
}

impl PdfDoc for PdfiumDoc {
    fn page_count(&self) -> usize {
        self.doc().page_count()
    }

    fn metadata(&self) -> CoreResult<PdfMetadata> {
        let (title, author) = self.doc().metadata();
        Ok(PdfMetadata { title, author })
    }

    fn pages(&self) -> CoreResult<Vec<CorePageInfo>> {
        let doc = self.doc();
        (0..doc.page_count())
            .map(|i| {
                let p = doc.page_info(i).map_err(core_err)?;
                Ok(CorePageInfo { width: p.width as f64, height: p.height as f64, rotation: p.rotation })
            })
            .collect()
    }

    fn render_png(&self, page: usize, scale: f64) -> CoreResult<Vec<u8>> {
        self.doc().render_png_scale(page_index(page)?, scale as f32).map_err(core_err)
    }

    fn thumbnail_png(&self, width_px: u32) -> CoreResult<Vec<u8>> {
        self.doc().thumbnail_png(width_px).map_err(core_err)
    }

    fn words(&self, page: usize) -> CoreResult<Vec<CoreWord>> {
        let words = self.doc().words(page_index(page)?).map_err(core_err)?;
        Ok(words
            .into_iter()
            .map(|w| CoreWord { x0: w.x0 as f64, y0: w.y0 as f64, x1: w.x1 as f64, y1: w.y1 as f64, text: w.text, block: 0, line: w.line })
            .collect())
    }

    fn annotations(&self) -> CoreResult<Vec<CoreAnnot>> {
        let annots = self.doc().annotations().map_err(core_err)?;
        Ok(annots.iter().map(core_annot).collect())
    }

    fn add_highlight(&mut self, h: &NewAnnot) -> CoreResult<CoreAnnot> {
        let page = page_index(h.page)?;
        let quads = to_rects(&h.quads);
        if quads.is_empty() {
            return Err(CoreError::bad("A PDF highlight needs at least one quad [x0, y0, x1, y1]."));
        }
        let colour = rgb_of(&h.colour);
        let stamp = h.created.clone().unwrap_or_else(kurasi_core::util::now_iso);
        let mut doc = self.doc();
        let (name, subtype) = if h.kind == "note" {
            let r = RestoreAnnot {
                page,
                subtype: "Text".into(),
                quads: vec![quads[0]],
                author: h.author.clone(),
                contents: h.contents.clone(),
                colour: Some(colour),
                created: to_pdf_date(&stamp),
                modified: to_pdf_date(&stamp),
                name: None,
                in_reply_to: None,
            };
            (doc.restore_annotation(&r).map_err(core_err)?, "Text")
        } else {
            let n = NewHighlight { page, quads: quads.clone(), colour, author: h.author.clone(), contents: h.contents.clone() };
            (doc.add_highlight(&n).map_err(core_err)?, "Highlight")
        };
        drop(doc);
        let xref = self.remember(&name);
        Ok(CoreAnnot {
            xref,
            page: h.page,
            subtype: subtype.into(),
            quads: from_rects(&quads),
            author: h.author.clone(),
            contents: h.contents.clone(),
            colour: hex_of(Some(colour)),
            created: Some(stamp.clone()),
            modified: Some(stamp),
            irt_xref: 0,
        })
    }

    fn update_highlight(&mut self, xref: i64, patch: &AnnotPatch) -> CoreResult<()> {
        let mut doc = self.doc();
        let name = self.name_for(&mut doc, xref)?;
        let p = HighlightPatch {
            contents: patch.contents.clone(),
            author: patch.author.clone(),
            colour: patch.colour.as_deref().map(rgb_of),
        };
        doc.update_highlight(&name, &p).map_err(core_err)
    }

    fn delete_highlight(&mut self, xref: i64) -> CoreResult<Vec<AnnotSnapshot>> {
        let mut doc = self.doc();
        let name = self.name_for(&mut doc, xref)?;
        let annots = doc.annotations().map_err(core_err)?;
        let target = annots
            .iter()
            .find(|a| a.name.as_deref() == Some(name.as_str()))
            .ok_or_else(|| CoreError::not_found("The annotation is gone from the PDF. Scan the folder to refresh the highlights."))?;
        let mut snaps = vec![snapshot(target)];
        let mut names = vec![name.clone()];
        for reply in annots.iter().filter(|a| a.in_reply_to.as_deref() == Some(name.as_str())) {
            snaps.push(snapshot(reply));
            if let Some(n) = &reply.name {
                names.push(n.clone());
            }
        }
        for n in &names {
            match doc.delete_highlight(n) {
                Ok(()) => {}
                Err(Error::AnnotationNotFound(_)) if n != &name => {}
                Err(e) => return Err(core_err(e)),
            }
        }
        Ok(snaps)
    }

    fn restore_annotations(&mut self, snaps: &[AnnotSnapshot]) -> CoreResult<BTreeMap<i64, i64>> {
        let mut mapping = BTreeMap::new();
        let mut names: BTreeMap<i64, String> = BTreeMap::new();
        let mut doc = self.doc();
        for s in snaps {
            let page = page_index(s.page)?;
            let kept = s.extra.get("name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()).map(str::to_string);
            let r = RestoreAnnot {
                page,
                subtype: s.subtype.clone(),
                quads: to_rects(&s.quads),
                author: s.author.clone(),
                contents: s.contents.clone(),
                colour: s.colour.as_deref().map(rgb_of),
                created: s.created.as_deref().and_then(to_pdf_date),
                modified: s.modified.as_deref().and_then(to_pdf_date),
                name: kept,
                in_reply_to: if s.irt_xref != 0 { names.get(&s.irt_xref).cloned() } else { None },
            };
            let name = doc.restore_annotation(&r).map_err(core_err)?;
            let id = id_of_name(&name);
            names.insert(s.xref, name.clone());
            mapping.insert(s.xref, id);
            drop(doc);
            self.remember(&name);
            doc = self.doc();
        }
        Ok(mapping)
    }

    fn rotate(&mut self, page: Option<usize>, delta: i32) -> CoreResult<Vec<i32>> {
        let pages: Vec<usize> = match page {
            Some(n) => vec![page_index(n)?],
            None => Vec::new(),
        };
        let mut doc = self.doc();
        doc.rotate(&pages, delta).map_err(core_err)?;
        (0..doc.page_count()).map(|i| doc.page_info(i).map(|p| p.rotation).map_err(core_err)).collect()
    }

    fn save_in_place(&mut self) -> CoreResult<()> {
        self.doc().save_in_place().map_err(core_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_ids_sit_above_object_numbers_and_within_53_bits() {
        let a = id_of_name("1c6b8c9e-4f5a-4d2e-9b1a-0f3c2d1e5a7b");
        let b = id_of_name("other");
        assert!(a >= NAME_ID_BIT && a < (1 << 53));
        assert!(b >= NAME_ID_BIT && b < (1 << 53));
        assert_ne!(a, b);
        assert_eq!(a, id_of_name("1c6b8c9e-4f5a-4d2e-9b1a-0f3c2d1e5a7b"));
    }
}
