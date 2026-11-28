//! What a push or toggle button shows: a leading icon (or spinner), a label and a trailing icon, centred in the
//! button unless it aligns them to an edge, and the look parts the buttons share.

use super::{ActiveLook, ControlPlace, GroupLevel, RibbonLook};
use crate::contract::{Component, Cx, MeasureCx, PaintUi, Styled as _};
use crate::list::ThumbAspect;
use crate::primitives::{HAlign, IconSpec, TextLayout, TextRun, Wrap};
use crate::style::{
    ComponentStyle as _, FocusLook, GlyphRole, IconLook, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole, UiTokens, VariantParts,
};
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
    /// The side of a stacked face's leading icon (`stacked_side`).
    pub stacked_icon: f32,
    /// How far under a stacked face's leading icon the split button's parts meet.
    pub divide_gap: f32,
    /// Whether a stacked face lays its block out from its top (`stacked_lines`): in a ribbon.
    pub stacked_top: bool,
    /// The room of a trailing menu arrow where the ribbon sets one (`RibbonLook::arrow`); else the icon side.
    pub arrow: Option<f32>,
}

impl FaceLook {
    /// The face look of the instance's variant: the control's gap, the variant's muted ink and the stacked icon side.
    pub(crate) fn of(cx: &StyleCx<'_>) -> FaceLook {
        let control = cx.control();
        FaceLook {
            gap: control.gap,
            muted: cx.variant_look().muted,
            stacked_icon: stacked_side(cx.tokens, control.icon, cx.env::<ControlPlace>(), cx.env::<RibbonLook>()),
            divide_gap: cx.tokens.space.gap.tight,
            stacked_top: stacked_lines(cx.env::<ControlPlace>()) > 0,
            arrow: arrow_room(cx.env::<ControlPlace>(), cx.env::<RibbonLook>()),
        }
    }
}

/// The label lines a stacked face keeps room for whatever its label, laying its block out from its top: two in a
/// ribbon, where the large buttons share one height and their icons one line, a one-line label leaving its second line
/// empty, as the ribbon's large buttons always did; none elsewhere, where the face is as high as its content.
fn stacked_lines(place: ControlPlace) -> usize {
    if place == ControlPlace::Ribbon { 2 } else { 0 }
}

/// The side of a stacked face's leading icon: the ribbon's large side where one is set (`RibbonLook`), else the
/// large icon, or the control's own icon when that is larger. The style and the measure both take it from here, so
/// the painted icon is the measured one.
pub(super) fn stacked_side(tokens: &UiTokens, control_icon: f32, place: ControlPlace, icons: RibbonLook) -> f32 {
    if place == ControlPlace::Ribbon && icons.large > 0 { f32::from(icons.large) } else { tokens.size.icon.lg.max(control_icon) }
}

/// The room a trailing menu arrow takes in a ribbon that sets one (`RibbonLook::arrow`).
pub(crate) fn arrow_room(place: ControlPlace, icons: RibbonLook) -> Option<f32> {
    (place == ControlPlace::Ribbon && icons.arrow > 0).then_some(f32::from(icons.arrow))
}

/// The most characters a stacked label keeps on one line, as the ribbon's large buttons always kept them: a longer
/// label breaks at the space nearest its middle.
const ONE_LINE_CHARS: usize = 10;

/// The label and icon looks of the instance's variant: the variant's text colours at the control's icon side.
pub(super) fn ink(cx: &StyleCx<'_>) -> (TextLook, IconLook) {
    let color = cx.variant_look().text;
    (TextLook::new(TypeRole::Label, color.clone()), IconLook::new(super::small_icon(cx, cx.control().icon), color))
}

/// How a button lays out its icon and label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arrangement {
    /// The icon beside the label; no label where the `GroupLevel` is `Compact`.
    #[default]
    Inline,
    /// The icon over a one- or two-line label.
    Stacked,
}

