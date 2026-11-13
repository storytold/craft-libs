//! Shape, space, size and stroke tokens in logical pixels.

use crate::theme::EdgeSeed;

use crate::color::Rgba;

/// Corner radii in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RadiusSet {
    /// Small parts: half of `control`, rounded; resolved after a theme's own shape applies, so it
    /// is not in a size set file.
    pub sm: f32,
    /// Buttons other than pills, fields, small controls.
    pub control: f32,
    /// Popups and menus.
    pub surface: f32,
    /// Windows and dialogs.
    pub window: f32,
    /// Docked panels.
    pub panel: f32,
    /// Push buttons; [`Radius::Full`](super::scale::Radius::Full) gives pills.
    pub button: f32,
    /// Pill-shaped parts (switch, slider, progress, badge); each clamps it to half its height.
    /// [`Radius::Full`](super::scale::Radius::Full) gives pills, 0 squares them.
    pub pill: f32,
}

/// How surfaces draw their edge. Only the `Surface` primitive reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Edge {
    /// No edge effect.
    #[default]
    Flat,
    /// A two-tone bevel: `light` on the lit sides, `dark` on the shaded ones.
    Bevel {
        /// The lit edge colour.
        light: Rgba,
        /// The shaded edge colour.
        dark: Rgba,
        /// Line width in logical pixels.
        width: f32,
    },
}

impl Edge {
    /// The seed this edge resolves from; a bevel drops its colours.
    pub fn seed(self) -> EdgeSeed {
        match self {
            Edge::Flat => EdgeSeed::Flat,
            Edge::Bevel { width, .. } => EdgeSeed::Bevel { width },
        }
    }
}

/// How list and tab rows are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowStyle {
    /// Square rows separated by dividers.
    #[default]
    Divided,
    /// Rounded rows without dividers.
    Rounded,
}

/// The shape of the knob a value track's thumb is drawn as (a slider's, a range slider's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnobShape {
    /// A round knob as wide as it is tall.
    #[default]
    Round,
    /// An upright tab, taller than it is wide.
    Tab,
}

/// Corner radii, edge style, row style and knob shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeTokens {
    /// Corner radii.
    pub radius: RadiusSet,
    /// Edge style.
    pub edge: Edge,
    /// How list and tab rows are drawn.
    pub rows: RowStyle,
    /// How the thumbs of value tracks are drawn.
    pub knob: KnobShape,
}

/// Gaps between neighbouring elements, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gaps {
    /// Between tightly packed items.
    pub tight: f32,
    /// Between items on one line.
    pub inline: f32,
    /// Between the rows of a panel body (PhotoCraft's `ROW_GAP`) and around a section header.
    pub row: f32,
    /// Between stacked rows.
    pub stack: f32,
    /// Between neighbouring controls in a bar, and between a bar caption and its control.
    pub item: f32,
    /// Between groups.
    pub section: f32,
    /// Between the columns of a grid of cards, outline included.
    pub card_column: f32,
    /// Between the rows of a grid of cards.
    pub card_row: f32,
}

/// Inner padding, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pads {
    /// Inside controls.
    pub control: f32,
    /// Inside panels and default dialog frames.
    pub regular: f32,
    /// Inside roomy dialogs.
    pub roomy: f32,
    /// Inside badges, key caps and chips: tighter than a control.
    pub chip: f32,
    /// Beside the label of a medium segment of a segmented control.
    pub segment: f32,
}

/// Spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpaceTokens {
    /// Gaps.
    pub gap: Gaps,
    /// Padding.
    pub pad: Pads,
    /// Insets from the edges of the screen.
    pub inset: Insets,
}

/// The gaps floating layers keep from the edges of the screen, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    /// The gap a floating layer (a dialog, a floating window) keeps from the screen edge.
    pub screen: f32,
}

/// One length per control size, in logical pixels: control heights, field widths.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlSizes {
    /// Small controls.
    pub sm: f32,
    /// Standard controls.
    pub md: f32,
}

/// Icon sizes, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IconSizes {
    /// Inline and small-control icons.
    pub sm: f32,
    /// Standard icons.
    pub md: f32,
    /// Rail and toolbar icons.
    pub lg: f32,
    /// The glyph of a small icon-only button: smaller than `sm`, so it reads below the cap height of the text beside it.
    pub glyph_sm: f32,
    /// The glyph of a standard icon-only button.
    pub glyph_md: f32,
}

