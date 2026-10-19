//! Annotation dictionaries through lopdf.
//!
//! Two jobs. Reading: every markup and note annotation of a saved file, exactly as its
//! dictionary says (/NM, /T, /Contents, /C, /QuadPoints, /M, /CreationDate, /IRT).
//! Writing: the fields pdfium-render has no setter for, applied to the bytes pdfium saved.
//! pdfium keeps the order of each page's /Annots array on save, so a (page, index) pair
//! found in pdfium names the same annotation in the lopdf parse.

use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat};

use crate::dates::pdf_date_to_iso;
use crate::error::{Error, Result};
use crate::types::{Annot, AnnotKind, Rect, RestoreAnnot};

/// One field write deferred until after pdfium has saved.
#[derive(Debug, Clone)]
pub struct Patch {
    pub page: usize,
    /// Position in the page's /Annots array; unused (`usize::MAX`) for `AddAnnot`.
    pub index: usize,
    pub op: PatchOp,
}

#[derive(Debug, Clone)]
pub enum PatchOp {
    /// Sets /NM.
    SetName(String),
    /// Sets /C and removes /AP so viewers draw the new colour.
    SetColour([f32; 3]),
    /// Appends a whole annotation dictionary to the page's /Annots (applied after the
    /// field patches, so their indexes stay valid).
    AddAnnot(Box<RestoreAnnot>),
}

impl Patch {
    pub fn is_add(&self) -> bool {
        matches!(self.op, PatchOp::AddAnnot(_))
    }
}

/// Unrotated page box in PDF user space: CropBox, else MediaBox, both inherited.
#[derive(Debug, Clone, Copy)]
pub struct PageBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PageBox {
    /// PDF-space rectangle to top-left-origin page points.
    pub fn to_top_left(&self, left: f32, bottom: f32, right: f32, top: f32) -> Rect {
        let (l, r) = if left <= right { (left, right) } else { (right, left) };
        let (b, t) = if bottom <= top { (bottom, top) } else { (top, bottom) };
        [l - self.x0, self.y1 - t, r - self.x0, self.y1 - b]
    }

    /// Top-left-origin page points to PDF space (left, bottom, right, top).
    pub fn to_pdf(&self, r: Rect) -> (f32, f32, f32, f32) {
        let (x0, x1) = if r[0] <= r[2] { (r[0], r[2]) } else { (r[2], r[0]) };
        let (y0, y1) = if r[1] <= r[3] { (r[1], r[3]) } else { (r[3], r[1]) };
        (x0 + self.x0, self.y1 - y1, x1 + self.x0, self.y1 - y0)
    }
}

fn deref<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    match doc.dereference(obj) {
        Ok((_, o)) => o,
        Err(_) => obj,
    }
}

fn number(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn numbers(doc: &Document, o: &Object) -> Vec<f32> {
    match deref(doc, o) {
        Object::Array(a) => a.iter().filter_map(|x| number(deref(doc, x))).collect(),
        _ => Vec::new(),
    }
}

/// Decodes a PDF text string: UTF-16BE with BOM, UTF-8 with BOM, else PDFDocEncoding read
/// as Latin-1 (identical for the printable ASCII range).
pub fn text_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    bytes.iter().map(|&b| b as char).collect()
}

fn string_of(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    match dict.get(key).ok().map(|o| deref(doc, o)) {
        Some(Object::String(b, _)) => Some(text_string(b)),
        _ => None,
    }
}

fn normalise_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn inherited<'a>(doc: &'a Document, mut dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..64 {
        if let Ok(o) = dict.get(key) {
            return Some(o);
        }
        let parent = dict.get(b"Parent").ok()?;
        dict = match deref(doc, parent) {
            Object::Dictionary(d) => d,
            _ => return None,
        };
    }
    None
}

/// The page box used for coordinate conversion.
pub fn page_box(doc: &Document, page_id: ObjectId) -> Result<PageBox> {
    let dict = doc.get_dictionary(page_id)?;
    let raw = inherited(doc, dict, b"CropBox")
        .map(|o| numbers(doc, o))
        .filter(|v| v.len() == 4)
        .or_else(|| inherited(doc, dict, b"MediaBox").map(|o| numbers(doc, o)).filter(|v| v.len() == 4))
        .unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
    Ok(PageBox {
        x0: raw[0].min(raw[2]),
        y0: raw[1].min(raw[3]),
        x1: raw[0].max(raw[2]),
        y1: raw[1].max(raw[3]),
    })
}

/// Page ids in page order (0-based index).
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.page_iter().collect()
}

