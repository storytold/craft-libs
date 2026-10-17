//! The resolution a PDF is drawn at for the Windows spooler, and the warnings for the job hints
//! the system spooler does not apply.
//!
//! The in-box PDF renderer draws a page as a bitmap, by default at 96 dpi whatever the images in
//! it are, and the driver gets that bitmap. The spooler backend draws at the printer's own
//! resolution instead (a very large page is drawn in bands, not at a lower resolution: see `winprint`'s
//! `PdfRenderOptions`), and these functions turn a job's hints into the choice:
//!
//! * [`JobOptions::resolution`] is a requirement: draw at that resolution (the higher of the two
//!   axes, one scale for the page), even above the printer's own.
//! * [`JobOptions::quality`] is a wish: draft 150 dpi, normal 300, high 600, photo 1200, never
//!   above what the printer prints. Ignored, with a note, when a resolution is given too.
//! * Neither: the printer's resolution.
//!
//! Everything here is plain arithmetic and text, and runs on every platform.
//!
//! The Windows spooler is the only caller of the resolution functions; CUPS uses `unhonoured`.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::{JobOptions, Quality};

/// The highest resolution an explicit request can ask for; a larger value is lowered to it.
pub(crate) const MAX_REQUESTED_DPI: u32 = 2400;

/// The resolution a quality stands for.
pub(crate) const fn quality_dpi(quality: Quality) -> u32 {
    match quality {
        Quality::Draft => 150,
        Quality::Normal => 300,
        Quality::High => 600,
        Quality::Photo => 1200,
    }
}

/// How a job's hints choose the render resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderChoice {
    /// The resolution asked for; `None` = the printer's, capped.
    pub(crate) dpi: Option<u32>,
    /// The resolution is a quality wish: never above the printer's own.
    pub(crate) limit_to_device: bool,
    /// What was lowered, ignored or merged on the way (text for people).
    pub(crate) notes: Vec<String>,
}

/// Choose the render resolution from a job's hints.
pub(crate) fn choose(options: &JobOptions) -> RenderChoice {
    let mut choice = RenderChoice::default();
    if let Some(resolution) = options.resolution {
        let (x, y) = (resolution.x_dpi, resolution.y_dpi);
        let wanted = x.max(y);
        if wanted == 0 {
            choice.notes.push("the resolution 0 dpi is not a resolution; it was ignored".into());
        } else {
            if x != y {
                choice.notes.push(format!("the page is drawn at one resolution; {wanted} dpi, the higher of {x} x {y} dpi, was used"));
            }
            if wanted > MAX_REQUESTED_DPI {
                choice.notes.push(format!("{wanted} dpi is more than the spooler draws at; {MAX_REQUESTED_DPI} dpi was used"));
            }
            choice.dpi = Some(wanted.min(MAX_REQUESTED_DPI));
        }
        if let Some(quality) = options.quality {
            choice.notes.push(format!("the {} quality was ignored because a resolution was given", quality.name()));
        }
    } else if let Some(quality) = options.quality {
        choice.dpi = Some(quality_dpi(quality));
        choice.limit_to_device = true;
    }
    choice
}

/// What the pages were drawn at, against what was asked: a note when they differ.
///
/// `rendered` holds each page's resolution. With no request (`None`) nothing was asked, so
/// nothing differs.
pub(crate) fn resolution_note(requested: Option<u32>, rendered: &[u32], limit_to_device: bool, device_dpi: Option<u32>) -> Option<String> {
    let requested = requested?;
    let (lowest, highest) = (rendered.iter().copied().min()?, rendered.iter().copied().max()?);
    if lowest == requested && highest == requested {
        return None;
    }
    let used = if lowest == highest { format!("{lowest} dpi") } else { format!("{lowest} to {highest} dpi") };
    let why = match (limit_to_device, device_dpi) {
        (true, Some(device)) if highest <= device => format!("the printer prints at {device} dpi"),
        _ => "the renderer draws at 96 dpi at the least".into(),
    };
    Some(format!("{requested} dpi was asked for; the pages were drawn at {used} ({why})"))
}