/// Tool-rail sizes, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RailSizes {
    /// Rail width.
    pub width: f32,
    /// Square button inside the rail.
    pub button: f32,
}

/// Bar heights, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarSizes {
    /// Window title bar.
    pub title: f32,
    /// Tab strip.
    pub tab: f32,
    /// Status bar.
    pub status: f32,
    /// Tool options bar.
    pub options: f32,
    /// Tool options bar in its narrow form, for a window too small for the full bar.
    pub options_narrow: f32,
}

/// Fixed sizes of controls and chrome.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeTokens {
    /// Control heights.
    pub control: ControlSizes,
    /// Icon sizes.
    pub icon: IconSizes,
    /// Tool rail.
    pub rail: RailSizes,
    /// Bars.
    pub bar: BarSizes,
    /// Panel header height.
    pub panel_header: f32,
    /// Least width of a popup surface: a menu or a popup list.
    pub popup_min: f32,
    /// The length of the fade at an edge of a scroll region past which its content continues.
    pub scroll_fade: f32,
    /// Thumbnail sides.
    pub thumb: ThumbSizes,
    /// Canvas drag handles.
    pub handle: HandleSizes,
    /// Pointer tolerance.
    pub hit: HitSizes,
    /// Side of a checkerboard cell in UI swatches and thumbnails.
    pub checker: f32,
    /// The least size of a plot or two-dimensional value area: the well of a point picker, before the
    /// picker's own fitting.
    pub plot: PlotSizes,
    /// Lengths of controls that stretch along one axis.
    pub length: LengthSizes,
    /// Row heights other than a control's.
    pub row: RowSizes,
    /// The thickness of the groove of a value track (a slider, a range slider, a progress bar).
    pub groove: f32,
    /// Gradient ramps: the colours a value runs through, drawn as a bar or as the groove of a track.
    pub ramp: RampSizes,
    /// Where a tip that follows the pointer (a gesture readout) sits from the pointer.
    pub pointer_tip: PointerTipOffset,
    /// The windows a dock floats its panels in.
    pub float: FloatSizes,
    /// The app's windows.
    pub window: WindowSizes,
    /// The panes a dock splits its area into.
    pub pane: PaneSizes,
    /// The docks' columns.
    pub dock: DockColumnSizes,
    /// Notice cards: toasts.
    pub notice: NoticeSizes,
    /// Dialogs: their content width and their buttons.
    pub dialog: DialogSizes,
    /// The rulers of a canvas.
    pub ruler: RulerSizes,
    /// The stage a story draws on.
    pub story: StorySizes,
}

/// The stage a story draws on, in logical pixels: a story of a canvas control, and each column of a story grid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StorySizes {
    /// The height of the stage a story of a canvas control shows its canvas on.
    pub height: f32,
    /// The least width of a story: of that stage, and of a column of a story grid.
    pub width_min: f32,
    /// The greatest width of that stage.
    pub width_max: f32,
}

/// The sizes of a canvas ruler, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerSizes {
    /// The tick marks.
    pub tick: RulerTickSizes,
}

/// The lengths of a ruler's tick marks, in logical pixels. The minor tick is `space.gap.tight` and the major tick
/// spans the ruler's depth.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerTickSizes {
    /// The tick halfway between two labelled ticks.
    pub mid: f32,
}

/// The sizes of the panes of a dock's split, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneSizes {
    /// The least extent of a pane along a split, and of a dock, before its panels' own limits.
    pub least: f32,
}

/// The sizes of a dock's columns, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DockColumnSizes {
    /// The width an expanded column has until the user resizes it.
    pub column: f32,
}

/// The sizes of a notice card, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoticeSizes {
    /// The width every card of a toast stack has where the room allows.
    pub width: f32,
}

/// The sizes of a dialog, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DialogSizes {
    /// The least width of a dialog's content: a dialog of a few short fields opens this wide.
    pub least: f32,
    /// The line length a dialog's paragraphs wrap at; content wider than it widens the dialog.
    pub wrap: f32,
    /// The least width of a footer button.
    pub button: f32,
    /// The least width of a button in a column beside the body.
    pub side_button: f32,
}

