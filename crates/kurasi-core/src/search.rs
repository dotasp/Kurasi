//! Loose relevance scoring (CONTRACT.md section 8, Search).
//!
//! Each query token is compared with each field: a substring hit scores 1, otherwise the best
//! bigram Dice similarity against the field's words counts when it reaches the threshold
//! (0.42 loose, 0.72 strict), scaled by 0.9. The token score is the best field score times the
//! field weight. A token that matches no field drops the reference. The total is the sum.
//!
//! [`FullText`] scores the `search_text` field alone for `GET /api/search`. The UI scores the
//! metadata fields itself and takes, per token, the higher of its own score and the server's,
//! so it follows the UI rules of `ui/app.js` (`score`, `fieldSim`, `dice`) exactly: the query is
//! lower-cased and split at JavaScript whitespace, a word is a run of `\p{L}`, `\p{N}` and the
//! hyphen (the underscore separates words here, unlike [`field_sim`]), and a bigram is a pair of
//! UTF-16 code units. The thresholds, the 0.9 scale and the substring rule are shared.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

use crate::model::Reference;

pub const LOOSE: f64 = 0.42;
pub const STRICT: f64 = 0.72;

fn is_word_char(c: char) -> bool {
    // Python's `\w` with re.UNICODE: letters, digits and the underscore; the hyphen is kept too.
    c.is_alphanumeric() || c == '_' || c == '-'
}

pub fn bigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    let mut out = HashSet::new();
    for i in 0..chars.len().saturating_sub(1) {
        out.insert(format!("{}{}", chars[i], chars[i + 1]));
    }
    out
}

pub fn dice(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let (ba, bb) = (bigrams(a), bigrams(b));
    if ba.is_empty() || bb.is_empty() {
        return 0.0;
    }
    let common = ba.intersection(&bb).count();
    2.0 * common as f64 / (ba.len() + bb.len()) as f64
}

pub fn field_sim(tok: &str, text: &str, thr: f64) -> f64 {
    let text = text.to_lowercase();
    if text.is_empty() {
        return 0.0;
    }
    if text.contains(tok) {
        return 1.0;
    }
    let mut best = 0.0f64;
    for w in text.split(|c: char| !is_word_char(c)) {
        if w.is_empty() {
            continue;
        }
        let d = dice(tok, w);
        if d > best {
            best = d;
        }
    }
    if best >= thr {
        best * 0.9
    } else {
        0.0
    }
}

/// Weighted fields of a reference record as built by the catalogue.
pub fn fields_of(r: &Reference) -> Vec<(String, f64)> {
    let tags = if r.tags.is_empty() { &r.top_tags } else { &r.tags };
    vec![
        (r.title.clone(), 3.0),
        (r.authors.join(" "), 2.0),
        (tags.join(" "), 2.0),
        (r.pub_.clone(), 1.0),
        (r.register_id.clone(), 1.0),
        (r.search_text.clone(), 1.0),
        (r.categories.join(" "), 1.0),
    ]
}

pub fn score_fields(fields: &[(String, f64)], query: &str, loose: bool) -> f64 {
    let toks: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    if toks.is_empty() {
        return 1.0;
    }
    let thr = if loose { LOOSE } else { STRICT };
    let mut total = 0.0;
    for t in &toks {
        let mut best = 0.0f64;
        for (text, weight) in fields {
            let s = field_sim(t, text, thr) * weight;
            if s > best {
                best = s;
            }
        }
        if best == 0.0 {
            return 0.0;
        }
        total += best;
    }
    total
}

pub fn score(r: &Reference, query: &str, loose: bool) -> f64 {
    score_fields(&fields_of(r), query, loose)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Scored references above zero, best first; ties go to more highlights, then the title.
pub fn rank(refs: &[Reference], query: &str, loose: bool, limit: usize) -> Vec<(f64, Reference)> {
    let mut scored: Vec<(f64, &Reference)> =
        refs.iter().map(|r| (score(r, query, loose), r)).filter(|(s, _)| *s > 0.0).collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.hl_count.cmp(&a.1.hl_count))
            .then_with(|| a.1.title.cmp(&b.1.title))
    });
    scored.into_iter().take(limit).map(|(s, r)| (round3(s), r.clone())).collect()
}

// ------------------------------------------------------------------ UI rules (ui/app.js)

/// JavaScript `\s`: the characters the UI splits a query at.
pub fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// The query tokens of the UI: `q.toLowerCase().split(/\s+/).filter(Boolean)`.
pub fn ui_tokens(query: &str) -> Vec<String> {
    query.to_lowercase().split(js_space).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

fn ui_word_split() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[^\p{L}\p{N}-]+").expect("word pattern"))
}

/// The bigram set of the UI (`s.slice(i, i + 2)` over UTF-16 code units) as sorted codes.
pub fn ui_bigrams(s: &str) -> Vec<u32> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out: Vec<u32> = units.windows(2).map(|w| ((w[0] as u32) << 16) | w[1] as u32).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn common(a: &[u32], b: &[u32]) -> usize {
    let (mut i, mut j, mut n) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    n
}

