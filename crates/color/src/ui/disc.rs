//! [`ColorDisc`]: a hue and saturation disc with one draggable marker per colour of a set, as VectorCraft's harmony
//! wheel.

use super::channels::{finite, primary};
use super::wheel::{hue_at, hue_direction, vertex};
use craft_behavior::{AccessAction, CancelReason, HitShape, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{ColorModel, Flags, HsbModel, ModelValues, Resolve as _, Srgba, State, StateMix, Stateful};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, Name, Numeric, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx};
use egui::{Color32, Mesh, Pos2, Response, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

/// The disc: a press on it grabs the nearest marker within reach.
const DISC: PartKey = PartKey::new("disc");
/// A marker: `MARKER.at(i)`; the selected one is the disc's Tab stop.
const MARKER: PartKey = PartKey::new("marker");

/// Share of `overlay.handle_fill` the marker spokes draw at, so the disc shows through.
const SPOKE_SHARE: f32 = 0.63;
/// Segments round the disc and rings out from its centre (VectorCraft's 72 and 8).
const SEGMENTS: u32 = 72;
const RINGS: u32 = 8;
/// The least brightness the disc draws, as a proportion.
const DISC_FLOOR: f32 = 0.15;
/// The distance below which the pointer has no hue, in points.
const CENTRE: f32 = 0.5;
/// Degrees a hue key moves, and percents a saturation key moves; Shift moves ten times as far.
const KEY_STEP: f32 = 1.0;
const FAR: f32 = 10.0;

craft_ui::words! {
    /// The words of colour discs.
    pub struct ColorDiscWords {
        marker: template "colorDisc.marker" = "Color {n}";
        value: template "colorDisc.value" = "Hue {hue} degrees, saturation {saturation} %";
    }
}

/// A marker's place on the disc: what a drag or a key changes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DiscMove {
    /// Index of the marker, in the order of the colours the disc was given.
    pub marker: usize,
    /// Degrees, 0 to 360, red at 0 and growing counter-clockwise.
    pub hue: f32,
    /// 0 (the centre) to 1 (the rim).
    pub saturation: f32,
}

impl DiscMove {
    /// Marker `marker` at `hue` degrees and `saturation`.
    pub const fn new(marker: usize, hue: f32, saturation: f32) -> Self {
        DiscMove { marker, hue, saturation }
    }
}

/// The style of a colour disc.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorDiscStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The distance from the area's edge to the disc.
    inset: f32,
    /// A marker's radius; the selected marker's is larger.
    marker: Stateful<f32>,
    /// A press within this of a marker grabs it.
    grab: f32,
    /// The disc's edge.
    edge: StyleColor,
    /// The line from the centre to a marker.
    spoke: StyleColor,
    /// A marker's inner ring and the halo round it.
    ring: StyleColor,
    halo: StyleColor,
}

/// What [`ColorDisc`] returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorDiscOutput {
    /// This pass's edit, carrying the moved marker.
    pub edit: EditResponse<DiscMove>,
    /// The marker a press grabbed: the host selects it.
    pub pressed: Option<usize>,
    /// The marker a press grabbed and a drag is moving, also on the pass the drag ends.
    pub grabbed: Option<usize>,
}

impl Outcome for ColorDiscOutput {
    type Value = DiscMove;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The marker a press grabbed, and the colour that marker had the pass before.
#[derive(Clone, Copy, Default)]
struct Grab {
    marker: Option<usize>,
    last: Option<Srgba>,
}

/// Hue and saturation disc with a marker per colour; the host owns the colours and the rule that moves them
/// together.
///
/// - Pointer (primary button): a press grabs the nearest marker within `size.hit.target`
///   ([`pressed`](ColorDiscOutput::pressed) reports it, for the host to select it); a press elsewhere grabs nothing.
///   A drag moves the grabbed marker's hue (the pointer's angle, whole degrees) and saturation (its distance over the
///   radius, whole percents, at most 1), one gesture; the marker's brightness stays, the host's slider sets it. A
///   click is one `Set`. A colour of the host's that is not what the drag reported cancels the drag.
/// - Keys: one Tab stop, the selected marker (the first without a selection). Left and Right change the hue by 1
///   degree, Up and Down the saturation by 1 percent; Shift steps ten; each key is a `Set`. During a drag keys change
///   nothing. Esc during a drag cancels it.
/// - AccessKit: a `Group` named `label` holding a `Slider` per marker (the hue, 0 to 360, step 1, increment, decrement
///   and set value), named by [`names`](ColorDisc::names) or "Color N", with the value text "Hue 120 degrees,
///   saturation 80 %".
/// - Right to left nothing mirrors: red stays at 3 o'clock.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorDisc<'a> {
    #[common]
    common: Common,
    markers: &'a [Srgba],
    drag: Bind<'a, DiscMove>,
    /// The marker drawn larger and reached by Tab; the host owns it.
    #[prop]
    chosen: Option<usize>,
    /// The brightness the disc is drawn at, 0 to 1; never below a floor.
    #[prop(quantity = "brightness")]
    brightness: f32,
    /// The markers' accessible names, in order; a marker without one is "Color N".
    #[prop(into)]
    names: Vec<Label>,
    pressed: Option<usize>,
    grabbed: Option<usize>,
}

