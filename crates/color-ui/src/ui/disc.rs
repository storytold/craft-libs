//! [`ColorDisc`]: a hue and saturation disc with one draggable marker per colour of a set, as VectorCraft's harmony
//! wheel.

use super::channels::{finite, primary};
use super::wheel::{hue_at, hue_direction, vertex};
use craft_behavior::nav::{Arrows, NavShape, Roving};
use craft_behavior::{CancelReason, HitShape, Metrics, NavMove, PartKey, PointerButton, PressPhase, Sense};
use craft_theme::{ColorModel, HsbModel, ModelValues, Srgba};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, Outcome,
    PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{Marker, MarkerSize, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, MarkerLook, StyleColor, StyleCx};
use egui::{Color32, Mesh, Pos2, Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The disc: a press on it grabs the nearest marker within reach.
const DISC: PartKey = PartKey::new("disc");
/// A marker: `MARKER.at(i)`, a member of the markers' roving group; Tab lands on the selected one.
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
/// The segmented look: hue sectors round the disc, rings out from its centre, and the degrees one sector spans.
const SECTORS: u32 = 24;
const PATCH_RINGS: u32 = 5;
const SECTOR_DEGREES: f32 = 15.0;
/// How far from where a drag put its marker, in radii, the host's colour may lie and still be the drag's: what an
/// 8-bit round trip moves a colour by, two of the hundredths a drag reports its radial in.
const ECHO_REACH: f32 = 0.02;

craft_ui::words! {
    /// The words of colour discs.
    pub struct ColorDiscWords {
        marker: template "colorDisc.marker" = "Color {n}";
        value: template "colorDisc.value" = "Hue {hue} degrees, saturation {saturation} %";
        value_brightness: template "colorDisc.valueBrightness" = "Hue {hue} degrees, brightness {brightness} %";
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
    /// 0 (the centre) to 1 (the rim): the channel the radius shows.
    pub radial: f32,
}

impl DiscMove {
    /// Marker `marker` at `hue` degrees and `radial`.
    pub const fn new(marker: usize, hue: f32, radial: f32) -> Self {
        DiscMove { marker, hue, radial }
    }
}

/// What a click on the disc reports: the marker it concerns and the point under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DiscHit {
    /// The marker grabbed (a primary click) or in grab reach (a secondary click).
    pub marker: Option<usize>,
    /// The marker whose spoke is within the spoke hit width (a secondary click).
    pub spoke: Option<usize>,
    /// Degrees, 0 to 360: the hue under the pointer.
    pub hue: f32,
    /// 0 to 1: the channel the radius shows, under the pointer.
    pub radial: f32,
}

impl DiscHit {
    /// A hit on `marker` or `spoke` at `hue` and `radial`.
    pub const fn new(marker: Option<usize>, spoke: Option<usize>, hue: f32, radial: f32) -> Self {
        DiscHit { marker, spoke, hue, radial }
    }
}

/// How a [`ColorDisc`] draws its colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiscLook {
    /// A smooth blend of hue around and the radial channel outwards.
    #[default]
    Smooth,
    /// 24 hue sectors by 5 rings of solid patches with thin edges.
    Segmented,
}

/// The channel the radius of a [`ColorDisc`] shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiscRadius {
    /// Saturation outwards.
    #[default]
    Saturation,
    /// Brightness outwards.
    Brightness,
}

/// The centre of the segmented patch at `hue` and `radial`, as `(hue, radial)`: sector `round(h/15) mod 24`, ring
/// `floor(r*5).min(4)`.
pub fn patch_at(hue: f32, radial: f32) -> (f32, f32) {
    let sector = (finite(hue).rem_euclid(360.0) / SECTOR_DEGREES).round() as u32 % SECTORS;
    let ring = ((finite(radial).clamp(0.0, 1.0) * PATCH_RINGS as f32).floor() as u32).min(PATCH_RINGS - 1);
    (sector as f32 * SECTOR_DEGREES, (ring as f32 + 0.5) / PATCH_RINGS as f32)
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
    /// The markers; the selected one, the set's base colour, is large.
    marker: MarkerLook,
    /// A press within this of a marker grabs it.
    grab: f32,
    /// The disc's edge.
    edge: StyleColor,
    /// The line from the centre to a marker.
    spoke: StyleColor,
    /// The least side of the picture, and the most it takes of the width it is given.
    least: f32,
    most: f32,
}

