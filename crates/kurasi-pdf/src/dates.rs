//! PDF date strings (`D:YYYYMMDDHHmmSS...`) to ISO 8601 (`YYYY-MM-DDTHH:MM:SS`), as pdfio.py does.

/// Parses a PDF date. Missing parts default to 1 for month and day and 0 for the time.
/// Returns `None` when the year is missing or a part is out of range.
pub fn pdf_date_to_iso(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s.strip_prefix("D:").or_else(|| s.strip_prefix('D')).unwrap_or(s);
    let s = s.strip_prefix(':').unwrap_or(s);
    let digits: Vec<u8> = s.bytes().take_while(|b| b.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let num = |from: usize, len: usize, default: u32| -> Option<u32> {
        if digits.len() < from + len {
            return Some(default);
        }
        std::str::from_utf8(&digits[from..from + len]).ok()?.parse().ok()
    };
    let year = num(0, 4, 0)?;
    let month = num(4, 2, 1)?;
    let day = num(6, 2, 1)?;
    let hour = num(8, 2, 0)?;
    let minute = num(10, 2, 0)?;
    let second = num(12, 2, 0)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"))
}

#[cfg(test)]
mod tests {
    use super::pdf_date_to_iso;

    #[test]
    fn parses_full_and_partial_dates() {
        assert_eq!(pdf_date_to_iso("D:20261017093015+07'00'").as_deref(), Some("2026-10-17T09:30:15"));
        assert_eq!(pdf_date_to_iso("D:2026").as_deref(), Some("2026-01-01T00:00:00"));
        assert_eq!(pdf_date_to_iso("20261017").as_deref(), Some("2026-10-17T00:00:00"));
        assert_eq!(pdf_date_to_iso("D:20261399").as_deref(), None);
        assert_eq!(pdf_date_to_iso(""), None);
    }
}
