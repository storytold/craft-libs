//! `Tour` (24.4): a guided walk through the interface, one step at a time, a card beside the part it explains.

use super::words::PanelWords;
use crate::buttons::{Button, IconButton};
use crate::contract::overlay::{Anchor, Dismiss, FloatPlace, Outside, OverlayFit, Place, ViewportShare};
use crate::contract::{Bind, Gap, Overlay, Pattern, Props, RowAlign, Show as _, ShowContainer as _, clicked};
use crate::display::{Shade, TargetMark, Text};
use crate::layout_components::{Row, Stack};
use crate::popover::Popover;
use crate::runtime::commands::{CommandSet as _, Commands as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterBar, FooterButton};
use crate::style::TypeRole;
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::ControlSize;
use craft_theme::variant::Variant;
use egui::{Rect, Ui};
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The keys of a tour: Right goes to the next step and Left back (the keymap mirrors them in a right-to-left
/// interface). They belong to the tour that holds the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "tour", scope = "tour")]
#[non_exhaustive]
pub(crate) enum TourCommands {
    /// Go to the next step, or finish on the last.
    #[command(label = "Next", default = "Right", modal = "always")]
    Next,
    /// Go back a step.
    #[command(label = "Back", default = "Left", modal = "always")]
    Back,
}

/// The card's widest share of the window.
const CARD_SHARE: ViewportShare = ViewportShare::new(0.3, 0.6);

/// One step of a tour: what it says and the part of the window it points at.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TourStep {
    /// The part the step points at, in window coordinates. The card sits beside it; a step without one shows its card
    /// in the middle of the window.
    pub target: Option<Rect>,
    /// The step's heading.
    pub title: Label,
    /// The step's text.
    pub text: Label,
}

impl TourStep {
    /// A step titled `title` saying `text`, pointing at nothing.
    pub fn new(title: impl Into<Label>, text: impl Into<Label>) -> Self {
        TourStep { target: None, title: title.into(), text: text.into() }
    }

    /// The part of the window the step points at.
    #[must_use]
    pub fn target(mut self, target: Rect) -> Self {
        self.target = Some(target);
        self
    }
}

/// What the user did to a tour in this pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TourEvent {
    /// Went on to the next step; the index binding holds it.
    Next,
    /// Went back a step; the index binding holds it.
    Back,
    /// Finished the last step. The app stops showing the tour.
    Finished,
    /// Left the tour before the end (Skip or Escape). The app stops showing the tour.
    Skipped,
}

/// What a tour reports: what the user did in this pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TourOutput {
    /// What the user did, if anything.
    pub event: Option<TourEvent>,
}

/// A guided tour of the interface: a card with the step count and a close button, the step's heading and text, Skip,
/// Back and Next (Done on the last step), beside the part the step points at, which a pulsing ring marks. The rest of
/// the window stays usable and undimmed: only the card takes presses, and a press outside it leaves the tour running.
///
/// - Show it while the tour runs; the app stops showing it on `Finished` and `Skipped`.
/// - The index binding holds the step; Next and Back change it and report the event. An index past the last step
///   shows the last step.
/// - Skip and the close button leave the tour on every step; Back is off on the first.
/// - Keys: Next has the focus when the step opens, so Enter goes on; Right and Left step forward and back (mirrored
///   in a right-to-left interface); Escape skips; Tab reaches the other buttons.
#[derive(Pattern, Props)]
pub struct Tour<'a> {
    label: Label,
    steps: &'a [TourStep],
    index: Bind<'a, usize>,
}

impl<'a> Tour<'a> {
    /// A tour named `label` over `steps`, at the step `index`.
    pub fn new(label: impl Into<Label>, steps: &'a [TourStep], index: Bind<'a, usize>) -> Self {
        Tour { label: label.into(), steps, index }
    }

