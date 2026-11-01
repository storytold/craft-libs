//! What the printer says about itself and its jobs, read from attribute groups.
//!
//! Every field is an `Option` or an empty list: a printer that does not publish an attribute simply
//! has no value here, and the planner treats "not published" as "not checked" (the printer will
//! answer when the job arrives), as the WS-Print layer does.

use std::time::{Duration, SystemTime};

use super::codec::{Attribute, Group, Value};
use super::protocol::{job_state, printer_state};

/// The cap on how many entries of one list attribute are kept.
const MAX_LIST: usize = 512;

/// A media size the printer lists (`media-size-supported`), in hundredths of a millimetre; a range
/// is `(low, high)` and a fixed size has both equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaSize {
    pub width: (i32, i32),
    pub height: (i32, i32),
}

impl MediaSize {
    /// Whether a width and height (hundredths of a millimetre) fall inside this size, within the
    /// rounding of PWG self-describing names (half a millimetre).
    pub fn fits(&self, width: i32, height: i32) -> bool {
        const TOL: i32 = 50;
        let inside = |(low, high): (i32, i32), n: i32| n >= low.saturating_sub(TOL) && n <= high.saturating_add(TOL);
        inside(self.width, width) && inside(self.height, height)
    }
}

/// Dimensions in hundredths of a millimetre from a PWG self-describing media name
/// (`iso_a4_210x297mm`, `na_letter_8.5x11in`); `None` when the name does not end in `WxHmm` or `WxHin`.
pub fn media_name_size(name: &str) -> Option<(i32, i32)> {
    let last = name.rsplit('_').next()?;
    let (body, per_unit) = if let Some(b) = last.strip_suffix("mm") { (b, 100.0_f64) } else { (last.strip_suffix("in")?, 2540.0_f64) };
    let (w, h) = body.split_once('x')?;
    let (w, h): (f64, f64) = (w.parse().ok()?, h.parse().ok()?);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 || w > 1.0e6 || h > 1.0e6 {
        return None;
    }
    Some(((w * per_unit).round() as i32, (h * per_unit).round() as i32))
}

/// The attributes of a printer this backend reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrinterAttrs {
    pub name: Option<String>,
    pub info: Option<String>,
    pub make_and_model: Option<String>,
    pub state: Option<i32>,
    pub state_reasons: Vec<String>,
    pub accepting_jobs: Option<bool>,
    /// `operations-supported` (operation ids).
    pub operations: Option<Vec<i32>>,
    /// `document-format-supported`, as listed.
    pub formats: Vec<String>,
    /// `pwg-raster-document-type-supported`, as listed (`srgb_8`, `adobe-rgb_16`...); `None` when the printer does not
    /// publish it.
    pub raster_types: Option<Vec<String>>,
    pub sides: Option<Vec<String>>,
    pub color_modes: Option<Vec<String>>,
    pub color_supported: Option<bool>,
    /// `media-supported`, as listed (sizes and also tray or type names).
    pub media: Option<Vec<String>>,
    pub media_sizes: Option<Vec<MediaSize>>,
    /// `copies-supported`: the range.
    pub copies: Option<(i32, i32)>,
    /// `printer-resolution-supported` in dots per inch.
    pub resolutions: Option<Vec<(u32, u32)>>,
    /// `print-quality-supported`.
    pub qualities: Option<Vec<i32>>,
    pub content_optimize: Option<Vec<String>>,
    /// `print-scaling-supported` (PWG 5100.13): `None` when the printer does not publish it.
    pub scaling: Option<Vec<String>>,
    /// `print-scaling-default`.
    pub scaling_default: Option<String>,
    pub orientations: Option<Vec<i32>>,
    /// `multiple-operation-time-out`, seconds.
    pub multiple_operation_timeout: Option<u32>,
    /// `printer-up-time`, seconds.
    pub up_time: Option<i32>,
    /// `ipp-versions-supported`.
    pub versions: Vec<String>,
}