/// What [`ColorDisc`] returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorDiscOutput {
    /// This pass's edit, carrying the moved marker.
    pub edit: EditResponse<DiscMove>,
    /// The marker a press grabbed, or the marker focus is on when it is not the selected one (assistive technology
    /// moved focus there): the host selects it.
    pub pressed: Option<usize>,
    /// The marker a press grabbed and a drag is moving, also on the pass the drag ends.
    pub grabbed: Option<usize>,
    /// A primary click (no drag): the marker it grabbed and the point under the release.
    pub clicked: Option<DiscHit>,
    /// A secondary click inside the disc: the marker in grab reach, else the marker whose spoke is within the spoke
    /// hit width, with the point; `None` outside the circle.
    pub context: Option<DiscHit>,
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
///   click is one `Set`. A colour of the host's that is not what the drag reported, after the host's own round trip
///   ([`stored_as`](ColorDisc::stored_as)), cancels the drag.
/// - Keys: the markers are one Tab stop, which lands on the selected marker (the first without a selection); a press
///   that grabs a marker focuses it. Left and Right change the focused marker's hue by 1 degree, Up and Down its
///   saturation by 1 percent; Shift steps ten; each key is a `Set`. During a drag keys change nothing. Esc during a
///   drag cancels it.
/// - AccessKit: a `Group` named `label` holding a focusable `Slider` per marker (the hue, 0 to 360, step 1, increment,
///   decrement and set value), named by [`names`](ColorDisc::names) or "Color N", with the value text "Hue 120
///   degrees, saturation 80 %". Focus on a marker reports it as [`pressed`](ColorDiscOutput::pressed).
/// - Right to left nothing mirrors: red stays at 3 o'clock.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorDisc<'a> {
    #[common]
    common: Common,
    markers: &'a [Srgba],
    drag: Bind<'a, DiscMove>,
    /// The marker drawn larger and where Tab lands; the host owns it.
    #[prop]
    chosen: Option<usize>,
    /// The marker drawn large: the set's base colour; `chosen` keeps the Tab stop and, when it is not the base, gets
    /// the marker look's selection ring.
    #[prop]
    base: Option<usize>,
    /// The channel the radius does not show, 0 to 1; never below a floor (on a saturation disc).
    #[prop(quantity = "level")]
    level: f32,
    /// How the disc draws its colours.
    #[prop]
    #[style_key]
    look: DiscLook,
    /// The channel the radius shows.
    #[prop]
    radius: DiscRadius,
    /// The markers' accessible names, in order; a marker without one is "Color N".
    #[prop(into)]
    names: Vec<Label>,
    stored: Option<&'a dyn Fn(usize, Srgba) -> Srgba>,
    pressed: Option<usize>,
    grabbed: Option<usize>,
    clicked: Option<DiscHit>,
    context: Option<DiscHit>,
}

