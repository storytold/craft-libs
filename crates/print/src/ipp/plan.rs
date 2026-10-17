//! From the neutral job settings to IPP job attributes, checked against what the printer lists.
//!
//! The rule is the WS-Print layer's. A setting the printer **lists as unsupported** (it publishes
//! the list and the value is not in it) is dropped with a warning, or refused in strict mode; a
//! dimension the printer does not publish is not checked (the printer answers at `Print-Job`, with
//! `ipp-attribute-fidelity` set in strict mode so that it refuses instead of substituting).

use super::client::JobRequest;
use super::codec::{Attribute, Value};
use super::error::{IppError, Result};
use super::model::PrinterAttrs;
use super::protocol::{orientation, quality};
use crate::{ColourMode, Duplex, Job, JobOptions, Orientation, Quality, Scaling};

/// The most copies a job may ask for.
pub const MAX_COPIES: u32 = 9999;

/// The settings of one job after planning.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub request: JobRequest,
    /// What was dropped or changed, as sentences for people.
    pub warnings: Vec<String>,
}

/// `sides` keyword of the neutral duplex setting.
pub fn sides_keyword(duplex: Duplex) -> &'static str {
    match duplex {
        Duplex::Off => "one-sided",
        Duplex::LongEdge => "two-sided-long-edge",
        Duplex::ShortEdge => "two-sided-short-edge",
    }
}

/// `orientation-requested` enum of the neutral orientation.
pub fn orientation_value(o: Orientation) -> i32 {
    match o {
        Orientation::Landscape => orientation::LANDSCAPE,
        Orientation::ReversePortrait => orientation::REVERSE_PORTRAIT,
        Orientation::ReverseLandscape => orientation::REVERSE_LANDSCAPE,
        _ => orientation::PORTRAIT,
    }
}

/// `print-quality` enum of the neutral quality. [`Quality::Photo`] is the highest the enum has.
pub fn quality_value(q: Quality) -> i32 {
    match q {
        Quality::Draft => quality::DRAFT,
        Quality::High | Quality::Photo => quality::HIGH,
        _ => quality::NORMAL,
    }
}

/// The `print-scaling` keyword (PWG 5100.13 section 5.2.7) of the neutral scaling: `auto-fit` for
/// [`Scaling::ShrinkToFit`] (oversized pages are fitted, pages that already fit stay as they are,
/// the meaning of the neutral setting; `fit` would also enlarge, so it is not substituted) and
/// `none` for [`Scaling::Exact`] (the actual size).
pub fn scaling_keyword(scaling: Scaling) -> &'static str {
    match scaling {
        Scaling::Exact => "none",
        Scaling::ShrinkToFit => "auto-fit",
    }
}

/// Whether the printer says it takes [`Scaling::ShrinkToFit`] (lists `auto-fit`).
pub fn scaling_advertised(printer: &PrinterAttrs) -> bool {
    printer.supports_scaling(scaling_keyword(Scaling::ShrinkToFit)) == Some(true)
}

/// The scaling attribute to send, if any. Nothing is sent to a printer that has not said it takes
/// `print-scaling`: shrink-to-fit is then a warning (a refusal in strict mode when the printer
/// publishes a list without the keyword, as for the other settings) and the document goes as it is.
/// Actual size is the default of the neutral job; `none` is sent only to a printer that lists it
/// and whose own default is something else.
fn plan_scaling(scaling: Scaling, strict: bool, printer: &PrinterAttrs, warnings: &mut Vec<String>) -> Result<Option<&'static str>> {
    let keyword = scaling_keyword(scaling);
    match scaling {
        Scaling::ShrinkToFit => match printer.supports_scaling(keyword) {
            Some(true) => Ok(Some(keyword)),
            Some(false) if strict => Err(IppError::Unsupported(format!("print-scaling {keyword}"))),
            Some(false) => {
                warnings.push(format!("scaling is not applied: the printer does not list print-scaling {keyword}; the document is sent as it is"));
                Ok(None)
            }
            None => {
                warnings
                    .push("scaling is not applied: the document is sent as it is (the printer does not publish print-scaling-supported)".to_owned());
                Ok(None)
            }
        },
        Scaling::Exact => {
            let differs = printer.scaling_default.as_deref().is_some_and(|d| d != keyword);
            Ok((differs && printer.supports_scaling(keyword) == Some(true)).then_some(keyword))
        }
    }
}

