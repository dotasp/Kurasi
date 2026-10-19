//! A small PDF writer and reader for files Kurasi generates itself: the sterile sample project
//! and the test fixtures. It needs no PDF library.
//!
//! The format is deliberately narrow: Courier text lines, one content stream per page, and
//! Highlight or Text annotations with author, contents, colour, dates and quad points. Every
//! file carries the producer line `Kurasi simple PDF`; the reader refuses any other PDF with
//! `Error::Unsupported`, so the pdfium backend in `kurasi-pdf` handles real documents.
//!
//! Courier is monospaced (600/1000 em per glyph), so word boxes are exact without font metrics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use regex::Regex;

use crate::error::{Error, Result};
use crate::lock::{replace_retry, temp_path_for};
use crate::pdf::{
    colour_hex, hex_rgb, pdf_date, rect_list, to_pdf_date, Annot, AnnotPatch, AnnotSnapshot, NewAnnot, PageInfo,
    PdfBackend, PdfDoc, PdfMetadata, Word,
};

pub const PRODUCER: &str = "Kurasi simple PDF";
const GLYPH: f64 = 0.6;
const ASCENT: f64 = 0.8;
const DESCENT: f64 = 0.2;

#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Left edge in points.
    pub x: f64,
    /// Baseline from the top of the page in points.
    pub baseline: f64,
    pub size: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimplePage {
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub lines: Vec<TextLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleAnnot {
    pub obj: i64,
    /// 0-based page index.
    pub page: usize,
    pub subtype: String,
    /// `[x0, y0, x1, y1]` in top-left page points.
    pub quads: Vec<[f64; 4]>,
    pub author: String,
    pub contents: String,
    pub colour: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub irt: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleDoc {
    pub title: String,
    pub author: String,
    pub pages: Vec<SimplePage>,
    pub annots: Vec<SimpleAnnot>,
    next_obj: i64,
    path: Option<PathBuf>,
}

impl Default for SimpleDoc {
    fn default() -> Self {
        SimpleDoc::new()
    }
}

impl SimpleDoc {
    pub fn new() -> SimpleDoc {
        SimpleDoc { title: String::new(), author: String::new(), pages: Vec::new(), annots: Vec::new(), next_obj: 0, path: None }
    }

    /// Adds an A4 portrait page (595 x 842 points) and returns its 0-based index.
    pub fn add_page(&mut self) -> usize {
        self.add_page_sized(595.0, 842.0)
    }

    pub fn add_page_sized(&mut self, width: f64, height: f64) -> usize {
        self.pages.push(SimplePage { width, height, rotation: 0, lines: Vec::new() });
        self.pages.len() - 1
    }

    /// One line of text; `baseline` is measured from the top of the page.
    pub fn text(&mut self, page: usize, x: f64, baseline: f64, size: f64, text: &str) {
        self.pages[page].lines.push(TextLine { x, baseline, size, text: text.to_string() });
    }

    /// A centred line.
    pub fn text_centered(&mut self, page: usize, baseline: f64, size: f64, text: &str) {
        let w = self.pages[page].width;
        let tw = text.chars().count() as f64 * GLYPH * size;
        self.text(page, ((w - tw) / 2.0).max(0.0), baseline, size, text);
    }

    /// Wraps paragraphs into lines of at most `width` points and returns the next free baseline.
    pub fn paragraphs(&mut self, page: usize, x: f64, top: f64, width: f64, size: f64, paras: &[String]) -> f64 {
        let max_chars = ((width / (GLYPH * size)).floor() as usize).max(1);
        let leading = size * 1.45;
        let mut y = top + size * ASCENT;
        for (i, para) in paras.iter().enumerate() {
            if i > 0 {
                y += leading * 0.6;
            }
            for line in wrap(para, max_chars) {
                self.text(page, x, y, size, &line);
                y += leading;
            }
        }
        y
    }

    fn page_or_err(&self, page: usize) -> Result<&SimplePage> {
        if page == 0 || page > self.pages.len() {
            return Err(Error::not_found(format!("Page {} does not exist; the PDF has {} pages.", page, self.pages.len())));
        }
        Ok(&self.pages[page - 1])
    }

    fn page_words(p: &SimplePage) -> Vec<Word> {
        let mut out = Vec::new();
        for (li, line) in p.lines.iter().enumerate() {
            let step = GLYPH * line.size;
            let mut idx = 0usize;
            for piece in line.text.split(' ') {
                let len = piece.chars().count();
                if len > 0 {
                    out.push(Word {
                        x0: round2(line.x + step * idx as f64),
                        y0: round2(line.baseline - ASCENT * line.size),
                        x1: round2(line.x + step * (idx + len) as f64),
                        y1: round2(line.baseline + DESCENT * line.size),
                        text: piece.to_string(),
                        block: 0,
                        line: li as u32,
                    });
                }
                idx += len + 1;
            }
        }
        out
    }

    fn annot_record(&self, a: &SimpleAnnot) -> Annot {
        Annot {
            xref: a.obj,
            page: a.page + 1,
            subtype: a.subtype.clone(),
            quads: rect_list(&a.quads),
            author: a.author.clone(),
            contents: a.contents.clone(),
            colour: a.colour.clone(),
            created: a.created.clone(),
            modified: a.modified.clone(),
            irt_xref: a.irt,
        }
    }

    fn alloc_obj(&mut self) -> i64 {
        let base = 5 + 2 * self.pages.len() as i64;
        let used = self.annots.iter().map(|a| a.obj).max().unwrap_or(0);
        let n = self.next_obj.max(base).max(used + 1);
        self.next_obj = n + 1;
        n
    }

    /// Finds `quote` on a page (1-based): quads per line and the verbatim text.
    pub fn find_quote(&self, page: usize, quote: &str) -> Result<Option<(Vec<Vec<f64>>, String)>> {
        let p = self.page_or_err(page)?;
        Ok(crate::pdf::find_quote(&Self::page_words(p), quote))
    }

    // ------------------------------------------------------------ serialisation

    pub fn to_bytes(&self) -> Vec<u8> {
        let npages = self.pages.len();
        let mut objects: BTreeMap<i64, Vec<u8>> = BTreeMap::new();
        let kids: Vec<String> = (0..npages).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
        objects.insert(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        objects.insert(2, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), npages).into_bytes());
        objects.insert(3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec());
        let mut info = format!("<< /Producer {} /KurasiNext {}", pdf_string(PRODUCER), self.next_obj);
        if !self.title.is_empty() {
            info.push_str(&format!(" /Title {}", pdf_string(&self.title)));
        }
        if !self.author.is_empty() {
            info.push_str(&format!(" /Author {}", pdf_string(&self.author)));
        }
        info.push_str(" >>");
        objects.insert(4, info.into_bytes());
        for (i, p) in self.pages.iter().enumerate() {
            let page_obj = 5 + 2 * i as i64;
            let content_obj = page_obj + 1;
            let annots: Vec<String> =
                self.annots.iter().filter(|a| a.page == i).map(|a| format!("{} 0 R", a.obj)).collect();
            let mut page = format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R",
                fmt_num(p.width),
                fmt_num(p.height),
                content_obj
            );
            if p.rotation != 0 {
                page.push_str(&format!(" /Rotate {}", p.rotation));
            }
            if !annots.is_empty() {
                page.push_str(&format!(" /Annots [{}]", annots.join(" ")));
            }
            page.push_str(" >>");
            objects.insert(page_obj, page.into_bytes());
            let mut stream = Vec::new();
            for line in &p.lines {
                let y = p.height - line.baseline;
                stream.extend_from_slice(
                    format!("BT /F1 {} Tf {} {} Td (", fmt_num(line.size), fmt_num(line.x), fmt_num(y)).as_bytes(),
                );
                stream.extend_from_slice(&content_text(&line.text));
                stream.extend_from_slice(b") Tj ET\n");
            }
            let mut content = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
            content.extend_from_slice(&stream);
            content.extend_from_slice(b"\nendstream");
            objects.insert(content_obj, content);
        }
        for a in &self.annots {
            let p = &self.pages[a.page];
            let mut body = format!("<< /Type /Annot /Subtype /{} /P {} 0 R", a.subtype, 5 + 2 * a.page);
            let rect = union(&a.quads);
            body.push_str(&format!(
                " /Rect [{} {} {} {}]",
                fmt_num(rect[0]),
                fmt_num(p.height - rect[3]),
                fmt_num(rect[2]),
                fmt_num(p.height - rect[1])
            ));
            if a.subtype != "Text" && a.subtype != "FreeText" {
                let mut qp = Vec::new();
                for q in &a.quads {
                    let (top, bottom) = (p.height - q[1], p.height - q[3]);
                    for v in [q[0], top, q[2], top, q[0], bottom, q[2], bottom] {
                        qp.push(fmt_num(v));
                    }
                }
                body.push_str(&format!(" /QuadPoints [{}]", qp.join(" ")));
            } else {
                body.push_str(" /Name /Note /Open false");
            }
            body.push_str(&format!(" /NM {}", pdf_string(&format!("kurasi-{}", a.obj))));
            body.push_str(&format!(" /T {}", pdf_string(&a.author)));
            body.push_str(&format!(" /Contents {}", pdf_string(&a.contents)));
            if let Some(c) = &a.colour {
                let (r, g, b) = hex_rgb(c);
                body.push_str(&format!(" /C [{} {} {}]", fmt_num(r), fmt_num(g), fmt_num(b)));
            }
            if let Some(d) = a.created.as_deref().and_then(to_pdf_date) {
                body.push_str(&format!(" /CreationDate ({})", d));
            }
            if let Some(d) = a.modified.as_deref().and_then(to_pdf_date) {
                body.push_str(&format!(" /M ({})", d));
            }
            if a.irt != 0 {
                body.push_str(&format!(" /IRT {} 0 R", a.irt));
            }
            body.push_str(" /F 4 >>");
            objects.insert(a.obj, body.into_bytes());
        }
        let max_obj = *objects.keys().max().unwrap_or(&4);
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets: BTreeMap<i64, usize> = BTreeMap::new();
        for (num, body) in &objects {
            offsets.insert(*num, out.len());
            out.extend_from_slice(format!("{} 0 obj\n", num).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_pos = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", max_obj + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for n in 1..=max_obj {
            match offsets.get(&n) {
                Some(off) => out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes()),
                None => out.extend_from_slice(b"0000000000 65535 f \n"),
            }
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{}\n%%EOF\n", max_obj + 1, xref_pos)
                .as_bytes(),
        );
        out
    }

    pub fn write_to(&mut self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_bytes())?;
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Parses a file this writer produced. Any other PDF gives `Error::Unsupported`.
    pub fn parse(bytes: &[u8]) -> Result<SimpleDoc> {
        if !bytes.starts_with(b"%PDF") {
            return Err(Error::bad("The PDF cannot be opened. Check that the file is a valid PDF."));
        }
        let text = latin1(bytes);
        if !text.contains(PRODUCER) {
            return Err(Error::unsupported("This PDF was made by another program; the built-in reader handles Kurasi sample files only."));
        }
        static OBJ: OnceLock<Regex> = OnceLock::new();
        let obj_re = OBJ.get_or_init(|| Regex::new(r"(?s)(\d+) 0 obj\s*(.*?)\s*endobj").unwrap());
        let mut objects: BTreeMap<i64, String> = BTreeMap::new();
        for caps in obj_re.captures_iter(&text) {
            if let Ok(n) = caps[1].parse::<i64>() {
                objects.insert(n, caps[2].to_string());
            }
        }
        let mut doc = SimpleDoc::new();
        if let Some(info) = objects.get(&4) {
            doc.title = named_string(info, "Title").unwrap_or_default();
            doc.author = named_string(info, "Author").unwrap_or_default();
            doc.next_obj = named_int(info, "KurasiNext").unwrap_or(0);
        }
        let kids: Vec<i64> = objects
            .get(&2)
            .and_then(|s| capture(s, r"/Kids\s*\[([^\]]*)\]"))
            .map(|k| {
                Regex::new(r"(\d+) 0 R").unwrap().captures_iter(&k).filter_map(|c| c[1].parse().ok()).collect()
            })
            .unwrap_or_default();
        let mut page_index: BTreeMap<i64, usize> = BTreeMap::new();
        for obj in kids {
            let body = match objects.get(&obj) {
                Some(b) => b,
                None => continue,
            };
            let mb = capture(body, r"/MediaBox\s*\[([^\]]*)\]").unwrap_or_default();
            let nums: Vec<f64> = mb.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let (width, height) = if nums.len() == 4 { (nums[2] - nums[0], nums[3] - nums[1]) } else { (595.0, 842.0) };
            let rotation = capture(body, r"/Rotate\s+(-?\d+)").and_then(|r| r.parse().ok()).unwrap_or(0);
            let idx = doc.add_page_sized(width, height);
            doc.pages[idx].rotation = rotation;
            page_index.insert(obj, idx);
            if let Some(cobj) = capture(body, r"/Contents\s+(\d+) 0 R").and_then(|c| c.parse::<i64>().ok()) {
                if let Some(content) = objects.get(&cobj) {
                    static LINE: OnceLock<Regex> = OnceLock::new();
                    let line_re = LINE.get_or_init(|| {
                        Regex::new(r"BT /F1 ([\d.]+) Tf ([\d.-]+) ([\d.-]+) Td \(((?:[^()\\]|\\.)*)\) Tj ET").unwrap()
                    });
                    for caps in line_re.captures_iter(content) {
                        let size: f64 = caps[1].parse().unwrap_or(10.0);
                        let x: f64 = caps[2].parse().unwrap_or(0.0);
                        let y: f64 = caps[3].parse().unwrap_or(0.0);
                        doc.pages[idx].lines.push(TextLine { x, baseline: height - y, size, text: unescape_literal(&caps[4]) });
                    }
                }
            }
        }
        for (num, body) in &objects {
            if !body.contains("/Type /Annot") {
                continue;
            }
            let subtype = capture(body, r"/Subtype\s*/(\w+)").unwrap_or_default();
            let page_obj = capture(body, r"/P\s+(\d+) 0 R").and_then(|p| p.parse::<i64>().ok()).unwrap_or(0);
            let page = match page_index.get(&page_obj) {
                Some(p) => *p,
                None => continue,
            };
            let height = doc.pages[page].height;
            let mut quads: Vec<[f64; 4]> = Vec::new();
            if let Some(qp) = capture(body, r"/QuadPoints\s*\[([^\]]*)\]") {
                let nums: Vec<f64> = qp.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                for chunk in nums.chunks(8) {
                    if chunk.len() == 8 {
                        let xs = [chunk[0], chunk[2], chunk[4], chunk[6]];
                        let ys = [chunk[1], chunk[3], chunk[5], chunk[7]];
                        let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
                        let (yb, yt) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
                        quads.push([x0, height - yt, x1, height - yb]);
                    }
                }
            }
            if quads.is_empty() {
                if let Some(rect) = capture(body, r"/Rect\s*\[([^\]]*)\]") {
                    let nums: Vec<f64> = rect.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    if nums.len() == 4 {
                        quads.push([nums[0], height - nums[3], nums[2], height - nums[1]]);
                    }
                }
            }
            let colour = capture(body, r"/C\s*\[([^\]]*)\]").and_then(|c| {
                let nums: Vec<f64> = c.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                colour_hex(&nums)
            });
            doc.annots.push(SimpleAnnot {
                obj: *num,
                page,
                subtype,
                quads,
                author: named_string(body, "T").unwrap_or_default(),
                contents: named_string(body, "Contents").unwrap_or_default(),
                colour,
                created: capture(body, r"/CreationDate\s*\(([^)]*)\)").and_then(|d| pdf_date(&d)),
                modified: capture(body, r"/M\s*\(([^)]*)\)").and_then(|d| pdf_date(&d)),
                irt: capture(body, r"/IRT\s+(\d+) 0 R").and_then(|p| p.parse().ok()).unwrap_or(0),
            });
        }
        Ok(doc)
    }

    pub fn open(path: &Path) -> Result<SimpleDoc> {
        let bytes = std::fs::read(path)?;
        let mut doc = SimpleDoc::parse(&bytes)?;
        doc.path = Some(path.to_path_buf());
        Ok(doc)
    }

    fn annot_mut(&mut self, xref: i64) -> Result<&mut SimpleAnnot> {
        self.annots.iter_mut().find(|a| a.obj == xref).ok_or_else(|| {
            Error::not_found("The annotation is gone from the PDF. Scan the folder to refresh the highlights.")
        })
    }

    fn snapshot(a: &SimpleAnnot) -> AnnotSnapshot {
        AnnotSnapshot {
            xref: a.obj,
            page: a.page + 1,
            subtype: a.subtype.clone(),
            quads: a.quads.iter().map(|q| q.to_vec()).collect(),
            author: a.author.clone(),
            contents: a.contents.clone(),
            colour: a.colour.clone(),
            created: a.created.clone(),
            modified: a.modified.clone(),
            irt_xref: a.irt,
            extra: serde_json::Value::Null,
        }
    }

    fn blank_png(width: u32, height: u32, p: Option<&SimplePage>, scale: f64) -> Result<Vec<u8>> {
        let mut img = image::RgbImage::from_pixel(width.max(1), height.max(1), image::Rgb([255, 255, 255]));
        if let Some(p) = p {
            for line in &p.lines {
                let x0 = (line.x * scale) as u32;
                let x1 = ((line.x + line.text.chars().count() as f64 * GLYPH * line.size) * scale) as u32;
                let y0 = ((line.baseline - ASCENT * line.size) * scale) as u32;
                let y1 = ((line.baseline + DESCENT * line.size) * scale) as u32;
                for y in y0..y1.min(height) {
                    for x in x0..x1.min(width) {
                        img.put_pixel(x, y, image::Rgb([60, 60, 60]));
                    }
                }
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| Error::Other(format!("PNG encoding failed: {}", e)))?;
        Ok(out.into_inner())
    }
}

impl PdfDoc for SimpleDoc {
    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn metadata(&self) -> Result<PdfMetadata> {
        Ok(PdfMetadata { title: self.title.trim().to_string(), author: self.author.trim().to_string() })
    }

    fn pages(&self) -> Result<Vec<PageInfo>> {
        Ok(self.pages.iter().map(|p| PageInfo { width: p.width, height: p.height, rotation: p.rotation }).collect())
    }

    fn render_png(&self, page: usize, scale: f64) -> Result<Vec<u8>> {
        let p = self.page_or_err(page)?;
        let (w, h) = if p.rotation % 180 == 0 { (p.width, p.height) } else { (p.height, p.width) };
        Self::blank_png((w * scale) as u32, (h * scale) as u32, if p.rotation == 0 { Some(p) } else { None }, scale)
    }

    fn thumbnail_png(&self, width_px: u32) -> Result<Vec<u8>> {
        let p = self.pages.first().ok_or_else(|| Error::not_found("The PDF has no pages to show."))?;
        let scale = width_px as f64 / p.width.max(1.0);
        Self::blank_png(width_px, (p.height * scale) as u32, Some(p), scale)
    }

    fn words(&self, page: usize) -> Result<Vec<Word>> {
        Ok(Self::page_words(self.page_or_err(page)?))
    }

    fn annotations(&self) -> Result<Vec<Annot>> {
        Ok(self.annots.iter().map(|a| self.annot_record(a)).collect())
    }

    fn add_highlight(&mut self, h: &NewAnnot) -> Result<Annot> {
        self.page_or_err(h.page)?;
        if h.quads.is_empty() {
            return Err(Error::bad("A PDF highlight needs at least one quad [x0, y0, x1, y1]."));
        }
        let obj = self.alloc_obj();
        let subtype = if h.kind == "note" { "Text" } else { "Highlight" };
        let quads: Vec<[f64; 4]> = if subtype == "Text" {
            let q = &h.quads[0];
            vec![[q[0], q[1], q[0] + 20.0, q[1] + 20.0]]
        } else {
            h.quads.iter().filter(|q| q.len() == 4).map(|q| [q[0].min(q[2]), q[1].min(q[3]), q[0].max(q[2]), q[1].max(q[3])]).collect()
        };
        let stamp = h.created.clone().or_else(|| Some(crate::util::now_iso()));
        let annot = SimpleAnnot {
            obj,
            page: h.page - 1,
            subtype: subtype.to_string(),
            quads,
            author: h.author.clone(),
            contents: h.contents.clone(),
            colour: Some(colour_hex(&{
                let (r, g, b) = hex_rgb(&h.colour);
                vec![r, g, b]
            })
            .unwrap_or_else(|| "#ffd84a".into())),
            created: stamp.clone(),
            modified: stamp,
            irt: 0,
        };
        let rec = self.annot_record(&annot);
        self.annots.push(annot);
        Ok(rec)
    }

    fn update_highlight(&mut self, xref: i64, patch: &AnnotPatch) -> Result<()> {
        let now = crate::util::now_iso();
        let a = self.annot_mut(xref)?;
        if let Some(c) = &patch.contents {
            a.contents = c.clone();
        }
        if let Some(t) = &patch.author {
            a.author = t.clone();
        }
        if let Some(c) = &patch.colour {
            a.colour = Some(c.clone());
        }
        a.modified = Some(now);
        Ok(())
    }

    fn delete_highlight(&mut self, xref: i64) -> Result<Vec<AnnotSnapshot>> {
        let parent = self.annots.iter().find(|a| a.obj == xref).cloned().ok_or_else(|| {
            Error::not_found("The annotation is gone from the PDF. Scan the folder to refresh the highlights.")
        })?;
        let mut snaps = vec![Self::snapshot(&parent)];
        for a in self.annots.iter().filter(|a| a.irt == xref) {
            snaps.push(Self::snapshot(a));
        }
        let gone: Vec<i64> = snaps.iter().map(|s| s.xref).collect();
        self.annots.retain(|a| !gone.contains(&a.obj));
        Ok(snaps)
    }

    fn restore_annotations(&mut self, snaps: &[AnnotSnapshot]) -> Result<BTreeMap<i64, i64>> {
        let mut mapping = BTreeMap::new();
        for s in snaps {
            if s.page == 0 || s.page > self.pages.len() {
                return Err(Error::not_found(format!("Page {} does not exist; the PDF has {} pages.", s.page, self.pages.len())));
            }
            let obj = self.alloc_obj();
            let irt = mapping.get(&s.irt_xref).copied().unwrap_or(0);
            self.annots.push(SimpleAnnot {
                obj,
                page: s.page - 1,
                subtype: s.subtype.clone(),
                quads: s.quads.iter().filter(|q| q.len() == 4).map(|q| [q[0], q[1], q[2], q[3]]).collect(),
                author: s.author.clone(),
                contents: s.contents.clone(),
                colour: s.colour.clone(),
                created: s.created.clone(),
                modified: s.modified.clone(),
                irt,
            });
            mapping.insert(s.xref, obj);
        }
        Ok(mapping)
    }

    fn rotate(&mut self, page: Option<usize>, delta: i32) -> Result<Vec<i32>> {
        if delta % 90 != 0 {
            return Err(Error::bad("Rotation must be a multiple of 90 degrees."));
        }
        let targets: Vec<usize> = match page {
            Some(n) => {
                self.page_or_err(n)?;
                vec![n - 1]
            }
            None => (0..self.pages.len()).collect(),
        };
        for i in targets {
            let p = &mut self.pages[i];
            p.rotation = (p.rotation + delta).rem_euclid(360);
        }
        Ok(self.pages.iter().map(|p| p.rotation).collect())
    }

    fn save_in_place(&mut self) -> Result<()> {
        let path = self.path.clone().ok_or_else(|| Error::bad("The document has no file path to save to."))?;
        let tmp = temp_path_for(&path);
        let bytes = self.to_bytes();
        let result = (|| -> Result<()> {
            std::fs::write(&tmp, &bytes)?;
            let check = SimpleDoc::parse(&std::fs::read(&tmp)?)?;
            if check.annots.len() != self.annots.len() || check.pages.len() != self.pages.len() {
                return Err(Error::conflict(
                    "The PDF could not be verified after saving, so it was left unchanged. Try again, or close other programs that use the file.",
                ));
            }
            replace_retry(&tmp, &path)
        })();
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

/// The backend for Kurasi-made PDFs. Real documents need `kurasi-pdf`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimplePdfBackend;

impl PdfBackend for SimplePdfBackend {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>> {
        Ok(Box::new(SimpleDoc::open(path)?))
    }
    fn name(&self) -> &'static str {
        "simple"
    }
}

/// The backend of the sample project: Kurasi-made files open with the built-in reader, any
/// other PDF (a sample folder the Python build made, a file dropped into it) goes to
/// `fallback`. A file never changes reader, so the two never write the same document.
pub struct SimpleFirst {
    pub fallback: Arc<dyn PdfBackend>,
}

impl SimpleFirst {
    pub fn new(fallback: Arc<dyn PdfBackend>) -> SimpleFirst {
        SimpleFirst { fallback }
    }
}

impl PdfBackend for SimpleFirst {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>> {
        match SimpleDoc::open(path) {
            Ok(doc) => Ok(Box::new(doc)),
            Err(Error::Unsupported(_)) => self.fallback.open(path),
            Err(e) => Err(e),
        }
    }
    fn name(&self) -> &'static str {
        self.fallback.name()
    }
}

// ------------------------------------------------------------------ helpers

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{:.3}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn union(quads: &[[f64; 4]]) -> [f64; 4] {
    let mut r = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for q in quads {
        r[0] = r[0].min(q[0]);
        r[1] = r[1].min(q[1]);
        r[2] = r[2].max(q[2]);
        r[3] = r[3].max(q[3]);
    }
    if quads.is_empty() {
        [0.0, 0.0, 0.0, 0.0]
    } else {
        r
    }
}

pub fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let wl = word.chars().count();
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + wl <= max_chars {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Text for a content stream: Latin-1 bytes with PDF escapes; other characters become `?`.
fn content_text(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in text.chars() {
        let code = ch as u32;
        match ch {
            '\\' => out.extend_from_slice(b"\\\\"),
            '(' => out.extend_from_slice(b"\\("),
            ')' => out.extend_from_slice(b"\\)"),
            _ if code < 0x20 => out.push(b' '),
            _ if code < 0x7f => out.push(code as u8),
            _ if code <= 0xff => out.extend_from_slice(format!("\\{:03o}", code).as_bytes()),
            _ => out.push(b'?'),
        }
    }
    out
}

/// A PDF text string: literal with escapes when ASCII, else UTF-16BE hex with a BOM.
fn pdf_string(text: &str) -> String {
    if text.chars().all(|c| (c as u32) < 0x7f && c != '\r') {
        let mut s = String::from("(");
        for ch in text.chars() {
            match ch {
                '\\' => s.push_str("\\\\"),
                '(' => s.push_str("\\("),
                ')' => s.push_str("\\)"),
                '\n' => s.push_str("\\n"),
                '\t' => s.push_str("\\t"),
                c if (c as u32) < 0x20 => {}
                c => s.push(c),
            }
        }
        s.push(')');
        s
    } else {
        let mut s = String::from("<FEFF");
        for unit in text.encode_utf16() {
            s.push_str(&format!("{:04X}", unit));
        }
        s.push('>');
        s
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn capture(body: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern).ok()?.captures(body).map(|c| c[1].to_string())
}

fn named_int(body: &str, key: &str) -> Option<i64> {
    capture(body, &format!(r"/{}\s+(-?\d+)", key)).and_then(|v| v.parse().ok())
}

/// The value of `/Key (literal)` or `/Key <hex>`.
fn named_string(body: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r"/{}\s*(\((?:[^()\\]|\\.)*\)|<[0-9A-Fa-f]*>)", regex::escape(key))).ok()?;
    let raw = re.captures(body)?.get(1)?.as_str().to_string();
    if raw.starts_with('<') {
        let hex = &raw[1..raw.len() - 1];
        let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
        if bytes.starts_with(&[0xFE, 0xFF]) {
            let units: Vec<u16> = bytes[2..].chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units))
        } else {
            Some(latin1(&bytes))
        }
    } else {
        Some(unescape_literal(&raw[1..raw.len() - 1]))
    }
}

