use std::path::PathBuf;

/// Errors of the kurasi-pdf crate. The `Display` text is written for the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pdfium.dll was not found. Looked in: {0}")]
    LibraryNotFound(String),

    #[error("pdfium.dll could not be loaded: {0}")]
    Library(String),

    #[error("The file is online-only in OneDrive, so it stays unopened: {0}")]
    CloudOnly(PathBuf),

    #[error("The PDF cannot be opened ({0}). Check that the file is a valid PDF.")]
    BadPdf(String),

    #[error("The PDF is password protected. Remove the password to use it here.")]
    Encrypted,

    #[error("Page {page} does not exist; the PDF has {count} pages.")]
    PageOutOfRange { page: usize, count: usize },

    #[error("The annotation {0} is gone from the PDF. Scan the folder to refresh the highlights.")]
    AnnotationNotFound(String),

    #[error("A PDF highlight needs at least one quad [x0, y0, x1, y1].")]
    NoQuads,

    #[error("Rotation must be a multiple of 90 degrees.")]
    BadRotation,

    #[error("The file is open in another program. Close it and try again.")]
    FileBusy,

    #[error("The PDF could not be verified after saving, so it was left unchanged. Try again, or close other programs that use the file.")]
    NotVerified,

    #[error("pdfium failed: {0}")]
    Pdfium(String),

    #[error("PDF structure error: {0}")]
    Structure(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PNG encoding failed: {0}")]
    Image(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<pdfium_render::prelude::PdfiumError> for Error {
    fn from(e: pdfium_render::prelude::PdfiumError) -> Self {
        use pdfium_render::prelude::{PdfiumError as E, PdfiumInternalError as I};
        match e {
            E::PdfiumLibraryInternalError(I::PasswordError) => Error::Encrypted,
            E::PdfiumLibraryInternalError(I::SecurityError) => Error::Encrypted,
            E::PdfiumLibraryInternalError(I::FormatError) => Error::BadPdf("format error".into()),
            E::PdfiumLibraryInternalError(I::FileError) => Error::BadPdf("file error".into()),
            E::IoError(io) => Error::Io(io),
            other => Error::Pdfium(format!("{other:?}")),
        }
    }
}

impl From<lopdf::Error> for Error {
    fn from(e: lopdf::Error) -> Self {
        Error::Structure(e.to_string())
    }
}

impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self {
        Error::Image(e.to_string())
    }
}
