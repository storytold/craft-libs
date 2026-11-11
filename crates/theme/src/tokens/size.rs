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

/// Corner radii, edge style and row style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeTokens {
    /// Corner radii.
    pub radius: RadiusSet,
    /// Edge style.
    pub edge: Edge,
    /// How list and tab rows are drawn.
    pub rows: RowStyle,
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

/// Control heights, in logical pixels.
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
    /// The least height of a list row that shows a thumbnail.
    pub thumb_row: f32,
    /// Canvas drag handles.
    pub handle: HandleSizes,
    /// Pointer tolerance.
    pub hit: HitSizes,
    /// Side of a checkerboard cell in UI swatches and thumbnails.
    pub checker: f32,
    /// The side of a rotary control: the dial of an angle dial.
    pub dial: f32,
    /// The least size of a plot or two-dimensional value area: the well of a point picker, before the
    /// picker's own fitting.
    pub plot: PlotSizes,
    /// Lengths of controls that stretch along one axis.
    pub length: LengthSizes,
    /// The thickness of the groove of a value track (a slider, a range slider, a progress bar).
    pub groove: f32,
    /// Gradient ramps: the colours a value runs through, drawn as a bar or as the groove of a track.
    pub ramp: RampSizes,
    /// The thickness of a bar that shows a quantity as a filled share of a bordered groove (a progress bar).
    pub meter: f32,
    /// The pointer marks on the edge of a value track.
    pub marker: MarkerSizes,
    /// The long side of the track of a two-state toggle (a switch).
    pub toggle: f32,
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
    /// The rulers of a canvas.
    pub ruler: RulerSizes,
    /// The controls drawn over a canvas.
    pub canvas: CanvasSizes,
}

/// The sizes of the controls drawn over a canvas, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasSizes {
    /// The side of the canvas compass, the control that shows and resets the view rotation.
    pub compass: f32,
    /// The height of the stage a story of a canvas control shows its canvas on.
    pub stage_height: f32,
    /// The least width of that stage.
    pub stage_width_min: f32,
    /// The greatest width of that stage.
    pub stage_width_max: f32,
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
    /// The width of a text or value field.
    pub field: f32,
    /// The length of a value track (a slider, a range slider, a thumb row).
    pub track: f32,
    /// The width of a short field, such as a number beside a slider.
    pub short: f32,
    /// The width of a number field over a slider.
    pub number: f32,
}

/// The size of a triangular pointer mark on the edge of a value track, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MarkerSizes {
    /// The width of the mark along the track.
    pub width: f32,
    /// The height of the mark across the track.
    pub height: f32,
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
    /// The greatest side of a square plot that takes the room it is given (the graph of a curve editor).
    pub most: f32,
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
    /// The hit radius of a transform box's handle.
    pub hit: f32,
    /// How far from a transform box's corner the rotate pointer shows outside the box.
    pub rotate_reach: f32,
    /// Where the rotate band outside a transform box's corner begins, from the corner.
    pub band_inner: f32,
    /// Where the rotate band outside a transform box's corner ends, from the corner.
    pub band_outer: f32,
}

/// Pointer tolerances, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitSizes {
    /// How far a press may move and still be a click.
    pub slop: f32,
    /// The least hit size of a part, in logical pixels; a thin handle or line is padded to it.
    pub target: f32,
    /// How far from the centre of a rotary control a press sets no value.
    pub dead_zone: f32,
    /// How far from the pointer the handles of an on-canvas object show.
    pub reveal: f32,
}

/// Line widths, dash lengths and gaps, in points.
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
}