fn keep(strict: bool, what: &str, supported: Option<bool>, warnings: &mut Vec<String>) -> Result<bool> {
    match supported {
        Some(false) if strict => Err(IppError::Unsupported(what.to_owned())),
        Some(false) => {
            warnings.push(format!("the printer does not list {what}; it was left out"));
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// Plan one job.
pub fn plan(job_name: String, document_mime: &str, job: &Job, per_job: &JobOptions, strict: bool, printer: &PrinterAttrs) -> Result<Plan> {
    let mut warnings = Vec::new();
    if job.copies == 0 || job.copies > MAX_COPIES {
        return Err(IppError::Invalid(format!("copies {}: use 1 to {MAX_COPIES}", job.copies)));
    }
    if printer.lists_format(document_mime) == Some(false) {
        return Err(IppError::UnsupportedFormat { document: document_mime.to_owned(), accepted: printer.formats.clone() });
    }
    let scaling = plan_scaling(job.scaling, strict, printer, &mut warnings)?;
    if !job.options.is_empty() {
        warnings.push("driver options are not applied by IPP".to_owned());
    }
    let mut attributes: Vec<Attribute> = Vec::new();

    if job.copies != 1 {
        let n = job.copies;
        let wanted = i32::try_from(n).unwrap_or(i32::MAX);
        // Below the published minimum is a mistake in the request, not "too many": refused here and
        // never turned into the maximum.
        if let Some((low, _)) = printer.copies
            && wanted < low.max(1)
        {
            return Err(IppError::Invalid(format!("{n} copies is below the printer's minimum of {}", low.max(1))));
        }
        match printer.supports_copies(n) {
            Some(false) if !strict => {
                let max = printer.copies.map_or(1, |(_, max)| max).max(1);
                warnings.push(format!("the printer takes at most {max} copies; {n} was lowered to {max}"));
                attributes.push(Attribute::integer("copies", max));
            }
            other => {
                if keep(strict, &format!("{n} copies"), other, &mut warnings)? {
                    attributes.push(Attribute::integer("copies", wanted));
                }
            }
        }
    }
    if let Some(duplex) = job.duplex {
        let keyword = sides_keyword(duplex);
        if keep(strict, &format!("sides {keyword}"), printer.supports_sides(keyword), &mut warnings)? {
            attributes.push(Attribute::keyword("sides", keyword));
        }
    }
    if let Some(o) = per_job.orientation {
        let value = orientation_value(o);
        if keep(strict, &format!("orientation {}", o.name()), printer.supports_orientation(value), &mut warnings)? {
            attributes.push(Attribute::enumeration("orientation-requested", value));
        }
    }
    if let Some(q) = per_job.quality {
        let value = quality_value(q);
        if keep(strict, &format!("quality {}", q.name()), printer.supports_quality(value), &mut warnings)? {
            attributes.push(Attribute::enumeration("print-quality", value));
        }
        // The printer's own keyword for photographic output, where it lists one.
        if q == Quality::Photo && printer.lists_content_optimize("photo") {
            attributes.push(Attribute::keyword("print-content-optimize", "photo"));
        }
    }
    if let Some((x, y)) = per_job.resolution.map(|r| (r.x_dpi, r.y_dpi)) {
        let ok = x > 0 && y > 0 && x <= 100_000 && y <= 100_000;
        if !ok {
            return Err(IppError::Invalid(format!("resolution {x}x{y}")));
        }
        if keep(strict, &format!("resolution {x}x{y}"), printer.supports_resolution((x, y)), &mut warnings)?
            && let (Ok(cross_feed), Ok(feed)) = (i32::try_from(x), i32::try_from(y))
        {
            attributes.push(Attribute::new("printer-resolution", Value::Resolution { cross_feed, feed, units: 3 }));
        }
    }
    if let Some(media) = per_job.media.as_deref().filter(|m| !m.is_empty()) {
        if media.len() > 255 || !media.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(IppError::Invalid(format!("media {media:?}: not a media name")));
        }
        if keep(strict, &format!("media {media}"), printer.supports_media(media), &mut warnings)? {
            attributes.push(Attribute::keyword("media", media));
        }
    }
    let mono = match (per_job.colour, job.grayscale) {
        (ColourMode::Colour, _) => Some(false),
        (ColourMode::Mono, _) | (ColourMode::Auto, true) => Some(true),
        (ColourMode::Auto, false) => None,
    };
    match mono {
        Some(false) => {
            let cannot = printer.color_supported == Some(false) || printer.supports_colour_mode("color") == Some(false);
            if cannot {
                if strict {
                    return Err(IppError::Unsupported("colour (the printer is monochrome)".into()));
                }
                warnings.push("the printer is monochrome; the document is printed in grey".into());
                if printer.supports_colour_mode("monochrome") != Some(false) {
                    attributes.push(Attribute::keyword("print-color-mode", "monochrome"));
                }
            } else {
                attributes.push(Attribute::keyword("print-color-mode", "color"));
            }
        }
        Some(true) if keep(strict, "monochrome output", printer.supports_colour_mode("monochrome"), &mut warnings)? => {
            attributes.push(Attribute::keyword("print-color-mode", "monochrome"));
        }
        Some(true) => {}
        None => {}
    }
    if let Some(keyword) = scaling {
        attributes.push(Attribute::keyword("print-scaling", keyword));
    }
    Ok(Plan {
        request: JobRequest { job_name, document_name: None, document_format: Some(document_mime.to_owned()), fidelity: strict, attributes },
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resolution;

    fn printer() -> PrinterAttrs {
        PrinterAttrs {
            formats: vec!["application/pdf".into(), "image/pwg-raster".into()],
            sides: Some(vec!["one-sided".into(), "two-sided-long-edge".into()]),
            color_modes: Some(vec!["color".into(), "monochrome".into()]),
            media: Some(vec!["iso_a4_210x297mm".into()]),
            copies: Some((1, 5)),
            resolutions: Some(vec![(300, 300), (600, 600)]),
            qualities: Some(vec![3, 4, 5]),
            orientations: Some(vec![3, 4]),
            ..PrinterAttrs::default()
        }
    }

    fn run(job: &Job, options: &JobOptions, strict: bool, p: &PrinterAttrs) -> Result<Plan> {
        plan("Report".into(), "application/pdf", job, options, strict, p)
    }

    fn names(plan: &Plan) -> Vec<&str> {
        plan.request.attributes.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn nothing_asked_nothing_sent() {
        let p = run(&Job::default(), &JobOptions::new(), false, &printer()).unwrap();
        assert!(p.request.attributes.is_empty() && p.warnings.is_empty() && !p.request.fidelity);
        assert_eq!(p.request.document_format.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn every_setting_becomes_its_attribute() {
        let job = Job { copies: 3, duplex: Some(Duplex::ShortEdge), grayscale: false, ..Job::default() };
        let options = JobOptions::new()
            .with_orientation(Orientation::Landscape)
            .with_quality(Quality::High)
            .with_resolution(Resolution::new(600, 600))
            .with_media("iso_a4_210x297mm")
            .with_colour(ColourMode::Colour);
        let mut p = printer();
        p.sides = Some(vec!["two-sided-short-edge".into()]);
        let planned = run(&job, &options, false, &p).unwrap();
        assert_eq!(names(&planned), ["copies", "sides", "orientation-requested", "print-quality", "printer-resolution", "media", "print-color-mode"]);
        let get = |n: &str| planned.request.attributes.iter().find(|a| a.name == n).unwrap().values.clone();
        assert_eq!(get("copies"), [Value::Integer(3)]);
        assert_eq!(get("sides"), [Value::Keyword("two-sided-short-edge".into())]);
        assert_eq!(get("orientation-requested"), [Value::Enum(4)]);
        assert_eq!(get("print-quality"), [Value::Enum(5)]);
        assert_eq!(get("printer-resolution"), [Value::Resolution { cross_feed: 600, feed: 600, units: 3 }]);
        assert_eq!(get("print-color-mode"), [Value::Keyword("color".into())]);
        assert!(planned.warnings.is_empty(), "{:?}", planned.warnings);
    }

    fn scaled(set: Option<&[&str]>, default: Option<&str>) -> PrinterAttrs {
        PrinterAttrs { scaling: set.map(|s| s.iter().map(|k| (*k).to_owned()).collect()), scaling_default: default.map(str::to_owned), ..printer() }
    }

    fn scaling_of(plan: &Plan) -> Option<Vec<Value>> {
        plan.request.attributes.iter().find(|a| a.name == "print-scaling").map(|a| a.values.clone())
    }

    #[test]
    fn scaling_keywords_follow_pwg_5100_13() {
        assert_eq!(scaling_keyword(Scaling::ShrinkToFit), "auto-fit");
        assert_eq!(scaling_keyword(Scaling::Exact), "none");
    }

    #[test]
    fn shrink_to_fit_is_sent_as_auto_fit_to_a_printer_that_lists_it() {
        let shrink = Job { scaling: Scaling::ShrinkToFit, ..Job::default() };
        for strict in [false, true] {
            let p = run(&shrink, &JobOptions::new(), strict, &scaled(Some(&["auto", "auto-fit", "fit", "fill", "none"]), Some("auto"))).unwrap();
            assert_eq!(scaling_of(&p), Some(vec![Value::Keyword("auto-fit".into())]));
            assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        }
        assert!(scaling_advertised(&scaled(Some(&["auto-fit"]), None)));
        assert!(!scaling_advertised(&scaled(Some(&["none", "fit"]), None)));
        assert!(!scaling_advertised(&scaled(None, None)));
    }

    #[test]
    fn shrink_to_fit_without_the_keyword_is_dropped_with_a_warning_or_refused_when_strict() {
        let shrink = Job { scaling: Scaling::ShrinkToFit, ..Job::default() };
        // Published list without auto-fit; `fit` is not a substitute (it enlarges).
        let listed = scaled(Some(&["none", "fit"]), Some("none"));
        let p = run(&shrink, &JobOptions::new(), false, &listed).unwrap();
        assert_eq!(scaling_of(&p), None);
        assert!(p.warnings.iter().any(|w| w.contains("scaling is not applied") && w.contains("auto-fit")), "{:?}", p.warnings);
        assert!(matches!(run(&shrink, &JobOptions::new(), true, &listed), Err(IppError::Unsupported(_))));
    }

    #[test]
    fn shrink_to_fit_is_not_sent_to_a_printer_that_never_said_it_takes_it() {
        let shrink = Job { scaling: Scaling::ShrinkToFit, ..Job::default() };
        for strict in [false, true] {
            let p = run(&shrink, &JobOptions::new(), strict, &scaled(None, None)).unwrap();
            assert_eq!(scaling_of(&p), None);
            assert_eq!(p.warnings.len(), 1);
            assert!(p.warnings[0].starts_with("scaling is not applied: the document is sent as it is"), "{:?}", p.warnings);
        }
        let bare = run(&shrink, &JobOptions::new(), false, &PrinterAttrs::default()).unwrap();
        assert_eq!((scaling_of(&bare), bare.warnings.len()), (None, 1));
    }

    #[test]
    fn actual_size_sends_none_only_where_the_default_differs() {
        let exact = Job::default();
        let sent = |set: Option<&[&str]>, default: Option<&str>| scaling_of(&run(&exact, &JobOptions::new(), false, &scaled(set, default)).unwrap());
        assert_eq!(sent(Some(&["auto", "none"]), Some("auto")), Some(vec![Value::Keyword("none".into())]));
        assert_eq!(sent(Some(&["auto", "none"]), Some("none")), None, "already the default");
        assert_eq!(sent(Some(&["auto", "none"]), None), None, "default unknown: nothing assumed");
        assert_eq!(sent(Some(&["auto", "fit"]), Some("auto")), None, "none not listed");
        assert_eq!(sent(None, Some("auto")), None);
        let p = run(&exact, &JobOptions::new(), true, &scaled(Some(&["auto"]), Some("auto"))).unwrap();
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn orientation_and_sides_enums_follow_rfc_8011() {
        assert_eq!(orientation_value(Orientation::Portrait), 3);
        assert_eq!(orientation_value(Orientation::Landscape), 4);
        assert_eq!(orientation_value(Orientation::ReverseLandscape), 5);
        assert_eq!(orientation_value(Orientation::ReversePortrait), 6);
        assert_eq!(
            (quality_value(Quality::Draft), quality_value(Quality::Normal), quality_value(Quality::High), quality_value(Quality::Photo)),
            (3, 4, 5, 5)
        );
        assert_eq!(sides_keyword(Duplex::Off), "one-sided");
        assert_eq!(sides_keyword(Duplex::LongEdge), "two-sided-long-edge");
    }

    #[test]
    fn photo_uses_the_printers_own_keyword_when_it_lists_one() {
        let mut p = printer();
        let plain = run(&Job::default(), &JobOptions::new().with_quality(Quality::Photo), false, &p).unwrap();
        assert_eq!(names(&plain), ["print-quality"]);
        p.content_optimize = Some(vec!["auto".into(), "photo".into()]);
        let tuned = run(&Job::default(), &JobOptions::new().with_quality(Quality::Photo), false, &p).unwrap();
        assert_eq!(names(&tuned), ["print-quality", "print-content-optimize"]);
        // A printer without a high quality still gets its photo keyword.
        p.qualities = Some(vec![3, 4]);
        let low = run(&Job::default(), &JobOptions::new().with_quality(Quality::Photo), false, &p).unwrap();
        assert_eq!(names(&low), ["print-content-optimize"]);
        assert_eq!(low.warnings.len(), 1);
    }

    #[test]
    fn unlisted_settings_are_dropped_with_a_warning_or_refused_when_strict() {
        let job = Job { copies: 9, duplex: Some(Duplex::ShortEdge), ..Job::default() };
        let options = JobOptions::new()
            .with_media("iso_a3_297x420mm")
            .with_resolution(Resolution::new(1200, 1200))
            .with_orientation(Orientation::ReverseLandscape);
        let lenient = run(&job, &options, false, &printer()).unwrap();
        assert_eq!(names(&lenient), ["copies"]);
        assert_eq!(lenient.request.attributes[0].values, [Value::Integer(5)], "lowered to what the printer takes");
        assert_eq!(lenient.warnings.len(), 5, "{:?}", lenient.warnings);
        for (j, o) in [
            (Job { copies: 9, ..Job::default() }, JobOptions::new()),
            (Job { duplex: Some(Duplex::ShortEdge), ..Job::default() }, JobOptions::new()),
            (Job::default(), JobOptions::new().with_media("iso_a3_297x420mm")),
            (Job::default(), JobOptions::new().with_resolution(Resolution::new(1200, 1200))),
            (Job::default(), JobOptions::new().with_orientation(Orientation::ReversePortrait)),
        ] {
            assert!(matches!(run(&j, &o, true, &printer()), Err(IppError::Unsupported(_))), "{j:?} {o:?}");
        }
        let ok = run(&Job { copies: 2, ..Job::default() }, &JobOptions::new(), true, &printer()).unwrap();
        assert!(ok.request.fidelity && ok.warnings.is_empty());
    }

    #[test]
    fn copies_below_the_published_minimum_are_refused_and_above_the_maximum_are_lowered() {
        let with = |range: Option<(i32, i32)>, n: u32, strict: bool| {
            let p = PrinterAttrs { copies: range, ..PrinterAttrs::default() };
            run(&Job { copies: n, ..Job::default() }, &JobOptions::new(), strict, &p)
        };
        for range in [Some((1, 99)), Some((3, 9)), None] {
            for strict in [false, true] {
                assert!(matches!(with(range, 0, strict), Err(IppError::Invalid(_))), "{range:?}");
            }
        }
        for strict in [false, true] {
            assert!(matches!(with(Some((3, 9)), 2, strict), Err(IppError::Invalid(_))));
        }
        let lowered = with(Some((1, 99)), 120, false).unwrap();
        assert_eq!(lowered.request.attributes[0].values, [Value::Integer(99)]);
        assert!(lowered.warnings.iter().any(|w| w.contains("lowered to 99")), "{:?}", lowered.warnings);
        assert!(matches!(with(Some((1, 99)), 120, true), Err(IppError::Unsupported(_))));
        let sent = with(None, 7, false).unwrap();
        assert_eq!(sent.request.attributes[0].values, [Value::Integer(7)]);
        assert_eq!(with(Some((3, 9)), 3, true).unwrap().request.attributes[0].values, [Value::Integer(3)]);
    }

    #[test]
    fn unpublished_dimensions_are_not_checked() {
        let bare = PrinterAttrs::default();
        let job = Job { copies: 7, duplex: Some(Duplex::LongEdge), ..Job::default() };
        let options = JobOptions::new().with_media("anything_1x1mm").with_quality(Quality::Draft);
        let p = run(&job, &options, true, &bare).unwrap();
        assert_eq!(names(&p), ["copies", "sides", "print-quality", "media"]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn colour_resolves_against_the_printer() {
        let colour =
            |c, gray: bool, p: &PrinterAttrs, strict| run(&Job { grayscale: gray, ..Job::default() }, &JobOptions::new().with_colour(c), strict, p);
        let mode = |p: &Plan| p.request.attributes.first().map(|a| a.values.clone());
        assert_eq!(mode(&colour(ColourMode::Mono, false, &printer(), false).unwrap()), Some(vec![Value::Keyword("monochrome".into())]));
        assert_eq!(mode(&colour(ColourMode::Auto, true, &printer(), false).unwrap()), Some(vec![Value::Keyword("monochrome".into())]));
        assert_eq!(mode(&colour(ColourMode::Auto, false, &printer(), false).unwrap()), None);
        assert_eq!(mode(&colour(ColourMode::Colour, true, &printer(), false).unwrap()), Some(vec![Value::Keyword("color".into())]));
        let mut mono = printer();
        mono.color_supported = Some(false);
        mono.color_modes = Some(vec!["monochrome".into()]);
        let grey = colour(ColourMode::Colour, false, &mono, false).unwrap();
        assert_eq!(mode(&grey), Some(vec![Value::Keyword("monochrome".into())]));
        assert_eq!(grey.warnings.len(), 1);
        assert!(matches!(colour(ColourMode::Colour, false, &mono, true), Err(IppError::Unsupported(_))));
        let mut no_mono = printer();
        no_mono.color_modes = Some(vec!["color".into()]);
        assert!(colour(ColourMode::Mono, false, &no_mono, false).unwrap().request.attributes.is_empty());
    }

    #[test]
    fn bad_input_and_foreign_formats_are_refused() {
        let p = printer();
        assert!(matches!(run(&Job { copies: 0, ..Job::default() }, &JobOptions::new(), false, &p), Err(IppError::Invalid(_))));
        assert!(matches!(run(&Job { copies: 10_000, ..Job::default() }, &JobOptions::new(), false, &p), Err(IppError::Invalid(_))));
        assert!(matches!(run(&Job::default(), &JobOptions::new().with_resolution(Resolution::new(0, 300)), false, &p), Err(IppError::Invalid(_))));
        assert!(matches!(run(&Job::default(), &JobOptions::new().with_media("a b"), false, &p), Err(IppError::Invalid(_))));
        let e = plan("x".into(), "image/jpeg", &Job::default(), &JobOptions::new(), false, &p).unwrap_err();
        assert!(matches!(e, IppError::UnsupportedFormat { .. }));
        // A printer that lists no formats is not asked.
        assert!(plan("x".into(), "image/jpeg", &Job::default(), &JobOptions::new(), false, &PrinterAttrs::default()).is_ok());
    }

    #[test]
    fn scaling_and_driver_options_are_noted() {
        let job = Job { scaling: Scaling::ShrinkToFit, options: vec![("InputSlot".into(), "Tray2".into())], ..Job::default() };
        let p = run(&job, &JobOptions::new(), false, &printer()).unwrap();
        assert_eq!(p.warnings.len(), 2);
    }
}