/// `dice(a, b)` of the UI on precomputed bigram codes.
pub fn ui_dice(a: &str, ga: &[u32], b: &str, gb: &[u32]) -> f64 {
    if a == b {
        return 1.0;
    }
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    2.0 * common(ga, gb) as f64 / (ga.len() + gb.len()) as f64
}

/// `fieldSim(tok, field, thr)` of the UI for one text: 1 for a substring hit, else 0.9 times the
/// best word similarity when it reaches `thr`, else 0. `FullText` gives the same values faster.
pub fn ui_field_sim(tok: &str, text: &str, thr: f64) -> f64 {
    let low = text.to_lowercase();
    if low.is_empty() {
        return 0.0;
    }
    if low.contains(tok) {
        return 1.0;
    }
    let gt = ui_bigrams(tok);
    let mut best = 0.0f64;
    let mut seen = HashSet::new();
    for w in ui_word_split().split(&low) {
        if w.is_empty() || !seen.insert(w) {
            continue;
        }
        let d = ui_dice(tok, &gt, w, &ui_bigrams(w));
        if d > best {
            best = d;
        }
    }
    if best >= thr {
        best * 0.9
    } else {
        0.0
    }
}

struct Word {
    text: String,
    grams: Vec<u32>,
}

struct Doc {
    low: String,
    words: Vec<u32>,
}

/// The full-text index of one project: the lower-cased `search_text` of every reference that has
/// one, with its distinct words as ids into a shared vocabulary. `seq` is the change number the
/// index reflects; the catalogue updates it from the change feed.
#[derive(Default)]
pub struct FullText {
    seq: i64,
    docs: HashMap<String, Doc>,
    vocab: HashMap<String, u32>,
    words: Vec<Word>,
}

impl FullText {
    pub fn new(seq: i64) -> FullText {
        FullText { seq, ..FullText::default() }
    }

    pub fn seq(&self) -> i64 {
        self.seq
    }

    pub fn set_seq(&mut self, seq: i64) {
        self.seq = seq;
    }