/// `v` cut to `MAX_LIST` entries.
fn cap<T>(mut v: Vec<T>) -> Vec<T> {
    v.truncate(MAX_LIST);
    v
}

/// The attribute, unless it is absent or carries only out-of-band values (`no-value`, `unknown`,
/// ...): a printer that answers `media-supported = no-value` has not said it supports nothing.
fn listed<'g>(group: &'g Group, name: &str) -> Option<&'g Attribute> {
    group.get(name).filter(|a| !a.values.iter().all(Value::is_out_of_band))
}

/// The string values of `name` (capped), unless it is absent or out-of-band only.
fn strings(group: &Group, name: &str) -> Option<Vec<String>> {
    listed(group, name).map(|a| cap(a.strings().into_iter().map(str::to_owned).collect()))
}

/// A collection member `name` as (lower, upper): an integer gives the same value twice, a range its
/// bounds; anything else is `None`.
fn dimension(values: &[(String, Vec<Value>)], name: &str) -> Option<(i32, i32)> {
    let value = values.iter().find(|(n, _)| n == name)?.1.first()?;
    match value {
        Value::Integer(n) => Some((*n, *n)),
        Value::Range { lower, upper } => Some((*lower, *upper)),
        _ => None,
    }
}

/// A `resolution` value in dots per inch; dots per centimetre are converted and snapped to the
/// nearest multiple of 25 dpi when within 1.27 dpi. Non-positive values or unknown units are `None`.
fn resolution_dpi(value: &Value) -> Option<(u32, u32)> {
    let Value::Resolution { cross_feed, feed, units } = value else { return None };
    let scale = match units {
        3 => 1.0,
        4 => 2.54,
        _ => return None,
    };
    // Dots per centimetre only approximate the usual dpi values (236 dpcm is 599.4 dpi): within half a
    // dot per centimetre (1.27 dpi) of a multiple of 25 dpi, that value is meant.
    let snap = |dpi: f64| {
        let near = (dpi / 25.0).round() * 25.0;
        if (dpi - near).abs() <= 1.27 { near } else { dpi.round() }
    };
    let to = |n: i32| u32::try_from(n).ok().filter(|n| *n > 0).map(|n| snap(f64::from(n) * scale) as u32);
    Some((to(*cross_feed)?, to(*feed)?))
}

impl PrinterAttrs {
    /// Read the printer group of a `Get-Printer-Attributes` reply.
    pub fn from_group(group: &Group) -> Self {
        let integers = |name: &str| listed(group, name).map(|a| cap(a.integers()));
        let copies = listed(group, "copies-supported").and_then(|a| a.first()).and_then(|v| match v {
            Value::Range { lower, upper } => Some((*lower, *upper)),
            Value::Integer(n) => Some((1, *n)),
            _ => None,
        });
        let media_sizes = listed(group, "media-size-supported").map(|a| {
            cap(a
                .values
                .iter()
                .filter_map(Value::as_collection)
                .filter_map(|members| Some(MediaSize { width: dimension(members, "x-dimension")?, height: dimension(members, "y-dimension")? }))
                .collect())
        });
        let resolutions = listed(group, "printer-resolution-supported").map(|a| cap(a.values.iter().filter_map(resolution_dpi).collect()));
        Self {
            name: group.first_str("printer-name").map(str::to_owned),
            info: group.first_str("printer-info").map(str::to_owned),
            make_and_model: group.first_str("printer-make-and-model").map(str::to_owned),
            state: group.first_i32("printer-state"),
            state_reasons: strings(group, "printer-state-reasons").unwrap_or_default(),
            accepting_jobs: group.first_bool("printer-is-accepting-jobs"),
            operations: integers("operations-supported"),
            formats: strings(group, "document-format-supported").unwrap_or_default(),
            raster_types: strings(group, "pwg-raster-document-type-supported"),
            sides: strings(group, "sides-supported"),
            color_modes: strings(group, "print-color-mode-supported"),
            color_supported: group.first_bool("color-supported"),
            media: strings(group, "media-supported"),
            media_sizes,
            copies,
            resolutions,
            qualities: integers("print-quality-supported"),
            content_optimize: strings(group, "print-content-optimize-supported"),
            scaling: strings(group, "print-scaling-supported"),
            scaling_default: group.first_str("print-scaling-default").map(str::to_owned),
            orientations: integers("orientation-requested-supported"),
            multiple_operation_timeout: group.first_i32("multiple-operation-time-out").and_then(|n| u32::try_from(n).ok()),
            up_time: group.first_i32("printer-up-time"),
            versions: strings(group, "ipp-versions-supported").unwrap_or_default(),
        }
    }