fn unescape_literal(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' | 'f' => {}
                '(' | ')' | '\\' => out.push(n),
                '0'..='7' => {
                    let mut digits = String::new();
                    let mut j = i + 1;
                    while j < chars.len() && digits.len() < 3 && ('0'..='7').contains(&chars[j]) {
                        digits.push(chars[j]);
                        j += 1;
                    }
                    if let Ok(v) = u32::from_str_radix(&digits, 8) {
                        out.push(char::from_u32(v).unwrap_or('?'));
                    }
                    i = j;
                    continue;
                }
                other => out.push(other),
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_parse_round_trip_with_annotations() {
        let mut d = SimpleDoc::new();
        d.title = "Die protestantische Ethik für alle".into();
        d.author = "Weber, M.".into();
        let p = d.add_page();
        d.text(p, 72.0, 100.0, 12.0, "The division of labour raises the output.");
        d.text(p, 72.0, 120.0, 12.0, "Second (line) with \\ escapes.");
        let (quads, verbatim) = d.find_quote(1, "division of labour").unwrap().unwrap();
        assert_eq!(verbatim, "division of labour");
        let added = d
            .add_highlight(&NewAnnot {
                page: 1,
                quads: quads.clone(),
                author: "Claude".into(),
                contents: "(I) Why.\n\nTags: #quote".into(),
                colour: "#ffd84a".into(),
                kind: "text".into(),
                created: Some("2026-03-01T12:00:00".into()),
            })
            .unwrap();
        let bytes = d.to_bytes();
        let back = SimpleDoc::parse(&bytes).unwrap();
        assert_eq!(back.title, d.title);
        assert_eq!(back.author, "Weber, M.");
        assert_eq!(back.pages.len(), 1);
        assert_eq!(back.pages[0].lines[1].text, "Second (line) with \\ escapes.");
        let annots = back.annotations().unwrap();
        assert_eq!(annots.len(), 1);
        assert_eq!(annots[0].xref, added.xref);
        assert_eq!(annots[0].author, "Claude");
        assert_eq!(annots[0].contents, "(I) Why.\n\nTags: #quote");
        assert_eq!(annots[0].colour.as_deref(), Some("#ffd84a"));
        assert_eq!(annots[0].created.as_deref(), Some("2026-03-01T12:00:00"));
        assert_eq!(annots[0].quads, quads);
        let words = back.words(1).unwrap();
        assert_eq!(crate::pdf::words_under(&words, &annots[0].quads), "division of labour");
        assert!(SimpleDoc::parse(b"%PDF-1.7 something else").is_err());
    }

    #[test]
    fn wrap_keeps_words_whole() {
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("", 7), vec![""]);
    }
}
