//! [`ColorRamp`]: the bar under a colour panel's slider rows, with white and black boxes at its end.

use super::channels::primary;
use craft_behavior::{Axis, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{Hsb, Srgba};
use craft_ui::canvas::context_click;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Mesh, Pos2, Rect, Response, Vec2, pos2};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

#[cfg(feature = "screen-testing")]
mod stories;

/// The bar: a press picks the colour under the pointer.
const BAR: PartKey = PartKey::new("bar");
/// The white box at the end.
const WHITE: PartKey = PartKey::new("white");
/// The black box at the end.
const BLACK: PartKey = PartKey::new("black");

/// Columns of the ramp grid.
const CELLS_X: usize = 48;
/// Rows of the ramp grid.
const CELLS_Y: usize = 8;
/// Cells a Shift arrow moves.
const FAR: i64 = 10;

craft_ui::words! {
    /// The words of colour ramps.
    pub struct ColorRampWords {
        white: "colorRamp.white" = "White";
        black: "colorRamp.black" = "Black";
    }
}

/// The style of a colour ramp.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorRampStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The edge round the bar and the boxes.
    edge: StyleColor,
    /// The side of an end box.
    boxes: f32,
    /// The gap between the bar and the boxes.
    boxes_gap: f32,
    /// The diameter of the cursor cell's marker.
    marker: f32,
}

/// Which colours the ramp shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RampKind {
    /// Hue across; white at the top through the full colour in the middle to black at the bottom.
    #[default]
    Spectrum,
    /// The [`Spectrum`](RampKind::Spectrum) in the host's CMYK gamut: each colour goes through
    /// [`gamut`](ColorRamp::gamut) (a CMYK round trip). Without a `gamut` it is the spectrum.
    CmykSpectrum,
    /// White at the left to black at the right.
    Grayscale,
    /// From one colour at the left to another at the right, in encoded sRGB ([`ends`](ColorRamp::ends)).
    Between,
}

/// A colour the ramp picked.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RampPick {
    /// The picked colour.
    pub color: Srgba,
    /// Alt was held at the press (or at the click or box press): the host sets the colour it is not editing.
    pub other: bool,
}

impl RampPick {
    /// A pick of `color`, with `other` when Alt was held.
    pub const fn new(color: Srgba, other: bool) -> Self {
        RampPick { color, other }
    }
}

/// Maps a spectrum colour to the host's gamut: `ColorRamp::gamut`.
pub struct Gamut<'a>(Box<dyn Fn(Srgba) -> Srgba + 'a>);

impl<'a> Gamut<'a> {
    /// The gamut that runs `map`.
    pub fn new(map: impl Fn(Srgba) -> Srgba + 'a) -> Self {
        Gamut(Box::new(map))
    }
}

/// What [`ColorRamp`] returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorRampOutput {
    /// The pick, through the edit lifecycle. The ramp has no value of the host's to compare with, so every pick is a
    /// change.
    pub edit: EditResponse<RampPick>,
    /// A click with Shift: the host moves to its next ramp. Nothing was picked.
    pub cycle: bool,
    /// A secondary click that began and ended on the bar: where the host opens its menu.
    pub menu_at: Option<Pos2>,
}

