//! What a push or toggle button shows: a leading icon (or spinner), a label and a trailing icon, centred in the
//! button unless it aligns them to an edge, and the look parts the buttons share.

use crate::contract::{Component, Cx, MeasureCx, PaintUi, Styled as _};
use crate::primitives::{HAlign, IconSpec, TextRun};
use crate::style::{FocusLook, GlyphRole, IconLook, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole, UiTokens, VariantParts};
use craft_behavior::{Flow, Insets, Metrics, Proposal};
use craft_theme::{
    Stateful, States, Variant,
    variant::{Emphasis, Tone},
};
use egui::{Rect, Vec2, pos2};

/// The parts of a button's look that the face paints besides its text and icon looks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FaceLook {
    /// The gap between icon and label.
    pub gap: f32,
    /// The colour of the chord and of the detail line: the variant's muted ink, as a `Kbd` chord is muted.
    pub muted: Stateful<StyleColor>,
}

impl FaceLook {
    /// The face look of the instance's variant: the control's gap and the variant's muted ink.
    pub(crate) fn of(cx: &StyleCx<'_>) -> FaceLook {
        FaceLook { gap: cx.control().gap, muted: cx.variant_look().muted }
    }
}

/// The label and icon looks of the instance's variant: the variant's text colours at the control's icon side.
pub(super) fn ink(cx: &StyleCx<'_>) -> (TextLook, IconLook) {
    let color = cx.variant_look().text;
    (TextLook::new(TypeRole::Label, color.clone()), IconLook::new(cx.control().icon, color))
}

/// What a face shows.
pub(crate) struct FaceSpec<'a> {
    pub label: &'a str,
    pub role: Option<TypeRole>,
    pub lead: Option<&'a IconSpec<'a>>,
    pub trail: Option<&'a IconSpec<'a>>,
    /// A spinner takes the place of the leading icon, or of the label when there is no leading icon: the face keeps
    /// the slot empty and the button shows a `Spinner` there.
    pub loading: bool,
    /// The character of `label` underlined as its mnemonic.
    pub mnemonic: Option<usize>,
    /// Where the icons and the label sit along the area when it is wider than they are.
    pub align: HAlign,
    /// The command's chord, shown after the label and dropped before the label elides.
    pub chord: Option<&'a str>,
    /// A caption-role second line under the icon and label, wrapped to the face's width.
    pub detail: Option<&'a str>,
}

impl FaceSpec<'_> {
    fn run(&self) -> TextRun<'_> {
        let run = TextRun::mnemonic(self.label, self.mnemonic).elide();
        match self.role {
            Some(role) => run.role(role),
            None => run,
        }
    }

    fn icons(&self) -> usize {
        usize::from(self.lead.is_some()) + usize::from(self.trail.is_some())
    }

    /// The gaps between the parts that are shown.
    fn gaps(&self) -> usize {
        (self.icons() + usize::from(!self.label.is_empty())).saturating_sub(1)
    }

    /// The chord as a run: only after a label, and not while the face is busy. It keeps its left-to-right order under
    /// right-to-left, as `ChordText` lays a chord out.
    fn chord_run(&self) -> Option<TextRun<'_>> {
        self.chord.filter(|chord| !chord.is_empty() && !self.loading && !self.label.is_empty()).map(|chord| {
            let mut run = TextRun::new(chord).role(TypeRole::Caption);
            run.ltr = true;
            run
        })
    }

    fn detail_run(&self) -> Option<TextRun<'_>> {
        self.detail.filter(|detail| !detail.is_empty()).map(|detail| TextRun::new(detail).role(TypeRole::Caption))
    }
}

/// Whether the chord, `chord` wide after a gap, fits with the whole label (`label` wide) and the icons and gaps (`fixed`)
/// in `room`: the chord is dropped before the label elides.
fn chord_fits(label: f32, chord: f32, fixed: f32, gap: f32, room: f32) -> bool {
    fixed + label + gap + chord <= room
}

/// How far the parts start from the reading start of a row with `free` room to spare.
fn lead_offset(align: HAlign, free: f32) -> f32 {
    match align {
        HAlign::Start => 0.0,
        HAlign::Center => free / 2.0,
        HAlign::End => free,
    }
}

/// The row of icons, label and chord, `row_height` tall: all of `area` without a detail line; with one `detail_height`
/// tall, the top of the block the two make, centred in `area`.
fn row_rect(area: Rect, row_height: f32, detail_height: Option<f32>, gap: f32) -> Rect {
    match detail_height {
        Some(detail) => {
            let block = row_height + gap + detail;
            Rect::from_min_size(pos2(area.min.x, area.center().y - block / 2.0), Vec2::new(area.width(), row_height))
        }
        None => area,
    }
}

