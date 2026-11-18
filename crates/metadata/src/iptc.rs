//! IPTC-IIM (Information Interchange Model 4.2) — the record-2 datasets people still carry in JPEGs.
//!
//! Stream of `0x1C record dataset size(u16 BE, or extended) data` tags. Text is UTF-8 when the record-1
//! `CodedCharacterSet` (1:90) is `ESC % G`, otherwise decoded as UTF-8 when valid and Latin-1 otherwise.

use crate::{DateTime, Metadata};

fn decode(b: &[u8], utf8: bool) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.trim_end_matches('\0').trim().to_string(),
        Err(_) if utf8 => String::from_utf8_lossy(b).trim_end_matches('\0').trim().to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect::<String>().trim_end_matches('\0').trim().to_string(),
    }
}

fn decimal_pair(bytes: &[u8]) -> Option<u16> {
    let [tens, ones] = bytes else { return None };
    if !tens.is_ascii_digit() || !ones.is_ascii_digit() {
        return None;
    }
    Some(u16::from(*tens - b'0') * 10 + u16::from(*ones - b'0'))
}

fn parse_created(date: Option<&str>, time: Option<&str>) -> Option<DateTime> {
    let date = date.filter(|value| value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()))?;
    let time = time.filter(|value| !value.is_empty());
    if let Some(time) = time {
        let bytes = time.as_bytes();
        let clock = bytes.get(..6)?;
        if !clock.iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let hour = decimal_pair(clock.get(..2)?)?;
        let minute = decimal_pair(clock.get(2..4)?)?;
        let second = decimal_pair(clock.get(4..6)?)?;
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        match bytes.len() {
            // IPTC-IIM permits a local wall-clock form, but without an offset
            // it is ambiguous; retain the date and omit this time.
            6 => {}
            11 => {
                let offset = bytes.get(6..11)?;
                if !matches!(offset.first().copied(), Some(b'+' | b'-')) || !offset.get(1..)?.iter().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                let offset_hour = decimal_pair(offset.get(1..3)?)?;
                let offset_minute = decimal_pair(offset.get(3..5)?)?;
                if offset_hour > 18 || offset_minute > 59 || (offset_hour == 18 && offset_minute != 0) {
                    return None;
                }
            }
            _ => return None,
        }
    }

    let mut iso = format!("{}-{}-{}", date.get(..4)?, date.get(4..6)?, date.get(6..8)?);
    if let Some(time) = time.filter(|time| time.len() == 11) {
        let bytes = time.as_bytes();
        iso.push('T');
        iso.push_str(time.get(..2)?);
        iso.push(':');
        iso.push_str(time.get(2..4)?);
        iso.push(':');
        iso.push_str(time.get(4..6)?);
        if bytes.len() == 11 {
            let offset = time.get(6..)?;
            iso.push_str(offset.get(..3)?);
            iso.push(':');
            iso.push_str(offset.get(3..)?);
        }
    }
    DateTime::parse_iso(&iso)
}