impl<'a> ColorDisc<'a> {
    /// A disc named `label` ("Harmony") with a marker at each of `markers`, reporting moves through `drag`.
    pub fn new(label: impl Into<Label>, markers: &'a [Srgba], drag: Bind<'a, DiscMove>) -> Self {
        ColorDisc { common: Common::new(), markers, drag, chosen: None, brightness: 1.0, names: Vec::new(), pressed: None, grabbed: None }
            .label(label)
            .ltr_axis()
    }

    /// The disc's centre and radius in the area `rect`, `inset` inside its shorter side.
    pub fn geometry(rect: egui::Rect, inset: f32) -> (Pos2, f32) {
        (rect.center(), (rect.width().min(rect.height()) / 2.0 - inset).max(1.0))
    }

    /// Where a marker of hue `hue` degrees and saturation `saturation` (0 to 1) lies.
    pub fn marker_pos(center: Pos2, radius: f32, hue: f32, saturation: f32) -> Pos2 {
        center + hue_direction(hue) * (radius * saturation.clamp(0.0, 1.0))
    }

    /// The marker with the Tab stop: the selected one, else the first.
    fn stop(&self) -> usize {
        self.chosen.filter(|selected| *selected < self.markers.len()).unwrap_or(0)
    }

    /// Marker `index` as the host's colour places it: hue, saturation (0 to 1) and brightness.
    fn hsb(&self, index: usize) -> [f32; 3] {
        let values = self.markers.get(index).map_or([0.0; 3], |color| {
            let v = HsbModel.from_srgb(*color).values;
            [finite(v[0]), finite(v[1]), finite(v[2])]
        });
        [values[0], values[1] / 100.0, values[2]]
    }

    /// Marker `index` where the host has it.
    fn resting(&self, index: usize) -> DiscMove {
        let [hue, saturation, _] = self.hsb(index);
        DiscMove::new(index, hue, saturation)
    }
}

