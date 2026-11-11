//! The black, grey and white point pickers of Levels and Curves: the button row
//! ([`TonePickerRow`]) and the canvas tool that samples the image for the armed picker
//! ([`TonePickTool`]).
//!
//! The library reports the sample; turning it into a Levels or Curves value needs the document's
//! colour management and mode, so the app does that, as with the Eyedropper.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{CancelReason, Cursor, IconId, PaintedCursor};
use craft_theme::Srgba;
use craft_ui::buttons::ToggleButton;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Held, NoValue, Outcome, Pattern, Props, Sampler as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Id, Response, Ui, Vec2};

use super::AdjustWords;

/// The side of the pipette icon, in cursor units.
const PIPETTE_BOX: f32 = 24.0;
/// How far the pipette's tip lies in from the icon's bottom left corner.
const PIPETTE_TIP_INSET: f32 = 2.0;
/// The cursor's hot spot: the pipette's tip, at the bottom left of its icon.
fn pipette_hotspot() -> Vec2 {
    Vec2::X * PIPETTE_TIP_INSET + Vec2::Y * (PIPETTE_BOX - PIPETTE_TIP_INSET)
}

/// One of the three point pickers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TonePicker {
    /// Set Black Point.
    Black,
    /// Set Neutral Gray Point.
    Gray,
    /// Set White Point.
    White,
}

impl TonePicker {
    /// The three, in the row's order.
    pub const ALL: [TonePicker; 3] = [TonePicker::Black, TonePicker::Gray, TonePicker::White];

    /// The grey of the picker's tone: 0, 128, 255. An image tone, the same in every theme.
    // pc: crates/ui-egui/src/dialogs/adjust_dialog.rs:196-198.
    pub fn tone(self) -> u8 {
        match self {
            TonePicker::Black => 0,
            TonePicker::Gray => 128,
            TonePicker::White => 255,
        }
    }

    /// The picker's name ("Set Black Point"), its tooltip and accessible name.
    pub fn label(self, words: &AdjustWords) -> Label {
        match self {
            TonePicker::Black => words.set_black_point.clone(),
            TonePicker::Gray => words.set_gray_point.clone(),
            TonePicker::White => words.set_white_point.clone(),
        }
    }

    /// Whether the picker works in a document of this kind (Photoshop, PhotoCraft
    /// `picker_supported`): all three in RGB, Black and White in Grayscale, none in other modes.
    // pc: crates/ui-egui/src/dialogs/adjust_dialog.rs:31-34 `picker_supported`.
    pub fn supported(self, rgb: bool, gray: bool) -> bool {
        rgb || (gray && self != TonePicker::Gray)
    }

    /// The picker's pipette: Photoshop's black, grey or white one.
    fn icon(self) -> IconId {
        IconId::new(match self {
            TonePicker::Black => "eyedropper_black",
            TonePicker::Gray => "eyedropper_gray",
            TonePicker::White => "eyedropper_white",
        })
    }
}

/// What [`TonePickerRow::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct TonePickerRowOutput {
    /// The row.
    pub response: Response,
    /// The armed picker changed this pass: `Some(Some(p))` arms `p`, `Some(None)` disarms. The
    /// binding has the new value already when it writes through.
    pub change: Option<Option<TonePicker>>,
}

impl Outcome for TonePickerRowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The "Eyedroppers:" row: three icon-only toggle buttons (Set Black Point, Set Neutral Gray Point,
/// Set White Point), each a pipette in its own tone, and the "Unavailable in this color mode" note.
///
/// - A button is pressed while its picker is armed. A click on a disarmed button arms it; a click on
///   the armed one disarms it.
/// - While a picker is armed and [`canvas`](Self::canvas) names the document's canvas, the row
///   arms the canvas's sampler in every pass: a primary press on the canvas under the modal dialog
///   reaches the canvas's tool (a [`TonePickTool`]), and the wheel and middle-drag navigate it.
///   Keys, focus and Escape stay the dialog's.
/// - A picker the host disabled ([`enabled_picker`](Self::enabled_picker)) shows disabled.
///   [`unavailable`](Self::unavailable) disables all three and shows the note after them.
/// - Keyboard: each button is a Tab stop; Space or Enter acts as a click.
#[must_use = "show the row with `show`"]
#[derive(Pattern, Props)]
pub struct TonePickerRow<'a> {
    armed: Bind<'a, Option<TonePicker>>,
    /// The document canvas the armed picker samples (the canvas component's id).
    #[prop]
    canvas: Option<Id>,
    enabled: [bool; 3],
    /// The document's mode has no point pickers (Lab, CMYK, Bitmap): every button is disabled and the
    /// note shows.
    #[prop(flag)]
    unavailable: bool,
}

