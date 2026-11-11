//! Size sets: the numbers behind a Size choice. A set holds the base values (what the semantic
//! `size`, `space`, `shape` and `typography` tokens start from). The built-in sets are the JSON
//! files in `sizes/`, read by [`format::parse_sizes`].

use std::sync::LazyLock;

use super::format;
use super::scale::Radius;
use super::theme::Size;
use super::tokens::{Edge, FontSlot, RadiusSet, RowStyle, SizeTokens, SpaceTokens, StrokeTokens, TypeTokens};

/// The Size-dependent semantic values: sizes, spacing, default shape and type roles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Preset {
    /// Control, icon, bar and thumbnail sizes.
    pub size: SizeTokens,
    /// Gaps and padding.
    pub space: SpaceTokens,
    /// Default corner radii.
    pub radius: RadiusSet,
    /// Default edge.
    pub edge: Edge,
    /// Default row style.
    pub rows: RowStyle,
    /// Line widths and dashes.
    pub stroke: StrokeTokens,
    /// Type roles.
    pub typography: TypeTokens,
}

/// A size set: what a [`Size`] choice stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeSet {
    /// The set's id, such as `compact`.
    pub id: String,
    /// Its display name.
    pub name: String,
    /// The base values, at `base.*` in the file.
    pub base: Preset,
}

static COMPACT: LazyLock<SizeSet> = LazyLock::new(|| builtin_set(include_str!("../sizes/compact.json")));
static COMFORTABLE: LazyLock<SizeSet> = LazyLock::new(|| builtin_set(include_str!("../sizes/comfortable.json")));

#[allow(clippy::panic)] // The built-in files are checked by a test.
fn builtin_set(text: &str) -> SizeSet {
    match format::parse_sizes(text.as_bytes()) {
        Ok(file) => file.set,
        Err(e) => panic!("built-in size set: {e}"),
    }
}

/// The built-in set a [`Size`] names.
pub fn builtin(size: Size) -> &'static SizeSet {
    match size {
        Size::Compact => &COMPACT,
        Size::Comfortable => &COMFORTABLE,
    }
}

