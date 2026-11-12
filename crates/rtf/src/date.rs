//! RTF dates: the DTTM bit pattern of revision marks, and the `\yr\mo\dy\hr\min` words of the
//! document information group.

/// An ISO 8601 date-time from year, month, day, hour and minute (the parameters of an `\info`
/// date's `\yr \mo \dy \hr \min`), each clamped into range.
pub fn iso_date((y, mo, d, h, mi): (u32, u32, u32, u32, u32)) -> String {
    format!("{y:04}-{:02}-{:02}T{:02}:{:02}:00Z", mo.clamp(1, 12), d.clamp(1, 31), h.min(23), mi.min(59))
}

/// An ISO 8601 date (`YYYY-MM-DD`, optionally `THH:MM`) as the words of an `\info` date:
/// `\yr2024\mo3\dy15\hr10\min30`.
pub fn date_words(s: &str) -> Option<String> {
    let num = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<u32>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi) = (num(11, 13).unwrap_or(0), num(14, 16).unwrap_or(0));
    Some(format!("\\yr{y}\\mo{mo}\\dy{d}\\hr{h}\\min{mi}"))
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Day of the week, 0 = Sunday (Sakamoto's method).
fn weekday(y: u32, m: u32, d: u32) -> u32 {
    const T: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y.saturating_sub(1) } else { y };
    (y + y / 4 - y / 100 + y / 400 + T.get(m.saturating_sub(1) as usize).copied().unwrap_or(0) + d) % 7
}

/// An RTF revision date (DTTM, RTF 1.9.1 "Revision Marks": minute in bits 0–5, hour 6–10, day
/// 11–15, month 16–19, year − 1900 20–28, weekday 29–31) as ISO 8601 (`2024-03-15T10:30:00Z`).
/// Zero or an impossible date is no date. DTTM has no time zone or seconds: times are read as
/// UTC (`Z`).
pub fn dttm_to_iso(v: u32) -> Option<String> {
    let (mi, h, d, mo, y) = (v & 0x3F, (v >> 6) & 0x1F, (v >> 11) & 0x1F, (v >> 16) & 0xF, 1900 + ((v >> 20) & 0x1FF));
    if v == 0 || mi > 59 || h > 23 || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:00Z"))
}

/// An ISO 8601 date (`YYYY-MM-DD`, optionally `THH:MM…`; seconds and zone ignored) as an RTF
/// DTTM; `None` when it isn't one or falls outside DTTM's years (1900–2411). DTTM has no time
/// zone or seconds: the clock time is written as given (a `Z` time stays the UTC clock time), and seconds
/// and any offset are dropped.
pub fn iso_to_dttm(s: &str) -> Option<u32> {
    let num = |a: usize, b: usize| s.get(a..b).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<u32>().ok());
    let sep = |i: usize, c: &[&str]| s.get(i..i + 1).is_some_and(|t| c.contains(&t));
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if !sep(4, &["-"]) || !sep(7, &["-"]) {
        return None;
    }
    let (h, mi) = if s.len() > 10 {
        if !sep(10, &["T", " "]) || !sep(13, &[":"]) {
            return None;
        }
        (num(11, 13)?, num(14, 16)?)
    } else {
        (0, 0)
    };
    if !(1900..=2411).contains(&y) || mi > 59 || h > 23 || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    Some(mi | (h << 6) | (d << 11) | (mo << 16) | ((y - 1900) << 20) | (weekday(y, mo, d) << 29))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dttm_round_trips_and_refuses_impossible_dates() {
        let v = iso_to_dttm("2024-03-15T10:30:59+02:00").unwrap();
        assert_eq!(dttm_to_iso(v).as_deref(), Some("2024-03-15T10:30:00Z"));
        assert_eq!(v >> 29, 5, "a Friday");
        assert_eq!(iso_to_dttm("2024-02-29").and_then(dttm_to_iso).as_deref(), Some("2024-02-29T00:00:00Z"));
        for bad in ["2023-02-29", "1899-01-01", "2412-01-01", "2024-13-01", "2024-03-15T24:00", "2024/03/15", "2024-03-15x10:30", "", "+024-03-15"] {
            assert_eq!(iso_to_dttm(bad), None, "{bad}");
        }
        assert_eq!(dttm_to_iso(0), None);
        assert_eq!(dttm_to_iso(u32::MAX), None);
        // Day 31 of a 30-day month, hour 24, minute 60.
        assert_eq!(dttm_to_iso((31 << 11) | (4 << 16) | (124 << 20)), None);
        assert_eq!(dttm_to_iso((24 << 6) | (1 << 11) | (1 << 16) | (124 << 20)), None);
        assert_eq!(dttm_to_iso(60 | (1 << 11) | (1 << 16) | (124 << 20)), None);
    }

    #[test]
    fn info_dates() {
        assert_eq!(date_words("2024-03-15T10:30:00Z").as_deref(), Some("\\yr2024\\mo3\\dy15\\hr10\\min30"));
        assert_eq!(date_words("2024-03-15").as_deref(), Some("\\yr2024\\mo3\\dy15\\hr0\\min0"));
        assert_eq!(date_words("soon"), None);
        assert_eq!(iso_date((2024, 3, 15, 10, 30)), "2024-03-15T10:30:00Z");
        assert_eq!(iso_date((2024, 0, 99, 99, 99)), "2024-01-31T23:59:00Z", "clamped");
    }
}
