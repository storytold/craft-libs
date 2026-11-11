//! `ColorPopover`: a colour well that opens a Compact colour picker in a popover.

use craft_behavior::EditEvents;
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::contract::{Bind, Outcome, Pattern, Remember as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::Label;
use egui::{Response, Ui};

use super::{ColorPicker, ColorPickerLayout, ColorPickerOutput, PickedColor, ScreenPickGesture};

#[cfg(feature = "screen-testing")]
mod stories;

/// The setup closure of the picker in the popover.
type Setup<'a> = Box<dyn for<'b> FnOnce(ColorPicker<'b>) -> ColorPicker<'b> + 'a>;

/// The popover's own state while its well is shown.
#[derive(Clone, Default)]
struct Session {
    open: bool,
}

/// A [`ColorPicker`] (Compact) in a popover under a colour well, as PhotoCraft's colour button.
///
/// - The well is a `Swatch` named `label`; a click, Enter or Space opens the popover and the hex field takes the
///   focus with its text selected.
/// - The popover hangs below the well, or above it when there is no room, and keeps that side while it is open. It is
///   as wide as the picker.
/// - It closes on a press outside, a click on the well and Escape while no gesture runs (an Escape that cancels a
///   gesture is the picker's). The colour stays as edited, and the focus returns to the well unless it moved to
///   another control.
/// - Edits are the picker's: one step per gesture, so the popover edits live.
/// - A caller that draws its own opener (a gradient stop) hands it over with [`anchor`](ColorPopover::anchor); the
///   caller closes the popover by writing `false` to the binding it gave.
#[must_use = "show the popover with `show`"]
#[derive(Pattern)]
pub struct ColorPopover<'a> {
    label: Label,
    value: Bind<'a, PickedColor>,
    setup: Option<Setup<'a>>,
    anchor: Option<(Response, Bind<'a, bool>)>,
}

/// What [`ColorPopover::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit"]
pub struct ColorPopoverOutput {
    /// The well's response, or the anchor's.
    pub response: Response,
    /// The edits of the picker in the popover, in order; empty while the popover is closed.
    pub events: EditEvents<PickedColor>,
    /// The picker's screen-pick button was pressed; the app drives its `ScreenPick`.
    pub screen_pick: Option<ScreenPickGesture>,
    /// Whether the popover is open after this pass.
    pub open: bool,
}

impl Outcome for ColorPopoverOutput {
    type Value = PickedColor;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.response.story_facts()
    }
}

impl<'a> ColorPopover<'a> {
    /// A well named `label` (its accessible name, translated: "Fill colour") over `value`, opening a picker.
    pub fn new(label: impl Into<Label>, value: Bind<'a, PickedColor>) -> Self {
        ColorPopover { label: label.into(), value, setup: None, anchor: None }
    }

    /// Sets up the picker in the popover (alpha, eyedropper, constraint, models): `setup` gets the Compact picker the
    /// popover made.
    pub fn picker(mut self, setup: impl for<'b> FnOnce(ColorPicker<'b>) -> ColorPicker<'b> + 'a) -> Self {
        self.setup = Some(Box::new(setup));
        self
    }

    /// Hangs the popover from `opener`, which the caller drew, and opens it while `open` is true. The caller opens it
    /// on the opener's click and closes it by writing `false`.
    pub fn anchor(mut self, opener: &Response, open: Bind<'a, bool>) -> Self {
        self.anchor = Some((opener.clone(), open));
        self
    }

    /// Shows the well and, while open, the popover.
    pub fn show(self, ui: &mut Ui) -> ColorPopoverOutput {
        let ColorPopover { label, mut value, mut setup, anchor } = self;
        if let Some((opener, mut open)) = anchor {
            let (events, screen_pick, shown) = popover(ui, &label, &mut value, &mut setup, &opener, open.reborrowed());
            return ColorPopoverOutput { response: opener, events, screen_pick, open: shown };
        }
        let shown = Stack::new().show(ui, |ui, cx| {
            let color = value.reborrow().read(cx).and_then(|held| held.same().map(|picked| picked.color)).unwrap_or(Srgba::rgb(0.0, 0.0, 0.0));
            let well = Swatch::new(label.clone(), color).show(ui);
            let state = cx.memory::<Session>();
            if well.clicked {
                state.open = !state.open;
            }
            let (events, screen_pick, open) = popover(ui, &label, &mut value, &mut setup, &well.response, Bind::bound(&mut state.open));
            ColorPopoverOutput { response: well.response, events, screen_pick, open }
        });
        match shown.inner {
            Some(output) => output,
            None => ColorPopoverOutput { response: shown.output, events: EditEvents::new(), screen_pick: None, open: false },
        }
    }
}

/// The popover under `opener` while `open` is true: its picker's edit and screen pick, and whether it stays open.
fn popover(
    ui: &mut Ui,
    label: &Label,
    value: &mut Bind<'_, PickedColor>,
    setup: &mut Option<Setup<'_>>,
    opener: &Response,
    open: Bind<'_, bool>,
) -> (EditEvents<PickedColor>, Option<ScreenPickGesture>, bool) {
    let shown = Popover::new(label.clone(), open).opened_by(opener).show(ui, |ui, _| {
        let mut picker = compact_picker(label.clone(), value.reborrowed(), None).focus_on_open(true);
        if let Some(setup) = setup.take() {
            picker = setup(picker);
        }
        picker.show(ui)
    });
    let open = shown.inner.is_some() && shown.output.closed.is_none();
    match shown.inner {
        Some(ColorPickerOutput { events, screen_pick, .. }) => (events, screen_pick, open),
        None => (EditEvents::new(), None, open),
    }
}

/// The Compact picker every colour popover of the crate shows: [`ColorPopover`] and the overlays of the swatch pair
/// and the paint chip build their picker here. `sampled` answers a pending screen pick.
pub(super) fn compact_picker<'a>(label: Label, value: Bind<'a, PickedColor>, sampled: Option<Srgba>) -> ColorPicker<'a> {
    let picker = ColorPicker::new(label, value).layout(ColorPickerLayout::Compact);
    match sampled {
        Some(color) => picker.sampled(color),
        None => picker,
    }
}
