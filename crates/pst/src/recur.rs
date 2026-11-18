//! How an appointment repeats, as Outlook keeps it in PidLidAppointmentRecur: the
//! AppointmentRecurrencePattern and RecurrencePattern of [MS-OXOCAL] 2.2.1.44, and the time zone
//! of PidLidAppointmentTimeZoneDefinitionRecur (TZDEFINITION, 2.2.1.41).
//!
//! The bytes are hostile: each count is checked against the bytes there are, nothing is
//! allocated by what the blob says, and a part past its end is an error. The extended
//! exceptions, which only add the wide-character subject and location, may be missing or
//! damaged without the rest being lost.

use crate::error::{Error, Result};

/// Most exceptions kept of a series.
pub const MAX_EXCEPTIONS: usize = 5_000;
/// Longest a time zone's key name may be, in UTF-16 units.
const MAX_ZONE_KEY: usize = 256;

/// RecurFrequency.
pub const FREQ_DAILY: u16 = 0x200A;
pub const FREQ_WEEKLY: u16 = 0x200B;
pub const FREQ_MONTHLY: u16 = 0x200C;
pub const FREQ_YEARLY: u16 = 0x200D;
/// PatternType.
pub const PATTERN_DAY: u16 = 0;
pub const PATTERN_WEEK: u16 = 1;
pub const PATTERN_MONTH: u16 = 2;
pub const PATTERN_MONTH_NTH: u16 = 3;
pub const PATTERN_MONTH_END: u16 = 4;
/// EndType: on a date, after a number of occurrences. Anything else never ends.
pub const END_DATE: u32 = 0x2021;
pub const END_COUNT: u32 = 0x2022;

const ARO_SUBJECT: u16 = 0x0001;
const ARO_MEETINGTYPE: u16 = 0x0002;
const ARO_REMINDERDELTA: u16 = 0x0004;
const ARO_REMINDER: u16 = 0x0008;
const ARO_LOCATION: u16 = 0x0010;
const ARO_BUSYSTATUS: u16 = 0x0020;
const ARO_ATTACHMENT: u16 = 0x0040;
const ARO_SUBTYPE: u16 = 0x0080;
const ARO_APPTCOLOR: u16 = 0x0100;

/// One occurrence of a series that was changed. Times are minutes since 1601 by the clock of
/// the series' zone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecurException {
    pub start: u32,
    pub end: u32,
    /// When it would have started.
    pub original: u32,
    /// In the item's code page.
    pub subject_8bit: Option<Vec<u8>>,
    pub location_8bit: Option<Vec<u8>>,
    /// From the extended exceptions.
    pub subject: Option<String>,
    pub location: Option<String>,
    pub reminder_delta: Option<u32>,
    pub reminder_set: Option<bool>,
    pub busy: Option<u32>,
    pub all_day: Option<bool>,
}

/// A repeating appointment. Dates are minutes since 1601, at midnight by the series' clock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recur {
    pub frequency: u16,
    pub pattern: u16,
    /// 0 and 1 are the Gregorian calendar.
    pub calendar: u16,
    /// Minutes for a daily series, weeks for a weekly one, months for the others.
    pub period: u32,
    /// The days of the week (bit 0 is Sunday), for a weekly or nth-weekday series.
    pub weekdays: u32,
    /// The day of the month, or which of the weekdays (1 to 4, 5 for the last).
    pub day_or_nth: u32,
    pub end_type: u32,
    pub count: u32,
    /// The day weeks start on (0 is Sunday): what an interval of weeks is counted by.
    pub first_day_of_week: u32,
    /// Days that do not happen as the series says: deleted, and changed ones too.
    pub deleted: Vec<u32>,
    pub modified: Vec<u32>,
    pub start_date: u32,
    pub end_date: u32,
    /// Minutes from midnight to the start and to the end of an occurrence.
    pub start_offset: u32,
    pub end_offset: u32,
    pub exceptions: Vec<RecurException>,
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

const CUT: Error = Error::Corrupt("recurrence cut short");

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(CUT)?;
        let s = self.b.get(self.at..end).ok_or(CUT)?;
        self.at = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes(<[u8; 2]>::try_from(s).map_err(|_| CUT)?))
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes(<[u8; 4]>::try_from(s).map_err(|_| CUT)?))
    }

    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }

    /// A list of `u32`s after its count, the count checked against the bytes there are.
    fn dates(&mut self) -> Result<Vec<u32>> {
        let n = usize::try_from(self.u32()?).map_err(|_| CUT)?;
        if n > self.left() / 4 {
            return Err(CUT);
        }
        (0..n).map(|_| self.u32()).collect()
    }

    /// A block a size is written before, skipped.
    fn skip_block(&mut self) -> Result<()> {
        let n = usize::try_from(self.u32()?).map_err(|_| CUT)?;
        self.take(n).map(|_| ())
    }

    fn wide(&mut self) -> Result<String> {
        let n = usize::from(self.u16()?);
        let s = self.take(n.checked_mul(2).ok_or(CUT)?)?;
        let units = s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c));
        Ok(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect())
    }

    /// An exception's 8-bit text: its length plus one, its length, then the bytes.
    fn text_8bit(&mut self) -> Result<Vec<u8>> {
        let _with_end = self.u16()?;
        let n = usize::from(self.u16()?);
        Ok(self.take(n)?.to_vec())
    }
}