    /// Whether the printer lists this operation (`None`: it does not publish the list).
    pub fn supports_operation(&self, code: u16) -> Option<bool> {
        self.operations.as_ref().map(|ops| ops.contains(&i32::from(code)))
    }

    /// `Some(true)` when the printer lists the copy count, `Some(false)` when it lists a range
    /// without it.
    pub fn supports_copies(&self, n: u32) -> Option<bool> {
        let n = i32::try_from(n).ok()?;
        self.copies.map(|(low, high)| (low..=high).contains(&n))
    }

    /// Whether `sides-supported` lists `keyword`; `None` when the printer did not say.
    pub fn supports_sides(&self, keyword: &str) -> Option<bool> {
        self.sides.as_ref().map(|s| s.iter().any(|k| k == keyword))
    }

    /// Whether `orientation-requested-supported` lists the enum `value`; `None` when the printer did not say.
    pub fn supports_orientation(&self, value: i32) -> Option<bool> {
        self.orientations.as_ref().map(|o| o.contains(&value))
    }

    /// Whether `print-quality-supported` lists the enum `value`; `None` when the printer did not say.
    pub fn supports_quality(&self, value: i32) -> Option<bool> {
        self.qualities.as_ref().map(|q| q.contains(&value))
    }

    /// Whether the printer lists the resolution (x, y) in dpi; `None` when it did not say.
    pub fn supports_resolution(&self, (x, y): (u32, u32)) -> Option<bool> {
        self.resolutions.as_ref().map(|r| r.contains(&(x, y)))
    }

    /// Whether the media name is listed, by name or by the size it spells out.
    pub fn supports_media(&self, name: &str) -> Option<bool> {
        if self.media.is_none() && self.media_sizes.is_none() {
            return None;
        }
        if self.media.as_ref().is_some_and(|m| m.iter().any(|k| k == name)) {
            return Some(true);
        }
        let by_size = match (media_name_size(name), &self.media_sizes) {
            (Some((w, h)), Some(sizes)) => sizes.iter().any(|s| s.fits(w, h)),
            _ => false,
        };
        Some(by_size)
    }

    /// Whether `print-color-mode-supported` lists `keyword`; `None` when the printer did not say.
    pub fn supports_colour_mode(&self, keyword: &str) -> Option<bool> {
        self.color_modes.as_ref().map(|m| m.iter().any(|k| k == keyword))
    }

    /// Whether the printer lists this `print-scaling` keyword (`None`: it does not publish
    /// `print-scaling-supported`, so it has not said it takes the attribute at all).
    pub fn supports_scaling(&self, keyword: &str) -> Option<bool> {
        self.scaling.as_ref().map(|s| s.iter().any(|k| k == keyword))
    }

    /// Whether `print-content-optimize-supported` lists `keyword` (false when absent).
    pub fn lists_content_optimize(&self, keyword: &str) -> bool {
        self.content_optimize.as_ref().is_some_and(|c| c.iter().any(|k| k == keyword))
    }