/// The metrics of a button with a face: the control's, with the horizontal padding at least the corner radius plus
/// the gap, so the label and icons of a capsule clear its round ends.
pub(super) fn metrics(cx: &StyleCx<'_>, parts: &VariantParts) -> Metrics {
    let control = cx.control();
    let round = f32::from(parts.radius.nw).min(control.height / 2.0);
    Metrics::new().padding(Insets::horizontal(control.pad_x.max(round + control.gap))).min_size(Vec2::new(0.0, control.height)).gap(control.gap)
}

/// The surface of a button from its variant: raised unless flat (ghost, or a custom look with no fill or edge at
/// rest), the variant's fill and edge.
pub(super) fn surface(cx: &StyleCx<'_>, parts: &VariantParts) -> SurfaceLook {
    let mut look = SurfaceLook::new(parts.fill.clone()).radius(parts.radius);
    let variant = cx.variant.resolve(cx.tokens);
    let flat = match cx.variant {
        Variant::Look { emphasis, .. } => emphasis == Emphasis::Ghost,
        _ => matches!(cx.variant, Variant::Custom(_)) && variant.edge.is_none() && variant.fill.rest.a() == 0,
    };
    if !flat {
        look = look.kind(SurfaceKind::Raised).kind_down(SurfaceKind::Pressed);
    }
    // A variant without an edge draws none in any state: the kind's own edge would frame a disabled solid button, and
    // a border in a clear colour would still leave an anti-aliased trace at the corners.
    match parts.edge.clone() {
        Some(edge) => look.border(edge, cx.tokens.stroke.thin).border_replaces_edge(),
        None => look.border_replaces_edge(),
    }
}

/// The surface of a button that shows the on look while on: the variant's surface, with the on plate and edge in
/// place of its fill and edge while on, and a raised surface drawn pressed in while on.
pub(super) fn on_surface(cx: &StyleCx<'_>, off: &VariantParts, on: &VariantParts) -> SurfaceLook {
    let mut look = surface(cx, off);
    if look.kind.is_some() {
        look = look.kind_on(SurfaceKind::Pressed);
    }
    look.fill = off.fill.clone().on(on.fill.off);
    look.border = off.edge.clone().map(|edge| edge.on(on.edge.as_ref().map_or(on.fill.off, |edge| edge.off)));
    look
}

/// The look of a button that shows only its icon: an icon button, or a toggle button that is `icon_only`.
pub(super) struct IconFace {
    pub surface: SurfaceLook,
    pub metrics: Metrics,
    /// The glyph at the side of the scope's glyph role, in the on look's text colour while on.
    pub glyph: IconLook,
}

/// The icon-only look of the instance's variant: a square of the control height with control corners, as a ghost
/// button of the old look (a capsule would round the on plate), showing the on look while on. The scope's glyph role
/// sets the glyph's side and, in a rail, the quieter ink it rests in.
pub(super) fn icon_face(cx: &StyleCx<'_>) -> IconFace {
    let (off, on) = (cx.variant_look(), on_parts(cx));
    let (icon, text) = (&cx.tokens.size.icon, &cx.tokens.color.text);
    let (side, rest) = match cx.glyph_role() {
        GlyphRole::Tool => (icon.md, None),
        GlyphRole::ToolRail => (icon.lg, Some(text.muted)),
        GlyphRole::PanelRail => (cx.glyph(), Some(text.faint)),
        _ => (cx.glyph(), None),
    };
    let mut ink = off.text.clone();
    if let Some(rest) = rest {
        ink.off.rest = cx.color(rest);
    }
    IconFace {
        surface: on_surface(cx, &off, &on).radius(UiTokens::corner(cx.tokens.shape.radius.control)),
        metrics: Metrics::new().min_size(Vec2::splat(cx.control().height)),
        glyph: IconLook::new(side, ink.on(on.text.off)),
    }
}

/// The look of a control that is on: the accent's soft fill with the accent's text colour,
/// inside the soft accent edge.
pub(super) fn on_parts(cx: &StyleCx<'_>) -> VariantParts {
    let soft = cx.with_variant(Variant::look(Emphasis::Soft, Tone::Accent)).variant_look();
    let colors = &cx.tokens.color;
    let fill = cx.color(colors.accent.soft);
    let ink = cx.color(colors.accent.on_soft);
    VariantParts { fill: Stateful::new(fill), text: Stateful::new(ink).off(States::new(ink, ink, ink, soft.text.off.disabled)), ..soft }
}

