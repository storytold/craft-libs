//! The Problems view: the contrast pairs the subject theme falls short on.
//!
//! The sweep ([`craft_theme::report::contrast_report`]) resolves the theme for every mode, contrast and size.
//! Standard contrast reports pairs as warnings (below their minimum); High contrast must meet every minimum, so its
//! pairs are errors.

use super::browser::value_of;
use super::chip::pair;
use super::subject::{Pick, ThemeSubject};
use super::words::summary;
use craft_behavior::{Selection, SelectionChange};
use craft_theme::report::ContrastReport;
use craft_theme::{Appearance, Contrast, ContrastCheck, Explained, Mode, TokenRef, Variants};
use craft_ui::contract::{Bind, CollectionEvent, Pattern, Show as _, ShowCollection as _, ShowContainer as _, WordSetCx as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::list::{Finding, Findings, FindingsList, FindingsSummary, FindingsWords, Severity, ThumbSize};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::TypeRole;
use egui::Ui;

/// The row selected, by its pair so that it survives a new report.
#[derive(Clone, Default)]
struct Session {
    selected: Option<(TokenRef, TokenRef)>,
}

/// What the Problems view reports: the foreground token of the pair the user selected or activated in this pass.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProblemsOutput {
    /// The token to show in the browser.
    pub picked: Option<Pick>,
}

/// The pairs below their minimum in the shown appearance, with a count line per mode of the theme and the list of
/// pairs. Selecting or activating a pair reports its foreground token.
#[derive(Pattern)]
pub struct Problems<'a> {
    report: &'a ContrastReport,
    explained: &'a Explained,
    variants: Variants,
    appearance: Appearance,
}

impl<'a> Problems<'a> {
    /// The problems of `subject` in `report`, with the colours of `explained`.
    pub fn new(report: &'a ContrastReport, explained: &'a Explained, subject: &ThemeSubject<'_>) -> Self {
        Problems { report, explained, variants: subject.theme.variants, appearance: subject.appearance }
    }

    /// The warnings of the shown mode, contrast and size.
    fn warnings(&self) -> &'a [ContrastCheck] {
        warnings_of(self.report, self.appearance.mode, self.appearance)
    }

    /// Shows the view across the room offered.
    pub fn show(self, ui: &mut Ui) -> ProblemsOutput {
        let mut out = ProblemsOutput::default();
        let warnings = self.warnings();
        let severity = if self.appearance.contrast == Contrast::High { Severity::Error } else { Severity::Warning };
        let findings: Vec<Finding> = warnings
            .iter()
            .enumerate()
            .map(|(at, check)| {
                Finding::new(at as u64, severity, format!("{} on {}", check.fg.path(), check.bg.path()))
                    .detail(format!("{:.1} of {:.1}", check.ratio, check.minimum))
            })
            .collect();
        Stack::new()
            .fill()
            .header(|ui, _| {
                for &mode in modes(self.variants) {
                    let count = warnings_of(self.report, mode, self.appearance).len();
                    Text::new(summary(mode, self.appearance.contrast, count)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                }
                FindingsSummary::new(&findings).show(ui);
            })
            .show(ui, |ui, cx| {
                let session = cx.memory::<Session>();
                let at = session.selected.and_then(|chosen| warnings.iter().position(|check| (check.fg, check.bg) == chosen));
                let mut selection = at.map_or_else(Selection::default, |at| Selection::single(at as u64));
                let source = Findings::new(&findings);
                let events = FindingsList::new("Contrast problems")
                    .selection(Bind::bound(&mut selection))
                    .show_items(ui, &source, &mut |key, icx| {
                        let at = *key as usize;
                        let row = findings[at].row(&icx.words::<FindingsWords>());
                        match warnings.get(at) {
                            Some(check) => row.thumb(ThumbSize::Small, pair(value_of(self.explained, check.fg), value_of(self.explained, check.bg))),
                            None => row,
                        }
                    })
                    .events;
                for event in events {
                    let key = match event {
                        CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last().copied(),
                        CollectionEvent::Activate(key) => Some(key),
                        _ => None,
                    };
                    if let Some(check) = key.and_then(|key| warnings.get(key as usize)) {
                        cx.memory::<Session>().selected = Some((check.fg, check.bg));
                        out.picked = Some(Pick::Token(check.fg));
                    }
                }
            });
        out
    }
}

/// The warnings of `report` for `mode` at the contrast and size of `appearance`.
fn warnings_of(report: &ContrastReport, mode: Mode, appearance: Appearance) -> &[ContrastCheck] {
    report.for_appearance(None, mode, appearance.contrast, appearance.size).map_or(&[], |found| found.warnings.as_slice())
}

/// The modes a theme has.
fn modes(variants: Variants) -> &'static [Mode] {
    match variants {
        Variants::Both => &[Mode::Light, Mode::Dark],
        Variants::LightOnly => &[Mode::Light],
        Variants::DarkOnly => &[Mode::Dark],
    }
}