impl Outcome for ColorRampOutput {
    type Value = RampPick;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// What a press began with, kept for its gesture.
#[derive(Clone, Copy, Default)]
struct Held {
    shift: bool,
    alt: bool,
    /// The ends of a Between ramp at the press.
    ends: Option<[Srgba; 2]>,
}

/// The cursor cell: column and row.
#[derive(Clone, Copy, Default)]
struct Cursor(Option<(usize, usize)>);

/// The colours of the mesh, with what they were computed for.
#[derive(Clone, Default)]
struct Cache {
    signature: u64,
    colors: Arc<Vec<Color32>>,
}

/// The bar under the Color panel's slider rows (Photoshop's colour ramp): a spectrum, a grey ramp or a ramp between
/// two colours, with a white and a black box at its end.
///
/// The bar is 48 by 8 cells, shaded between the vertices of a 49 by 9 grid; a [`web_safe`](ColorRamp::web_safe) ramp
/// is flat cells coloured at their centres. Hue is a function of x only, so a pick carries hue and saturation. The
/// colours are computed when the kind, ends, web flag or [`gamut_revision`](ColorRamp::gamut_revision) change, and
/// kept.
///
/// - Pointer: a click picks the colour under the pointer (the cell's centre on a web-safe ramp); a drag picks while it
///   moves, also outside the bar, where the pick stays at the edge, and commits at the release. A
///   [`Between`](RampKind::Between) ramp keeps the ends it had at the press for the whole gesture. Alt at the press
///   sets [`other`](RampPick::other) on every event. A click with Shift starts no gesture and sets
///   [`cycle`](ColorRampOutput::cycle). A secondary click on the bar is reported as
///   [`menu_at`](ColorRampOutput::menu_at).
/// - Boxes: a click on the white or the black box (also by keyboard) is a `Set` of that colour; they are Tab stops of
///   their own.
/// - Keys, with the bar focused: it has a cursor cell, starting at the left end of the middle row. Arrows move it one
///   cell (Shift ten), Home and End to the row's ends, Page Up and Page Down to the top and bottom rows. Enter or Space picks the cursor cell. A press moves the cursor to its cell.
/// - AccessKit: the bar is a `Group` named `label` whose value is [`kind_name`](ColorRamp::kind_name); the boxes are
///   `Button`s.
/// - Nothing mirrors right to left; the boxes stay at the right.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorRamp<'a> {
    #[common]
    common: Common,
    kind: RampKind,
    pick: Bind<'a, RampPick>,
    /// The colours at the left and the right of a [`Between`](RampKind::Between) ramp; black and white without them.
    #[prop]
    ends: Option<[Srgba; 2]>,
    /// Maps each colour of a spectrum ramp to the host's gamut (a CMYK round trip). It runs for the cached colours
    /// only, and again for a pick.
    #[prop(func = "Fn(Srgba) -> Srgba")]
    gamut: Option<Gamut<'a>>,
    /// The host's counter for the working space `gamut` maps into; a change recomputes the colours.
    #[prop]
    gamut_revision: u64,
    /// Whether every channel is rounded to the nearest of 0, 0.2, ... 1 and the ramp is flat cells.
    #[prop]
    web_safe: bool,
    /// Whether the white and black boxes follow the bar; on by default.
    #[prop]
    boxes: bool,
    /// The ramp's kind as the host names it ("RGB Spectrum"), the bar's accessible value.
    #[prop(into)]
    kind_name: Option<Label>,
    colors: Arc<Vec<Color32>>,
    shown_ends: [Srgba; 2],
    cursor: (usize, usize),
    cycle: bool,
    menu_at: Option<Pos2>,
}

impl<'a> ColorRamp<'a> {
    /// A ramp named `label` ("Color ramp") of `kind`, reporting its picks through `pick`.
    pub fn new(label: impl Into<Label>, kind: RampKind, pick: Bind<'a, RampPick>) -> Self {
        let ends = Self::default_ends();
        ColorRamp {
            common: Common::new(),
            kind,
            pick,
            ends: None,
            gamut: None,
            gamut_revision: 0,
            web_safe: false,
            boxes: true,
            kind_name: None,
            colors: Arc::default(),
            shown_ends: ends,
            cursor: (0, CELLS_Y / 2),
            cycle: false,
            menu_at: None,
        }
        .label(label)
        .ltr_axis()
    }