impl<'a> ColorDisc<'a> {
    /// A disc named `label` ("Harmony") with a marker at each of `markers`, reporting moves through `drag`.
    pub fn new(label: impl Into<Label>, markers: &'a [Srgba], drag: Bind<'a, DiscMove>) -> Self {
        ColorDisc {
            common: Common::new(),
            markers,
            drag,
            chosen: None,
            base: None,
            level: 1.0,
            look: DiscLook::Smooth,
            radius: DiscRadius::Saturation,
            names: Vec::new(),
            stored: None,
            pressed: None,
            grabbed: None,
            clicked: None,
            context: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// How the host stores marker `index`'s colours: `color` written in the host's own model (CMYK through a profile)
    /// and read back. A drag's colour that comes back moved by that round trip is still the drag's, not an outside
    /// change; without it the host is taken to keep the colour as given, to 8 bits.
    pub fn stored_as(mut self, stored: &'a dyn Fn(usize, Srgba) -> Srgba) -> Self {
        self.stored = Some(stored);
        self
    }

    /// The disc's centre and radius in the area `rect`, `inset` inside its shorter side.
    pub fn geometry(rect: egui::Rect, inset: f32) -> (Pos2, f32) {
        (rect.center(), (rect.width().min(rect.height()) / 2.0 - inset).max(1.0))
    }

    /// Where a marker of hue `hue` degrees and saturation `saturation` (0 to 1) lies.
    pub fn marker_pos(center: Pos2, radius: f32, hue: f32, saturation: f32) -> Pos2 {
        center + hue_direction(hue) * (radius * saturation.clamp(0.0, 1.0))
    }

    /// The marker Tab lands on: the selected one, else the first.
    fn stop(&self) -> usize {
        self.chosen.filter(|selected| *selected < self.markers.len()).unwrap_or(0)
    }

    /// Marker `index` as the host's colour places it: hue, saturation and brightness (0 to 1).
    fn hsb(&self, index: usize) -> [f32; 3] {
        self.markers.get(index).map_or([0.0; 3], |color| hsb_of(*color))
    }

    /// Marker `index` where the host has it: the radial channel is the one the radius shows.
    fn resting(&self, index: usize) -> DiscMove {
        self.at(index, self.hsb(index))
    }

    /// Marker `index` at the hue, saturation and brightness `hsb`: the radial channel is the one the radius shows.
    fn at(&self, index: usize, [hue, saturation, brightness]: [f32; 3]) -> DiscMove {
        let radial = match self.radius {
            DiscRadius::Saturation => saturation,
            DiscRadius::Brightness => brightness,
        };
        DiscMove::new(index, hue, radial)
    }

    /// The HSB values (degrees, percents) `moved` makes of `color`: its radial channel on the radius's axis, the other
    /// channel `color`'s.
    fn values_on(&self, color: Srgba, moved: &DiscMove) -> [f32; 3] {
        let [_, saturation, brightness] = hsb_of(color);
        match self.radius {
            DiscRadius::Saturation => [moved.hue, moved.radial * 100.0, brightness * 100.0],
            DiscRadius::Brightness => [moved.hue, saturation * 100.0, moved.radial * 100.0],
        }
    }

    /// Where marker `index` lies as the host has it.
    fn place(&self, index: usize, center: Pos2, radius: f32) -> Pos2 {
        let at = self.resting(index);
        Self::marker_pos(center, radius, at.hue, at.radial)
    }

    /// The channel the radius does not show, 0 to 1; never below a floor on a saturation disc.
    fn shown_level(&self) -> f32 {
        let level = if self.level.is_finite() { self.level.clamp(0.0, 1.0) } else { 1.0 };
        match self.radius {
            DiscRadius::Saturation => level.max(DISC_FLOOR),
            DiscRadius::Brightness => level,
        }
    }

    /// The marker focus is on.
    fn focused(&self, cx: &Cx<'_, Self>) -> Option<usize> {
        (0..self.markers.len()).find(|index| cx.active_part() == Some(MARKER.at(*index as u64)))
    }
}

impl Component for ColorDisc<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
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
            metrics: Metrics::new(),
            least: tokens.size.plot.height,
            most: tokens.size.plot.width,
            focus: cx.focus_look(),
            inset: tokens.space.pad.regular,
            marker: cx.marker(),
            grab: tokens.size.hit.target,
            edge: cx.color(colors.line.control_edge),
            spoke: cx.color(colors.overlay.handle_fill.faded(SPOKE_SHARE)),
        }
    }

    /// A band across the width it is given, as tall as the square that fits it: the picture is centred in it.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let most = cx.style.most.max(cx.style.least);
        cx.picture_size(cx.style.least, f32::INFINITY, |width| width.min(most))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        let (center, radius) = Self::geometry(rect, cx.style.inset);
        cx.part(DISC, HitShape::circle(center, radius + cx.style.grab))
            .sense(Sense::DRAG | Sense::CLICK | Sense::CONTEXT_CLICK)
            .pointer_only("the arrow keys move the marker");
        let stop = self.stop();
        for index in 0..self.markers.len() {
            let at = match cx.value().filter(|moved| moved.marker == index) {
                Some(moved) => *moved,
                None => self.resting(index),
            };
            let reach = Marker::reach(&cx.style.marker, marker_size(self.base == Some(index)));
            let part = cx.part(MARKER.at(index as u64), HitShape::circle(Self::marker_pos(center, radius, at.hue, at.radial), reach));
            // A marker takes no press of its own: the disc grabs the nearest.
            let part = part.focusable().focus_shape(FocusShape::Circle);
            if index == stop {
                part.selected();
            }
        }
        // The arrows move the focused marker, so the group takes none of them; Home and End do nothing.
        cx.roving(Roving::new(NavShape::Flat).arrows(Arrows::None).home_end(false));
    }

    fn bind(&mut self) -> Option<BindMut<'_, DiscMove>> {
        Some(self.drag.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (center, radius) = Self::geometry(cx.rect, cx.style.inset);
        let grab_reach = cx.style.grab;
        let press = cx.part(DISC).press;
        let secondary = press.phase.start().is_some_and(|start| start.button == PointerButton::Secondary);
        if primary(&press) && matches!(press.phase, PressPhase::Down { .. }) {
            let near = press.phase.start().and_then(|start| {
                nearest_within((0..self.markers.len()).map(|index| self.place(index, center, radius).distance(start.origin)), grab_reach)
            });
            cx.memory::<Grab>().marker = near;
            self.pressed = near;
            if let Some(index) = near {
                cx.focus_part(MARKER.at(index as u64));
            }
        }
        let grabbed = cx.memory::<Grab>().marker;
        self.grabbed = grabbed;
        let running = cx.gesture().is_some();
        // A colour of the host's that changed and does not lie where the drag put the marker is an outside change. Only
        // the hue and the radial count: the host keeps the channel the radius does not show as it likes (from the
        // colour before, or from the press against the drift of 8-bit colours) and stores the colour in its own
        // model, whose round trip the host gives. A colour whose 8 bits lose its hue (black on a saturation disc) is
        // the drag's when it is the 8-bit colour the drag made of the colour before.
        if running && let Some(marker) = grabbed.filter(|marker| *marker < self.markers.len()) {
            let last = cx.memory::<Grab>().last.replace(self.markers[marker]);
            if let Some(moved) = cx.value().filter(|moved| moved.marker == marker)
                && let Some(last) = last.filter(|last| *last != self.markers[marker])
            {
                let mut echo = HsbModel.to_srgb(&ModelValues::new("hsb", &self.values_on(last, moved)).values);
                let mut sent = *moved;
                if let Some(stored) = self.stored {
                    echo = stored(marker, echo);
                    sent = self.at(marker, hsb_of(echo));
                }
                if !lies_at(self.resting(marker), &sent) && !echo.same_rgb8(self.markers[marker]) {
                    cx.cancel(CancelReason::HostChanged);
                }
            }
        }
        let value_at = |cx: &Cx<'_, Self>, marker: usize, pos: Pos2| {
            let base = cx.value().filter(|moved| moved.marker == marker && cx.gesture().is_some()).copied().unwrap_or_else(|| self.resting(marker));
            moved_to(base, center, radius, pos)
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
        if let PressPhase::Release { pos, click: true, .. } = press.phase {
            let point = moved_to(DiscMove::new(0, 0.0, 0.0), center, radius, pos);
            if primary(&press) {
                self.clicked = Some(DiscHit::new(grabbed, None, point.hue, point.radial));
            } else if secondary && pos.distance(center) <= radius {
                let places: Vec<Pos2> = (0..self.markers.len()).map(|index| self.place(index, center, radius)).collect();
                let marker = nearest_within(places.iter().map(|place| place.distance(pos)), grab_reach);
                let spoke = match marker {
                    Some(_) => None,
                    None => nearest_within(places.iter().map(|place| segment_distance(center, *place, pos)), grab_reach),
                };
                self.context = Some(DiscHit::new(marker, spoke, point.hue, point.radial));
            }
        }
        if matches!(press.phase, PressPhase::Release { .. } | PressPhase::Cancel { .. }) {
            *cx.memory::<Grab>() = Grab::default();
        }
        let focused = self.focused(&*cx);
        if self.pressed.is_none() && focused != Some(self.stop()) {
            self.pressed = focused;
        }
        let mut now = self.resting(focused.unwrap_or(0));
        while let Some((mv, mods)) = cx.intents().take_move() {
            if running || focused.is_none() {
                continue;
            }
            let by = if mods.extend { FAR } else { KEY_STEP };
            match mv {
                NavMove::Forward => now.hue += by,
                NavMove::Back => now.hue -= by,
                NavMove::Up => now.radial += by / 100.0,
                NavMove::Down => now.radial -= by / 100.0,
                _ => continue,
            }
            now.hue = now.hue.round().rem_euclid(360.0);
            now.radial = (now.radial * 100.0).round().clamp(0.0, 100.0) / 100.0;
            cx.set(now);
        }
        for index in 0..self.markers.len() {
            if let Some(value) = cx.intents().take_set_value_at(MARKER.at(index as u64)).filter(|value| value.is_finite() && !running) {
                let mut moved = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
                moved.hue = value.round().rem_euclid(360.0) as f32;
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
            let radial = format!("{:.0}", live.radial * 100.0);
            let node = access.node(MARKER.at(index as u64), Role::SLIDER);
            node.name = Name::Text(name);
            let text = match self.radius {
                DiscRadius::Saturation => words.value.fill(&[("hue", hue.as_str()), ("saturation", radial.as_str())]),
                DiscRadius::Brightness => words.value_brightness.fill(&[("hue", hue.as_str()), ("brightness", radial.as_str())]),
            };
            node.numeric = Some(Numeric::new(f64::from(live.hue.rem_euclid(360.0)), 0.0..=360.0).step(f64::from(KEY_STEP)).wraps(true).text(text));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (center, radius) = Self::geometry(cx.rect, cx.style.inset);
        let level = self.shown_level();
        let painter = ui.content_painter(cx);
        let patches = match self.look {
            DiscLook::Segmented => {
                let (mesh, edges) = patch_mesh(center, radius, level, self.radius);
                painter.painter.add(mesh);
                edges
            }
            _ => {
                painter.painter.add(disc_mesh(center, radius, level, self.radius));
                Vec::new()
            }
        };
        let draw = ui.draw();
        for edge in patches {
            draw.stroke(&PaintShape::Polygon(edge), StrokeWidth::Thin, cx.style.edge);
        }
        draw.stroke(&PaintShape::Circle(center, radius), StrokeWidth::Thin, cx.style.edge);
        let places: Vec<Pos2> = (0..self.markers.len())
            .map(|index| {
                let at = cx.value().filter(|moved| moved.marker == index).copied().unwrap_or_else(|| self.resting(index));
                Self::marker_pos(center, radius, at.hue, at.radial)
            })
            .collect();
        for place in &places {
            draw.stroke(&PaintShape::Path(vec![center, *place]), StrokeWidth::Thin, cx.style.spoke);
        }
        let base = self.base.filter(|base| *base < places.len());
        let chosen = self.chosen.filter(|chosen| *chosen < places.len());
        // The base is drawn last, over the chosen marker, over the rest.
        let order = (0..places.len())
            .filter(|index| Some(*index) != chosen && Some(*index) != base)
            .chain(chosen.filter(|chosen| Some(*chosen) != base))
            .chain(base);
        for index in order {
            let marker = Marker::filled(places[index], Color32::from(self.markers[index]))
                .size(marker_size(Some(index) == base))
                .selected(Some(index) == chosen && Some(index) != base);
            draw.marker(&cx.style.marker, &marker);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorDiscOutput {
        let (pressed, grabbed, clicked, context) = (self.pressed, self.grabbed, self.clicked, self.context);
        ColorDiscOutput { edit: cx.edit_response(), pressed, grabbed, clicked, context }
    }
}

/// `base` moved to the pointer at `pos`: the hue is the pointer's angle (kept at the centre), the radial channel its
/// distance over `radius`, in whole percents.
fn moved_to(base: DiscMove, center: Pos2, radius: f32, pos: Pos2) -> DiscMove {
    let offset = pos - center;
    let mut out = base;
    if offset.length().is_finite() {
        out.radial = ((offset.length() / radius).clamp(0.0, 1.0) * 100.0).round() / 100.0;
        if offset.length() >= CENTRE
            && let Some(hue) = hue_at(center, pos)
        {
            out.hue = hue;
        }
    }
    out
}

/// Whether `at` lies within `ECHO_REACH` of `moved` on the disc.
fn lies_at(at: DiscMove, moved: &DiscMove) -> bool {
    let point = |m: &DiscMove| Vec2::angled(m.hue.to_radians()) * m.radial;
    (point(&at) - point(moved)).length() <= ECHO_REACH
}

/// Hue, saturation and brightness (0 to 1) of `color`.
fn hsb_of(color: Srgba) -> [f32; 3] {
    let v = HsbModel.from_srgb(color).values;
    [finite(v[0]), finite(v[1]) / 100.0, finite(v[2]) / 100.0]
}

/// The index of the least of `distances` that is within `reach`.
pub(super) fn nearest_within(distances: impl Iterator<Item = f32>, reach: f32) -> Option<usize> {
    distances.enumerate().filter(|(_, distance)| *distance <= reach).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| index)
}

/// The distance from `p` to the segment `a` to `b`.
fn segment_distance(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let along = b - a;
    let length_sq = along.length_sq();
    let t = if length_sq > 0.0 { ((p - a).dot(along) / length_sq).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + along * t)
}

/// The colour at `hue` and `radial` on a disc whose radius shows `kind`, the other channel at `level`.
fn disc_color(kind: DiscRadius, hue: f32, radial: f32, level: f32) -> Color32 {
    let (saturation, brightness) = match kind {
        DiscRadius::Saturation => (radial, level),
        DiscRadius::Brightness => (level, radial),
    };
    vertex(hue, saturation * 100.0, brightness * 100.0)
}

/// The segmented disc: a solid patch per sector and ring, centred on `patch_at`'s colour, and each patch's outline.
fn patch_mesh(center: Pos2, radius: f32, level: f32, kind: DiscRadius) -> (Mesh, Vec<Vec<Pos2>>) {
    let mut mesh = Mesh::default();
    let mut edges = Vec::new();
    for sector in 0..SECTORS {
        for ring in 0..PATCH_RINGS {
            let (hue, radial) = patch_at(sector as f32 * SECTOR_DEGREES, (ring as f32 + 0.5) / PATCH_RINGS as f32);
            let color = disc_color(kind, hue, radial, level);
            let corners = [(-1.0, ring), (1.0, ring), (1.0, ring + 1), (-1.0, ring + 1)]
                .map(|(side, edge)| ColorDisc::marker_pos(center, radius, hue + side * SECTOR_DEGREES / 2.0, edge as f32 / PATCH_RINGS as f32));
            let first = mesh.vertices.len() as u32;
            for corner in corners {
                mesh.colored_vertex(corner, color);
            }
            mesh.add_triangle(first, first + 1, first + 2);
            mesh.add_triangle(first, first + 2, first + 3);
            edges.push(corners.to_vec());
        }
    }
    (mesh, edges)
}

/// The smooth disc at `level`: a centre vertex and rings out to the rim, hue around.
fn disc_mesh(center: Pos2, radius: f32, level: f32, kind: DiscRadius) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, disc_color(kind, 0.0, 0.0, level));
    for ring in 1..=RINGS {
        let s = ring as f32 / RINGS as f32;
        for k in 0..SEGMENTS {
            let hue = k as f32 * 360.0 / SEGMENTS as f32;
            mesh.colored_vertex(ColorDisc::marker_pos(center, radius, hue, s), disc_color(kind, hue, s, level));
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

/// The size of a marker: large while it is the base.
fn marker_size(base: bool) -> MarkerSize {
    if base { MarkerSize::Large } else { MarkerSize::Regular }
}