    /// Shows the current step; a tour with no steps shows nothing.
    pub fn show(self, ui: &mut Ui) -> TourOutput {
        let Tour { label, steps, mut index } = self;
        let Some(last) = steps.len().checked_sub(1) else { return TourOutput::default() };
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let mut event: Option<TourEvent> = None;
        // The tour's root container holds the step's mark; the step's card is the popover it opens.
        Stack::new().show(ui, |ui, root| {
            let held = index.reborrow().read(root).and_then(|held| held.same().copied()).unwrap_or(0);
            let at = held.min(last);
            let step = &steps[at];
            let count = words.tour_count.fill(&[("n", &env.format().integer(at as u64 + 1)), ("total", &env.format().integer(steps.len() as u64))]);
            let layer = layer(step.target.map(|target| target.expand(env.tokens.space.pad.regular)));
            if let Some(target) = step.target {
                TargetMark::new(target).id_salt("mark").show_mark(ui);
            }
            let mut taken: Option<TourEvent> = None;
            let forward = if at < last { TourEvent::Next } else { TourEvent::Finished };
            let shown = Popover::new(step.title.clone(), Bind::given(true))
                .overlay(|_| layer)
                .initial_focus(Initial::Default)
                .command_scope(TourCommands::SCOPE)
                .show(ui, |ui, cx| {
                    while let Some((command, _)) = cx.take_command::<TourCommands>() {
                        taken = match command {
                            TourCommands::Next => Some(forward),
                            TourCommands::Back if at > 0 => Some(TourEvent::Back),
                            TourCommands::Back => taken,
                        };
                    }
                    Stack::new().gap(Gap::Row).show(ui, |ui, _| {
                        Row::new().align(RowAlign::SpaceBetween).fill_width().show(ui, |ui, _| {
                            Text::new(count.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                            let close = IconButton::new(IconId::new("close")).size(ControlSize::Sm).label(words.tour_skip.clone()).id_salt("close");
                            if clicked(&close.show(ui)) {
                                taken = Some(TourEvent::Skipped);
                            }
                        });
                        Text::new(step.title.clone()).role(TypeRole::Title).show(ui);
                        Text::new(step.text.clone()).show(ui);
                        Row::new().align(RowAlign::SpaceBetween).fill_width().show(ui, |ui, _| {
                            if clicked(&Button::new(words.tour_skip.clone()).variant(Variant::GHOST).id_salt("skip").show(ui)) {
                                taken = Some(TourEvent::Skipped);
                            }
                            let next = if at < last { words.tour_next.clone() } else { words.tour_done.clone() };
                            // The given order is the events' order; the footer shows them in the platform's order.
                            let picks = [TourEvent::Back, forward];
                            let buttons = [
                                FooterButton::new(words.tour_back.clone(), DialogRole::Other).enabled(at > 0),
                                FooterButton::new(next, DialogRole::Accept),
                            ];
                            if let Some(event) = FooterBar::buttons(buttons).show_footer(ui).picked.and_then(|at| picks.get(at).copied()) {
                                taken = Some(event);
                            }
                        });
                    });
                    let moved = match taken {
                        Some(TourEvent::Next) => Some(at + 1),
                        Some(TourEvent::Back) => Some(at - 1),
                        _ => None,
                    };
                    if let Some(to) = moved {
                        let name: Arc<str> = Arc::from(label.as_str());
                        let _ = index.reborrow().write(cx, &EditEvent::set(Mixed::Same(held), to, name, None));
                    }
                });
            event = taken.or(shown.output.closed.map(|_| TourEvent::Skipped));
        });
        TourOutput { event }
    }
}

/// The card's layer: a popup beside `anchor` (the step's target with a panel's padding around it, clear of its mark),
/// else in the middle of the window. It blocks nothing and dims nothing; a press outside leaves it open.
fn layer(anchor: Option<Rect>) -> Overlay {
    let mut overlay = Overlay::popover().dismiss(Dismiss::new().outside(Outside::Keep));
    overlay.fit = OverlayFit::ContentCapped(CARD_SHARE);
    overlay.place = match anchor {
        Some(anchor) => Place::anchored(Anchor::Rect(anchor), Side::End, Align::Center),
        None => Place::Float(FloatPlace::centred()),
    };
    overlay
}
