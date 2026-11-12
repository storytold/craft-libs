//! The adornment layout of a field (18.1): glyphs and buttons before and after the text, placed by one function so a
//! field's parts and its paint read the same rects.

use super::FieldStyle;
use crate::buttons::IconButton;
use crate::contract::{MeasureCx, PaintUi, Props as _};
use crate::primitives::{IconSpec, Span, TextRun};
use crate::runtime::env::Label;
use crate::style::{StyleColor, TypeRole};
use craft_behavior::{Flow, PartKey};
use craft_theme::ControlSize;
use egui::{Rect, Vec2, vec2};
use smallvec::SmallVec;

/// Something a field draws beside its text.
pub struct Adornment {
    kind: Kind,
}

enum Kind {
    Glyph(IconSpec<'static>),
    Button(PartKey, IconSpec<'static>, Label),
    /// Text in the line, of `role`, monospace with `mono`; `size` is its measured run, zero until
    /// [`Adornment::measured`].
    Text {
        label: Label,
        role: TypeRole,
        mono: bool,
        size: Vec2,
    },
}

impl Adornment {
    /// A decorative icon of the style's glyph side.
    pub fn glyph(icon: impl Into<IconSpec<'static>>) -> Adornment {
        Adornment { kind: Kind::Glyph(icon.into()) }
    }

    /// An icon button, the part `part` named `name`, of the style's button side.
    pub fn button(part: PartKey, icon: impl Into<IconSpec<'static>>, name: Label) -> Adornment {
        Adornment { kind: Kind::Button(part, icon.into(), name) }
    }

    /// Text in the field's line, in `role`: a unit after a number, a fixed `#` before hex digits. It takes the width of
    /// its run, which the field measures with [`Adornment::measured`] before placing it, and sits against the text
    /// with no gap between them.
    pub fn text(label: impl Into<Label>, role: TypeRole) -> Adornment {
        Adornment { kind: Kind::Text { label: label.into(), role, mono: false, size: Vec2::ZERO } }
    }

    /// A text adornment in the monospace face, as the text of a monospace field; a glyph or a button is returned as it
    /// is.
    pub fn mono(mut self) -> Adornment {
        if let Kind::Text { mono, .. } = &mut self.kind {
            *mono = true;
        }
        self
    }

    /// The run of a text adornment.
    fn run(label: &Label, role: TypeRole, mono: bool) -> TextRun<'_> {
        let span = Span::new(label.as_str());
        TextRun::spans(vec![if mono { span.mono() } else { span }]).role(role)
    }

    /// The adornment with a text adornment's run measured in `cx`; a glyph or a button is returned as it is.
    pub fn measured<C: crate::contract::Component>(mut self, cx: &MeasureCx<'_, C>) -> Adornment {
        if let Kind::Text { label, role, mono, size } = &mut self.kind {
            *size = cx.layout(&Self::run(label, *role, *mono)).size();
        }
        self
    }

    /// The part key of a button; `None` for a glyph or text.
    pub fn part(&self) -> Option<PartKey> {
        match &self.kind {
            Kind::Button(part, ..) => Some(*part),
            _ => None,
        }
    }

    /// The size the adornment takes in the line: a square for a glyph or a button, the run for text.
    fn extent(&self, style: &FieldStyle) -> Vec2 {
        match &self.kind {
            Kind::Glyph(_) => Vec2::splat(style.glyph.side),
            Kind::Button(..) => Vec2::splat(style.button),
            Kind::Text { size, .. } => *size,
        }
    }

    /// The gap between the adornment and its neighbour towards the text: none for text, which reads as part of it.
    fn gap_after(&self, style: &FieldStyle, innermost: bool) -> f32 {
        if innermost && matches!(self.kind, Kind::Text { .. }) { 0.0 } else { style.metrics.gap }
    }

    /// The icon, painted in `color` when there is one.
    fn spec(icon: &IconSpec<'static>, color: Option<StyleColor>) -> IconSpec<'static> {
        IconSpec { source: icon.source.clone(), ink: icon.ink, color: color.or(icon.color), mirror: icon.mirror, set: icon.set }
    }

    /// Paints the adornment in `rect`, one of `place`'s rects: a glyph or text in `color`, a button as an icon button
    /// face over its part.
    pub fn paint(&self, ui: &mut PaintUi<'_>, rect: Rect, color: StyleColor) {
        match &self.kind {
            Kind::Glyph(icon) => ui.draw().icon(rect, &Self::spec(icon, Some(color))),
            Kind::Button(part, icon, name) => {
                ui.face(*part, IconButton::new(Self::spec(icon, None)).label(name.clone()).focusable(false).size(ControlSize::Sm))
            }
            Kind::Text { label, role, mono, .. } => {
                ui.draw().text(rect, &Self::run(label, *role, *mono).color(color));
            }
        }
    }
}

/// Where the text and each adornment sit.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placed {
    /// What is left for the text.
    pub text: Rect,
    /// The rects of the leading adornments, in the order given.
    pub leading: SmallVec<[Rect; 2]>,
    /// The rects of the trailing adornments, in the order given.
    pub trailing: SmallVec<[Rect; 2]>,
}

/// The room a button at the end of a line takes from the box edge: `(line - side) / 2`, the same on every side, so
/// the button sits in the line's square. A glyph keeps the field's padding.
fn inset(style: &FieldStyle, first: Option<&Adornment>, padding: f32) -> f32 {
    match first {
        Some(adornment) if adornment.part().is_some() => ((style.metrics.min_size.y - adornment.extent(style).y) / 2.0).max(0.0),
        _ => padding,
    }
}

/// The width the adornments take from the content rect (the box minus its padding), gaps included, and the change of
/// the edge inset a button makes.
pub fn reserve(style: &FieldStyle, leading: &[Adornment], trailing: &[Adornment]) -> f32 {
    let metrics = style.metrics;
    let edges = inset(style, leading.first(), metrics.padding.start) - metrics.padding.start + inset(style, trailing.last(), metrics.padding.end)
        - metrics.padding.end;
    // The adornment next to the text is the last leading one and the first trailing one.
    let room = |adornments: &[Adornment], innermost: usize| {
        adornments.iter().enumerate().map(|(at, adornment)| adornment.extent(style).x + adornment.gap_after(style, at == innermost)).sum::<f32>()
    };
    edges + room(leading, leading.len().saturating_sub(1)) + room(trailing, 0)
}

/// Places `leading` from the reading start and `trailing` from the reading end of the field's box (`content`, the
/// content rect, with the padding added back), each `style.metrics.gap` from its neighbour and centred on the line,
/// and leaves the rest to the text; a text adornment next to the text sits against it. A glyph or text at an edge
/// sits one padding from it and a button `(line - side) / 2` from it. `trailing` is in reading order: its last
/// adornment is at the end. Text adornments are measured first ([`Adornment::measured`]).
pub fn place(content: Rect, style: &FieldStyle, leading: &[Adornment], trailing: &[Adornment], rtl: bool) -> Placed {
    let padding = style.metrics.padding;
    let centred = |cell: Rect, height: f32| Rect::from_center_size(cell.center(), vec2(cell.width(), height));
    let (before, after) = if rtl { (padding.end, padding.start) } else { (padding.start, padding.end) };
    let rect = Rect::from_min_max(content.min - vec2(before, 0.0), content.max + vec2(after, 0.0));
    let mut flow = Flow::new(rect, rtl);
    flow = flow.take_start(inset(style, leading.first(), padding.start)).1;
    flow = flow.take_end(inset(style, trailing.last(), padding.end)).1;
    let mut lead = SmallVec::new();
    for (at, adornment) in leading.iter().enumerate() {
        let extent = adornment.extent(style);
        let (cell, rest) = flow.take_start(extent.x);
        lead.push(centred(cell, extent.y));
        flow = rest.take_start(adornment.gap_after(style, at + 1 == leading.len())).1;
    }
    let mut trail: SmallVec<[Rect; 2]> = SmallVec::new();
    for (at, adornment) in trailing.iter().enumerate().rev() {
        let extent = adornment.extent(style);
        let (cell, rest) = flow.take_end(extent.x);
        trail.push(centred(cell, extent.y));
        flow = rest.take_end(adornment.gap_after(style, at == 0)).1;
    }
    trail.reverse();
    Placed { text: flow.rect(), leading: lead, trailing: trail }
}