/// The focus ring of a button.
pub(crate) fn focus(cx: &StyleCx<'_>) -> FocusLook {
    cx.focus_look()
}

/// The size of the face: its parts and the gaps between them, the detail line under them.
/// The chord is dropped before the label elides; the label then elides in the room the icons and gaps leave, so a
/// squeezed face is as wide as its room; at its least the label is at its readable least and the chord is dropped.
pub(crate) fn measure<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>) -> Vec2 {
    let (icon, gap) = (cx.icon(), cx.gap());
    let fixed = icon * spec.icons() as f32 + gap * spec.gaps() as f32;
    let room = cx.room.unwrap_or(f32::INFINITY);
    let (text, chord) = if spec.label.is_empty() {
        (Vec2::ZERO, None)
    } else if cx.proposal == Proposal::Min {
        let run = spec.run();
        (Vec2::new(cx.least_width(&run), cx.layout_at(&run, f32::INFINITY).size().y), None)
    } else {
        let beside = spec.chord_run().and_then(|chord| {
            let label = cx.layout_at(&spec.run(), f32::INFINITY).size();
            let chord = cx.layout_at(&chord, f32::INFINITY).size();
            chord_fits(label.x, chord.x, fixed, gap, room).then_some((label, chord))
        });
        match beside {
            Some((label, chord)) => (Vec2::new(label.x, label.y.max(chord.y)), Some(chord.x)),
            None => (cx.layout_at(&spec.run(), (room - fixed).max(0.0)).size(), None),
        }
    };
    let row = Vec2::new(text.x + fixed + chord.map_or(0.0, |width| width + gap), text.y.max(icon));
    let Some(detail) = spec.detail_run() else { return row };
    let detail = if cx.proposal == Proposal::Min {
        Vec2::new(cx.least_width(&detail), cx.layout_at(&detail, f32::INFINITY).size().y)
    } else {
        cx.layout_at(&detail, room).size()
    };
    Vec2::new(row.x.max(detail.x), row.y + gap + detail.y)
}

/// `icon` in `color`, unless the spec names a colour of its own.
pub(super) fn tinted<'a>(icon: &IconSpec<'a>, color: StyleColor) -> IconSpec<'a> {
    let icon = icon.clone();
    if icon.color.is_some() { icon } else { icon.color(color) }
}

/// The slots of the parts a face shows, placed in an area by the spec's alignment.
struct Slots {
    lead: Option<Rect>,
    text: Option<Rect>,
    chord: Option<Rect>,
    trail: Option<Rect>,
}

/// Places the icons of `side` and a label `text_width` wide with `gap` between them, and the chord of `chord` width
/// after the label, in `area` by `spec.align`: the leading icon at the reading start, the trailing icon at the reading
/// end.
fn slots(spec: &FaceSpec<'_>, side: f32, gap: f32, text_width: f32, chord: Option<f32>, area: Rect, rtl: bool) -> Slots {
    let width = side * spec.icons() as f32 + gap * spec.gaps() as f32 + text_width + chord.map_or(0.0, |width| width + gap);
    let lead = lead_offset(spec.align, (area.width() - width).max(0.0));
    let (_, mut rest) = Flow::new(area, rtl).take_start(lead);
    let mut take = |extent: f32| {
        let (slot, next) = rest.take_start(extent);
        rest = next;
        slot
    };
    let mut slots = Slots { lead: None, text: None, chord: None, trail: None };
    let mut gapped = false;
    if spec.lead.is_some() {
        slots.lead = Some(take(side));
        gapped = true;
    }
    if !spec.label.is_empty() {
        if gapped {
            take(gap);
        }
        slots.text = Some(take(text_width));
        gapped = true;
        if let Some(width) = chord {
            take(gap);
            slots.chord = Some(take(width));
        }
    }
    if spec.trail.is_some() {
        if gapped {
            take(gap);
        }
        slots.trail = Some(take(side));
    }
    slots
}

/// The square an icon of `side` takes in `slot`, vertically centred in `area`.
fn icon_at(slot: Rect, area: Rect, side: f32) -> Rect {
    Rect::from_center_size(pos2(slot.center().x, area.center().y), Vec2::splat(side))
}

