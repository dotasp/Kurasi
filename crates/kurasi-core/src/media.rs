//! Image and Markdown helpers: thumbnails, sizes, text.
//!
//! Markdown has no raster thumbnail: the thumb route answers 404 and the UI draws a CSS thumb.

use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use regex::Regex;

use crate::error::{Error, Result};

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];
pub const MARKDOWN_EXTS: &[&str] = &["md"];

pub fn content_type(ext: &str) -> &'static str {
    match ext {
        "pdf" => "application/pdf",
        "md" => "text/markdown; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn decode(path: &Path) -> Result<DynamicImage> {
    let reader = ImageReader::open(path)?
        .with_guessed_format()
        .map_err(|e| Error::bad(format!("The image cannot be read: {}.", e)))?;
    let mut decoder = reader.into_decoder().map_err(|e| Error::bad(format!("The image cannot be decoded: {}.", e)))?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| Error::bad(format!("The image cannot be decoded: {}.", e)))?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Width and height as a browser shows the image (EXIF orientation applied).
pub fn image_size(path: &Path) -> Result<[f64; 2]> {
    let img = decode(path)?;
    Ok([img.width() as f64, img.height() as f64])
}

/// PNG thumbnail `width` pixels wide, aspect kept.
pub fn image_thumbnail_png(path: &Path, width: u32) -> Result<Vec<u8>> {
    let img = decode(path)?;
    let height = ((img.height() as f64 * width as f64 / img.width().max(1) as f64).round() as u32).max(1);
    let small = img.resize_exact(width.max(1), height, image::imageops::FilterType::Lanczos3);
    let mut out = Cursor::new(Vec::new());
    small.write_to(&mut out, ImageFormat::Png).map_err(|e| Error::Other(format!("PNG encoding failed: {}", e)))?;
    Ok(out.into_inner())
}

/// Reads a Markdown file as UTF-8 (BOM stripped), falling back to Windows-1252 bytes.
pub fn read_markdown(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&data);
    Ok(match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| cp1252(b)).collect(),
    })
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}',
        '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}',
        '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}',
        '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// A rough rendered-text view: link targets, emphasis marks, heading and list markers removed.
pub fn markdown_plain(text: &str) -> String {
    static HEAD: OnceLock<Regex> = OnceLock::new();
    static LIST: OnceLock<Regex> = OnceLock::new();
    static QUOTE: OnceLock<Regex> = OnceLock::new();
    static RULE: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    static MARKS: OnceLock<Regex> = OnceLock::new();
    let head = HEAD.get_or_init(|| Regex::new(r"^\s{0,3}#{1,6}\s+").unwrap());
    let list = LIST.get_or_init(|| Regex::new(r"^\s*([-*+]|\d+\.)\s+").unwrap());
    let quote = QUOTE.get_or_init(|| Regex::new(r"^\s*>\s?").unwrap());
    let rule = RULE.get_or_init(|| Regex::new(r"^\s*\|?\s*:?-{3,}").unwrap());
    let link = LINK.get_or_init(|| Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap());
    let marks = MARKS.get_or_init(|| Regex::new(r"(\*\*|__|\*|`|~~)").unwrap());
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = head.replace(raw, "");
        let line = list.replace(&line, "");
        let line = quote.replace(&line, "");
        if rule.is_match(&line) {
            continue;
        }
        let line = line.replace('|', " ");
        let line = link.replace_all(&line, "$1");
        let line = marks.replace_all(&line, "");
        out.push(line.into_owned());
    }
    crate::util::norm_space(&out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_plain_strips_markup() {
        let text = "# Notes\n\nRead [the copy](../x.pdf) and **bold** `code`.\n\n| a | b |\n| --- | --- |\n- item one\n> quoted";
        assert_eq!(markdown_plain(text), "Notes Read the copy and bold code. a b item one quoted");
    }

    #[test]
    fn cp1252_fallback_maps_high_bytes() {
        assert_eq!(cp1252(0x93), '\u{201C}');
        assert_eq!(cp1252(0xE9), 'é');
    }
}