    /// Whether a document of this MIME type is listed. A printer that lists no formats is not asked.
    pub fn lists_format(&self, mime: &str) -> Option<bool> {
        if self.formats.is_empty() {
            return None;
        }
        Some(self.formats.iter().any(|f| f.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case(mime)))
    }
}

/// The attributes of a job this backend reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobAttrs {
    pub id: Option<i32>,
    pub uri: Option<String>,
    pub state: Option<i32>,
    pub reasons: Vec<String>,
    pub name: Option<String>,
    pub user: Option<String>,
    /// `time-at-creation`, seconds on the printer's `printer-up-time` clock.
    pub time_at_creation: Option<i32>,
    /// `date-time-at-creation`, when the printer has a clock.
    pub created: Option<SystemTime>,
    pub impressions_completed: Option<i32>,
    pub sheets_completed: Option<i32>,
    pub impressions: Option<i32>,
    pub sheets: Option<i32>,
}

impl JobAttrs {
    /// Read a job group; every attribute is optional, and `date-time-at-creation` is converted to a
    /// wall-clock time when valid.
    pub fn from_group(group: &Group) -> Self {
        let created = group.get("date-time-at-creation").and_then(|a| a.first()).and_then(|v| match v {
            Value::DateTime(d) => d.to_system_time(),
            _ => None,
        });
        Self {
            id: group.first_i32("job-id"),
            uri: group.first_str("job-uri").map(str::to_owned),
            state: group.first_i32("job-state"),
            reasons: strings(group, "job-state-reasons").unwrap_or_default(),
            name: group.first_str("job-name").map(str::to_owned),
            user: group.first_str("job-originating-user-name").map(str::to_owned),
            time_at_creation: group.first_i32("time-at-creation"),
            created,
            impressions_completed: group.first_i32("job-impressions-completed"),
            sheets_completed: group.first_i32("job-media-sheets-completed"),
            impressions: group.first_i32("job-impressions"),
            sheets: group.first_i32("job-media-sheets"),
        }
    }

    /// Whether `job-state-reasons` includes `reason`.
    pub fn has_reason(&self, reason: &str) -> bool {
        self.reasons.iter().any(|r| r == reason)
    }

    /// The wall-clock creation time: the printer's own date when it has one, else the printer's
    /// uptime clock related to `up_time` (its `printer-up-time` read about `now`).
    pub fn created_at(&self, up_time: Option<i32>, now: SystemTime) -> Option<SystemTime> {
        if self.created.is_some() {
            return self.created;
        }
        let (created, up) = (self.time_at_creation?, up_time?);
        let age = u64::try_from(up.checked_sub(created)?).ok()?;
        now.checked_sub(Duration::from_secs(age))
    }
}

/// The neutral job state of a `job-state` value and its reasons (RFC 8011 section 5.3.7 and the
/// registered `job-state-reasons`).
pub fn neutral_state(job: &JobAttrs) -> crate::JobState {
    use crate::JobState as S;
    let Some(state) = job.state else { return S::Unknown };
    match state {
        job_state::PENDING if job.has_reason("job-hold-until-specified") => S::Held,
        job_state::PENDING => S::Pending,
        job_state::PENDING_HELD => S::Held,
        job_state::PROCESSING => S::Processing,
        job_state::PROCESSING_STOPPED => S::NeedsAttention,
        job_state::CANCELED => S::Cancelled,
        job_state::ABORTED => S::Aborted,
        job_state::COMPLETED if job.has_reason("job-completed-with-errors") => S::CompletedWithErrors,
        job_state::COMPLETED => S::Completed,
        _ => S::Unknown,
    }
}