    fn default_ends() -> [Srgba; 2] {
        [Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)]
    }

    /// The colour at (`x`, `y`) as fractions of the bar, `y` down. A non-finite or outside fraction is clamped into
    /// the bar. Spectrum: hue `x` (below 360), white at the top through the full colour at the middle to black at the
    /// bottom, through `gamut` when given; grayscale: white to black; between: `ends` from the left to the right.
    /// `web_safe` rounds each channel.
    pub fn color_at(kind: RampKind, web_safe: bool, gamut: Option<&dyn Fn(Srgba) -> Srgba>, ends: [Srgba; 2], x: f32, y: f32) -> Srgba {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let (x, y) = (unit(x), unit(y));
        let color = match kind {
            RampKind::Spectrum | RampKind::CmykSpectrum => {
                let (s, b) = if y < 0.5 { (y * 2.0, 1.0) } else { (1.0, 1.0 - (y - 0.5) * 2.0) };
                let color = Srgba::from_hsb(Hsb { h: x.min(0.9999) * 360.0, s, b });
                gamut.map_or(color, |g| g(color))
            }
            RampKind::Grayscale => Srgba::new(1.0 - x, 1.0 - x, 1.0 - x, 1.0),
            RampKind::Between => {
                let [a, b] = ends;
                let mix = |from: f32, to: f32| from + (to - from) * x;
                Srgba::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
            }
        };
        let web = |v: f32| if web_safe { (unit(v) * 5.0).round() / 5.0 } else { unit(v) };
        Srgba::new(web(color.r), web(color.g), web(color.b), unit(color.a))
    }

    fn gamut_fn(&self) -> Option<&dyn Fn(Srgba) -> Srgba> {
        self.gamut.as_ref().map(|gamut| &*gamut.0)
    }

    /// The pick at the fractions (`fx`, `fy`) of the bar.
    fn pick_at(&self, ends: [Srgba; 2], fx: f32, fy: f32, other: bool) -> RampPick {
        let (fx, fy) = if self.web_safe { cell_centre(fx, fy) } else { (fx, fy) };
        RampPick::new(Self::color_at(self.kind, self.web_safe, self.gamut_fn(), ends, fx, fy), other)
    }

    /// The bar's rect and the boxes' in the room `rect`.
    fn layout(&self, rect: Rect, style: &ColorRampStyle) -> (Rect, Rect) {
        let boxes_width = if self.boxes { style.boxes.min(rect.width() / 2.0) } else { 0.0 };
        let gap = if self.boxes { style.boxes_gap } else { 0.0 };
        let bar = Rect::from_min_max(rect.min, pos2((rect.right() - boxes_width - gap).max(rect.left()), rect.bottom()));
        (bar, Rect::from_min_max(pos2(rect.right() - boxes_width, rect.top()), rect.max))
    }

    /// The colours of the mesh for `ends`: 49 by 9 vertices, or 48 by 8 cell centres for a web-safe ramp.
    fn colors(&self, cache: &mut Cache, ends: [Srgba; 2]) -> Arc<Vec<Color32>> {
        let mut hasher = DefaultHasher::new();
        (self.kind, self.web_safe).hash(&mut hasher);
        if self.kind == RampKind::Between {
            ends.iter().for_each(|c| [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(&mut hasher));
        }
        if matches!(self.kind, RampKind::Spectrum | RampKind::CmykSpectrum) {
            (self.gamut.is_some(), self.gamut_revision).hash(&mut hasher);
        }
        let signature = hasher.finish();
        if cache.signature == signature && !cache.colors.is_empty() {
            return cache.colors.clone();
        }
        let (cols, rows, half) = if self.web_safe { (CELLS_X, CELLS_Y, 0.5) } else { (CELLS_X + 1, CELLS_Y + 1, 0.0) };
        let mut colors = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let (x, y) = ((col as f32 + half) / CELLS_X as f32, (row as f32 + half) / CELLS_Y as f32);
                colors.push(Color32::from(Self::color_at(self.kind, self.web_safe, self.gamut_fn(), ends, x, y)));
            }
        }
        *cache = Cache { signature, colors: Arc::new(colors) };
        cache.colors.clone()
    }
}

/// `pos` as fractions of `bar`, kept inside it.
fn fractions(bar: Rect, pos: Pos2) -> (f32, f32) {
    let along = |p: f32, from: f32, length: f32| if length > 0.0 && p.is_finite() { ((p - from) / length).clamp(0.0, 1.0) } else { 0.0 };
    (along(pos.x, bar.left(), bar.width()), along(pos.y, bar.top(), bar.height()))
}

/// The cell of the grid under the fractions (`fx`, `fy`).
fn cell_of(fx: f32, fy: f32) -> (usize, usize) {
    (((fx * CELLS_X as f32) as usize).min(CELLS_X - 1), ((fy * CELLS_Y as f32) as usize).min(CELLS_Y - 1))
}

/// The centre of the cell holding the fractions (`fx`, `fy`).
fn cell_centre(fx: f32, fy: f32) -> (f32, f32) {
    let (col, row) = cell_of(fx, fy);
    centre_of((col, row))
}

/// The centre of `cell` as fractions of the bar.
fn centre_of(cell: (usize, usize)) -> (f32, f32) {
    ((cell.0 as f32 + 0.5) / CELLS_X as f32, (cell.1 as f32 + 0.5) / CELLS_Y as f32)
}