/// What a face shows.
#[derive(Clone, Copy)]
pub(crate) struct FaceSpec<'a> {
    /// How the icon and label are laid out.
    pub arrangement: Arrangement,
    pub label: &'a str,
    pub role: Option<TypeRole>,
    pub lead: Option<&'a IconSpec<'a>>,
    /// A picture in the leading icon's place, as high as the leading icon and this many times as wide. The face
    /// reserves its box and paints nothing there: the button shows the picture in that box (`lead_rect`).
    pub picture: Option<ThumbAspect>,
    pub trail: Option<&'a IconSpec<'a>>,
    /// A spinner takes the place of the leading icon, or of the label when there is no leading icon: the face keeps
    /// the slot empty and the button shows a `Spinner` there.
    pub loading: bool,
    /// The character of `label` underlined as its mnemonic.
    pub mnemonic: Option<usize>,
    /// Where the icons and the label sit along the area when it is wider than they are.
    pub align: HAlign,
    /// The command's chord, shown after the label of an inline face and dropped before the label elides.
    pub chord: Option<&'a str>,
    /// A caption under the icon and label of an inline face, with its wrapping policy.
    pub detail: Option<(&'a str, Wrap)>,
}

impl FaceSpec<'_> {
    fn run(&self) -> TextRun<'_> {
        let run = TextRun::mnemonic(self.label, self.mnemonic).elide();
        match self.role {
            Some(role) => run.role(role),
            None => run,
        }
    }

    /// Whether the face has a leading part: an icon or a picture.
    fn leads(&self) -> bool {
        self.lead.is_some() || self.picture.is_some()
    }

    /// The width of the leading part when it is `side` high: `side` for an icon, the picture's aspect of it for a
    /// picture, 0 without either.
    fn lead_width(&self, side: f32) -> f32 {
        match (self.picture, self.lead) {
            (Some(aspect), _) => aspect.of(side),
            (None, Some(_)) => side,
            (None, None) => 0.0,
        }
    }

    /// The width the leading part of `lead_side` and the trailing icon of `side` take together.
    fn icons_width(&self, lead_side: f32, side: f32) -> f32 {
        self.lead_width(lead_side) + if self.trail.is_some() { side } else { 0.0 }
    }

    /// The gaps between the parts that are shown.
    fn gaps(&self) -> usize {
        (usize::from(self.leads()) + usize::from(self.trail.is_some()) + usize::from(!self.label.is_empty())).saturating_sub(1)
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

    pub(super) fn detail_run(&self) -> Option<TextRun<'_>> {
        self.detail.filter(|(detail, _)| !detail.is_empty()).map(|(detail, wrap)| TextRun::new(detail).role(TypeRole::Caption).wrap(wrap))
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

impl<'a> FaceSpec<'a> {
    /// Whether the face drops its label under `level`: an inline face beside a leading icon in a compact group. The
    /// icon still shows and the label stays the accessible name and the tooltip. A stacked face keeps its label at
    /// every level, and a face without an icon keeps it, since nothing else would show.
    fn drops_label(&self, level: GroupLevel) -> bool {
        self.arrangement == Arrangement::Inline && level == GroupLevel::Compact && self.leads() && !self.label.is_empty()
    }

    /// The spec as the face shows it under `level`, with the label dropped where `drops_label` says.
    fn at(self, level: GroupLevel) -> FaceSpec<'a> {
        if self.drops_label(level) { FaceSpec { label: "", mnemonic: None, ..self } } else { self }
    }

    /// One line of a stacked label: `text` with its character `at` marked.
    fn line(&self, text: &'a str, at: Option<usize>) -> TextRun<'a> {
        let run = TextRun::mnemonic(text, at).elide();
        match self.role {
            Some(role) => run.role(role),
            None => run,
        }
    }

    /// The lines of a stacked label: broken where the label breaks its line itself (`"Page\nNumber"`, as the ribbon's
    /// large buttons name the break), else whole, or broken at its middle space when it has more than
    /// `ONE_LINE_CHARS` characters. The mnemonic stays on the character it marks.
    fn lines(&self) -> Vec<TextRun<'a>> {
        if self.label.is_empty() {
            return Vec::new();
        }
        let label = self.label;
        let whole = self.line(label, self.mnemonic);
        let split = match label.split_once('\n') {
            Some(lines) => Some(lines),
            None => middle_space(label).filter(|_| label.chars().count() > ONE_LINE_CHARS).and_then(|at| label.split_at_checked(at)),
        };
        let Some((first, rest)) = split else { return vec![whole] };
        let (first, second) = (first.trim_end(), rest.trim_start());
        // A label that ends at its break keeps one line.
        if second.is_empty() {
            return vec![self.line(first, self.mnemonic.filter(|at| *at < first.chars().count()))];
        }
        // The characters before the second line: the first line and the spaces between.
        let skipped = label.chars().count() - second.chars().count();
        let (on_first, on_second) = match self.mnemonic {
            Some(at) if at < first.chars().count() => (Some(at), None),
            Some(at) if at >= skipped => (None, Some(at - skipped)),
            _ => (None, None),
        };
        vec![self.line(first, on_first), self.line(second, on_second)]
    }
}