/// The printer's state as the neutral one; `offline-report` turns a stopped printer offline.
pub fn neutral_printer_state(attrs: &PrinterAttrs) -> Option<crate::PrinterState> {
    use crate::PrinterState as P;
    Some(match attrs.state? {
        printer_state::IDLE => P::Idle,
        printer_state::PROCESSING => P::Processing,
        printer_state::STOPPED if attrs.state_reasons.iter().any(|r| r.starts_with("offline-report")) => P::Offline,
        printer_state::STOPPED => P::Stopped,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::codec::{Attribute, GroupTag};

    #[test]
    fn media_names_spell_their_sizes() {
        assert_eq!(media_name_size("iso_a4_210x297mm"), Some((21000, 29700)));
        assert_eq!(media_name_size("na_letter_8.5x11in"), Some((21590, 27940)));
        assert_eq!(media_name_size("om_small-photo_100x150mm"), Some((10000, 15000)));
        for no in ["tray-1", "auto", "iso_a4", "x_0x5mm", "x_axbmm", "x_-1x2mm", ""] {
            assert_eq!(media_name_size(no), None, "{no}");
        }
        let size = MediaSize { width: (21000, 21000), height: (14800, 29700) };
        assert!(size.fits(21000, 29700) && size.fits(20960, 14800) && !size.fits(21000, 30000) && !size.fits(22000, 20000));
    }

    #[test]
    fn a_size_range_at_the_ends_of_i32_neither_overflows_nor_widens() {
        let all = MediaSize { width: (i32::MIN, i32::MAX), height: (i32::MIN, i32::MAX) };
        assert!(all.fits(21000, 29700) && all.fits(i32::MIN, i32::MAX));
        let edge = MediaSize { width: (i32::MAX - 10, i32::MAX), height: (0, 0) };
        assert!(edge.fits(i32::MAX, 50) && !edge.fits(0, 0));
    }

    #[test]
    fn out_of_band_values_mean_not_listed_rather_than_supports_nothing() {
        let group = Group::new(GroupTag::Printer)
            .with(Attribute::new("media-supported", Value::NoValue))
            .with(Attribute::new("sides-supported", Value::Unknown))
            .with(Attribute::new("print-quality-supported", Value::NoValue))
            .with(Attribute::new("copies-supported", Value::Unknown))
            .with(Attribute::new("printer-resolution-supported", Value::NoValue));
        let a = PrinterAttrs::from_group(&group);
        assert_eq!(a.supports_media("iso_a4_210x297mm"), None);
        assert_eq!(a.supports_sides("one-sided"), None);
        assert_eq!(a.supports_quality(4), None);
        assert_eq!(a.supports_copies(2), None);
        assert_eq!(a.supports_resolution((300, 300)), None);
    }

    #[test]
    fn dots_per_centimetre_land_on_the_dpi_they_stand_for() {
        let res = |n: i32| resolution_dpi(&Value::Resolution { cross_feed: n, feed: n, units: 4 });
        assert_eq!(res(236), Some((600, 600)), "599.4 dpi is 600");
        assert_eq!(res(118), Some((300, 300)));
        assert_eq!(res(472), Some((1200, 1200)));
        assert_eq!(res(47), Some((119, 119)), "not near a usual value: rounded as it is");
    }

    /// A stopped, offline printer group with copies, resolutions and media attributes for the readers.
    fn printer() -> Group {
        Group::new(GroupTag::Printer)
            .with(Attribute::name_value("printer-name", "Office"))
            .with(Attribute::enumeration("printer-state", 5))
            .with(Attribute::keywords("printer-state-reasons", &["offline-report", "media-empty-warning"]))
            .with(Attribute::set("operations-supported", vec![Value::Enum(2), Value::Enum(11)]))
            .with(Attribute::new("copies-supported", Value::Range { lower: 1, upper: 20 }))
            .with(Attribute::set(
                "printer-resolution-supported",
                vec![
                    Value::Resolution { cross_feed: 600, feed: 600, units: 3 },
                    Value::Resolution { cross_feed: 118, feed: 118, units: 4 },
                    Value::Resolution { cross_feed: 1, feed: 1, units: 9 },
                ],
            ))
            .with(Attribute::keywords("media-supported", &["iso_a4_210x297mm", "tray-1"]))
            .with(Attribute::set(
                "media-size-supported",
                vec![Value::Collection(vec![
                    ("x-dimension".into(), vec![Value::Integer(21590)]),
                    ("y-dimension".into(), vec![Value::Integer(27940)]),
                ])],
            ))
            .with(Attribute::new("multiple-operation-time-out", Value::Integer(120)))
    }

    #[test]
    fn a_printer_group_is_read_and_checked() {
        let a = PrinterAttrs::from_group(&printer());
        assert_eq!(a.name.as_deref(), Some("Office"));
        assert_eq!(neutral_printer_state(&a), Some(crate::PrinterState::Offline));
        assert_eq!(a.supports_operation(2), Some(true));
        assert_eq!(a.supports_operation(5), Some(false));
        assert_eq!((a.supports_copies(20), a.supports_copies(21)), (Some(true), Some(false)));
        assert_eq!(a.resolutions, Some(vec![(600, 600), (300, 300)]), "dots per centimetre are converted, unknown units dropped");
        assert_eq!(a.supports_resolution((300, 300)), Some(true));
        assert_eq!(a.supports_media("iso_a4_210x297mm"), Some(true));
        assert_eq!(a.supports_media("na_letter_8.5x11in"), Some(true), "listed by size");
        assert_eq!(a.supports_media("iso_a3_297x420mm"), Some(false));
        assert_eq!(a.multiple_operation_timeout, Some(120));
        // Nothing published: nothing is checked.
        let empty = PrinterAttrs::from_group(&Group::new(GroupTag::Printer));
        assert_eq!(
            (
                empty.supports_copies(5),
                empty.supports_sides("one-sided"),
                empty.supports_media("x"),
                empty.supports_quality(4),
                empty.lists_format("application/pdf")
            ),
            (None, None, None, None, None)
        );
    }

    #[test]
    fn job_states_map_to_the_neutral_ones() {
        use crate::JobState as S;
        let job = |state: i32, reasons: &[&str]| JobAttrs {
            state: Some(state),
            reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
            ..JobAttrs::default()
        };
        assert_eq!(neutral_state(&job(3, &["job-incoming"])), S::Pending);
        assert_eq!(neutral_state(&job(3, &["job-hold-until-specified"])), S::Held);
        assert_eq!(neutral_state(&job(4, &[])), S::Held);
        assert_eq!(neutral_state(&job(5, &["job-printing"])), S::Processing);
        assert_eq!(neutral_state(&job(6, &["printer-stopped"])), S::NeedsAttention);
        assert_eq!(neutral_state(&job(7, &["job-canceled-by-user"])), S::Cancelled);
        assert_eq!(neutral_state(&job(8, &["aborted-by-system"])), S::Aborted);
        assert_eq!(neutral_state(&job(9, &["job-completed-successfully"])), S::Completed);
        assert_eq!(neutral_state(&job(9, &["job-completed-with-warnings"])), S::Completed);
        assert_eq!(neutral_state(&job(9, &["job-completed-with-errors"])), S::CompletedWithErrors);
        assert_eq!(neutral_state(&job(42, &[])), S::Unknown);
        assert_eq!(neutral_state(&JobAttrs::default()), S::Unknown);
    }

    #[test]
    fn creation_times_come_from_the_date_or_the_uptime_clock() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let rel = JobAttrs { time_at_creation: Some(900), ..JobAttrs::default() };
        assert_eq!(rel.created_at(Some(1000), now), Some(now - Duration::from_secs(100)));
        assert_eq!(rel.created_at(None, now), None);
        assert_eq!(rel.created_at(Some(800), now), None, "a clock that went backwards says nothing");
        let dated = JobAttrs { created: Some(SystemTime::UNIX_EPOCH), time_at_creation: Some(1), ..JobAttrs::default() };
        assert_eq!(dated.created_at(Some(1000), now), Some(SystemTime::UNIX_EPOCH));
    }
}