/// The JSON form of one value in a set.
pub(crate) enum Slot<'a> {
    /// A number.
    Num(&'a mut f32),
    /// A number or `"full"`.
    Length(&'a mut f32),
    /// A font slot name.
    Font(&'a mut FontSlot),
    /// `"flat"`.
    Edge(&'a mut Edge),
    /// `"divided"` or `"rounded"`.
    Rows(&'a mut RowStyle),
}

/// One value of a set: its path in the file, the range it must lie in, and where it is stored.
pub(crate) struct Leaf<'a> {
    pub path: &'static str,
    pub lo: f32,
    pub hi: f32,
    pub slot: Slot<'a>,
}

/// The radius value of `"full"`.
pub(crate) const FULL: f32 = Radius::Full.px();

macro_rules! leaves {
    ($out:ident; $( $path:literal $kind:ident $lo:literal..=$hi:literal => $place:expr; )*) => {
        $( $out.push(Leaf { path: $path, lo: $lo, hi: $hi, slot: Slot::$kind(&mut $place) }); )*
    };
}

/// Every value of a set with its range, in file order. The one table the parser, validator and
/// writer share.
pub(crate) fn leaves(b: &mut Preset) -> Vec<Leaf<'_>> {
    let mut out = Vec::new();
    let s = &mut b.size;
    leaves! { out;
        "base.size.control.sm" Num 16.0..=48.0 => s.control.sm;
        "base.size.control.md" Num 16.0..=48.0 => s.control.md;
        "base.size.icon.sm" Num 10.0..=32.0 => s.icon.sm;
        "base.size.icon.md" Num 10.0..=32.0 => s.icon.md;
        "base.size.icon.lg" Num 10.0..=32.0 => s.icon.lg;
        "base.size.icon.glyph_sm" Num 8.0..=32.0 => s.icon.glyph_sm;
        "base.size.icon.glyph_md" Num 8.0..=32.0 => s.icon.glyph_md;
        "base.size.rail.width" Num 24.0..=96.0 => s.rail.width;
        "base.size.rail.button" Num 16.0..=64.0 => s.rail.button;
        "base.size.bar.title" Num 16.0..=80.0 => s.bar.title;
        "base.size.bar.tab" Num 16.0..=64.0 => s.bar.tab;
        "base.size.bar.status" Num 16.0..=64.0 => s.bar.status;
        "base.size.bar.options" Num 16.0..=80.0 => s.bar.options;
        "base.size.bar.options_narrow" Num 16.0..=80.0 => s.bar.options_narrow;
        "base.size.panel_header" Num 16.0..=64.0 => s.panel_header;
        "base.size.popup_min" Num 64.0..=640.0 => s.popup_min;
        "base.size.scroll_fade" Num 4.0..=64.0 => s.scroll_fade;
        "base.size.thumb.sm" Num 12.0..=96.0 => s.thumb.sm;
        "base.size.thumb.md" Num 12.0..=96.0 => s.thumb.md;
        "base.size.thumb.lg" Num 12.0..=96.0 => s.thumb.lg;
        "base.size.thumb.card" Num 48.0..=512.0 => s.thumb.card;
        "base.size.thumb_row" Num 16.0..=96.0 => s.thumb_row;
        "base.size.handle.sm" Num 2.0..=32.0 => s.handle.sm;
        "base.size.handle.md" Num 2.0..=32.0 => s.handle.md;
        "base.size.handle.ring" Num 2.0..=64.0 => s.handle.ring;
        "base.size.handle.hit" Num 2.0..=64.0 => s.handle.hit;
        "base.size.handle.rotate_reach" Num 2.0..=128.0 => s.handle.rotate_reach;
        "base.size.handle.band_inner" Num 0.0..=64.0 => s.handle.band_inner;
        "base.size.handle.band_outer" Num 2.0..=128.0 => s.handle.band_outer;
        "base.size.hit.slop" Num 0.0..=32.0 => s.hit.slop;
        "base.size.hit.target" Num 0.0..=32.0 => s.hit.target;
        "base.size.hit.dead_zone" Num 0.0..=32.0 => s.hit.dead_zone;
        "base.size.hit.reveal" Num 8.0..=256.0 => s.hit.reveal;
        "base.size.checker" Num 1.0..=32.0 => s.checker;
        "base.size.dial" Num 16.0..=128.0 => s.dial;
        "base.size.plot.width" Num 48.0..=640.0 => s.plot.width;
        "base.size.plot.height" Num 32.0..=480.0 => s.plot.height;
        "base.size.plot.most" Num 48.0..=1024.0 => s.plot.most;
        "base.size.length.field" Num 64.0..=1024.0 => s.length.field;
        "base.size.length.track" Num 48.0..=1024.0 => s.length.track;
        "base.size.length.short" Num 32.0..=512.0 => s.length.short;
        "base.size.length.number" Num 32.0..=512.0 => s.length.number;
        "base.size.groove" Num 0.25..=16.0 => s.groove;
        "base.size.ramp.bar" Num 1.0..=64.0 => s.ramp.bar;
        "base.size.ramp.groove" Num 0.25..=32.0 => s.ramp.groove;
        "base.size.meter" Num 1.0..=32.0 => s.meter;
        "base.size.marker.width" Num 4.0..=32.0 => s.marker.width;
        "base.size.marker.height" Num 4.0..=32.0 => s.marker.height;
        "base.size.toggle" Num 12.0..=96.0 => s.toggle;
        "base.size.pointer_tip.x" Num 0.0..=64.0 => s.pointer_tip.x;
        "base.size.pointer_tip.y" Num 0.0..=64.0 => s.pointer_tip.y;
        "base.size.float.width" Num 64.0..=1024.0 => s.float.width;
        "base.size.float.height" Num 48.0..=1024.0 => s.float.height;
        "base.size.window.cascade" Num 8.0..=96.0 => s.window.cascade;
        "base.size.pane.least" Num 16.0..=128.0 => s.pane.least;
        "base.size.ruler.tick.mid" Num 1.0..=32.0 => s.ruler.tick.mid;
        "base.size.canvas.compass" Num 16.0..=128.0 => s.canvas.compass;
        "base.size.canvas.stage_height" Num 120.0..=1024.0 => s.canvas.stage_height;
        "base.size.canvas.stage_width_min" Num 120.0..=1024.0 => s.canvas.stage_width_min;
        "base.size.canvas.stage_width_max" Num 120.0..=2048.0 => s.canvas.stage_width_max;
        "base.size.dock.column" Num 120.0..=640.0 => s.dock.column;
    }
    let sp = &mut b.space;
    leaves! { out;
        "base.space.gap.tight" Num 0.0..=32.0 => sp.gap.tight;
        "base.space.gap.inline" Num 0.0..=32.0 => sp.gap.inline;
        "base.space.gap.row" Num 0.0..=32.0 => sp.gap.row;
        "base.space.gap.stack" Num 0.0..=32.0 => sp.gap.stack;
        "base.space.gap.item" Num 0.0..=32.0 => sp.gap.item;
        "base.space.gap.section" Num 0.0..=32.0 => sp.gap.section;
        "base.space.gap.card_column" Num 0.0..=32.0 => sp.gap.card_column;
        "base.space.gap.card_row" Num 0.0..=32.0 => sp.gap.card_row;
        "base.space.pad.control" Num 0.0..=32.0 => sp.pad.control;
        "base.space.pad.regular" Num 0.0..=32.0 => sp.pad.regular;
        "base.space.pad.roomy" Num 0.0..=32.0 => sp.pad.roomy;
        "base.space.pad.chip" Num 0.0..=32.0 => sp.pad.chip;
        "base.space.pad.segment" Num 0.0..=32.0 => sp.pad.segment;
        "base.space.inset.screen" Num 0.0..=64.0 => sp.inset.screen;
    }
    let r = &mut b.radius;
    leaves! { out;
        "base.radius.control" Length 0.0..=64.0 => r.control;
        "base.radius.surface" Length 0.0..=64.0 => r.surface;
        "base.radius.window" Length 0.0..=64.0 => r.window;
        "base.radius.panel" Length 0.0..=64.0 => r.panel;
        "base.radius.button" Length 0.0..=64.0 => r.button;
        "base.radius.pill" Length 0.0..=64.0 => r.pill;
    }
    leaves! { out;
        "base.edge" Edge 0.0..=0.0 => b.edge;
        "base.rows" Rows 0.0..=0.0 => b.rows;
    }
    let st = &mut b.stroke;
    leaves! { out;
        "base.stroke.thin" Num 0.25..=8.0 => st.thin;
        "base.stroke.medium" Num 0.25..=8.0 => st.medium;
        "base.stroke.thick" Num 0.25..=8.0 => st.thick;
        "base.stroke.heavy" Num 0.25..=8.0 => st.heavy;
        "base.stroke.dash" Num 1.0..=32.0 => st.dash;
        "base.stroke.dash_gap" Num 1.0..=32.0 => st.dash_gap;
    }
    let t = &mut b.typography;
    leaves! { out;
        "base.typography.caption.size" Num 8.0..=32.0 => t.caption.size;
        "base.typography.caption.line_height" Num 8.0..=48.0 => t.caption.line_height;
        "base.typography.caption.font" Font 0.0..=0.0 => t.caption.font;
        "base.typography.label.size" Num 8.0..=32.0 => t.label.size;
        "base.typography.label.line_height" Num 8.0..=48.0 => t.label.line_height;
        "base.typography.label.font" Font 0.0..=0.0 => t.label.font;
        "base.typography.body.size" Num 8.0..=32.0 => t.body.size;
        "base.typography.body.line_height" Num 8.0..=48.0 => t.body.line_height;
        "base.typography.body.font" Font 0.0..=0.0 => t.body.font;
        "base.typography.body_strong.size" Num 8.0..=32.0 => t.body_strong.size;
        "base.typography.body_strong.line_height" Num 8.0..=48.0 => t.body_strong.line_height;
        "base.typography.body_strong.font" Font 0.0..=0.0 => t.body_strong.font;
        "base.typography.title.size" Num 8.0..=32.0 => t.title.size;
        "base.typography.title.line_height" Num 8.0..=48.0 => t.title.line_height;
        "base.typography.title.font" Font 0.0..=0.0 => t.title.font;
        "base.typography.heading.size" Num 8.0..=32.0 => t.heading.size;
        "base.typography.heading.line_height" Num 8.0..=48.0 => t.heading.line_height;
        "base.typography.heading.font" Font 0.0..=0.0 => t.heading.font;
        "base.typography.body_large.size" Num 8.0..=32.0 => t.body_large.size;
        "base.typography.body_large.line_height" Num 8.0..=48.0 => t.body_large.line_height;
        "base.typography.body_large.font" Font 0.0..=0.0 => t.body_large.font;
        "base.typography.mono.size" Num 8.0..=32.0 => t.mono.size;
        "base.typography.mono.line_height" Num 8.0..=48.0 => t.mono.line_height;
        "base.typography.mono.font" Font 0.0..=0.0 => t.mono.font;
    }
    out
}