/// Parse an IIM stream into the fields [`Metadata`] carries (title, keywords, artist, copyright, caption,
/// creation date/time).
pub fn parse_iptc(b: &[u8]) -> Metadata {
    let mut m = Metadata::default();
    let mut utf8 = false;
    let mut date: Option<String> = None;
    let mut time: Option<String> = None;
    let mut seen_keywords = std::collections::HashSet::new();
    let mut i = 0usize;
    while i + 5 <= b.len() {
        if b[i] != 0x1c {
            break;
        }
        let (rec, ds) = (b[i + 1], b[i + 2]);
        let mut size = u16::from_be_bytes([b[i + 3], b[i + 4]]) as usize;
        let mut at = i + 5;
        if size & 0x8000 != 0 {
            // extended dataset: next (size & 0x7fff) bytes hold the length
            let n = size & 0x7fff;
            if n == 0 || n > 4 || at + n > b.len() {
                break;
            }
            size = b[at..at + n].iter().fold(0usize, |a, &c| (a << 8) | c as usize);
            at += n;
        }
        let Some(data) = b.get(at..at.saturating_add(size)) else { break };
        match (rec, ds) {
            (1, 90) => utf8 = data == b"\x1b%G",
            (2, 5) => m.title = Some(decode(data, utf8)).filter(|s| !s.is_empty()),
            (2, 25) => {
                let k = decode(data, utf8);
                if !k.is_empty() && seen_keywords.insert(k.clone()) {
                    m.keywords.push(k);
                }
            }
            (2, 80) => m.artist = Some(decode(data, utf8)).filter(|s| !s.is_empty()),
            (2, 116) => m.copyright = Some(decode(data, utf8)).filter(|s| !s.is_empty()),
            (2, 120) => m.caption = Some(decode(data, utf8)).filter(|s| !s.is_empty()),
            (2, 55) => date = Some(decode(data, utf8)),
            (2, 60) => time = Some(decode(data, utf8)),
            _ => {}
        }
        i = at + size;
    }
    m.capture_time = parse_created(date.as_deref(), time.as_deref());
    m
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn ds(out: &mut Vec<u8>, rec: u8, ds: u8, data: &[u8]) {
        out.extend_from_slice(&[0x1c, rec, ds]);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
        out.extend_from_slice(data);
    }

    #[test]
    fn distinct_keywords_are_kept_once_each_without_rescanning() {
        let mut b = Vec::new();
        for i in 0..50_000u32 {
            ds(&mut b, 2, 25, format!("k{i}").as_bytes());
            ds(&mut b, 2, 25, b"same");
        }
        let m = parse_iptc(&b);
        assert_eq!(m.keywords.len(), 50_001);
        assert_eq!(m.keywords.iter().filter(|k| *k == "same").count(), 1);
    }

    pub(crate) fn sample_iim() -> Vec<u8> {
        let mut v = Vec::new();
        ds(&mut v, 1, 90, b"\x1b%G");
        ds(&mut v, 2, 0, &[0, 4]);
        ds(&mut v, 2, 5, "Tïtle".as_bytes());
        ds(&mut v, 2, 25, b"alpha");
        ds(&mut v, 2, 25, b"beta");
        ds(&mut v, 2, 25, b"alpha");
        ds(&mut v, 2, 80, b"Photographer");
        ds(&mut v, 2, 116, b"CC0");
        ds(&mut v, 2, 120, b"A caption");
        ds(&mut v, 2, 55, b"20210304");
        ds(&mut v, 2, 60, b"101112+0100");
        v
    }

    #[test]
    fn parses_record2() {
        let m = parse_iptc(&sample_iim());
        assert_eq!(m.title.as_deref(), Some("Tïtle"));
        assert_eq!(m.keywords, vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(m.artist.as_deref(), Some("Photographer"));
        assert_eq!(m.copyright.as_deref(), Some("CC0"));
        assert_eq!(m.caption.as_deref(), Some("A caption"));
        assert_eq!(m.capture_time.unwrap().to_iso(), "2021-03-04T10:11:12+01:00");
    }

    fn capture_time(time: Option<&[u8]>) -> Metadata {
        let mut iim = Vec::new();
        ds(&mut iim, 2, 55, b"20260101");
        if let Some(time) = time {
            ds(&mut iim, 2, 60, time);
        }
        ds(&mut iim, 2, 5, b"kept title");
        ds(&mut iim, 2, 25, b"kept keyword");
        parse_iptc(&iim)
    }

    #[test]
    fn iptc_time_created_rejects_incomplete_or_trailing_text() {
        for time in [
            &b"121314+01"[..],
            b"121314junk",
            b"12A314",
            b"12131499",
            b"121314+0100junk",
            b"121314+",
            b"121314+010",
            b"121314+01000",
            b"12345",
            b"240000",
            b"250000+0100",
            b"126100+0100",
            b"235960",
            b"235960+0000",
            b"121314+1801",
        ] {
            let metadata = capture_time(Some(time));
            assert_eq!(metadata.capture_time, None, "accepted malformed Time Created {time:?}");
            assert_eq!(metadata.title.as_deref(), Some("kept title"), "unrelated metadata was lost for {time:?}");
            assert_eq!(metadata.keywords, ["kept keyword"], "unrelated metadata was lost for {time:?}");
        }
    }

    #[test]
    fn iptc_time_created_preserves_supported_forms() {
        let cases: &[(Option<&[u8]>, Option<&str>)] = &[
            (None, Some("2026-01-01T00:00:00")),
            (Some(b""), Some("2026-01-01T00:00:00")),
            (Some(b" \t"), Some("2026-01-01T00:00:00")),
            (Some(b"121314"), Some("2026-01-01T00:00:00")),
            (Some(b"121314+0100"), Some("2026-01-01T12:13:14+01:00")),
            (Some(b"121314-0530"), Some("2026-01-01T12:13:14-05:30")),
            (Some(b"121314+1800"), Some("2026-01-01T12:13:14+18:00")),
            (Some(b" 121314+0000 \0"), Some("2026-01-01T12:13:14+00:00")),
        ];
        for (time, expected) in cases {
            let metadata = capture_time(*time);
            assert_eq!(metadata.capture_time.map(|value| value.to_iso()).as_deref(), *expected, "Time Created {time:?}");
        }
    }

    #[test]
    fn non_utf8_text_drops_trailing_nul() {
        let mut v = Vec::new();
        ds(&mut v, 2, 5, &[0x43, 0xe9, 0x00]);
        ds(&mut v, 2, 25, &[0x6b, 0xe9, 0x00]);
        let m = parse_iptc(&v);
        assert_eq!(m.title.as_deref(), Some("Cé"));
        assert_eq!(m.keywords, vec!["ké".to_string()]);
        let mut u = Vec::new();
        ds(&mut u, 1, 90, b"\x1b%G");
        ds(&mut u, 2, 5, &[0x43, 0xff, 0x00]);
        assert_eq!(parse_iptc(&u).title.as_deref(), Some("C\u{fffd}"));
    }

    #[test]
    fn latin1_and_extended_and_garbage() {
        let mut v = Vec::new();
        ds(&mut v, 2, 5, &[0x43, 0xe9]); // "Cé" in Latin-1
        v.extend_from_slice(&[0x1c, 2, 120, 0x80, 0x02, 0x00, 0x03]);
        v.extend_from_slice(b"abc");
        let m = parse_iptc(&v);
        assert_eq!(m.title.as_deref(), Some("Cé"));
        assert_eq!(m.caption.as_deref(), Some("abc"));
        let s = sample_iim();
        for n in 0..s.len() {
            let _ = parse_iptc(&s[..n]);
        }
        assert_eq!(parse_iptc(&[0x1c, 2, 5, 0xff, 0xff]), Metadata::default());
    }
}
