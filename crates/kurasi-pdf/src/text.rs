//! Words from pdfium characters.
//!
//! pdfium has no word API. Characters are read in pdfium order, split at whitespace and
//! control characters, and the loose glyph boxes of one word are joined into its box.
//! Ligature code points are expanded, as PyMuPDF does without `TEXT_PRESERVE_LIGATURES`.

use crate::types::Rect;

/// Expands a ligature to its letters; other characters pass through.
pub fn expand_ligature(c: char) -> &'static str {
    match c {
        '\u{FB00}' => "ff",
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{FB05}' | '\u{FB06}' => "st",
        _ => "",
    }
}

/// Builds words from `(char, box)` pairs. A `None` box means pdfium gave no geometry for
/// the character; its text still joins the word.
pub fn group_words<I>(chars: I) -> Vec<crate::types::Word>
where
    I: IntoIterator<Item = (Option<char>, Option<Rect>)>,
{
    let mut words: Vec<crate::types::Word> = Vec::new();
    let mut text = String::new();
    let mut rect: Option<Rect> = None;
    // The running line: its box and number. A word opens a new line when it fails to overlap
    // the line vertically by half the smaller height, or when it starts left of the line's
    // right edge by more than its own height (a return to the margin).
    let mut line: Option<(Rect, u32)> = None;
    let flush = |text: &mut String, rect: &mut Option<Rect>, line: &mut Option<(Rect, u32)>, words: &mut Vec<crate::types::Word>| {
        if !text.is_empty() {
            let r = rect.unwrap_or([0.0, 0.0, 0.0, 0.0]);
            let has_box = r[2] > r[0] || r[3] > r[1];
            let number = match (line.as_mut(), has_box) {
                (Some((lr, n)), true) => {
                    let overlap = r[3].min(lr[3]) - r[1].max(lr[1]);
                    let min_h = (r[3] - r[1]).min(lr[3] - lr[1]).max(0.01);
                    let back = r[0] < lr[2] - (r[3] - r[1]).max(1.0);
                    if overlap >= 0.5 * min_h && !back {
                        *lr = [lr[0].min(r[0]), lr[1].min(r[1]), lr[2].max(r[2]), lr[3].max(r[3])];
                        *n
                    } else {
                        let next = *n + 1;
                        *line = Some((r, next));
                        next
                    }
                }
                (Some((_, n)), false) => *n,
                (None, _) => {
                    *line = Some((r, 0));
                    0
                }
            };
            words.push(crate::types::Word { text: std::mem::take(text), x0: r[0], y0: r[1], x1: r[2], y1: r[3], line: number });
        }
        *rect = None;
    };
    for (c, b) in chars {
        let Some(c) = c else {
            flush(&mut text, &mut rect, &mut line, &mut words);
            continue;
        };
        if c.is_whitespace() || c.is_control() || c == '\u{FFFE}' || c == '\u{FFFF}' {
            flush(&mut text, &mut rect, &mut line, &mut words);
            continue;
        }
        let lig = expand_ligature(c);
        if lig.is_empty() {
            text.push(c);
        } else {
            text.push_str(lig);
        }
        if let Some(b) = b {
            if b[2] > b[0] || b[3] > b[1] {
                rect = Some(match rect {
                    None => b,
                    Some(r) => [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])],
                });
            }
        }
    }
    flush(&mut text, &mut rect, &mut line, &mut words);
    words
}

#[cfg(test)]
mod tests {
    use super::group_words;

    #[test]
    fn splits_on_whitespace_and_joins_boxes() {
        let chars = vec![
            (Some('a'), Some([0.0, 0.0, 5.0, 10.0])),
            (Some('b'), Some([5.0, 1.0, 10.0, 11.0])),
            (Some(' '), None),
            (Some('\u{FB01}'), Some([20.0, 0.0, 26.0, 10.0])),
            (Some('n'), Some([26.0, 0.0, 30.0, 10.0])),
            (Some('\n'), Some([0.0, 0.0, 0.0, 0.0])),
        ];
        let words = group_words(chars);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "ab");
        assert_eq!(words[0].rect(), [0.0, 0.0, 10.0, 11.0]);
        assert_eq!(words[1].text, "fin");
        assert_eq!(words[1].rect(), [20.0, 0.0, 30.0, 10.0]);
        assert_eq!((words[0].line, words[1].line), (0, 0));
    }

    #[test]
    fn line_numbers_follow_vertical_overlap_and_returns_to_the_margin() {
        let word = |x: f32, y: f32, t: &str| {
            let mut v: Vec<(Option<char>, Option<super::Rect>)> = t.chars().map(|c| (Some(c), Some([x, y, x + 10.0, y + 10.0]))).collect();
            v.push((Some(' '), None));
            v
        };
        let mut chars = Vec::new();
        chars.extend(word(0.0, 0.0, "a"));
        chars.extend(word(20.0, 1.0, "b"));
        chars.extend(word(0.0, 14.0, "c"));
        chars.extend(word(20.0, 14.0, "d"));
        chars.extend(word(0.0, 15.0, "e"));
        let words = group_words(chars);
        let lines: Vec<u32> = words.iter().map(|w| w.line).collect();
        assert_eq!(lines, vec![0, 0, 1, 1, 2]);
    }
}