impl Component for ColorDisc<'_> {
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = ColorDiscStyle;
    type Output = ColorDiscOutput;

    fn style(cx: &StyleCx<'_>, _: ColorDiscStyleKey) -> ColorDiscStyle {
        let tokens = cx.tokens;
        let colors = &tokens.color;
        ColorDiscStyle {
            metrics: Metrics::new().min_size(Vec2::new(tokens.size.plot.width, tokens.size.plot.height)),
            focus: cx.focus_look(),
            inset: tokens.space.pad.regular,
            marker: Stateful::new(tokens.size.handle.sm).when(Flags::SELECTED, tokens.size.handle.md),
            grab: tokens.size.hit.target,
            edge: cx.color(colors.line.control_edge),
            spoke: cx.color(colors.overlay.handle_fill.faded(SPOKE_SHARE)),
            ring: cx.color(colors.overlay.handle_fill),
            halo: cx.color(colors.overlay.halo),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        let (center, radius) = Self::geometry(rect, cx.style.inset);
        cx.part(DISC, HitShape::circle(center, radius + cx.style.grab))
            .sense(Sense::DRAG | Sense::CLICK)
            .pointer_only("the arrow keys move the marker");
        let stop = self.stop();
        for index in 0..self.markers.len() {
            let at = match cx.value().filter(|moved| moved.marker == index) {
                Some(moved) => *moved,
                None => self.resting(index),
            };
            let radius_of = marker_radius(cx.style, self.chosen == Some(index));
            let part = cx.part(MARKER.at(index as u64), HitShape::circle(Self::marker_pos(center, radius, at.hue, at.saturation), radius_of));
            // A marker takes no press of its own (the disc grabs the nearest) and only the selected one is a Tab stop.
            if index == stop {
                part.focusable().focus_shape(FocusShape::Circle);
            } else {
                part.keyboard_via(MARKER);
            }
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, DiscMove>> {
        Some(self.drag.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (center, radius) = Self::geometry(cx.rect, cx.style.inset);
        let grab_reach = cx.style.grab;
        let press = cx.part(DISC).press;
        let stop = self.stop();
        if primary(&press) && matches!(press.phase, PressPhase::Down { .. }) {
            let origin = press.phase.start().map(|start| start.origin);
            let near = origin.and_then(|origin| {
                let distance = |index: usize| {
                    let at = self.resting(index);
                    Self::marker_pos(center, radius, at.hue, at.saturation).distance(origin)
                };
                (0..self.markers.len()).map(|index| (index, distance(index))).filter(|(_, d)| *d <= grab_reach).min_by(|a, b| a.1.total_cmp(&b.1))
            });
            cx.memory::<Grab>().marker = near.map(|(index, _)| index);
            self.pressed = near.map(|(index, _)| index);
            if near.is_some() && !self.markers.is_empty() {
                cx.focus_part(MARKER.at(stop as u64));
            }
        }
        let grabbed = cx.memory::<Grab>().marker;
        self.grabbed = grabbed;
        let running = cx.gesture().is_some();
        // A colour of the host's that changed and is not what the drag reported is an outside change.
        if running && let Some(marker) = grabbed.filter(|marker| *marker < self.markers.len()) {
            let last = cx.memory::<Grab>().last.replace(self.markers[marker]);
            if let Some(moved) = cx.value().filter(|moved| moved.marker == marker) {
                let echo = HsbModel.to_srgb(&ModelValues::new("hsb", &[moved.hue, moved.saturation * 100.0, self.hsb(marker)[2]]).values);
                if last.is_some_and(|last| last != self.markers[marker]) && !echo.same_rgb8(self.markers[marker]) {
                    cx.cancel(CancelReason::HostChanged);
                }
            }
        }
        let value_at = |cx: &Cx<'_, Self>, marker: usize, pos: Pos2| {
            let base = cx.value().filter(|moved| moved.marker == marker && cx.gesture().is_some()).copied().unwrap_or_else(|| self.resting(marker));
            let offset = pos - center;
            let mut out = base;
            if offset.length().is_finite() {
                out.saturation = ((offset.length() / radius).clamp(0.0, 1.0) * 100.0).round() / 100.0;
                if offset.length() >= CENTRE
                    && let Some(hue) = hue_at(center, pos)
                {
                    out.hue = hue;
                }
            }
            out
        };
        if primary(&press)
            && let Some(marker) = grabbed
        {
            match press.phase {
                PressPhase::DragStart { pos, .. } => {
                    cx.begin();
                    cx.propose(value_at(&*cx, marker, pos));
                }
                PressPhase::Drag { pos, .. } if cx.gesture().is_some() => cx.propose(value_at(&*cx, marker, pos)),
                PressPhase::Release { pos, click, .. } => {
                    if cx.gesture().is_some() {
                        cx.propose(value_at(&*cx, marker, pos));
                        cx.commit();
                    } else if click {
                        cx.set(value_at(&*cx, marker, pos));
                    }
                }
                _ => {}
            }
        }
        if matches!(press.phase, PressPhase::Release { .. } | PressPhase::Cancel { .. }) {
            *cx.memory::<Grab>() = Grab::default();
        }
        let mut now = self.resting(stop);
        let on_stop = cx.active_part() == Some(MARKER.at(stop as u64));
        while let Some((mv, mods)) = cx.intents().take_move() {
            if running || !on_stop || self.markers.is_empty() {
                continue;
            }
            let by = if mods.extend { FAR } else { KEY_STEP };
            match mv {
                NavMove::Forward => now.hue += by,
                NavMove::Back => now.hue -= by,
                NavMove::Up => now.saturation += by / 100.0,
                NavMove::Down => now.saturation -= by / 100.0,
                _ => continue,
            }
            now.hue = now.hue.round().rem_euclid(360.0);
            now.saturation = (now.saturation * 100.0).round().clamp(0.0, 100.0) / 100.0;
            cx.set(now);
        }
        for index in 0..self.markers.len() {
            if let Some(value) = cx.intents().take_set_value_at(MARKER.at(index as u64)).filter(|value| value.is_finite() && !running) {
                let mut moved = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
                moved.hue = value.round().rem_euclid(360.0) as f32;
                cx.set(moved);
            }
        }
        while let Some(request) = cx.intents().take_access() {
            let Some(index) = (0..self.markers.len()).find(|index| request.target == Some(MARKER.at(*index as u64))) else { continue };
            let step = match request.action {
                AccessAction::Increment => KEY_STEP,
                AccessAction::Decrement => -KEY_STEP,
                _ => continue,
            };
            if !running {
                let mut moved = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
                moved.hue = (moved.hue + step).round().rem_euclid(360.0);
                cx.set(moved);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ColorDiscWords>();
        for index in 0..self.markers.len() {
            let live = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
            let name = match self.names.get(index) {
                Some(name) => cx.env.resolve(name).as_str().to_owned(),
                None => words.marker.fill(&[("n", (index + 1).to_string().as_str())]),
            };
            let hue = format!("{:.0}", live.hue.rem_euclid(360.0));
            let saturation = format!("{:.0}", live.saturation * 100.0);
            let node = access.node(MARKER.at(index as u64), Role::SLIDER);
            node.name = Name::Text(name);
            let text = words.value.fill(&[("hue", hue.as_str()), ("saturation", saturation.as_str())]);
            node.numeric = Some(Numeric::new(f64::from(live.hue.rem_euclid(360.0)), 0.0..=360.0).step(f64::from(KEY_STEP)).wraps(true).text(text));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (center, radius) = Self::geometry(cx.rect, cx.style.inset);
        let brightness = if self.brightness.is_finite() { self.brightness.clamp(0.0, 1.0).max(DISC_FLOOR) } else { 1.0 };
        let painter = ui.content_painter(cx);
        painter.painter.add(disc_mesh(center, radius, brightness));
        let draw = ui.draw();
        draw.stroke(&PaintShape::Circle(center, radius), StrokeWidth::Thin, cx.style.edge);
        let places: Vec<Pos2> = (0..self.markers.len())
            .map(|index| {
                let at = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
                Self::marker_pos(center, radius, at.hue, at.saturation)
            })
            .collect();
        for place in &places {
            draw.stroke(&PaintShape::Path(vec![center, *place]), StrokeWidth::Thin, cx.style.spoke);
        }
        let selected = self.chosen.filter(|selected| *selected < places.len());
        let order = (0..places.len()).filter(|index| Some(*index) != selected).chain(selected);
        for index in order {
            let r = marker_radius(cx.style, Some(index) == selected);
            let circle = PaintShape::Circle(places[index], r);
            draw.data_fill(&circle, DataColor::new(Color32::from(self.markers[index])));
            draw.stroke(&circle, StrokeWidth::Thick, cx.style.halo);
            draw.stroke(&circle, StrokeWidth::Medium, cx.style.ring);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorDiscOutput {
        let (pressed, grabbed) = (self.pressed, self.grabbed);
        ColorDiscOutput { edit: cx.edit_response(), pressed, grabbed }
    }
}

/// The disc at `brightness`: a centre vertex and rings out to the rim, hue around.
fn disc_mesh(center: Pos2, radius: f32, brightness: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let b = brightness * 100.0;
    mesh.colored_vertex(center, vertex(0.0, 0.0, b));
    for ring in 1..=RINGS {
        let s = ring as f32 / RINGS as f32;
        for k in 0..SEGMENTS {
            let hue = k as f32 * 360.0 / SEGMENTS as f32;
            mesh.colored_vertex(ColorDisc::marker_pos(center, radius, hue, s), vertex(hue, s * 100.0, b));
        }
    }
    let at = |ring: u32, k: u32| if ring == 0 { 0 } else { 1 + (ring - 1) * SEGMENTS + k % SEGMENTS };
    for ring in 0..RINGS {
        for k in 0..SEGMENTS {
            if ring == 0 {
                mesh.add_triangle(0, at(1, k), at(1, k + 1));
            } else {
                mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
                mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
            }
        }
    }
    mesh
}

/// The radius of a marker, larger while it is the selected one.
fn marker_radius(style: &ColorDiscStyle, selected: bool) -> f32 {
    let flags = if selected { Flags::SELECTED } else { Flags::empty() };
    style.marker.resolve(State::Rest, flags, StateMix::default())
}
