//! Data types of the kurasi-pdf interface.
//!
//! Coordinates are PDF points in the unrotated page with the origin at the top-left corner
//! of the page box (CropBox, else MediaBox), the frame PyMuPDF uses for `get_text("words")`
//! and annotation vertices. Page numbers are 0-based indexes throughout this crate.

/// `[x0, y0, x1, y1]` in top-left-origin page points.
pub type Rect = [f32; 4];

/// One word of page text with its bounding box.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Text line of the page, counted from 0 in reading order. A new line starts when a word
    /// no longer overlaps the running line vertically or jumps back to the left.
    pub line: u32,
}

impl Word {
    pub fn rect(&self) -> Rect {
        [self.x0, self.y0, self.x1, self.y1]
    }
}

/// Annotation subtypes the catalogue reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotKind {
    Highlight,
    Underline,
    Squiggly,
    StrikeOut,
    Text,
    FreeText,
}

impl AnnotKind {
    /// Highlight, Underline, Squiggly and StrikeOut.
    pub fn is_markup(self) -> bool {
        !matches!(self, AnnotKind::Text | AnnotKind::FreeText)
    }

    /// The PDF /Subtype name.
    pub fn subtype(self) -> &'static str {
        match self {
            AnnotKind::Highlight => "Highlight",
            AnnotKind::Underline => "Underline",
            AnnotKind::Squiggly => "Squiggly",
            AnnotKind::StrikeOut => "StrikeOut",
            AnnotKind::Text => "Text",
            AnnotKind::FreeText => "FreeText",
        }
    }
}

/// One annotation as stored in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Annot {
    /// The /NM name. `None` when the file carries none.
    pub name: Option<String>,
    /// Object number of the annotation dictionary when it is an indirect object (PyMuPDF
    /// calls it the xref). `None` for a dictionary written directly into /Annots.
    pub object: Option<u32>,
    /// 0-based page index.
    pub page: usize,
    /// Position in the page's /Annots array.
    pub index: usize,
    pub kind: AnnotKind,
    /// The /T field with runs of whitespace collapsed. Empty when absent.
    pub author: String,
    /// The /Contents field with line ends normalised to `\n`. Empty when absent.
    pub contents: String,
    /// The /C colour as RGB in 0..1. `None` when absent.
    pub colour: Option<[f32; 3]>,
    /// One rectangle per quad of /QuadPoints; the /Rect when there are no quads.
    pub quads: Vec<Rect>,
    /// The /Rect.
    pub rect: Rect,
    /// /CreationDate as ISO 8601 (`YYYY-MM-DDTHH:MM:SS`).
    pub created: Option<String>,
    /// /M as ISO 8601.
    pub modified: Option<String>,
    /// The /NM of the annotation this one replies to (/IRT), or `#page:index` when the
    /// parent has no name.
    pub in_reply_to: Option<String>,
}

/// A highlight to add.
#[derive(Debug, Clone, PartialEq)]
pub struct NewHighlight {
    /// 0-based page index.
    pub page: usize,
    /// At least one rectangle in top-left-origin page points.
    pub quads: Vec<Rect>,
    /// RGB in 0..1.
    pub colour: [f32; 3],
    /// Written to /T.
    pub author: String,
    /// Written to /Contents.
    pub contents: String,
}

/// Fields of a highlight to change. `None` leaves the field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HighlightPatch {
    pub contents: Option<String>,
    pub colour: Option<[f32; 3]>,
    pub author: Option<String>,
}

/// An annotation to write back as a whole dictionary (undo of a deletion). It is written
/// with lopdf after the next save, so any subtype can be restored, and it becomes
/// addressable by name once `save_in_place` has run.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreAnnot {
    /// 0-based page index.
    pub page: usize,
    /// The /Subtype name: Highlight, Underline, Squiggly, StrikeOut, Text or FreeText.
    pub subtype: String,
    /// Rectangles in top-left-origin page points; markup subtypes get them as /QuadPoints.
    pub quads: Vec<Rect>,
    /// Written to /T.
    pub author: String,
    /// Written to /Contents.
    pub contents: String,
    /// RGB in 0..1, written to /C.
    pub colour: Option<[f32; 3]>,
    /// /CreationDate in PDF form (`D:YYYYMMDDHHmmSS`).
    pub created: Option<String>,
    /// /M in PDF form.
    pub modified: Option<String>,
    /// The /NM to write; a UUID is generated when `None`.
    pub name: Option<String>,
    /// The /NM of the annotation this one replies to (/IRT), resolved at save time.
    pub in_reply_to: Option<String>,
}

/// Size and rotation of one page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageInfo {
    /// Width of the unrotated page box in points.
    pub width: f32,
    /// Height of the unrotated page box in points.
    pub height: f32,
    /// The /Rotate value: 0, 90, 180 or 270.
    pub rotation: i32,
}
