//! The stroke editor and its button (F09.01): one editor body ([`StrokeEditor`]) that a popover
//! ([`StrokeButton`], PhotoCraft's shape options bar) or a docked panel (VectorCraft's Stroke
//! panel, DesignCraft's Properties) hosts, with live line samples ([`StrokePreview`]), mixed
//! values ([`StrokeMixed`]) and app-filled rows ([`StrokeChoiceRow`]) for arrowheads, profiles
//! and named stroke types. The editor edits geometry only; width, colour and paint are the host's
//! controls. The value is [`StrokeStyle`], re-exported here.

mod button;
mod editor;
mod preview;
#[cfg(feature = "screen-testing")]
mod stories;
mod words;

use std::ops::BitOr;

use craft_behavior::Mixed;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Label;
use egui::Rect;

pub use crate::stroke::{DashUnit, LineCap, LineJoin, LineStyle, StrokeAlign, StrokeError, StrokeStyle};
pub use button::StrokeButton;
pub use editor::{StrokeEditor, StrokeEditorOutput};
pub use preview::StrokePreview;
pub use words::StrokeWords;

/// Which parts of the editor show. Combine with `|`.
///
/// The Alignment, Caps and Corners rows and the Miter limit row always show; a section adds the
/// rest. [`POPOVER`](Self::POPOVER) is PhotoCraft's popover, [`PANEL`](Self::PANEL) VectorCraft's
/// docked panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct StrokeSections(u8);

impl StrokeSections {
    /// The Solid, Dashed and Dotted preset rows with their line samples.
    pub const PRESETS: Self = Self(1);
    /// The Opacity row, 0–100 % (lead call 18: PhotoCraft has it, VectorCraft and DesignCraft do
    /// not).
    pub const OPACITY: Self = Self(1 << 1);
    /// The "Dashed line" check, the Dash and Gap pairs, Add pair, Remove pair and Dash offset.
    pub const DASHES: Self = Self(1 << 2);
    /// The tall line sample at the bottom.
    pub const SAMPLE: Self = Self(1 << 3);
    /// Alignment, Caps and Corners as rows of icon toggle buttons (`Segmented` with icons) in
    /// place of selects (VectorCraft's panel).
    pub const ICONS: Self = Self(1 << 4);
    /// PhotoCraft's popover: presets, opacity, dashes and the sample, choices as selects.
    pub const POPOVER: Self = Self(Self::PRESETS.0 | Self::OPACITY.0 | Self::DASHES.0 | Self::SAMPLE.0);
    /// A docked panel: dashes, choices as icon buttons.
    pub const PANEL: Self = Self(Self::DASHES.0 | Self::ICONS.0);

    /// Whether every section of `other` is in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl Default for StrokeSections {
    fn default() -> Self {
        Self::POPOVER
    }
}

impl BitOr for StrokeSections {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The fields whose values differ across the host's selection: such a field shows the mixed
/// placeholder and lights no button; the first edit sets the field for every item. Default:
/// nothing mixed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct StrokeMixed {
    /// The alignment differs.
    pub align: bool,
    /// The cap differs.
    pub cap: bool,
    /// The corner join differs.
    pub join: bool,
    /// The miter limit differs.
    pub miter_limit: bool,
    /// The pattern: no preset row is selected, the "Dashed line" check shows its mixed state and
    /// the pair rows are hidden until the check is set.
    pub dashes: bool,
    /// The dash offset differs.
    pub dash_offset: bool,
    /// The opacity differs.
    pub opacity: bool,
}