fn exception(c: &mut Cursor) -> Result<(RecurException, u16)> {
    let mut e = RecurException { start: c.u32()?, end: c.u32()?, original: c.u32()?, ..Default::default() };
    let flags = c.u16()?;
    if flags & ARO_SUBJECT != 0 {
        e.subject_8bit = Some(c.text_8bit()?);
    }
    if flags & ARO_MEETINGTYPE != 0 {
        c.u32()?;
    }
    if flags & ARO_REMINDERDELTA != 0 {
        e.reminder_delta = Some(c.u32()?);
    }
    if flags & ARO_REMINDER != 0 {
        e.reminder_set = Some(c.u32()? != 0);
    }
    if flags & ARO_LOCATION != 0 {
        e.location_8bit = Some(c.text_8bit()?);
    }
    if flags & ARO_BUSYSTATUS != 0 {
        e.busy = Some(c.u32()?);
    }
    if flags & ARO_ATTACHMENT != 0 {
        c.u32()?;
    }
    if flags & ARO_SUBTYPE != 0 {
        e.all_day = Some(c.u32()? != 0);
    }
    if flags & ARO_APPTCOLOR != 0 {
        c.u32()?;
    }
    Ok((e, flags))
}

/// One extended exception: the wide-character subject and location of the exception before it.
fn extended(c: &mut Cursor, writer: u32, e: &mut RecurException, flags: u16) -> Result<()> {
    if writer >= 0x3009 {
        c.skip_block()?;
    }
    c.skip_block()?;
    if flags & (ARO_SUBJECT | ARO_LOCATION) != 0 {
        c.take(12)?;
        if flags & ARO_SUBJECT != 0 {
            e.subject = Some(c.wide()?);
        }
        if flags & ARO_LOCATION != 0 {
            e.location = Some(c.wide()?);
        }
        c.skip_block()?;
    }
    Ok(())
}

/// The series in an AppointmentRecurrencePattern.
pub fn appointment_recurrence(blob: &[u8]) -> Result<Recur> {
    let mut c = Cursor { b: blob, at: 0 };
    if c.u16()? != 0x3004 || c.u16()? != 0x3004 {
        return Err(Error::Corrupt("recurrence of a version not known"));
    }
    let mut r = Recur { frequency: c.u16()?, pattern: c.u16()?, calendar: c.u16()?, ..Default::default() };
    let _first_date_time = c.u32()?;
    r.period = c.u32()?;
    let _sliding = c.u32()?;
    match r.pattern {
        PATTERN_DAY => {}
        PATTERN_WEEK => r.weekdays = c.u32()?,
        // (The Hijri and Hebrew patterns have the shape of these.)
        PATTERN_MONTH | PATTERN_MONTH_END | 0x0A | 0x0C => r.day_or_nth = c.u32()?,
        PATTERN_MONTH_NTH | 0x0B => {
            r.weekdays = c.u32()?;
            r.day_or_nth = c.u32()?;
        }
        _ => return Err(Error::Corrupt("recurrence of a pattern not known")),
    }
    r.end_type = c.u32()?;
    r.count = c.u32()?;
    r.first_day_of_week = c.u32()?;
    r.deleted = c.dates()?;
    r.modified = c.dates()?;
    r.start_date = c.u32()?;
    r.end_date = c.u32()?;
    let _reader = c.u32()?;
    let writer = c.u32()?;
    r.start_offset = c.u32()?;
    r.end_offset = c.u32()?;
    let count = usize::from(c.u16()?);
    if count > MAX_EXCEPTIONS {
        return Err(Error::Corrupt("recurrence with too many exceptions"));
    }
    let mut flags = Vec::new();
    for _ in 0..count {
        let (e, f) = exception(&mut c)?;
        r.exceptions.push(e);
        flags.push(f);
    }
    // The wide-character texts add to what is read: without them the 8-bit ones stand.
    if c.skip_block().is_ok() {
        for (e, f) in r.exceptions.iter_mut().zip(flags) {
            if extended(&mut c, writer, e, f).is_err() {
                break;
            }
        }
    }
    Ok(r)
}

/// The Windows name of the time zone in a TZDEFINITION (`W. Europe Standard Time`).
pub fn time_zone_key(blob: &[u8]) -> Result<String> {
    let mut c = Cursor { b: blob, at: 0 };
    let head = c.take(2)?;
    if head.first() != Some(&2) {
        return Err(Error::Corrupt("time zone of a version not known"));
    }
    let _header_size = c.u16()?;
    let _reserved = c.u16()?;
    let key = c.wide()?;
    if key.is_empty() || key.chars().count() > MAX_ZONE_KEY {
        return Err(Error::Corrupt("time zone without a usable name"));
    }
    Ok(key)
}