    /// Number of references with text.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    fn word_id(&mut self, w: &str) -> u32 {
        if let Some(&id) = self.vocab.get(w) {
            return id;
        }
        let id = self.words.len() as u32;
        self.words.push(Word { text: w.to_string(), grams: ui_bigrams(w) });
        self.vocab.insert(w.to_string(), id);
        id
    }

    /// Puts the text of one reference in, replacing what it had; an empty text takes it out.
    pub fn set(&mut self, id: &str, text: &str) {
        let low = text.to_lowercase();
        if low.is_empty() {
            self.docs.remove(id);
            return;
        }
        let mut ids: Vec<u32> = Vec::new();
        for w in ui_word_split().split(&low) {
            if !w.is_empty() {
                let wid = self.word_id(w);
                ids.push(wid);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        self.docs.insert(id.to_string(), Doc { low, words: ids });
    }

    pub fn remove(&mut self, id: &str) {
        self.docs.remove(id);
    }

    /// The per-token scores (aligned with `tokens`) of every reference that scores on at least
    /// one token, in no particular order. Each score equals `ui_field_sim(token, text, thr)`.
    pub fn search(&self, tokens: &[String], loose: bool) -> Vec<(String, Vec<f64>)> {
        if tokens.is_empty() {
            return Vec::new();
        }
        let thr = if loose { LOOSE } else { STRICT };
        // Similarity of each token to each vocabulary word, kept only where it reaches the
        // threshold. Dice can never exceed 2 min(a, b) / (a + b), which skips most words.
        let sims: Vec<Vec<f64>> = tokens
            .iter()
            .map(|t| {
                let gt = ui_bigrams(t);
                let a = gt.len();
                self.words
                    .iter()
                    .map(|w| {
                        let b = w.grams.len();
                        let bound = if a == 0 || b == 0 { 0.0 } else { 2.0 * a.min(b) as f64 / (a + b) as f64 };
                        if w.text != *t && bound < thr {
                            return 0.0;
                        }
                        let d = ui_dice(t, &gt, &w.text, &w.grams);
                        if d >= thr {
                            d
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect();
        let mut out = Vec::new();
        for (id, doc) in &self.docs {
            let mut scores = vec![0.0f64; tokens.len()];
            let mut any = false;
            for (i, t) in tokens.iter().enumerate() {
                let s = if doc.low.contains(t.as_str()) {
                    1.0
                } else {
                    let row = &sims[i];
                    let mut best = 0.0f64;
                    for &w in &doc.words {
                        let d = row[w as usize];
                        if d > best {
                            best = d;
                        }
                    }
                    if best >= thr {
                        best * 0.9
                    } else {
                        0.0
                    }
                };
                if s > 0.0 {
                    any = true;
                }
                scores[i] = s;
            }
            if any {
                out.push((id.clone(), scores));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(title: &str, authors: &[&str], tags: &[&str], text: &str) -> Vec<(String, f64)> {
        vec![
            (title.to_string(), 3.0),
            (authors.join(" "), 2.0),
            (tags.join(" "), 2.0),
            (String::new(), 1.0),
            (String::new(), 1.0),
            (text.to_string(), 1.0),
            ("Uncategorized".to_string(), 1.0),
        ]
    }

    #[test]
    fn dice_and_bigrams() {
        assert_eq!(dice("night", "nacht"), 0.25);
        assert_eq!(dice("same", "same"), 1.0);
        assert_eq!(dice("a", "b"), 0.0);
        assert_eq!(bigrams("Ab").len(), 1);
    }

    #[test]
    fn substring_hit_scores_the_weight() {
        let f = fields("The wealth of nations", &["Smith, A."], &["quote"], "division of labour");
        assert_eq!(score_fields(&f, "wealth", true), 3.0);
        assert_eq!(score_fields(&f, "smith", true), 2.0);
        assert_eq!(score_fields(&f, "labour", true), 1.0);
        assert_eq!(score_fields(&f, "", true), 1.0);
    }

    #[test]
    fn loose_tolerates_a_typo_and_strict_refuses_it() {
        let f = fields("The wealth of nations", &["Smith, A."], &[], "");
        // dice("welth", "wealth") = 2*3/(4+5) = 0.666... (we, lt, th shared), times 0.9 and the
        // title weight 3, plus 3 for the exact "nations".
        let loose = score_fields(&f, "welth nations", true);
        assert!((loose - 4.8).abs() < 1e-9, "{}", loose);
        // Strict mode (0.72) drops "welth", and a missed token drops the reference.
        assert_eq!(score_fields(&f, "welth nations", false), 0.0);
        assert_eq!(score_fields(&f, "xylophone", true), 0.0);
        // dice("natons", "nations") = 2*4/(5+6) = 0.727..., above strict.
        assert!(score_fields(&f, "natons", false) > 0.0);
        // dice("wlth", "wealth") = 2*2/(3+5) = 0.5: loose keeps it, strict drops it.
        assert!(score_fields(&f, "wlth", true) > 0.0);
        assert_eq!(score_fields(&f, "wlth", false), 0.0);
    }

    #[test]
    fn ui_tokens_split_at_javascript_whitespace() {
        assert_eq!(ui_tokens("  Wealth\u{A0}OF\tNations\u{FEFF}x "), vec!["wealth", "of", "nations", "x"]);
        // U+0085 is whitespace for Rust and a word character for JavaScript `\s`.
        assert_eq!(ui_tokens("a\u{85}b"), vec!["a\u{85}b"]);
        assert!(ui_tokens(" \u{3000} ").is_empty());
    }

    #[test]
    fn ui_rules_split_words_at_the_underscore_and_count_utf16_units() {
        // "scan_0007": the UI sees the words "scan" and "0007".
        let t = "file scan_0007 here";
        assert_eq!(ui_field_sim("sacn", t, LOOSE), 0.0);
        assert_eq!(ui_field_sim("scab", t, LOOSE), 0.9 * dice("scab", "scan"));
        // An astral character is two UTF-16 units: "a😀" has the bigrams (a, high) and (high, low).
        assert_eq!(ui_bigrams("a\u{1F600}").len(), 2);
        assert_eq!(ui_bigrams("aa").len(), 1);
    }

    #[test]
    fn full_text_index_gives_the_ui_scores() {
        let texts = [
            ("r-1", "The pin factory: ten workers divide the steps.\n(I) Dexterity, saved time, machinery."),
            ("r-2", "Bricklaying motions cut from eighteen to five per brick."),
            ("r-3", "scan_0007 Gr\u{FC}\u{DF}e aus K\u{F6}ln, \u{1F600} emoji-word"),
            ("r-4", ""),
        ];
        let mut idx = FullText::new(0);
        for (id, t) in texts {
            idx.set(id, t);
        }
        assert_eq!(idx.len(), 3);
        let queries = ["factory", "factroy", "dexterty", "brick", "brik moton", "scan", "0007", "koln", "köln", "grüße", "emoji", "xyz", "the five", "wordemoji"];
        for q in queries {
            let toks = ui_tokens(q);
            for loose in [true, false] {
                let thr = if loose { LOOSE } else { STRICT };
                let hits: HashMap<String, Vec<f64>> = idx.search(&toks, loose).into_iter().collect();
                for (id, t) in texts {
                    let want: Vec<f64> = toks.iter().map(|tok| ui_field_sim(tok, t, thr)).collect();
                    let got = hits.get(id).cloned().unwrap_or_else(|| vec![0.0; toks.len()]);
                    assert_eq!(got, want, "query {:?} loose {} doc {}", q, loose, id);
                }
            }
        }
        // Replacing and removing a text.
        idx.set("r-2", "A different text about pins.");
        assert!(idx.search(&ui_tokens("brick"), true).is_empty());
        idx.remove("r-1");
        assert_eq!(idx.search(&ui_tokens("pins"), true).len(), 1);
    }

    #[test]
    fn a_missing_token_drops_the_reference() {
        let f = fields("Motion study", &["Gilbreth, F. B."], &["method"], "");
        assert_eq!(score_fields(&f, "motion zzzz", true), 0.0);
        assert_eq!(score_fields(&f, "motion method", true), 5.0);
    }
}