fn annot_dicts<'a>(doc: &'a Document, page_id: ObjectId) -> Vec<Option<&'a Dictionary>> {
    let Ok(page) = doc.get_dictionary(page_id) else { return Vec::new() };
    let Ok(annots) = page.get(b"Annots") else { return Vec::new() };
    let Object::Array(items) = deref(doc, annots) else { return Vec::new() };
    items
        .iter()
        .map(|o| match deref(doc, o) {
            Object::Dictionary(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn kind_of(dict: &Dictionary) -> Option<AnnotKind> {
    let sub = dict.get(b"Subtype").ok()?.as_name().ok()?;
    Some(match sub {
        b"Highlight" => AnnotKind::Highlight,
        b"Underline" => AnnotKind::Underline,
        b"Squiggly" => AnnotKind::Squiggly,
        b"StrikeOut" => AnnotKind::StrikeOut,
        b"Text" => AnnotKind::Text,
        b"FreeText" => AnnotKind::FreeText,
        _ => return None,
    })
}

/// Reads every markup and note annotation of the document.
pub fn read_annotations(doc: &Document) -> Result<Vec<Annot>> {
    let mut out = Vec::new();
    for (page_no, page_id) in page_ids(doc).into_iter().enumerate() {
        let pbox = page_box(doc, page_id)?;
        let dicts = annot_dicts(doc, page_id);
        // Names of the page's annotations by index, for resolving /IRT references.
        let names: Vec<Option<String>> =
            dicts.iter().map(|d| d.and_then(|d| string_of(doc, d, b"NM"))).collect();
        let page_dict = doc.get_dictionary(page_id)?;
        let refs: Vec<Option<ObjectId>> = match page_dict.get(b"Annots").ok().map(|o| deref(doc, o)) {
            Some(Object::Array(items)) => items.iter().map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        };
        for (index, dict) in dicts.iter().enumerate() {
            let Some(dict) = dict else { continue };
            let Some(kind) = kind_of(dict) else { continue };
            let rect_raw = dict.get(b"Rect").ok().map(|o| numbers(doc, o)).unwrap_or_default();
            let rect = if rect_raw.len() == 4 {
                pbox.to_top_left(rect_raw[0], rect_raw[1], rect_raw[2], rect_raw[3])
            } else {
                [0.0, 0.0, 0.0, 0.0]
            };
            let qp = dict.get(b"QuadPoints").ok().map(|o| numbers(doc, o)).unwrap_or_default();
            let mut quads: Vec<Rect> = qp
                .chunks(8)
                .filter(|c| c.len() == 8)
                .map(|c| {
                    let xs = [c[0], c[2], c[4], c[6]];
                    let ys = [c[1], c[3], c[5], c[7]];
                    let minx = xs.iter().cloned().fold(f32::INFINITY, f32::min);
                    let maxx = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let miny = ys.iter().cloned().fold(f32::INFINITY, f32::min);
                    let maxy = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    pbox.to_top_left(minx, miny, maxx, maxy)
                })
                .filter(|r| r[2] > r[0] && r[3] > r[1])
                .collect();
            if quads.is_empty() {
                quads.push(rect);
            }
            let colour_raw = dict.get(b"C").ok().map(|o| numbers(doc, o)).unwrap_or_default();
            let colour = match colour_raw.len() {
                3 => Some([colour_raw[0], colour_raw[1], colour_raw[2]]),
                4 => {
                    // CMYK to RGB, the plain conversion.
                    let [c, m, y, k] = [colour_raw[0], colour_raw[1], colour_raw[2], colour_raw[3]];
                    Some([(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)])
                }
                1 => Some([colour_raw[0], colour_raw[0], colour_raw[0]]),
                _ => None,
            };
            let in_reply_to = dict.get(b"IRT").ok().and_then(|o| {
                let target = o.as_reference().ok()?;
                let pos = refs.iter().position(|r| *r == Some(target))?;
                names.get(pos).cloned().flatten().or_else(|| Some(format!("#{page_no}:{pos}")))
            });
            out.push(Annot {
                name: names[index].clone(),
                object: refs.get(index).copied().flatten().map(|id| id.0),
                page: page_no,
                index,
                kind,
                author: string_of(doc, dict, b"T").map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default(),
                contents: string_of(doc, dict, b"Contents").map(|s| normalise_newlines(&s)).unwrap_or_default(),
                colour,
                quads,
                rect,
                created: string_of(doc, dict, b"CreationDate").and_then(|s| pdf_date_to_iso(&s)),
                modified: string_of(doc, dict, b"M").and_then(|s| pdf_date_to_iso(&s)),
                in_reply_to,
            });
        }
    }
    Ok(out)
}

fn annot_target(doc: &mut Document, page: usize, index: usize) -> Result<AnnotTarget> {
    let ids = page_ids(doc);
    let page_id = *ids.get(page).ok_or(Error::PageOutOfRange { page, count: ids.len() })?;
    let page_dict = doc.get_dictionary(page_id)?;
    let annots = page_dict.get(b"Annots").map_err(|_| Error::Structure("page has no /Annots".into()))?;
    let (array_id, items) = match annots {
        Object::Reference(id) => (Some(*id), doc.get_object(*id)?.as_array()?),
        Object::Array(items) => (None, items),
        _ => return Err(Error::Structure("/Annots is neither an array nor a reference".into())),
    };
    let item = items
        .get(index)
        .ok_or_else(|| Error::Structure(format!("annotation {index} is missing on page {page}")))?;
    Ok(match item {
        Object::Reference(id) => AnnotTarget::Indirect(*id),
        Object::Dictionary(_) => AnnotTarget::Direct { page_id, array_id, index },
        _ => return Err(Error::Structure(format!("annotation {index} on page {page} is no dictionary"))),
    })
}

enum AnnotTarget {
    Indirect(ObjectId),
    Direct { page_id: ObjectId, array_id: Option<ObjectId>, index: usize },
}

fn with_annot_dict<F: FnOnce(&mut Dictionary)>(doc: &mut Document, target: AnnotTarget, f: F) -> Result<()> {
    match target {
        AnnotTarget::Indirect(id) => f(doc.get_dictionary_mut(id)?),
        AnnotTarget::Direct { page_id, array_id, index } => {
            let array = match array_id {
                Some(id) => doc.get_object_mut(id)?.as_array_mut()?,
                None => doc.get_dictionary_mut(page_id)?.get_mut(b"Annots")?.as_array_mut()?,
            };
            f(array[index].as_dict_mut()?)
        }
    }
    Ok(())
}

/// Turns every annotation dictionary written directly inside a page's /Annots array into
/// an indirect object. pdfium writes new annotations directly; PyMuPDF's `page.annots()`
/// lists annotations by xref and skips direct dictionaries.
fn hoist_direct_annotations(doc: &mut Document) {
    let page_ids = page_ids(doc);
    for page_id in page_ids {
        let array_ref = match doc.get_dictionary(page_id).ok().and_then(|d| d.get(b"Annots").ok()) {
            Some(Object::Reference(id)) => Some(*id),
            Some(Object::Array(_)) => None,
            _ => continue,
        };
        let items: Vec<Object> = match array_ref {
            Some(id) => match doc.get_object(id) {
                Ok(Object::Array(a)) => a.clone(),
                _ => continue,
            },
            None => match doc.get_dictionary(page_id).ok().and_then(|d| d.get(b"Annots").ok()) {
                Some(Object::Array(a)) => a.clone(),
                _ => continue,
            },
        };
        if !items.iter().any(|o| matches!(o, Object::Dictionary(_))) {
            continue;
        }
        let hoisted: Vec<Object> = items
            .into_iter()
            .map(|o| match o {
                Object::Dictionary(d) => Object::Reference(doc.add_object(Object::Dictionary(d))),
                other => other,
            })
            .collect();
        match array_ref {
            Some(id) => {
                if let Ok(obj) = doc.get_object_mut(id) {
                    *obj = Object::Array(hoisted);
                }
            }
            None => {
                if let Ok(d) = doc.get_dictionary_mut(page_id) {
                    d.set("Annots", Object::Array(hoisted));
                }
            }
        }
    }
}

/// A PDF text string: a literal when every character is ASCII, else UTF-16BE with a BOM in
/// hexadecimal form.
pub fn text_object(s: &str) -> Object {
    if s.is_ascii() {
        Object::string_literal(s)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

/// The /NM of every annotation on `page_id`, with its object id when it is indirect.
fn names_on_page(doc: &Document, page_id: ObjectId) -> Vec<(Option<String>, Option<ObjectId>)> {
    let Ok(page) = doc.get_dictionary(page_id) else { return Vec::new() };
    let Ok(annots) = page.get(b"Annots") else { return Vec::new() };
    let Object::Array(items) = deref(doc, annots) else { return Vec::new() };
    items
        .iter()
        .map(|o| {
            let id = o.as_reference().ok();
            let name = match deref(doc, o) {
                Object::Dictionary(d) => string_of(doc, d, b"NM"),
                _ => None,
            };
            (name, id)
        })
        .collect()
}

/// Builds the dictionary of `r` and appends it to the page's /Annots as an indirect object.
fn add_annotation(doc: &mut Document, r: &RestoreAnnot, name: &str) -> Result<ObjectId> {
    let ids = page_ids(doc);
    let page_id = *ids.get(r.page).ok_or(Error::PageOutOfRange { page: r.page, count: ids.len() })?;
    let pbox = page_box(doc, page_id)?;
    let is_markup = !matches!(r.subtype.as_str(), "Text" | "FreeText");
    let quads: Vec<Rect> = r.quads.iter().filter(|q| q[2] > q[0] || q[3] > q[1]).copied().collect();
    if quads.is_empty() {
        return Err(Error::NoQuads);
    }
    let union = quads.iter().fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |u, q| {
        [u[0].min(q[0]), u[1].min(q[1]), u[2].max(q[2]), u[3].max(q[3])]
    });
    let (l, b, rt, t) = pbox.to_pdf(union);
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(r.subtype.as_bytes().to_vec()));
    dict.set("Rect", Object::Array(vec![Object::Real(l), Object::Real(b), Object::Real(rt), Object::Real(t)]));
    dict.set("P", Object::Reference(page_id));
    dict.set("F", Object::Integer(4));
    dict.set("NM", text_object(name));
    dict.set("T", text_object(&r.author));
    dict.set("Contents", text_object(&r.contents));
    if is_markup {
        let mut qp = Vec::with_capacity(quads.len() * 8);
        for q in &quads {
            let (l, b, rt, t) = pbox.to_pdf(*q);
            for v in [l, t, rt, t, l, b, rt, b] {
                qp.push(Object::Real(v));
            }
        }
        dict.set("QuadPoints", Object::Array(qp));
    } else if r.subtype == "Text" {
        dict.set("Name", Object::Name(b"Note".to_vec()));
        dict.set("Open", Object::Boolean(false));
    }
    if let Some(c) = r.colour {
        dict.set("C", Object::Array(c.iter().map(|v| Object::Real(*v)).collect()));
    }
    if let Some(d) = &r.created {
        dict.set("CreationDate", Object::string_literal(d.as_str()));
    }
    if let Some(d) = &r.modified {
        dict.set("M", Object::string_literal(d.as_str()));
    }
    if let Some(parent) = &r.in_reply_to {
        if let Some((_, Some(id))) = names_on_page(doc, page_id).into_iter().find(|(n, _)| n.as_deref() == Some(parent.as_str())) {
            dict.set("IRT", Object::Reference(id));
        }
    }
    let annot_id = doc.add_object(Object::Dictionary(dict));
    let annots_ref = match doc.get_dictionary(page_id)?.get(b"Annots") {
        Ok(Object::Reference(id)) => Some(*id),
        _ => None,
    };
    match annots_ref {
        Some(id) => doc.get_object_mut(id)?.as_array_mut()?.push(Object::Reference(annot_id)),
        None => {
            let page = doc.get_dictionary_mut(page_id)?;
            match page.get_mut(b"Annots") {
                Ok(Object::Array(a)) => a.push(Object::Reference(annot_id)),
                _ => page.set("Annots", Object::Array(vec![Object::Reference(annot_id)])),
            }
        }
    }
    Ok(annot_id)
}

/// Applies the patches to the bytes pdfium saved and returns the new bytes. Field patches
/// go first; whole annotations are appended after them.
pub fn apply_patches(bytes: &[u8], patches: &[Patch]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(bytes)?;
    hoist_direct_annotations(&mut doc);
    for p in patches.iter().filter(|p| !p.is_add()) {
        let target = annot_target(&mut doc, p.page, p.index)?;
        let op = p.op.clone();
        with_annot_dict(&mut doc, target, |d| match op {
            PatchOp::SetName(name) => d.set("NM", Object::string_literal(name)),
            PatchOp::SetColour(rgb) => {
                d.set("C", Object::Array(rgb.iter().map(|v| Object::Real(*v)).collect()));
                d.remove(b"AP");
            }
            PatchOp::AddAnnot(_) => {}
        })?;
    }
    for p in patches.iter().filter(|p| p.is_add()) {
        if let PatchOp::AddAnnot(r) = &p.op {
            let name = r.name.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            add_annotation(&mut doc, r, &name)?;
        }
    }
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

/// The /NM of the annotation at (page, index), if the file carries one.
pub fn name_at(doc: &Document, page: usize, index: usize) -> Option<String> {
    let page_id = *page_ids(doc).get(page)?;
    let dicts = annot_dicts(doc, page_id);
    dicts.get(index).copied().flatten().and_then(|d| string_of(doc, d, b"NM"))
}
