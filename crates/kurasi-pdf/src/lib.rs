//! PDF access for Kurasi through pdfium (`pdfium-render`, dynamic binding to `pdfium.dll`).
//!
//! What this crate does:
//! - opens a PDF into memory (no file handle stays open on the file),
//! - renders pages and thumbnails to PNG,
//! - extracts words with boxes in PDF points (top-left origin, like PyMuPDF),
//! - reads highlight and note annotations (/NM, /T, /Contents, /C, /QuadPoints, /M),
//! - adds, edits and deletes highlights, rotates pages,
//! - saves to `<file>.kurasi-tmp` and renames it over the original with six tries.
//!
//! Where `pdfium-render` has no setter (the /NM name) or where pdfium refuses a write (the
//! colour of an annotation that carries an appearance stream), the field is patched with
//! `lopdf` on the bytes pdfium saved. Annotation reads go through `lopdf` as well, so every
//! field is read exactly as the file stores it.
//!
//! Page numbers are 0-based indexes in every function of this crate. [`backend`] adapts
//! `Doc` to `kurasi_core::pdf::PdfBackend` (1-based pages, integer annotation ids).

pub mod backend;
pub mod dates;
mod dict;
mod doc;
pub mod error;
mod loader;
mod text;
pub mod types;

pub use backend::PdfiumBackend;
pub use doc::{open, Doc, REPLACE_DELAY, REPLACE_TRIES};
pub use error::{Error, Result};
pub use loader::Pdfium;
pub use types::{Annot, AnnotKind, HighlightPatch, NewHighlight, PageInfo, Rect, RestoreAnnot, Word};

/// `true` when the file is an online-only placeholder (RECALL_ON_DATA_ACCESS or OFFLINE).
/// Opening such a file would make OneDrive download it, so callers check this first.
pub fn is_cloud_only(path: &std::path::Path) -> bool {
    doc::cloud_only_attrs(path)
}