impl<'a> TonePickerRow<'a> {
    /// The row over the armed picker `armed`.
    pub fn new(armed: Bind<'a, Option<TonePicker>>) -> Self {
        Self { armed, canvas: None, enabled: [true; 3], unavailable: false }
    }

    /// Enables or disables one picker (the host passes [`TonePicker::supported`] for its mode).
    pub fn enabled_picker(mut self, picker: TonePicker, enabled: bool) -> Self {
        self.enabled[picker as usize] = enabled;
        self
    }

    /// Draws the row and reports an arm or disarm.
    pub fn show(self, ui: &mut Ui) -> TonePickerRowOutput {
        // pc: crates/ui-egui/src/dialogs/adjust_dialog.rs:177-183 `picker_button`, :186-216
        // `picker_controls` ("Eyedroppers:" caption, the note in the caption role, faint).
        let Self { armed, canvas, enabled, unavailable } = self;
        let words = Env::of(ui).words::<AdjustWords>();
        let mut held = Held::new(armed);
        let mut change = None;
        let shown = Row::new().label(words.eyedroppers.clone()).show(ui, |ui, cx| {
            let current = held.read(cx).flatten();
            if let (Some(canvas), Some(_)) = (canvas, current) {
                cx.arm_sampler(canvas);
            }
            Text::new(words.eyedroppers.clone()).shade(Shade::Muted).role(TypeRole::Label).show(ui);
            for picker in TonePicker::ALL {
                let on = current == Some(picker);
                let usable = enabled[picker as usize] && !unavailable;
                let toggle = ToggleButton::new(picker.label(&words), Bind::given(on))
                    .icon(picker.icon())
                    .icon_only()
                    .enabled(usable)
                    .id_salt(picker as u8)
                    .escape_disarms()
                    .show(ui);
                if let Some(next) = toggle.value().copied() {
                    change = Some(next.then_some(picker));
                }
            }
            if unavailable {
                Text::new(words.unavailable_mode.clone()).shade(Shade::Faint).role(TypeRole::Caption).show(ui);
            }
            if let Some(next) = change {
                held.write(cx, &current, next, "Point picker");
            }
        });
        TonePickerRowOutput { response: shown.output, change }
    }
}

/// A sample the armed picker took on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TonePick {
    /// The picker that took the sample.
    pub picker: TonePicker,
    /// The document point sampled, in document units.
    pub at: [f64; 2],
    /// The composite colour there, as the host's `sample` function read it (the committed merged
    /// document, PhotoCraft `sample_at`).
    pub color: Srgba,
}

/// The canvas tool of an armed point picker: a press samples the host's `sample` function at the
/// document point and reports a [`TonePick`] for the picker (taken with [`take`](Self::take)
/// after the tool host's pass). Off the image `sample` returns `None` and nothing is reported. A
/// drag does nothing more; the release ends the gesture. The cursor is the pipette with its
/// hotspot at its tip.
///
/// The host applies the pick with its engine (Photoshop maps the sample to black, neutral grey
/// or white in each channel) and passes the new value back to the panel, which takes it as an
/// outside change; the picker stays armed until the row disarms it.
pub struct TonePickTool<'a> {
    picker: TonePicker,
    sample: Box<dyn FnMut([f64; 2]) -> Option<Srgba> + 'a>,
    picked: Option<TonePick>,
}

impl<'a> TonePickTool<'a> {
    /// The tool for `picker`, sampling the composite with `sample` (document units in, `None` off
    /// the image).
    pub fn new(picker: TonePicker, sample: impl FnMut([f64; 2]) -> Option<Srgba> + 'a) -> Self {
        Self { picker, sample: Box::new(sample), picked: None }
    }

    /// The picker the tool samples for.
    pub fn picker(&self) -> TonePicker {
        self.picker
    }

    /// The pick of the last press, once.
    pub fn take(&mut self) -> Option<TonePick> {
        self.picked.take()
    }
}

impl CanvasTool for TonePickTool<'_> {
    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::Painted(PaintedCursor::tool(IconId::new("eyedropper"), pipette_hotspot(), None))
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // pc: crates/ui-egui/src/dialogs/adjust_dialog.rs:52-80 `sample_at` (the armed picker samples
        // the composite at the click).
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let at = space.view.to_doc(pointer.pos);
        self.picked = (self.sample)([at.x, at.y]).map(|color| TonePick { picker: self.picker, at: [at.x, at.y], color });
        ToolAction::Claimed
    }

    fn drag(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    fn release(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        // A pick not yet taken is dropped.
        self.picked = None;
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        // Photoshop has no tool for the dialog's point pickers: the sample is the dialog's own, and the
        // value it sets is a field of the same dialog.
        ToolPath::Command("the Levels or Curves dialog's point picker")
    }
}