/// The byte offset of the space nearest the middle of `label` by characters, where a two-line label breaks.
fn middle_space(label: &str) -> Option<usize> {
    let middle = label.chars().count() / 2;
    label.char_indices().enumerate().filter(|(_, (_, c))| *c == ' ').min_by_key(|(n, _)| n.abs_diff(middle)).map(|(_, (at, _))| at)
}

/// The width line `last` or not of a stacked face has in `room`: the last line leaves the trailing icon (a menu
/// button's chevron) its gap and `side`, so the chevron follows it.
fn line_room(spec: &FaceSpec<'_>, last: bool, room: f32, side: f32, gap: f32) -> f32 {
    if last && spec.trail.is_some() { (room - side - gap).max(0.0) } else { room.max(0.0) }
}

/// Where a stacked face's parts sit.
struct Stack {
    /// The content size: the widest row, and the rows' heights with the gap under the icon.
    size: Vec2,
    lead: Option<Rect>,
    /// Each line's text rect.
    lines: Vec<Rect>,
    trail: Option<Rect>,
}

/// Lays a stacked face out in `area` from its lines' sizes: the leading icon of `lead_side` (a picture that high)
/// centred at the top, a gap,
/// then each line centred under it, the trailing icon of `side` after the last line at its reading end (on a row of
/// its own when there is no line). The block is centred vertically, or sits at the area's top when `top`
/// (`FaceLook::stacked_top`); `size` is the block's.
fn stack(spec: &FaceSpec<'_>, sides: (f32, f32), gap: f32, lines: &[Vec2], area: Rect, rtl: bool, top: bool) -> Stack {
    let (lead_side, side) = sides;
    let trail = spec.trail.is_some();
    let last = lines.len().checked_sub(1);
    let mut rows: Vec<Vec2> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| if trail && Some(i) == last { Vec2::new(line.x + gap + side, line.y.max(side)) } else { *line })
        .collect();
    if lines.is_empty() && trail {
        rows.push(Vec2::splat(side));
    }
    let icon = if spec.leads() { lead_side } else { 0.0 };
    let lead_width = spec.lead_width(lead_side);
    let under_icon = if spec.leads() && !rows.is_empty() { gap } else { 0.0 };
    let width = rows.iter().map(|row| row.x).fold(lead_width, f32::max);
    let size = Vec2::new(width, icon + under_icon + rows.iter().map(|row| row.y).sum::<f32>());
    let middle = area.center().x;
    let mut y = if top { area.top() } else { area.top() + (area.height() - size.y).max(0.0) / 2.0 };
    let lead = spec.leads().then(|| Rect::from_min_size(pos2(middle - lead_width / 2.0, y), Vec2::new(lead_width, lead_side)));
    y += icon + under_icon;
    let mut placed = Stack { size, lead, lines: Vec::new(), trail: None };
    for (i, row) in rows.iter().enumerate() {
        let band = Rect::from_min_size(pos2(middle - row.x / 2.0, y), *row);
        match lines.get(i) {
            Some(line) => {
                let (text, rest) = Flow::new(band, rtl).take_start(line.x);
                placed.lines.push(Rect::from_min_size(pos2(text.min.x, y + (row.y - line.y) / 2.0), *line));
                if trail && Some(i) == last {
                    let (icon, _) = rest.take_start(gap).1.take_start(side);
                    placed.trail = Some(Rect::from_center_size(icon.center(), Vec2::splat(side)));
                }
            }
            None => placed.trail = Some(band),
        }
        y += row.y;
    }
    placed
}

/// The sizes of a stacked face's `lines` as `measure` takes them: at their readable least for `Proposal::Min`, else
/// laid out in `room` (`line_room`).
fn stacked_sizes<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>, lines: &[TextRun<'_>], room: f32, side: f32) -> Vec<Vec2> {
    let count = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(i, run)| {
            if cx.proposal == Proposal::Min {
                cx.least(run).size()
            } else {
                cx.layout_at(run, line_room(spec, i + 1 == count, room, side, cx.gap())).size()
            }
        })
        .collect()
}