/// Job hints the system spooler does not apply, as warnings. `windows` is whether the spooler is
/// the Windows one, which applies [`JobOptions::quality`] and [`JobOptions::resolution`] (to the
/// resolution of the page drawing); CUPS leaves them to the driver's own options.
pub(crate) fn unhonoured(options: &JobOptions, windows: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if options.media.is_some() {
        notes.push("the paper size hint was ignored; the spooler takes the paper from the job".into());
    }
    if options.orientation.is_some() {
        notes.push("the orientation hint was ignored; the spooler takes it from the pages".into());
    }
    if !windows {
        if options.quality.is_some() {
            notes.push("the quality hint was ignored; set the printer's own quality option instead".into());
        }
        if options.resolution.is_some() {
            notes.push("the resolution hint was ignored; set the printer's own resolution option instead".into());
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Orientation, Resolution};

    #[test]
    fn no_hint_leaves_the_printers_resolution() {
        assert_eq!(choose(&JobOptions::new()), RenderChoice::default());
    }

    #[test]
    fn a_quality_is_a_wish_in_tiers() {
        let tiers: Vec<_> = Quality::ALL.iter().map(|q| choose(&JobOptions::new().with_quality(*q))).collect();
        assert_eq!(tiers.iter().map(|c| c.dpi).collect::<Vec<_>>(), [Some(150), Some(300), Some(600), Some(1200)]);
        assert!(tiers.iter().all(|c| c.limit_to_device && c.notes.is_empty()));
    }

    #[test]
    fn a_resolution_is_a_requirement_and_beats_a_quality() {
        let choice = choose(&JobOptions::new().with_resolution(Resolution::square(300)).with_quality(Quality::High));
        assert_eq!((choice.dpi, choice.limit_to_device), (Some(300), false));
        assert_eq!(choice.notes, ["the high quality was ignored because a resolution was given"]);
    }

    #[test]
    fn odd_resolutions_are_merged_lowered_or_ignored_with_a_note() {
        let uneven = choose(&JobOptions::new().with_resolution(Resolution::new(300, 600)));
        assert_eq!(uneven.dpi, Some(600));
        assert_eq!(uneven.notes.len(), 1);
        let huge = choose(&JobOptions::new().with_resolution(Resolution::square(100_000)));
        assert_eq!(huge.dpi, Some(MAX_REQUESTED_DPI));
        assert_eq!(huge.notes.len(), 1);
        let zero = choose(&JobOptions::new().with_resolution(Resolution::square(0)));
        assert_eq!(zero.dpi, None);
        assert_eq!(zero.notes.len(), 1);
    }

    #[test]
    fn a_difference_between_asked_and_drawn_is_worded() {
        assert_eq!(resolution_note(None, &[300], false, None), None);
        assert_eq!(resolution_note(Some(300), &[300, 300], false, Some(600)), None);
        assert_eq!(resolution_note(Some(300), &[], false, None), None);
        let limited = resolution_note(Some(1200), &[600], true, Some(600)).unwrap_or_default();
        assert!(limited.contains("1200 dpi was asked for") && limited.contains("600 dpi") && limited.contains("prints at 600 dpi"), "{limited}");
        let reduced = resolution_note(Some(600), &[600, 250], false, Some(600)).unwrap_or_default();
        assert!(reduced.contains("250 to 600 dpi") && reduced.contains("96 dpi"), "{reduced}");
    }

    #[test]
    fn unapplied_hints_are_never_silent() {
        let options = JobOptions::new()
            .with_quality(Quality::Photo)
            .with_resolution(Resolution::square(600))
            .with_orientation(Orientation::Landscape)
            .with_media("iso_a4_210x297mm");
        assert_eq!(unhonoured(&options, true).len(), 2, "windows applies quality and resolution only");
        assert_eq!(unhonoured(&options, false).len(), 4);
        assert!(unhonoured(&JobOptions::new(), false).is_empty());
    }
}
