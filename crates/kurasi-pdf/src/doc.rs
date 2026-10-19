//! `Doc`: one opened PDF with the read and write operations of CORE_API.md.

use std::io::Cursor;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use image::{DynamicImage, ImageFormat, RgbImage};
use pdfium_render::prelude::*;

use crate::dict::{self, PageBox, Patch, PatchOp};
use crate::error::{Error, Result};
use crate::loader::Pdfium as Loader;
use crate::text::group_words;
use crate::types::{Annot, HighlightPatch, NewHighlight, PageInfo, RestoreAnnot, Word};

/// Attempts at renaming the temp file over the original.
pub const REPLACE_TRIES: u32 = 6;
/// Wait between two attempts.
pub const REPLACE_DELAY: Duration = Duration::from_millis(250);

/// Height of the thumbnail box relative to its width (the catalogue card ratio 1 : 1.3).
const THUMB_RATIO: f32 = 1.3;

/// pdfium is single threaded. pdfium-render serialises each call, which left pdfium's
/// internal state open between calls; running tests in parallel corrupted the heap. Every
/// operation of this crate therefore runs under one process-wide lock, as PyMuPDF calls do in
/// the Python build.
static PDFIUM_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    PDFIUM_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

pub(crate) fn cloud_only_attrs(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        match std::fs::metadata(path) {
            Ok(m) => m.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Opens the PDF at `path` into memory. Online-only placeholders are refused before any read.
pub fn open(path: &Path) -> Result<Doc> {
    if cloud_only_attrs(path) {
        return Err(Error::CloudOnly(path.to_path_buf()));
    }
    let bytes = std::fs::read(path)?;
    let _guard = lock();
    let pdf = load_bytes(bytes)?;
    Ok(Doc { path: path.to_path_buf(), pdf: ManuallyDrop::new(pdf), pending: Vec::new(), names: None, saved: None })
}

fn load_bytes(bytes: Vec<u8>) -> Result<PdfDocument<'static>> {
    let lib = Loader::get()?;
    Ok(lib.load_pdf_from_byte_vec(bytes, None)?)
}

/// One opened PDF. Reads of annotations reflect the file as saved; writes queue in memory
/// until `save_in_place`.
pub struct Doc {
    path: PathBuf,
    /// Closed in `Drop` under the pdfium lock: `FPDF_CloseDocument` must never run beside
    /// pdfium work on another thread, and a cached document may close on any request thread.
    pdf: ManuallyDrop<PdfDocument<'static>>,
    /// Field writes applied with lopdf after pdfium saves.
    pending: Vec<Patch>,
    /// Mirror of each page's /Annots order with the /NM of every entry, kept in step with
    /// the pdfium document so a name resolves to (page, index) before the next save.
    names: Option<Vec<Vec<Option<String>>>>,
    /// lopdf parse of the saved file, loaded on first use and dropped on save.
    saved: Option<lopdf::Document>,
}

impl std::fmt::Debug for Doc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Doc")
            .field("path", &self.path)
            .field("pages", &self.pdf.pages().len())
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl Doc {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn page_count_(&self) -> usize {
        self.pdf.pages().len() as usize
    }

    fn page(&self, page: usize) -> Result<PdfPage<'static>> {
        let count = self.page_count_();
        if page >= count {
            return Err(Error::PageOutOfRange { page, count });
        }
        let mut p = self.pdf.pages().get(page as PdfPageIndex)?;
        // Annotations and rotation never need the page content stream rewritten.
        p.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
        Ok(p)
    }

    fn page_box(page: &PdfPage<'_>) -> PageBox {
        match page.boundaries().bounding() {
            Ok(b) => PageBox {
                x0: b.bounds.left().value,
                y0: b.bounds.bottom().value,
                x1: b.bounds.right().value,
                y1: b.bounds.top().value,
            },
            Err(_) => PageBox { x0: 0.0, y0: 0.0, x1: page.width().value, y1: page.height().value },
        }
    }

    /// Unrotated page size and /Rotate of one page.
    fn page_info_(&self, page: usize) -> Result<PageInfo> {
        let p = self.page(page)?;
        let b = Self::page_box(&p);
        Ok(PageInfo { width: b.x1 - b.x0, height: b.y1 - b.y0, rotation: rotation_degrees(p.rotation()?) })
    }

    /// The document /Title and /Author, trimmed; empty when absent.
    fn metadata_(&self) -> (String, String) {
        let meta = self.pdf.metadata();
        let get = |tag: PdfDocumentMetadataTagType| meta.get(tag).map(|t| t.value().trim().to_string()).unwrap_or_default();
        (get(PdfDocumentMetadataTagType::Title), get(PdfDocumentMetadataTagType::Author))
    }

    // ------------------------------------------------------------------ rendering

    /// Renders one page at `scale` times 72 dpi to PNG, page rotation applied, annotations
    /// left out.
    fn render_png_scale_(&self, page: usize, scale: f32) -> Result<Vec<u8>> {
        let p = self.page(page)?;
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let config = PdfRenderConfig::new()
            .scale_page_by_factor(scale)
            .render_annotations(false)
            .render_form_data(false)
            .set_clear_color(PdfColor::WHITE);
        let image = p.render_with_config(&config)?.as_image()?;
        png_bytes(&image.to_rgb8())
    }

    /// The first page scaled to `width_px`, placed top-centre on a white portrait box of
    /// `width_px` by `1.3 * width_px`.
    fn thumbnail_png_(&self, width_px: u32) -> Result<Vec<u8>> {
        let width_px = width_px.max(1);
        let height_px = ((width_px as f32) * THUMB_RATIO).round().max(1.0) as u32;
        let mut canvas = RgbImage::from_pixel(width_px, height_px, image::Rgb([255, 255, 255]));
        if self.page_count_() > 0 {
            let p = self.page(0)?;
            let config = PdfRenderConfig::new()
                .set_target_width(width_px as Pixels)
                .render_annotations(false)
                .render_form_data(false)
                .set_clear_color(PdfColor::WHITE);
            let page_img = p.render_with_config(&config)?.as_image()?.to_rgb8();
            let x = (width_px as i64 - page_img.width() as i64) / 2;
            image::imageops::overlay(&mut canvas, &page_img, x, 0);
        }
        png_bytes(&canvas)
    }

    // ------------------------------------------------------------------ text

    /// Words of one page with boxes in top-left-origin points of the unrotated page.
    fn words_(&self, page: usize) -> Result<Vec<Word>> {
        let p = self.page(page)?;
        let pbox = Self::page_box(&p);
        let text = p.text()?;
        let chars = text.chars();
        let mut items = Vec::with_capacity(chars.len() as usize);
        for ch in chars.iter() {
            let c = ch.unicode_char();
            let bounds = ch.loose_bounds().or_else(|_| ch.tight_bounds()).ok();
            let rect = bounds
                .filter(|b| b.width().value > 0.0 || b.height().value > 0.0)
                .map(|b| pbox.to_top_left(b.left().value, b.bottom().value, b.right().value, b.top().value));
            items.push((c, rect));
        }
        Ok(group_words(items))
    }

    // ------------------------------------------------------------------ annotations

    fn saved(&mut self) -> Result<&lopdf::Document> {
        if self.saved.is_none() {
            let doc = lopdf::Document::load(&self.path)?;
            self.saved = Some(doc);
        }
        Ok(self.saved.as_ref().expect("loaded above"))
    }

    /// Every markup and note annotation of the file as last saved. Call `save_in_place`
    /// first to see this session's unsaved changes.
    fn annotations_(&mut self) -> Result<Vec<Annot>> {
        let doc = self.saved()?;
        dict::read_annotations(doc)
    }

    fn names(&mut self) -> Result<&mut Vec<Vec<Option<String>>>> {
        if self.names.is_none() {
            let count = self.page_count_();
            let doc = self.saved()?;
            let annots = dict::read_annotations(doc)?;
            let mut names: Vec<Vec<Option<String>>> = vec![Vec::new(); count];
            // read_annotations skips other subtypes, so sizes come from pdfium.
            for (page_no, list) in names.iter_mut().enumerate() {
                let n = self.pdf.pages().get(page_no as PdfPageIndex).map(|p| p.annotations().len() as usize).unwrap_or(0);
                list.resize(n, None);
            }
            for a in annots {
                if let Some(slot) = names.get_mut(a.page).and_then(|l| l.get_mut(a.index)) {
                    *slot = a.name;
                }
            }
            self.names = Some(names);
        }
        Ok(self.names.as_mut().expect("built above"))
    }

    fn locate(&mut self, name: &str) -> Result<(usize, usize)> {
        let names = self.names()?;
        for (page, list) in names.iter().enumerate() {
            if let Some(index) = list.iter().position(|n| n.as_deref() == Some(name)) {
                return Ok((page, index));
            }
        }
        Err(Error::AnnotationNotFound(name.to_string()))
    }

    fn is_fresh(&self, page: usize, index: usize) -> bool {
        self.pending
            .iter()
            .any(|p| p.page == page && p.index == index && matches!(p.op, PatchOp::SetName(_)))
    }

    /// Adds a highlight and returns its generated /NM (a UUID).
    fn add_highlight_(&mut self, h: &NewHighlight) -> Result<String> {
        if h.quads.is_empty() {
            return Err(Error::NoQuads);
        }
        let name = uuid::Uuid::new_v4().to_string();
        // Build the name mirror before pdfium appends, so the sizes match the saved file.
        self.names()?;
        let mut page = self.page(h.page)?;
        let pbox = Self::page_box(&page);
        let mut hl = page.annotations_mut().create_highlight_annotation()?;
        hl.set_creator(&h.author)?;
        hl.set_contents(&h.contents)?;
        hl.set_stroke_color(colour_of(h.colour))?;
        let mut union: Option<(f32, f32, f32, f32)> = None;
        for q in &h.quads {
            let (l, b, r, t) = pbox.to_pdf(*q);
            // Acrobat order: upper-left, upper-right, lower-left, lower-right. pdfium reads
            // /QuadPoints in this order when it builds the appearance.
            hl.attachment_points_mut().create_attachment_point_at_end(PdfQuadPoints::new(
                pt(l),
                pt(t),
                pt(r),
                pt(t),
                pt(l),
                pt(b),
                pt(r),
                pt(b),
            ))?;
            union = Some(match union {
                None => (l, b, r, t),
                Some(u) => (u.0.min(l), u.1.min(b), u.2.max(r), u.3.max(t)),
            });
        }
        let (l, b, r, t) = union.expect("at least one quad");
        hl.set_bounds(PdfRect::new(pt(b), pt(l), pt(t), pt(r)))?;
        let index = page.annotations().len() as usize - 1;
        drop(hl);
        drop(page);
        self.names()?[h.page].push(Some(name.clone()));
        self.pending.push(Patch { page: h.page, index, op: PatchOp::SetName(name.clone()) });
        Ok(name)
    }

    /// Changes contents, colour or author of the highlight named `name`.
    fn update_highlight_(&mut self, name: &str, patch: &HighlightPatch) -> Result<()> {
        let (page_no, index) = self.locate(name)?;
        let mut page = self.page(page_no)?;
        let mut annot = page
            .annotations_mut()
            .get(index as PdfPageAnnotationIndex)
            .map_err(|_| Error::AnnotationNotFound(name.to_string()))?;
        if let Some(c) = &patch.contents {
            annot.set_contents(c)?;
        }
        if let Some(a) = &patch.author {
            annot.set_creator(a)?;
        }
        if let Some(rgb) = patch.colour {
            if self.is_fresh(page_no, index) {
                // Created in this session, so it has no appearance stream and pdfium accepts
                // the colour directly.
                annot.set_stroke_color(colour_of(rgb))?;
            } else {
                // pdfium refuses /C on an annotation with an appearance stream, and
                // pdfium-render would then call a page-object function on the annotation
                // handle. The colour is written with lopdf after the save instead.
                self.pending.retain(|p| !(p.page == page_no && p.index == index && matches!(p.op, PatchOp::SetColour(_))));
                self.pending.push(Patch { page: page_no, index, op: PatchOp::SetColour(rgb) });
            }
        }
        Ok(())
    }

    /// Deletes the highlight named `name`.
    fn delete_highlight_(&mut self, name: &str) -> Result<()> {
        let (page_no, index) = self.locate(name)?;
        let mut page = self.page(page_no)?;
        let annot = page
            .annotations_mut()
            .get(index as PdfPageAnnotationIndex)
            .map_err(|_| Error::AnnotationNotFound(name.to_string()))?;
        page.annotations_mut().delete_annotation(annot)?;
        drop(page);
        self.names()?[page_no].remove(index);
        self.pending.retain(|p| p.is_add() || !(p.page == page_no && p.index == index));
        for p in self.pending.iter_mut() {
            if !p.is_add() && p.page == page_no && p.index > index {
                p.index -= 1;
            }
        }
        Ok(())
    }

    /// Queues a whole annotation (any subtype) for the next save and returns its /NM. The
    /// annotation is written with lopdf after pdfium has saved, so it answers `annotations`
    /// and name lookups only after `save_in_place`.
    fn restore_annotation_(&mut self, r: &RestoreAnnot) -> Result<String> {
        let count = self.page_count_();
        if r.page >= count {
            return Err(Error::PageOutOfRange { page: r.page, count });
        }
        if r.quads.iter().all(|q| q[2] <= q[0] && q[3] <= q[1]) {
            return Err(Error::NoQuads);
        }
        let mut r = r.clone();
        let name = r.name.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        r.name = Some(name.clone());
        self.pending.push(Patch { page: r.page, index: usize::MAX, op: PatchOp::AddAnnot(Box::new(r)) });
        Ok(name)
    }

    // ------------------------------------------------------------------ rotation

    /// Turns the given pages (every page when `pages` is empty) by `degrees`, a multiple of 90.
    fn rotate_(&mut self, pages: &[usize], degrees: i32) -> Result<()> {
        if degrees % 90 != 0 {
            return Err(Error::BadRotation);
        }
        let targets: Vec<usize> = if pages.is_empty() { (0..self.page_count_()).collect() } else { pages.to_vec() };
        for &i in &targets {
            let mut p = self.page(i)?;
            let current = rotation_degrees(p.rotation()?);
            p.set_rotation(rotation_from_degrees((current + degrees).rem_euclid(360)));
        }
        Ok(())
    }

    // ------------------------------------------------------------------ saving

    /// The temp file written before the rename: `<file>.kurasi-tmp` in the same folder.
    pub fn temp_path(&self) -> PathBuf {
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.path.with_file_name(format!("{name}.kurasi-tmp"))
    }

    /// Saves through pdfium, applies the queued field patches with lopdf, verifies the
    /// result, writes `<file>.kurasi-tmp` and renames it over the original with
    /// `REPLACE_TRIES` attempts `REPLACE_DELAY` apart. On failure the original is untouched
    /// and the temp file is removed.
    fn save_in_place_(&mut self) -> Result<()> {
        let mut bytes = self.pdf.save_to_bytes()?;
        if !self.pending.is_empty() {
            bytes = dict::apply_patches(&bytes, &self.pending)?;
        }
        // Verify: pdfium parses the result with the same page count, and every pending name
        // is in place.
        let check = load_bytes(bytes.clone()).map_err(|_| Error::NotVerified)?;
        if check.pages().len() != self.pdf.pages().len() {
            return Err(Error::NotVerified);
        }
        let parsed = lopdf::Document::load_mem(&bytes).map_err(|_| Error::NotVerified)?;
        for p in &self.pending {
            if let PatchOp::SetName(n) = &p.op {
                if dict::name_at(&parsed, p.page, p.index).as_deref() != Some(n.as_str()) {
                    return Err(Error::NotVerified);
                }
            }
        }
        let tmp = self.temp_path();
        let written = std::fs::write(&tmp, &bytes)
            .map_err(Error::Io)
            .and_then(|_| replace_with_retry(&tmp, &self.path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        // The old document closes here, under the lock `save_in_place` holds.
        *self.pdf = check;
        self.pending.clear();
        self.names = None;
        self.saved = Some(parsed);
        Ok(())
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        let _g = lock();
        // SAFETY: `pdf` is never used again; `Drop` runs once.
        unsafe { ManuallyDrop::drop(&mut self.pdf) };
    }
}

/// The public operations. Each one holds the process-wide pdfium lock for its duration.
impl Doc {
    pub fn page_count(&self) -> usize {
        let _g = lock();
        self.page_count_()
    }

    /// Unrotated page size and /Rotate of one page.
    pub fn page_info(&self, page: usize) -> Result<PageInfo> {
        let _g = lock();
        self.page_info_(page)
    }

    /// Unrotated size and /Rotate of every page, under one hold of the lock, so a walk over a
    /// long document is never interleaved with page requests of other threads.
    pub fn page_infos(&self) -> Result<Vec<PageInfo>> {
        let _g = lock();
        (0..self.page_count_()).map(|i| self.page_info_(i)).collect()
    }

    /// The document /Title and /Author, trimmed; empty when absent.
    pub fn metadata(&self) -> (String, String) {
        let _g = lock();
        self.metadata_()
    }

    /// Renders one page at `dpi` to PNG, page rotation applied, annotations left out.
    pub fn render_png(&self, page: usize, dpi: u32) -> Result<Vec<u8>> {
        let _g = lock();
        self.render_png_scale_(page, dpi.max(1) as f32 / 72.0)
    }

    /// Renders one page at `scale` times 72 dpi to PNG (1.0 is 72 dpi).
    pub fn render_png_scale(&self, page: usize, scale: f32) -> Result<Vec<u8>> {
        let _g = lock();
        self.render_png_scale_(page, scale)
    }

    /// The first page scaled to `width_px`, placed top-centre on a white portrait box of
    /// `width_px` by `1.3 * width_px`.
    pub fn thumbnail_png(&self, width_px: u32) -> Result<Vec<u8>> {
        let _g = lock();
        self.thumbnail_png_(width_px)
    }

    /// Words of one page with boxes in top-left-origin points of the unrotated page.
    pub fn words(&self, page: usize) -> Result<Vec<Word>> {
        let _g = lock();
        self.words_(page)
    }

    /// Every markup and note annotation of the file as last saved. Call `save_in_place`
    /// first to see this session's unsaved changes.
    pub fn annotations(&mut self) -> Result<Vec<Annot>> {
        let _g = lock();
        self.annotations_()
    }

    /// Adds a highlight and returns its generated /NM (a UUID).
    pub fn add_highlight(&mut self, h: &NewHighlight) -> Result<String> {
        let _g = lock();
        self.add_highlight_(h)
    }

    /// Changes contents, colour or author of the highlight named `name`.
    pub fn update_highlight(&mut self, name: &str, patch: &HighlightPatch) -> Result<()> {
        let _g = lock();
        self.update_highlight_(name, patch)
    }

    /// Deletes the highlight named `name`.
    pub fn delete_highlight(&mut self, name: &str) -> Result<()> {
        let _g = lock();
        self.delete_highlight_(name)
    }

    /// Queues a whole annotation (any subtype) for the next save and returns its /NM.
    pub fn restore_annotation(&mut self, r: &RestoreAnnot) -> Result<String> {
        let _g = lock();
        self.restore_annotation_(r)
    }

    /// Turns the given pages (every page when `pages` is empty) by `degrees`, a multiple of 90.
    pub fn rotate(&mut self, pages: &[usize], degrees: i32) -> Result<()> {
        let _g = lock();
        self.rotate_(pages, degrees)
    }

    /// Saves through pdfium, applies the queued field patches with lopdf, verifies the
    /// result, writes `<file>.kurasi-tmp` and renames it over the original with
    /// `REPLACE_TRIES` attempts `REPLACE_DELAY` apart. On failure the original is untouched
    /// and the temp file is removed.
    pub fn save_in_place(&mut self) -> Result<()> {
        let _g = lock();
        self.save_in_place_()
    }
}

fn replace_with_retry(src: &Path, dst: &Path) -> Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(src, dst) {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) => {
                attempt += 1;
                if attempt >= REPLACE_TRIES {
                    return Err(Error::FileBusy);
                }
                std::thread::sleep(REPLACE_DELAY);
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
}

/// Access denied, sharing violation and lock violation: the file is held by another program.
fn is_busy(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(32) | Some(33)) || e.kind() == std::io::ErrorKind::PermissionDenied
}

fn png_bytes(img: &RgbImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(img.clone()).write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

fn pt(v: f32) -> PdfPoints {
    PdfPoints::new(v)
}

fn colour_of(rgb: [f32; 3]) -> PdfColor {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    PdfColor::new(ch(rgb[0]), ch(rgb[1]), ch(rgb[2]), 255)
}

fn rotation_degrees(r: PdfPageRenderRotation) -> i32 {
    match r {
        PdfPageRenderRotation::None => 0,
        PdfPageRenderRotation::Degrees90 => 90,
        PdfPageRenderRotation::Degrees180 => 180,
        PdfPageRenderRotation::Degrees270 => 270,
    }
}

fn rotation_from_degrees(d: i32) -> PdfPageRenderRotation {
    match d.rem_euclid(360) {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    }
}