impl StrokeMixed {
    /// The first of `styles` and which of its fields differ in the others (VectorCraft's panel
    /// over a multi-selection). `None` for no styles.
    pub fn of<'s>(styles: impl IntoIterator<Item = &'s StrokeStyle>) -> Option<(StrokeStyle, StrokeMixed)> {
        let mut styles = styles.into_iter();
        let first = styles.next()?;
        let mut mixed = StrokeMixed::default();
        for other in styles {
            mixed.align |= other.align != first.align;
            mixed.cap |= other.cap != first.cap;
            mixed.join |= other.join != first.join;
            mixed.miter_limit |= other.miter_limit != first.miter_limit;
            mixed.dashes |= other.dashes != first.dashes;
            mixed.dash_offset |= other.dash_offset != first.dash_offset;
            mixed.opacity |= other.opacity != first.opacity;
        }
        Some((first.clone(), mixed))
    }
}

/// The control of the editor an edit came from, so the host can coalesce edits by field
/// ([`StrokeEditorOutput::field`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StrokeField {
    /// A preset row.
    Preset,
    /// The alignment.
    Align,
    /// The cap.
    Cap,
    /// The corner join.
    Join,
    /// The miter limit.
    MiterLimit,
    /// The opacity.
    Opacity,
    /// The "Dashed line" check.
    Dashed,
    /// The dash or gap at this index of [`StrokeStyle::dashes`].
    Dash(usize),
    /// The add-pair button.
    AddPair,
    /// The remove-pair button.
    RemovePair,
    /// The dash offset.
    DashOffset,
}

/// One choice of a [`StrokeChoiceRow`]: the app's id and the name the select shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct StrokeChoice {
    /// The app's id for the choice.
    pub id: String,
    /// The name the select shows.
    pub label: String,
}

impl StrokeChoice {
    /// A choice with `id` shown as `label`.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into() }
    }
}

type ChoicePaintFn<'a> = Box<dyn Fn(&ContentPainterCx, Rect, &StrokeChoice) + 'a>;

/// The painter of a choice's preview in a [`StrokeChoiceRow`]: it paints the choice into the rect with the content
/// painter.
pub struct ChoicePainter<'a>(ChoicePaintFn<'a>);

impl<'a> ChoicePainter<'a> {
    /// The painter that runs `paint`.
    pub fn new(paint: impl Fn(&ContentPainterCx, Rect, &StrokeChoice) + 'a) -> Self {
        ChoicePainter(Box::new(paint))
    }

    /// Paints `choice` into `rect`.
    pub fn paint(&self, cx: &ContentPainterCx, rect: Rect, choice: &StrokeChoice) {
        (self.0)(cx, rect, choice);
    }
}

/// A row the app fills: a label and a `Select` of [`StrokeChoice`]s with painted previews
/// (arrowheads at the start and end, a width profile, DesignCraft's named stroke types). The
/// editor owns no arrowhead or profile data (lead call 19); picking reports
/// [`StrokeEditorOutput::choice`] with the row's and the choice's ids. The connector bar uses the
/// same rows for its caps.
#[must_use = "pass the row to `StrokeEditor::rows`"]
pub struct StrokeChoiceRow<'a> {
    pub(crate) id: String,
    pub(crate) label: Label,
    pub(crate) choices: &'a [StrokeChoice],
    pub(crate) selected: Option<Mixed<&'a str>>,
    pub(crate) paint: Option<ChoicePainter<'a>>,
}

impl<'a> StrokeChoiceRow<'a> {
    /// A row `id` labelled `label` offering `choices`, nothing picked, previews as text only.
    pub fn new(id: impl Into<String>, label: impl Into<Label>, choices: &'a [StrokeChoice]) -> Self {
        Self { id: id.into(), label: label.into(), choices, selected: None, paint: None }
    }

    /// The picked choice's id, `Mixed::Mixed` across a selection.
    pub fn selected(mut self, selected: Mixed<&'a str>) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Paints each choice's preview beside its name, in the select's list.
    pub fn paint(mut self, paint: ChoicePainter<'a>) -> Self {
        self.paint = Some(paint);
        self
    }

    /// The app's id for the row.
    pub fn id(&self) -> &str {
        &self.id
    }
}
