//! The resolution a PDF is drawn at for the Windows spooler, and the warnings for the job hints
//! the system spooler does not apply.
//!
//! The in-box PDF renderer draws a page as a bitmap, by default at 96 dpi whatever the images in
//! it are, and the driver gets that bitmap. `winprint` draws at the job's own resolution instead:
//! the one in the PrintTicket's DEVMODE (the printer's default, or the page resolution the ticket
//! asks for). These functions turn a job's hints into the page resolution the ticket asks for:
//!
//! * [`JobOptions::resolution`] is a requirement: the printer's resolution at or above it (the
//!   higher of the two axes), else its highest.
//! * [`JobOptions::quality`] is a wish: draft 150 dpi, normal 300, high 600, photo 1200, never
//!   above what the printer offers when it offers a lower one. Ignored, with a note, when a
//!   resolution is given too.
//! * Neither: the printer's default resolution.
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

/// Which of the printer's page resolutions the ticket asks for, and what to say when it is not
/// the one asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolutionPick {
    /// Index into the offered resolutions; `None` when the printer offers none.
    pub(crate) index: Option<usize>,
    /// Text for people when the resolution used differs from the one asked for.
    pub(crate) note: Option<String>,
}

/// Choose among the `offered` (x, y) resolutions of the printer, by the higher axis. `None` when
/// the job asks for no resolution (the printer's default applies).
///
/// A requirement takes the lowest offered resolution at or above the request, else the highest. A
/// wish takes the highest at or below it, else the lowest.
pub(crate) fn pick_resolution(offered: &[(u32, u32)], choice: &RenderChoice) -> Option<ResolutionPick> {
    let wanted = choice.dpi?;
    let dpi = |i: &usize| offered[*i].0.max(offered[*i].1);
    let all = 0..offered.len();
    let index = if choice.limit_to_device {
        all.clone().filter(|i| dpi(i) <= wanted).max_by_key(dpi).or_else(|| all.min_by_key(dpi))
    } else {
        all.clone().filter(|i| dpi(i) >= wanted).min_by_key(dpi).or_else(|| all.max_by_key(dpi))
    };
    let note = match index {
        None => Some(format!("{wanted} dpi was asked for; the printer lists no resolutions, so its own was used")),
        Some(i) if dpi(&i) != wanted => {
            let mut list: Vec<u32> = (0..offered.len()).map(|j| dpi(&j)).collect();
            list.sort_unstable();
            list.dedup();
            let list = list.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
            Some(format!("{wanted} dpi was asked for; the printer offers {list} dpi, so {} dpi was used", dpi(&i)))
        }
        Some(_) => None,
    };
    Some(ResolutionPick { index, note })
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
    fn the_ticket_asks_for_the_offered_resolution_that_fits() {
        let offered = [(300, 300), (600, 600), (1200, 1200)];
        let pick = |dpi: Option<u32>, limit: bool| pick_resolution(&offered, &RenderChoice { dpi, limit_to_device: limit, notes: Vec::new() });
        assert_eq!(pick(None, false), None, "no request leaves the printer's default");
        let exact = pick(Some(600), false).unwrap();
        assert_eq!((exact.index, exact.note), (Some(1), None));
        // a requirement rounds up, and above the printer's best it takes the best
        assert_eq!(pick(Some(400), false).unwrap().index, Some(1));
        let too_high = pick(Some(2400), false).unwrap();
        assert_eq!(too_high.index, Some(2));
        assert!(too_high.note.unwrap_or_default().contains("2400 dpi was asked for"));
        // a wish rounds down, and below the printer's lowest it takes the lowest
        assert_eq!(pick(Some(1000), true).unwrap().index, Some(1));
        assert_eq!(pick(Some(150), true).unwrap().index, Some(0));
        let none = pick_resolution(&[], &RenderChoice { dpi: Some(300), ..RenderChoice::default() }).unwrap();
        assert_eq!(none.index, None);
        assert!(none.note.unwrap_or_default().contains("lists no resolutions"));
    }

    #[test]
    fn an_uneven_resolution_is_compared_by_its_higher_axis() {
        let pick = pick_resolution(&[(600, 300), (300, 300)], &RenderChoice { dpi: Some(600), ..RenderChoice::default() }).unwrap();
        assert_eq!((pick.index, pick.note), (Some(0), None));
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