/// The top of a label in `area`: its baseline sits where the baseline of centred body text sits, the text of the
/// labels and fields beside a control, so a row of controls shares one baseline whatever the label's own role.
fn text_top(draw: &crate::primitives::Draw<'_>, area: Rect, text: &crate::primitives::TextLayout) -> f32 {
    let body = draw.layout(&TextRun::new("x").role(TypeRole::Body), f32::INFINITY);
    area.center().y - body.size().y / 2.0 + body.first_baseline - text.first_baseline
}

/// Paints the face in `area`, placed by `spec.align`: the leading icon at the reading start, the trailing icon at the
/// reading end, the chord after the label when it fits beside the whole label, the detail line centred in `area` with
/// them as one block.
pub(crate) fn paint<C: Component>(ui: &PaintUi<'_>, cx: &Cx<'_, C>, ink: (&TextLook, &IconLook), look: &FaceLook, spec: &FaceSpec<'_>, area: Rect) {
    let (text_look, icon_look) = ink;
    let (text_color, icon_color, side) = (cx.resolve(&text_look.color), cx.resolve(&icon_look.color), icon_look.side);
    let draw = ui.draw();
    let fixed = side * spec.icons() as f32 + look.gap * spec.gaps() as f32;
    let run = spec.run().color(text_color);
    let muted = cx.resolve(&look.muted);
    let beside = spec.chord_run().and_then(|chord| {
        let label = draw.layout(&run, f32::INFINITY);
        let chord = draw.layout(&chord.color(muted), f32::INFINITY);
        chord_fits(label.size().x, chord.size().x, fixed, look.gap, area.width()).then_some((label, chord))
    });
    let (text, chord) = match beside {
        Some((label, chord)) => (Some(label), Some(chord)),
        None if spec.label.is_empty() => (None, None),
        None => (Some(draw.layout(&run, (area.width() - fixed).max(0.0))), None),
    };
    let detail = spec.detail_run().map(|detail| draw.layout(&detail.color(muted), area.width()));
    let height = |layout: &Option<crate::primitives::TextLayout>| layout.as_ref().map_or(0.0, |layout| layout.size().y);
    let row = row_rect(area, height(&text).max(height(&chord)).max(side), detail.as_ref().map(|detail| detail.size().y), look.gap);
    let rtl = cx.flow.is_rtl();
    let text_width = text.as_ref().map_or(0.0, |text| text.size().x);
    let slots = slots(spec, side, look.gap, text_width, chord.as_ref().map(|chord| chord.size().x), row, rtl);
    if let (Some(lead), Some(slot)) = (spec.lead, slots.lead)
        && !spec.loading
    {
        draw.icon(icon_at(slot, row, side), &tinted(lead, icon_color));
    }
    if let (Some(text), Some(slot)) = (&text, slots.text)
        && !(spec.loading && spec.lead.is_none())
    {
        draw.galley(pos2(slot.min.x, text_top(&draw, row, text)), text);
    }
    if let (Some(chord), Some(slot)) = (&chord, slots.chord) {
        draw.galley(pos2(slot.min.x, text_top(&draw, row, chord)), chord);
    }
    if let (Some(trail), Some(slot)) = (spec.trail, slots.trail) {
        draw.icon(icon_at(slot, row, side), &tinted(trail, icon_color));
    }
    if let Some(detail) = &detail {
        let width = detail.size().x;
        let (_, rest) = Flow::new(area, rtl).take_start(lead_offset(spec.align, (area.width() - width).max(0.0)));
        let (slot, _) = rest.take_start(width);
        draw.galley(pos2(slot.min.x, row.max.y + look.gap), detail);
    }
}

/// Where the busy spinner of a loading face goes: the square of the icon side in the leading icon's slot, or in the
/// label's slot when there is no leading icon. `None` when the face has neither.
pub(super) fn spinner_rect<C: Component>(
    cx: &MeasureCx<'_, C>,
    spec: &FaceSpec<'_>,
    look: &FaceLook,
    side: f32,
    area: Rect,
    rtl: bool,
) -> Option<Rect> {
    let fixed = side * spec.icons() as f32 + look.gap * spec.gaps() as f32;
    let text = if spec.label.is_empty() { Vec2::ZERO } else { cx.layout_at(&spec.run(), (area.width() - fixed).max(0.0)).size() };
    let detail = spec.detail_run().map(|detail| cx.layout_at(&detail, area.width()).size().y);
    let row = row_rect(area, text.y.max(side), detail, look.gap);
    let slots = slots(spec, side, look.gap, text.x, None, row, rtl);
    let slot = if spec.lead.is_some() { slots.lead } else { slots.text }?;
    Some(icon_at(slot, row, side))
}