/// The sizes of a floating dock window, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FloatSizes {
    /// The width a floating dock window opens at.
    pub width: f32,
    /// The height a floating dock window opens at.
    pub height: f32,
}

/// The sizes of the app's windows, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowSizes {
    /// The offset between cascaded windows (document windows and floating dock windows), along both axes.
    pub cascade: f32,
}

/// The offset of a pointer-following tip from the pointer, toward the reading end and down, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerTipOffset {
    /// Toward the reading end.
    pub x: f32,
    /// Down.
    pub y: f32,
}

/// Lengths of controls that stretch along one axis, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LengthSizes {
    /// The width of a text or value field, one per control size: a field in a small options bar is narrower than one
    /// in a dialog.
    pub field: ControlSizes,
    /// The length of a value track (a slider, a range slider, a thumb row).
    pub track: f32,
    /// The width of a short field, such as a number beside a slider.
    pub short: f32,
    /// The width of a number field over a slider, one per control size.
    pub number: ControlSizes,
}

impl LengthSizes {
    /// The field widths multiplied by `factor` and rounded to whole pixels: a field holds a number of characters, so
    /// its width follows the text size. The track holds no text and keeps its length.
    pub(crate) fn text_scaled(self, factor: f32) -> LengthSizes {
        let round = |v: f32| (v * factor).round();
        let sizes = |c: ControlSizes| ControlSizes { sm: round(c.sm), md: round(c.md) };
        LengthSizes { field: sizes(self.field), short: round(self.short), number: sizes(self.number), ..self }
    }
}

/// Row heights other than a control's, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowSizes {
    /// The height of a row that shows text only: one body line and `space.gap.tight`. Read-only display tables (a
    /// metadata table) take it; rows the user picks from stay a control high.
    pub text: f32,
}

/// Thicknesses of gradient ramps, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RampSizes {
    /// A ramp drawn as a bar of its own: the output bar of a levels editor, a hue or saturation bar.
    pub bar: f32,
    /// A ramp drawn as the groove of a value track: the track of a colour channel slider.
    pub groove: f32,
}

/// Plot sizes, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlotSizes {
    /// Least width.
    pub width: f32,
    /// Least height.
    pub height: f32,
}

/// Canvas drag handle sizes, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandleSizes {
    /// Small handle: side or diameter.
    pub sm: f32,
    /// Standard handle: side or diameter.
    pub md: f32,
    /// The radius of the ring a pin's fixed rotation is set on.
    pub ring: f32,
}

/// Pointer tolerances, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitSizes {
    /// How far a press may move and still be a click.
    pub slop: f32,
    /// The least hit size of a part, in logical pixels; a thin handle or line is padded to it.
    pub target: f32,
}

/// Line widths, dash lengths and gaps in points, and the icon line as a share of the icon side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StrokeTokens {
    /// Hairline.
    pub thin: f32,
    /// Standard line.
    pub medium: f32,
    /// Heavy line.
    pub thick: f32,
    /// Heaviest line: an arc or indicator that must read at a glance.
    pub heavy: f32,
    /// Length of a dash.
    pub dash: f32,
    /// Gap between dashes.
    pub dash_gap: f32,
    /// The line of a stroke-set icon, as a share of the icon's side rather than in points, so a small glyph keeps the
    /// weight of a large one.
    pub icon: f32,
}

/// Thumbnail sides, in logical pixels, for the three user settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThumbSizes {
    /// Small.
    pub sm: f32,
    /// Medium.
    pub md: f32,
    /// Large.
    pub lg: f32,
    /// The picture side of a tile card at zoom 1: the cards of a tile grid.
    pub card: f32,
    /// The samples of a thumbnail size choice in a panel's options dialog (Layers, Channels, Paths).
    pub sample: ThumbSamples,
}

/// The sample pictures beside the three thumbnail size choices of a panel's options dialog, in logical pixels: larger
/// than the panel's own thumbnails, so the three read apart at a glance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThumbSamples {
    /// The sample of the small size.
    pub sm: f32,
    /// The sample of the medium size.
    pub md: f32,
    /// The sample of the large size.
    pub lg: f32,
}
