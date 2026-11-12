//! Capture date/time with optional UTC offset (Exif `DateTimeOriginal` + `OffsetTimeOriginal`, ISO 8601 in XMP).

use serde::{Deserialize, Serialize};

/// A civil date-time as recorded by the camera; `offset_minutes` is the UTC offset when known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Sub-second part in milliseconds.
    pub millis: u16,
    pub offset_minutes: Option<i16>,
    /// How much of the date was stated. XMP dates may stop at the year (`1983`) or the month
    /// (`1983-04`), as for a scanned print whose day isn't known; the missing parts read as 1 and
    /// the time as midnight, and the date writes back as stated.
    #[serde(default, skip_serializing_if = "DatePrecision::is_full")]
    pub precision: DatePrecision,
}

/// How much of a [`DateTime`] was stated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatePrecision {
    /// The day at least (a time of day may be midnight because none was given).
    #[default]
    Full,
    /// The year and month only (`1983-04`).
    Month,
    /// The year only (`1983`).
    Year,
}

impl DatePrecision {
    pub fn is_full(&self) -> bool {
        *self == DatePrecision::Full
    }
}

fn num(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl DateTime {
    fn valid(self) -> Option<Self> {
        // A timestamp must represent a real Gregorian calendar day, not just
        // numbers in the range 1..=31. Invalid dates would otherwise be
        // accepted from EXIF/XMP and silently normalized during time arithmetic.
        // chrono rejects impossible days (leap years and all); the time is checked
        // by hand so a leap second (`second == 60`) stays acceptable.
        let day_ok = self.year > 0 && chrono::NaiveDate::from_ymd_opt(i32::from(self.year), u32::from(self.month), u32::from(self.day)).is_some();
        let ok = day_ok && self.hour < 24 && self.minute < 60 && self.second < 61 && self.offset_minutes.is_none_or(|o| o.unsigned_abs() <= 18 * 60);
        ok.then_some(self)
    }

    /// Exif form `YYYY:MM:DD HH:MM:SS` (also tolerates `-` date separators and a missing time).
    pub fn parse_exif(s: &str) -> Option<DateTime> {
        let s = s.trim().trim_end_matches('\0');
        let (d, t) = s.split_once([' ', 'T']).unwrap_or((s, ""));
        let dp: Vec<&str> = d.split([':', '-']).collect();
        if dp.len() != 3 {
            return None;
        }
        let (hour, minute, second) = if t.is_empty() {
            (0, 0, 0)
        } else {
            let mut parts = t.split(':');
            let hour = num(parts.next()?.trim())?;
            let minute = parts.next().map(|part| num(part.trim())).unwrap_or(Some(0))?;
            let second = parts.next().map(|part| num(part.trim())).unwrap_or(Some(0))?;
            // A fourth value cannot be represented; accepting only the prefix could turn a
            // malformed DateTimeOriginal into a plausible capture time and block fallbacks.
            if parts.next().is_some() {
                return None;
            }
            (hour, minute, second)
        };
        DateTime {
            year: num(dp[0])?.try_into().ok()?,
            month: num(dp[1])?.try_into().ok()?,
            day: num(dp[2])?.try_into().ok()?,
            hour: hour.try_into().ok()?,
            minute: minute.try_into().ok()?,
            second: second.try_into().ok()?,
            millis: 0,
            offset_minutes: None,
            precision: DatePrecision::Full,
        }
        .valid()
    }

    /// Exif `OffsetTime*` form `+HH:MM` / `-HH:MM` (or `Z`).
    pub fn parse_offset(s: &str) -> Option<i16> {
        let s = s.trim().trim_end_matches('\0');
        if s == "Z" {
            return Some(0);
        }
        let (sign, rest) = match s.as_bytes().first()? {
            b'+' => (1, &s[1..]),
            b'-' => (-1, &s[1..]),
            _ => return None,
        };
        let (h, m) = match rest.split_once(':') {
            Some((h, m)) if m.len() == 2 => (num(h)?, num(m)?),
            Some(_) => return None,
            None if rest.len() == 4 => (num(rest.get(..2)?)?, num(rest.get(2..)?)?),
            None => (num(rest)?, 0),
        };
        if h > 18 || m > 59 || (h == 18 && m > 0) {
            return None;
        }
        Some(sign * (h * 60 + m) as i16)
    }

    /// Set the sub-second part from an Exif `SubSecTime*` string (digits are a decimal fraction).
    pub fn with_subsec(mut self, s: &str) -> DateTime {
        let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).take(3).collect();
        if !digits.is_empty() {
            let v: u16 = digits.parse().unwrap_or(0);
            self.millis = v * 10u16.pow(3 - digits.len() as u32);
        }
        self
    }

    /// ISO 8601 (XMP date): `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, `YYYY-MM-DDThh:mm[:ss[.s+]][TZD]`. A
    /// year or a month alone keeps its [`DatePrecision`].
    pub fn parse_iso(s: &str) -> Option<DateTime> {
        let s = s.trim();
        let (d, t) = match s.split_once('T') {
            Some((d, t)) if !t.is_empty() => (d, t),
            Some(_) => return None,
            None => (s, ""),
        };
        let dp: Vec<&str> = d.split('-').collect();
        let year = num(dp.first()?)?.try_into().ok()?;
        let month = dp.get(1).map(|m| num(m)).unwrap_or(Some(1))?.try_into().ok()?;
        let day = dp.get(2).map(|m| num(m)).unwrap_or(Some(1))?.try_into().ok()?;
        if dp.len() > 3 {
            return None;
        }
        let precision = match dp.len() {
            1 => DatePrecision::Year,
            2 => DatePrecision::Month,
            _ => DatePrecision::Full,
        };
        let mut dt = DateTime { year, month, day, precision, ..Default::default() };
        if !t.is_empty() {
            // a time of day needs the day it is on
            if !dt.precision.is_full() {
                return None;
            }
            let (clock, off) = match t.find(['Z', '+', '-']) {
                Some(i) => (&t[..i], Some(&t[i..])),
                None => (t, None),
            };
            let mut cp = clock.split(':');
            let hour = cp.next()?;
            let minute = cp.next()?;
            if hour.len() != 2 || minute.len() != 2 {
                return None;
            }
            dt.hour = num(hour)?.try_into().ok()?;
            dt.minute = num(minute)?.try_into().ok()?;
            if let Some(sec) = cp.next() {
                let (whole, fraction) = match sec.split_once('.') {
                    Some((whole, fraction)) => {
                        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                            return None;
                        }
                        (whole, Some(fraction))
                    }
                    None => (sec, None),
                };
                dt.second = num(whole)?.try_into().ok()?;
                if let Some(fraction) = fraction {
                    dt = dt.with_subsec(fraction);
                }
            }
            if cp.next().is_some() {
                return None;
            }
            if let Some(o) = off {
                dt.offset_minutes = Some(Self::parse_offset(o)?);
            }
        }
        dt.valid()
    }

    /// Whether the day was stated: Exif's date form can't say a year or a month alone, so only a
    /// full date is written there.
    pub fn is_full(&self) -> bool {
        self.precision.is_full()
    }

    /// Exif form `YYYY:MM:DD HH:MM:SS`.
    pub fn to_exif(&self) -> String {
        format!("{:04}:{:02}:{:02} {:02}:{:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }

    /// ISO 8601 with milliseconds when non-zero and the offset when known; a year or a month
    /// alone as stated (`1983`, `1983-04`).
    pub fn to_iso(&self) -> String {
        match self.precision {
            DatePrecision::Year => return format!("{:04}", self.year),
            DatePrecision::Month => return format!("{:04}-{:02}", self.year, self.month),
            DatePrecision::Full => {}
        }
        let mut s = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second);
        if self.millis != 0 {
            s += &format!(".{:03}", self.millis);
        }
        if let Some(o) = self.offset_minutes {
            let sign = if o < 0 { '-' } else { '+' };
            let o = o.unsigned_abs();
            s += &format!("{sign}{:02}:{:02}", o / 60, o % 60);
        }
        s
    }

    /// Seconds since 1970-01-01T00:00:00 of the civil time (UTC when the offset is known, else "local" seconds).
    pub fn unix_seconds(&self) -> i64 {
        // Every `DateTime` reaching here is a real calendar day (`valid()` ran in the parsers);
        // an out-of-range one (built by hand) reads as the epoch rather than panicking.
        let days = chrono::NaiveDate::from_ymd_opt(i32::from(self.year), u32::from(self.month), u32::from(self.day))
            .zip(chrono::NaiveDate::from_ymd_opt(1970, 1, 1))
            .map(|(d, epoch)| (d - epoch).num_days())
            .unwrap_or(0);
        days * 86400 + self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64 - self.offset_minutes.unwrap_or(0) as i64 * 60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_rejects_short_minute_field() {
        assert_eq!(DateTime::parse_offset("+08:5"), None);
        assert_eq!(DateTime::parse_offset("+1:2"), None);
        assert_eq!(DateTime::parse_offset("+08:05"), Some(485));
        assert_eq!(DateTime::parse_offset("-05:30"), Some(-330));
    }

    #[test]
    fn exif_forms() {
        let d = DateTime::parse_exif("2024:06:01 13:45:59").unwrap();
        assert_eq!((d.year, d.month, d.day, d.hour, d.minute, d.second), (2024, 6, 1, 13, 45, 59));
        assert_eq!(d.to_exif(), "2024:06:01 13:45:59");
        assert!(DateTime::parse_exif("0000:00:00 00:00:00").is_none());
        assert!(DateTime::parse_exif("    :  :     :  :  ").is_none());
        assert!(DateTime::parse_exif("2024:13:01 00:00:00").is_none());
        // Invalid calendar days must never enter persisted capture metadata.
        for value in ["2023:02:29", "2024:02:30", "2024:04:31", "1900:02:29", "2024:01:00"] {
            assert!(DateTime::parse_exif(value).is_none(), "{value}");
        }
        assert!(DateTime::parse_exif("2024:02:29").is_some());
        assert!(DateTime::parse_exif("2000:02:29").is_some());
        for value in ["2023-02-29", "2024-02-30", "1900-02-29", "2024-06-31T10:00:00Z"] {
            assert!(DateTime::parse_iso(value).is_none(), "{value}");
        }
        assert!(DateTime::parse_iso("2000-02-29T23:59:59Z").is_some());
        assert_eq!(DateTime::parse_exif("2024-06-01").unwrap().hour, 0);
        assert_eq!(DateTime::parse_exif("2024:06:01 13:45:59").unwrap().with_subsec("5").millis, 500);
        assert_eq!(DateTime::parse_exif("2024:06:01 13:45:59").unwrap().with_subsec("0123").millis, 12);
    }

    #[test]
    fn exif_clock_rejects_extra_components() {
        for value in
            ["2026:01:01 12:34:56:99", "2026:01:01 12:34:56:junk", "2026:01:01 12:34:56:", "2026:01:01 12:34:56::", "2026:01:01 12:34:56:99:junk"]
        {
            assert!(DateTime::parse_exif(value).is_none(), "accepted malformed EXIF clock {value:?}");
        }
    }

    #[test]
    fn exif_clock_supported_forms_keep_their_meaning() {
        for (value, expected) in [
            ("2026:01:01", "2026-01-01T00:00:00"),
            ("2026:01:01 12", "2026-01-01T12:00:00"),
            ("2026:01:01 12:34", "2026-01-01T12:34:00"),
            ("2026:01:01 12:34:56", "2026-01-01T12:34:56"),
            ("2026-01-01T12:34:56", "2026-01-01T12:34:56"),
            (" 2026:01:01 12:34:56\0", "2026-01-01T12:34:56"),
            ("2026:01:01T12:34:60", "2026-01-01T12:34:60"),
        ] {
            assert_eq!(DateTime::parse_exif(value).map(|date| date.to_iso()), Some(expected.to_string()), "{value:?}");
        }
    }

    /// XMP dates may stop at the year or the month (a scanned print's `photoshop:DateCreated`):
    /// read with that precision, written back as stated, and never as an Exif date.
    #[test]
    fn a_year_or_month_alone_reads_and_writes_as_stated() {
        for (iso, precision) in [("1983", DatePrecision::Year), ("1983-04", DatePrecision::Month)] {
            let d = DateTime::parse_iso(iso).unwrap();
            assert_eq!((d.year, d.month, d.day, d.precision), (1983, if iso.len() == 7 { 4 } else { 1 }, 1, precision), "{iso}");
            assert!(!d.is_full());
            assert_eq!(d.to_iso(), iso);
            let meta = crate::Metadata { capture_time: Some(d), ..Default::default() };
            let xmp = crate::write_xmp(&meta);
            assert!(!xmp.contains("DateTimeOriginal"), "{xmp}");
            assert_eq!(crate::parse_xmp(&xmp).unwrap().metadata.capture_time, Some(d), "{xmp}");
            assert_eq!(crate::read_exif(&crate::write_exif(&meta)).capture_time, None, "Exif can't say {iso}");
        }
        for not in ["0000", "1983-00", "1983-13", "1983-04T10:00:00", "1983T10:00"] {
            assert_eq!(DateTime::parse_iso(not), None, "{not}");
        }
        let full = DateTime::parse_iso("1983-04-12").unwrap();
        assert!(full.is_full());
        assert_eq!(full.to_iso(), "1983-04-12T00:00:00");
    }

    #[test]
    fn offsets() {
        assert_eq!(DateTime::parse_offset("+02:00"), Some(120));
        assert_eq!(DateTime::parse_offset("-05:30"), Some(-330));
        assert_eq!(DateTime::parse_offset("Z"), Some(0));
        assert_eq!(DateTime::parse_offset("+0930"), Some(570));
        assert_eq!(DateTime::parse_offset("+18:00"), Some(1080));
        assert_eq!(DateTime::parse_offset("-18:00"), Some(-1080));
        assert_eq!(DateTime::parse_offset("+18:01"), None);
        assert_eq!(DateTime::parse_offset("-18:01"), None);
        assert_eq!(DateTime::parse_offset("   :  "), None);
        assert_eq!(DateTime::parse_offset("+99:00"), None);
    }

    #[test]
    fn extreme_offsets_do_not_overflow() {
        // `DateTime` deserializes with public fields, so an offset can arrive without passing `valid()`
        let d = DateTime { offset_minutes: Some(i16::MIN), ..DateTime::parse_iso("2024-06-01T13:45:59").unwrap() };
        assert_eq!(d.to_iso(), "2024-06-01T13:45:59-546:08");
        assert_eq!(d.valid(), None);
    }

    #[test]
    fn unicode_offsets_are_rejected_without_panicking() {
        for offset in ["+1€", "-1€", "+€1", "-€1"] {
            assert_eq!(DateTime::parse_offset(offset), None);
            let date = format!("2024-06-01T13:45:59{offset}");
            let metadata = crate::Metadata { capture_time: DateTime::parse_iso("2024-06-01T13:45:59Z"), ..Default::default() };
            let xmp = crate::write_xmp(&metadata).replace("2024-06-01T13:45:59+00:00", &date);
            assert!(crate::parse_xmp(&xmp).unwrap().metadata.capture_time.is_none());
        }
    }

    #[test]
    fn iso_roundtrip() {
        for s in ["2024-06-01T13:45:59", "2024-06-01T13:45:59.250+02:00", "1999-12-31T23:59:00-05:30", "2024-02-29T00:00:00Z"] {
            let d = DateTime::parse_iso(s).unwrap();
            let again = DateTime::parse_iso(&d.to_iso()).unwrap();
            assert_eq!(d, again, "{s}");
        }
        assert_eq!(DateTime::parse_iso("2024-02-29T00:00:00Z").unwrap().to_iso(), "2024-02-29T00:00:00+00:00");
        let d = DateTime::parse_iso("2019").unwrap();
        assert_eq!((d.year, d.month, d.day), (2019, 1, 1));
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11").unwrap().minute, 11);
        assert!(DateTime::parse_iso("nonsense").is_none());
        assert!(DateTime::parse_iso("2019-07-04Tab:cd").is_none());
    }

    #[test]
    fn iso_rejects_incomplete_or_unconsumed_clock_text() {
        for value in [
            "2026-01-01T",
            "2026-01-01T12:34:56:99Z",
            "2026-01-01T12:34:56.Z",
            "2026-01-01T12:34:56.123junkZ",
            "2026-01-01T12:34:56.1.2Z",
            "2026-01-01T12:34:56.12 3Z",
            "2026-01-01T12:34:56:99Z",
            "2026-01-01T12::34Z",
        ] {
            assert!(DateTime::parse_iso(value).is_none(), "accepted malformed timestamp {value:?}");
        }
    }

    #[test]
    fn iso_rejects_single_digit_clock_fields() {
        for value in ["2024-06-01T1:02:03Z", "2024-06-01T01:2:03Z"] {
            assert!(DateTime::parse_iso(value).is_none(), "accepted malformed timestamp {value:?}");
        }
    }

    #[test]
    fn iso_validates_clock_while_preserving_supported_forms() {
        assert_eq!(DateTime::parse_iso("2019").unwrap().to_iso(), "2019");
        assert_eq!(DateTime::parse_iso("2019-07").unwrap().to_iso(), "2019-07");
        assert_eq!(DateTime::parse_iso("2019-07-04").unwrap().to_iso(), "2019-07-04T00:00:00");
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11").unwrap().to_iso(), "2019-07-04T10:11:00");
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11:12").unwrap().to_iso(), "2019-07-04T10:11:12");
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11:12Z").unwrap().offset_minutes, Some(0));
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11:12-05:30").unwrap().offset_minutes, Some(-330));
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11:12.123456Z").unwrap().millis, 123);
        assert_eq!(DateTime::parse_iso(" 2019-07-04T10:11:12.5Z ").unwrap().millis, 500);
        assert_eq!(DateTime::parse_iso("2019-07-04T10:11:60Z").unwrap().second, 60);
    }

    #[test]
    fn unix() {
        assert_eq!(DateTime::parse_iso("1970-01-01T00:00:00Z").unwrap().unix_seconds(), 0);
        assert_eq!(DateTime::parse_iso("2000-03-01T00:00:00Z").unwrap().unix_seconds(), 951868800);
        assert_eq!(DateTime::parse_iso("2000-03-01T02:00:00+02:00").unwrap().unix_seconds(), 951868800);
    }
}