/// The metrics of a button with a face: a button high (`StyleCx::button_height`), with the control's horizontal padding
/// at least the corner radius plus the gap, so the label and icons of a capsule clear its round ends.
pub(super) fn metrics(cx: &StyleCx<'_>, parts: &VariantParts) -> Metrics {
    let (control, height) = (cx.control(), cx.button_height());
    let round = f32::from(parts.radius.nw).min(height / 2.0);
    // The sides clear the corner round, so an edge never crosses the label. A ribbon's flat button has no edge at rest
    // (`ControlPlace::Ribbon`): its sides take the control's padding, as a ribbon's controls sit close.
    let side = if super::flat_in_ribbon(cx) { control.pad_x } else { control.pad_x.max(round + control.gap) };
    // A ribbon look's row height (`RibbonLook::row`) stands in for the button's.
    Metrics::new().padding(Insets::horizontal(side)).min_size(Vec2::new(0.0, super::row_height(cx, height))).gap(control.gap)
}

/// The metrics of a stacked face: `metrics`, with the control's gap above and below the stack, so its height is the
/// icon, the label lines and that padding, and never less than the control's height.
pub(super) fn stacked_metrics(cx: &StyleCx<'_>, parts: &VariantParts) -> Metrics {
    let gap = cx.control().gap;
    let mut metrics = metrics(cx, parts);
    metrics.padding.top = gap;
    metrics.padding.bottom = gap;
    metrics
}

