//! `BlendIfBar`: one Blend If slider (This Layer or Underlying Layer) with its four handles, over `ThumbRow`.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvents, Mixed};
use craft_ui::contract::{Bind, EditResponse, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{DataColor, TypeRole};
use craft_ui::value_drag::{Order, Thumb, ThumbRow, TrackFace};
use egui::{Color32, Ui};

use crate::layer_style::BlendIfRange;
use crate::ui::curve::ToneRamp;

craft_ui::words! {
    /// The words of a Blend If bar: its handles' names and its readout.
    #[non_exhaustive]
    pub struct BlendIfWords {
        black_low: "blendIf.blackLow" = "Black low";
        black_high: "blendIf.blackHigh" = "Black high";
        white_low: "blendIf.whiteLow" = "White low";
        white_high: "blendIf.whiteHigh" = "White high";
        /// A handle's whole name: `{handle}` is the handle's word and `{layer}` the bar's name.
        handle: "blendIf.handle" = "{handle}, {layer}";
        /// A split side of the readout: `{low}` and `{high}` are the halves' levels.
        split_levels: "blendIf.splitLevels" = "{low} / {high}";
        /// The readout above the bar: `{black}` and `{white}` are the sides' texts.
        readout: "blendIf.readout" = "{black}    {white}";
    }
}

/// The levels of the bar: image tones, not theme colours.
const LEVELS: std::ops::RangeInclusive<f64> = 0.0..=255.0;
/// The face of the black handles: grey 20.
const BLACK_FACE: u8 = 20;
/// The face of the white handles: grey 240.
const WHITE_FACE: u8 = 240;

fn gray(level: u8) -> DataColor {
    DataColor::new(Color32::from_gray(level))
}

impl BlendIfRange {
    /// The range as the thumb row holds it.
    fn row(self) -> [Mixed<f64>; 4] {
        Mixed::each(self.0.map(f64::from))
    }

    /// The range of a thumb row; a mixed part reads as the full range's.
    fn of_row(row: &[Mixed<f64>; 4]) -> BlendIfRange {
        let full = BlendIfRange::FULL.0;
        let level = |at: usize| row[at].same().map_or(full[at], |value| value.round().clamp(0.0, 255.0) as u8);
        BlendIfRange([level(0), level(1), level(2), level(3)])
    }
}

/// A Blend If slider: a black-to-white ramp with a black and a white handle under it, each split into two halves by
/// Alt (the layer fades between them). A split pair shows two half triangles; a pair whose halves coincide, one whole
/// triangle. The readout ("0 / 20    255": a split side as "low / high") sits above the bar at the trailing end. The
/// bar mirrors with the row, like every thumb row.
///
/// - Pointer: a press picks the nearest handle; a drag moves its pair, keeping the spread, or with Alt the half alone.
///   A drag stops at the other pair. One gesture per drag; Esc cancels.
/// - Keys: the four handles are Tab stops in order. Arrows, Page keys, Home, End and the wheel move one half.
/// - AccessKit: each handle is a slider named by [`BlendIfWords::handle`] ("Black low, This Layer").
#[must_use = "show the bar with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct BlendIfBar<'a> {
    label: Label,
    range: Bind<'a, BlendIfRange>,
}

impl<'a> BlendIfBar<'a> {
    /// A bar named `label` ("This Layer", "Underlying Layer") over `range`.
    pub fn new(label: impl Into<Label>, range: Bind<'a, BlendIfRange>) -> Self {
        Self { label: label.into(), range }
    }

    /// Shows the readout and the bar and returns this pass's edit.
    pub fn show(self, ui: &mut Ui) -> EditResponse<BlendIfRange> {
        let Self { label, mut range } = self;
        let env = Env::of(ui);
        let words = env.words::<BlendIfWords>();
        let units = env.units();
        let mut out: Option<EditResponse<BlendIfRange>> = None;
        let shown_stack = Stack::new().fill_width().label(label.clone()).show(ui, |ui, root| {
            let held = range.reborrow().read(root);
            let mut row = match &held {
                Some(Mixed::Same(range)) => range.row(),
                _ => [Mixed::Mixed; 4],
            };
            if let Some(Mixed::Same(range)) = &held {
                let side = |low: u8, high: u8| {
                    let text = |level: u8| units.number(f64::from(level), 0..=0);
                    match low == high {
                        true => text(low),
                        false => craft_i18n::fmt(words.split_levels.as_str(), &[("low", &text(low)), ("high", &text(high))]),
                    }
                };
                let readout =
                    craft_i18n::fmt(words.readout.as_str(), &[("black", &side(range.0[0], range.0[1])), ("white", &side(range.0[2], range.0[3]))]);
                Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
                    Text::new(readout.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                });
            }
            let named = |handle: &str| Label::from(craft_i18n::fmt(words.handle.as_str(), &[("handle", handle), ("layer", label.as_str())]));
            let ordered = |below, above| Order::between(below, above, 0.0);
            let thumbs = [
                Thumb::new(named(words.black_low.as_str())).face(gray(BLACK_FACE)).split().order(ordered(None, Some(2))),
                Thumb::new(named(words.black_high.as_str())).face(gray(BLACK_FACE)).order(ordered(Some(0), Some(2))),
                Thumb::new(named(words.white_low.as_str())).face(gray(WHITE_FACE)).split().order(ordered(Some(1), None)),
                Thumb::new(named(words.white_high.as_str())).face(gray(WHITE_FACE)).order(ordered(Some(1), None)),
            ];
            let ramp = [gray(0), gray(255)];
            let shown = ThumbRow::new(Bind::bound(&mut row), LEVELS)
                .step(1.0)
                .thumbs(thumbs)
                .track_face(TrackFace::new(ToneRamp::new(&ramp)))
                .label(label.clone())
                .show(ui)
                .edit
                .map(|row| BlendIfRange::of_row(&row));
            for event in &shown.events {
                range.reborrow().write(root, event);
            }
            out = Some(shown);
        });
        out.unwrap_or_else(|| EditResponse::lifted(shown_stack.output, EditEvents::new(), None))
    }
}