/// The cursor after the key `mv`: arrows one cell (ten with `far`), Home and End the row's ends, Page Up and Page Down
/// the top and bottom rows.
fn moved(cursor: (usize, usize), mv: NavMove, far: bool) -> (usize, usize) {
    let by = if far { FAR } else { 1 };
    let along = |at: usize, delta: i64, n: usize| (at as i64 + delta).clamp(0, n as i64 - 1) as usize;
    match mv {
        NavMove::Forward => (along(cursor.0, by, CELLS_X), cursor.1),
        NavMove::Back => (along(cursor.0, -by, CELLS_X), cursor.1),
        NavMove::Down => (cursor.0, along(cursor.1, by, CELLS_Y)),
        NavMove::Up => (cursor.0, along(cursor.1, -by, CELLS_Y)),
        NavMove::Start => (0, cursor.1),
        NavMove::End => (CELLS_X - 1, cursor.1),
        NavMove::PageUp => (cursor.0, 0),
        NavMove::PageDown => (cursor.0, CELLS_Y - 1),
        _ => cursor,
    }
}

impl Component for ColorRamp<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows);
    type Style = ColorRampStyle;
    type Output = ColorRampOutput;

    fn style(cx: &StyleCx<'_>, _: ColorRampStyleKey) -> ColorRampStyle {
        let tokens = cx.tokens;
        ColorRampStyle {
            metrics: Metrics::new().min_size(Vec2::new(tokens.size.length.track, tokens.size.thumb.lg)),
            focus: cx.focus_look(),
            edge: cx.color(tokens.color.line.control_edge),
            boxes: tokens.size.icon.lg,
            boxes_gap: tokens.space.gap.tight,
            marker: tokens.size.handle.md,
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (bar, boxes) = self.layout(rect, cx.style);
        cx.part(BAR, bar).sense(Sense::DRAG | Sense::CLICK).focusable().value_part();
        if self.boxes {
            let half = boxes.center().y;
            cx.part(WHITE, Rect::from_min_max(boxes.min, pos2(boxes.right(), half))).sense(Sense::CLICK).focusable();
            cx.part(BLACK, Rect::from_min_max(pos2(boxes.left(), half), boxes.max)).sense(Sense::CLICK).focusable();
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, RampPick>> {
        Some(self.pick.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (bar, _) = self.layout(cx.rect, cx.style);
        let configured = self.ends.unwrap_or_else(Self::default_ends);
        let press = cx.part(BAR).press;
        if primary(&press) && matches!(press.phase, PressPhase::Down { .. }) {
            *cx.memory::<Held>() = Held { shift: press.mods.shift, alt: press.mods.alt, ends: Some(configured) };
        }
        let held = *cx.memory::<Held>();
        let running = cx.gesture().is_some();
        let ends = if running || press.phase.is_press() { held.ends.unwrap_or(configured) } else { configured };
        self.shown_ends = ends;
        let mut cursor = cx.memory::<Cursor>().0.unwrap_or((0, CELLS_Y / 2));
        if primary(&press) {
            let at = |pos: Pos2| fractions(bar, pos);
            match press.phase {
                PressPhase::DragStart { pos, .. } if !held.shift => {
                    let (fx, fy) = at(pos);
                    cursor = cell_of(fx, fy);
                    cx.begin();
                    cx.propose(self.pick_at(ends, fx, fy, held.alt));
                }
                PressPhase::Drag { pos, .. } if !held.shift && cx.gesture().is_some() => {
                    let (fx, fy) = at(pos);
                    cursor = cell_of(fx, fy);
                    cx.propose(self.pick_at(ends, fx, fy, held.alt));
                }
                PressPhase::Release { pos, click, .. } => {
                    let (fx, fy) = at(pos);
                    if cx.gesture().is_some() {
                        cx.propose(self.pick_at(ends, fx, fy, held.alt));
                        cx.commit();
                    } else if click {
                        cursor = cell_of(fx, fy);
                        if held.shift {
                            self.cycle = true;
                        } else {
                            cx.set(self.pick_at(ends, fx, fy, held.alt));
                        }
                    }
                    if !held.shift {
                        cursor = cell_of(fx, fy);
                    }
                }
                _ => {}
            }
        }
        self.menu_at = context_click(&press);
        let bar_active = cx.active_part() == Some(BAR.into());
        while let Some((mv, mods)) = cx.intents().take_move() {
            if bar_active && !running {
                cursor = moved(cursor, mv, mods.extend);
            }
        }
        if !running && matches!(cx.part_activated(BAR), Some(FireSource::Key | FireSource::Access)) {
            let (fx, fy) = centre_of(cursor);
            cx.set(self.pick_at(ends, fx, fy, false));
        }
        for (key, color) in [(WHITE, Srgba::rgb(1.0, 1.0, 1.0)), (BLACK, Srgba::rgb(0.0, 0.0, 0.0))] {
            if cx.part_activated(key).is_some() {
                let other = cx.part(key).press.mods.alt;
                cx.set(RampPick::new(color, other));
            }
        }
        cx.memory::<Cursor>().0 = Some(cursor);
        self.cursor = cursor;
        let mut cache = std::mem::take(cx.memory::<Cache>());
        self.colors = self.colors(&mut cache, ends);
        *cx.memory::<Cache>() = cache;
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ColorRampWords>();
        let bar = access.node(BAR, Role::GROUP);
        bar.name = Name::Text(cx.env.resolve(&cx.label()).as_str().to_owned());
        if let Some(kind) = &self.kind_name {
            bar.value(cx.env.resolve(kind).as_str().to_owned());
        }
        if self.boxes {
            access.node(WHITE, Role::BUTTON).name = Name::Text(words.white.as_str().to_owned());
            access.node(BLACK, Role::BUTTON).name = Name::Text(words.black.as_str().to_owned());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (bar, boxes) = self.layout(cx.rect, cx.style);
        let painter = ui.content_painter(cx);
        painter.painter.add(ramp_mesh(bar, &self.colors, self.web_safe));
        let draw = ui.draw();
        draw.stroke(&PaintShape::Rect(bar, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.edge);
        if cx.focus_visible() && cx.active_part() == Some(BAR.into()) {
            let (fx, fy) = centre_of(self.cursor);
            let at = pos2(bar.left() + bar.width() * fx, bar.top() + bar.height() * fy);
            painter.outline(&PaintShape::Circle(at, cx.style.marker / 2.0));
        }
        if self.boxes {
            let half = boxes.center().y;
            let white = Rect::from_min_max(boxes.min, pos2(boxes.right(), half));
            let black = Rect::from_min_max(pos2(boxes.left(), half), boxes.max);
            draw.data_fill(&PaintShape::Rect(white, CornerRadius::ZERO), DataColor::new(Color32::from(Srgba::WHITE)));
            draw.data_fill(&PaintShape::Rect(black, CornerRadius::ZERO), DataColor::new(Color32::from(Srgba::BLACK)));
            draw.stroke(&PaintShape::Rect(boxes, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.edge);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorRampOutput {
        let (cycle, menu_at) = (self.cycle, self.menu_at);
        ColorRampOutput { edit: cx.edit_response(), cycle, menu_at }
    }
}

/// The ramp mesh over `bar`: shaded between the vertices, or flat cells when web safe.
fn ramp_mesh(bar: Rect, colors: &[Color32], flat: bool) -> Mesh {
    let mut mesh = Mesh::default();
    let at = |i: usize, j: usize| pos2(bar.left() + bar.width() * i as f32 / CELLS_X as f32, bar.top() + bar.height() * j as f32 / CELLS_Y as f32);
    if flat {
        for j in 0..CELLS_Y {
            for i in 0..CELLS_X {
                let color = colors.get(j * CELLS_X + i).copied().unwrap_or(Color32::from(Srgba::BLACK));
                mesh.add_colored_rect(Rect::from_min_max(at(i, j), at(i + 1, j + 1)), color);
            }
        }
        return mesh;
    }
    for j in 0..=CELLS_Y {
        for i in 0..=CELLS_X {
            mesh.colored_vertex(at(i, j), colors.get(j * (CELLS_X + 1) + i).copied().unwrap_or(Color32::from(Srgba::BLACK)));
        }
    }
    let row = (CELLS_X + 1) as u32;
    for j in 0..CELLS_Y as u32 {
        for i in 0..CELLS_X as u32 {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row + 1);
            mesh.add_triangle(a, a + row + 1, a + row);
        }
    }
    mesh
}