/// `metrics` or `stacked_metrics`, by `arrangement`.
pub(super) fn metrics_of(cx: &StyleCx<'_>, parts: &VariantParts, arrangement: Arrangement) -> Metrics {
    match arrangement {
        Arrangement::Stacked => stacked_metrics(cx, parts),
        _ => metrics(cx, parts),
    }
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
pub(crate) struct IconFace {
    pub surface: SurfaceLook,
    pub metrics: Metrics,
    /// The glyph at the side of the scope's glyph role, in the on look's text colour while on.
    pub glyph: IconLook,
}

/// The icon-only look of the instance's variant: a square of the control height with control corners, as a ghost
/// button of the old look (a capsule would round the on plate), showing the on look while on. The scope's glyph role
/// sets the glyph's side and, in a rail, the quieter ink it rests in.
pub(crate) fn icon_face(cx: &StyleCx<'_>) -> IconFace {
    let on = match cx.env::<ActiveLook>() {
        ActiveLook::Accent => on_parts(cx),
        ActiveLook::Pressed => filled_parts(cx, cx.tokens.color.state.pressed),
        ActiveLook::Control => filled_parts(cx, cx.tokens.color.surface.control),
    };
    let off = cx.variant_look();
    let (icon, text) = (&cx.tokens.size.icon, &cx.tokens.color.text);
    let (side, rest) = match cx.glyph_role() {
        GlyphRole::Tool => (icon.md, None),
        GlyphRole::ToolRail => (icon.lg, Some(text.muted)),
        GlyphRole::PanelRail => (cx.glyph(), Some(text.faint)),
        GlyphRole::Compact => (icon.glyph_sm, None),
        _ => (cx.glyph(), None),
    };
    let side = if cx.glyph_role() == GlyphRole::Compact { side } else { super::small_icon(cx, side) };
    let mut ink = off.text.clone();
    if let Some(rest) = rest {
        ink.off.rest = cx.color(rest);
    }
    IconFace {
        surface: on_surface(cx, &off, &on).radius(UiTokens::corner(cx.tokens.shape.radius.control)),
        metrics: Metrics::new().min_size(Vec2::splat(super::row_height(cx, cx.control().height))),
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

/// The on look under `ActiveLook::Pressed` and `ActiveLook::Control`: `fill` with no edge, the normal text colour.
fn filled_parts(cx: &StyleCx<'_>, fill: craft_theme::Rgba) -> VariantParts {
    let soft = on_parts(cx);
    let (fill, ink) = (cx.color(fill), cx.color(cx.tokens.color.text.normal));
    VariantParts { fill: Stateful::new(fill), text: Stateful::new(ink).off(States::new(ink, ink, ink, soft.text.off.disabled)), edge: None, ..soft }
}

/// The focus ring of a button.
pub(crate) fn focus(cx: &StyleCx<'_>) -> FocusLook {
    cx.focus_look()
}

/// The side of a non-stacked icon as a measure takes it: the side the style paints (`small_icon`), the ribbon's small
/// side where one is set, else the control's icon.
pub(crate) fn measured_icon<C: Component>(cx: &MeasureCx<'_, C>) -> f32 {
    let small = cx.env.get::<RibbonLook>().small;
    if small > 0 && cx.env.get::<ControlPlace>() == ControlPlace::Ribbon { f32::from(small) } else { cx.icon() }
}

/// The size of the face: its parts and the gaps between them, the detail line under them.
/// The chord is dropped before the label elides; the label then elides in the room the icons and gaps leave, so a
/// squeezed face is as wide as its room; at its least the label is at its readable least and the chord is dropped.
/// The group level comes from the env (`GroupLevel` is a style value, so the measurement is keyed on it): a compact
/// group's inline face measures without its label.
pub(crate) fn measure<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>) -> Vec2 {
    let spec = &spec.at(cx.env.get::<GroupLevel>());
    if spec.arrangement == Arrangement::Stacked {
        let side = arrow_room(cx.env.get::<ControlPlace>(), cx.env.get::<RibbonLook>()).unwrap_or_else(|| measured_icon(cx));
        let lead_side = stacked_side(cx.tokens, cx.icon(), cx.env.get::<ControlPlace>(), cx.env.get::<RibbonLook>());
        let lines = spec.lines();
        let sizes = stacked_sizes(cx, spec, &lines, cx.room.unwrap_or(f32::INFINITY), side);
        let size = stack(spec, (lead_side, side), cx.gap(), &sizes, Rect::ZERO, false, false).size;
        // Room for the lines the place keeps (`stacked_lines`) under the icon, whatever the label's own count.
        let keep = stacked_lines(cx.env.get::<ControlPlace>());
        if keep <= lines.len() {
            return size;
        }
        let line = cx.layout_at(&spec.line("Ag", None), f32::INFINITY).size().y;
        let icon = if spec.leads() { lead_side + cx.gap() } else { 0.0 };
        return Vec2::new(size.x, size.y.max(icon + keep as f32 * line.max(if spec.trail.is_some() { side } else { 0.0 })));
    }
    let (icon, gap) = (measured_icon(cx), cx.gap());
    let trail = arrow_room(cx.env.get::<ControlPlace>(), cx.env.get::<RibbonLook>()).unwrap_or(icon);
    let fixed = spec.icons_width(icon, trail) + gap * spec.gaps() as f32;
    let detail = spec.detail_run();
    let wraps = detail.as_ref().is_some_and(|run| run.wrap == Wrap::Wrap);
    let room = cx.room.or_else(|| {
        if !wraps {
            return None;
        }
        let width = match cx.proposal {
            Proposal::Width(width) => width,
            Proposal::Size(size) => size.x,
            _ => return None,
        };
        let pad = cx.style.metrics().padding;
        width.is_finite().then(|| (width - pad.start - pad.end).max(0.0))
    });
    let room = room.unwrap_or(f32::INFINITY);
    let (text, chord) = if spec.label.is_empty() {
        (Vec2::ZERO, None)
    } else if cx.proposal == Proposal::Min {
        (cx.least(&spec.run()).size(), None)
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
    let Some(detail) = detail else { return row };
    let detail = if cx.proposal == Proposal::Min { cx.least(&detail).size() } else { cx.layout_at(&detail, room).size() };
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

/// Places the leading icon (or picture) of `side` and the trailing icon of `trail`, a label `text_width` wide and the
/// chord of `chord` width after the label, with `gap` between them, in `area` by `spec.align`: the leading part at the
/// reading start, the trailing icon at the reading end.
fn slots(spec: &FaceSpec<'_>, (side, trail): (f32, f32), gap: f32, text_width: f32, chord: Option<f32>, area: Rect, rtl: bool) -> Slots {
    let width = spec.icons_width(side, trail) + gap * spec.gaps() as f32 + text_width + chord.map_or(0.0, |width| width + gap);
    let lead = lead_offset(spec.align, (area.width() - width).max(0.0));
    let (_, mut rest) = Flow::new(area, rtl).take_start(lead);
    let mut take = |extent: f32| {
        let (slot, next) = rest.take_start(extent);
        rest = next;
        slot
    };
    let mut slots = Slots { lead: None, text: None, chord: None, trail: None };
    let mut gapped = false;
    if spec.leads() {
        slots.lead = Some(take(spec.lead_width(side)));
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
        slots.trail = Some(take(trail));
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
/// them as one block. A face that drops its label (`FaceSpec::drops_label`) says so in the paint record, so the
/// tooltip shows the label.
pub(crate) fn paint<C: Component>(ui: &PaintUi<'_>, cx: &Cx<'_, C>, ink: (&TextLook, &IconLook), look: &FaceLook, spec: &FaceSpec<'_>, area: Rect) {
    let level = cx.env.get::<GroupLevel>();
    if spec.drops_label(level) {
        ui.draw().drop_label();
    }
    let spec = &spec.at(level);
    if spec.arrangement == Arrangement::Stacked {
        paint_stacked(ui, cx, ink, look, spec, area);
        return;
    }
    let (text_look, icon_look) = ink;
    let (text_color, icon_color, side) = (cx.resolve(&text_look.color), cx.resolve(&icon_look.color), icon_look.side);
    let draw = ui.draw();
    let trail_side = look.arrow.unwrap_or(side);
    let fixed = spec.icons_width(side, trail_side) + look.gap * spec.gaps() as f32;
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
    let height = |layout: &Option<TextLayout>| layout.as_ref().map_or(0.0, |layout| layout.size().y);
    let row = row_rect(area, height(&text).max(height(&chord)).max(side), detail.as_ref().map(|detail| detail.size().y), look.gap);
    let rtl = cx.flow.is_rtl();
    let text_width = text.as_ref().map_or(0.0, |text| text.size().x);
    let slots = slots(spec, (side, trail_side), look.gap, text_width, chord.as_ref().map(|chord| chord.size().x), row, rtl);
    if let (Some(lead), Some(slot)) = (spec.lead, slots.lead)
        && !spec.loading
    {
        draw.icon(icon_at(slot, row, side), &tinted(lead, icon_color));
    }
    if let (Some(text), Some(slot)) = (&text, slots.text)
        && !(spec.loading && !spec.leads())
    {
        draw.galley(pos2(slot.min.x, text_top(&draw, row, text)), text);
    }
    if let (Some(chord), Some(slot)) = (&chord, slots.chord) {
        draw.galley(pos2(slot.min.x, text_top(&draw, row, chord)), chord);
    }
    if let (Some(trail), Some(slot)) = (spec.trail, slots.trail) {
        draw.icon(icon_at(slot, row, trail_side), &tinted(trail, icon_color));
    }
    if let Some(detail) = &detail {
        let width = detail.size().x;
        let (_, rest) = Flow::new(area, rtl).take_start(lead_offset(spec.align, (area.width() - width).max(0.0)));
        let (slot, _) = rest.take_start(width);
        draw.galley(pos2(slot.min.x, row.max.y + look.gap), detail);
    }
}

/// Paints a stacked face in `area` (`stack`): the leading icon over the label's lines, the trailing icon after the
/// last line. A loading face leaves the leading icon's slot (the label's, without an icon) to the spinner.
fn paint_stacked<C: Component>(ui: &PaintUi<'_>, cx: &Cx<'_, C>, ink: (&TextLook, &IconLook), look: &FaceLook, spec: &FaceSpec<'_>, area: Rect) {
    let (text_look, icon_look) = ink;
    let (text_color, icon_color) = (cx.resolve(&text_look.color), cx.resolve(&icon_look.color));
    let side = look.arrow.unwrap_or(icon_look.side);
    let draw = ui.draw();
    let lines = spec.lines();
    let colored: Vec<TextRun<'_>> = lines.into_iter().map(|run| run.color(text_color)).collect();
    let count = colored.len();
    let layouts: Vec<TextLayout> =
        colored.iter().enumerate().map(|(i, run)| draw.layout(run, line_room(spec, i + 1 == count, area.width(), side, look.gap))).collect();
    let sizes: Vec<Vec2> = layouts.iter().map(TextLayout::size).collect();
    let placed = stack(spec, (look.stacked_icon, side), look.gap, &sizes, area, cx.flow.is_rtl(), look.stacked_top);
    if let (Some(lead), Some(slot)) = (spec.lead, placed.lead)
        && !spec.loading
    {
        draw.icon(slot, &tinted(lead, icon_color));
    }
    if !(spec.loading && !spec.leads()) {
        for (text, slot) in layouts.iter().zip(&placed.lines) {
            draw.galley(slot.min, text);
        }
    }
    if let (Some(trail), Some(slot)) = (spec.trail, placed.trail) {
        draw.icon(slot, &tinted(trail, icon_color));
    }
}

/// A stacked face placed in `area` as `paint_stacked` places it, its lines laid out for `area`'s width.
fn stacked_in<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>, look: &FaceLook, side: f32, area: Rect, rtl: bool) -> Stack {
    let lines = spec.lines();
    let count = lines.len();
    let sizes: Vec<Vec2> =
        lines.iter().enumerate().map(|(i, run)| cx.layout_at(run, line_room(spec, i + 1 == count, area.width(), side, look.gap)).size()).collect();
    stack(spec, (look.stacked_icon, side), look.gap, &sizes, area, rtl, look.stacked_top)
}

/// Where a stacked split button's parts meet in `area`: halfway down the gap under the face's leading icon, so the
/// icon is the main part and the label and its chevron are the arrow part. Without an icon the parts meet at the
/// content's top: the whole face is the arrow part.
pub(super) fn stacked_divide<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>, look: &FaceLook, side: f32, area: Rect) -> f32 {
    let spec = &spec.at(cx.env.get::<GroupLevel>());
    match stacked_in(cx, spec, look, side, area, false).lead {
        Some(lead) => lead.bottom() + look.divide_gap,
        None => area.top(),
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
    let spec = &spec.at(cx.env.get::<GroupLevel>());
    if spec.arrangement == Arrangement::Stacked {
        let placed = stacked_in(cx, spec, look, side, area, rtl);
        return match placed.lead {
            Some(lead) => Some(Rect::from_center_size(lead.center(), Vec2::splat(lead.height()))),
            None => placed.lines.first().map(|line| Rect::from_center_size(line.center(), Vec2::splat(side))),
        };
    }
    let (slots, row) = inline_slots(cx, spec, look, side, area, rtl);
    let slot = if spec.leads() { slots.lead } else { slots.text }?;
    Some(icon_at(slot, row, side))
}

/// The slots of an inline face in `area` as `paint` places them, and the row of icons, label and chord they sit on:
/// the chord beside the whole label when it fits, else the label laid out in the room the icons and gaps leave.
fn inline_slots<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>, look: &FaceLook, side: f32, area: Rect, rtl: bool) -> (Slots, Rect) {
    let trail = look.arrow.unwrap_or(side);
    let fixed = spec.icons_width(side, trail) + look.gap * spec.gaps() as f32;
    let beside = spec.chord_run().and_then(|chord| {
        let label = cx.layout_at(&spec.run(), f32::INFINITY).size();
        let chord = cx.layout_at(&chord, f32::INFINITY).size();
        chord_fits(label.x, chord.x, fixed, look.gap, area.width()).then_some((label, chord))
    });
    let (text, chord) = match beside {
        Some((label, chord)) => (label, Some(chord)),
        None if spec.label.is_empty() => (Vec2::ZERO, None),
        None => (cx.layout_at(&spec.run(), (area.width() - fixed).max(0.0)).size(), None),
    };
    let detail = spec.detail_run().map(|detail| cx.layout_at(&detail, area.width()).size().y);
    let row = row_rect(area, text.y.max(chord.map_or(0.0, |chord| chord.y)).max(side), detail, look.gap);
    (slots(spec, (side, trail), look.gap, text.x, chord.map(|chord| chord.x), row, rtl), row)
}

/// The box of the face's leading part in `area`, as `paint` places it: the leading icon's square, or the picture's
/// box, as high as the leading icon and its aspect times as wide. `None` when the face has neither.
pub(super) fn lead_rect<C: Component>(cx: &MeasureCx<'_, C>, spec: &FaceSpec<'_>, look: &FaceLook, side: f32, area: Rect, rtl: bool) -> Option<Rect> {
    let spec = &spec.at(cx.env.get::<GroupLevel>());
    if spec.arrangement == Arrangement::Stacked {
        return stacked_in(cx, spec, look, look.arrow.unwrap_or(side), area, rtl).lead;
    }
    let (slots, row) = inline_slots(cx, spec, look, side, area, rtl);
    let slot = slots.lead?;
    Some(Rect::from_center_size(pos2(slot.center().x, row.center().y), Vec2::new(slot.width(), side)))
}
