//! [`SwatchPair`]: two overlapping colour wells, foreground over background or fill over stroke.

use craft_behavior::{EditEvents, Flow, HitLevel, HitPriority, HitShape, Metrics, Mixed, NavShape, PartKey, PressPhase, Roving, Sense};
use craft_theme::{Srgba, WellShape};
use craft_ui::buttons::IconButton;
use craft_ui::choice::{ChoiceWords, Swatch, SwatchMetrics, SwatchStyle};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DragPreview, DropMark, DropOperation, DropTarget, Edit as _, EditResponse, Focus as _, MeasureCx,
    Meta as _, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{Glyph, IconSpec, PaintShape, StrokeWidth, WellFill};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, Flags, FocusLook, GlyphRole, StyleColor, StyleCx, SurfaceEdge, SurfaceStyle, UiTokens};
use egui::emath::GuiRounding as _;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Response, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::paint_chip::{WellLook, paint_well};
use super::popover::compact_picker;
use super::{ColorPicker, Paint, PaintWords, PickedColor, ScreenPickGesture};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// One of the wells, `WELL.at(0)` the primary and `WELL.at(1)` the secondary.
const WELL: PartKey = PartKey::new("well");
/// The Default Colours button.
const DEFAULT: PartKey = PartKey::new("default");
/// The Swap Colours button.
const SWAP: PartKey = PartKey::new("swap");

/// One of a [`SwatchPair`]'s wells. The primary one is at the start, top, the secondary one is
/// offset towards the end and down: foreground and background, or fill and stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Well {
    #[default]
    /// The primary well: at the start, top.
    Primary,
    /// The secondary well: offset towards the end and down.
    Secondary,
}

impl Well {
    /// The other well.
    pub fn other(self) -> Well {
        match self {
            Well::Primary => Well::Secondary,
            Well::Secondary => Well::Primary,
        }
    }

    fn index(self) -> u64 {
        match self {
            Well::Primary => 0,
            Well::Secondary => 1,
        }
    }

    fn of(index: u64) -> Well {
        if index == 0 { Well::Primary } else { Well::Secondary }
    }
}

/// What a [`SwatchPair`] shows for one well.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PairWell {
    label: Label,
    paint: Mixed<Paint>,
    ring: bool,
}

impl PairWell {
    /// A well named `label` (its accessible name and tooltip, translated: "Foreground colour") of the host's `color`;
    /// `Mixed::Mixed` for a selection whose fills differ, shown as the mixed well.
    pub fn new(label: impl Into<Label>, color: impl Into<Mixed<Srgba>>) -> Self {
        let paint = match color.into() {
            Mixed::Same(color) => Mixed::Same(Paint::Solid(color)),
            Mixed::Mixed => Mixed::Mixed,
        };
        Self { label: label.into(), paint, ring: false }
    }

    /// A well named `label` showing any paint through `paint_well`; `Mixed::Mixed` for a selection whose paints differ.
    /// A ring well clips the face to its band; the popover picker starts from the solid colour or the first stop.
    pub fn paint(label: impl Into<Label>, paint: impl Into<Mixed<Paint>>) -> Self {
        Self { label: label.into(), paint: paint.into(), ring: false }
    }

    /// Draws the well as a ring (a stroke, as DesignCraft's stroke well) instead of a filled square. The middle shows
    /// what is behind it.
    #[must_use]
    pub fn ring(mut self, ring: bool) -> Self {
        self.ring = ring;
        self
    }
}

/// The style of a swatch pair.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwatchPairStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The side of a well: `size.icon.lg`.
    well: f32,
    /// The offset of the secondary well from the primary one on both axes: `size.icon.sm`.
    step: f32,
    /// The corners of a well: the swatch's, or half the side where the theme's wells are round (`shape.wells`).
    radius: CornerRadius,
    /// Whether the wells are round (`shape.wells`): they take the pointer inside their circle only.
    round: bool,
    /// The gap in the panel's colour round the front well: `space.gap.tight`.
    gap: f32,
    /// The side of a button: the icon slot of a control.
    buttons: f32,
    /// The outline just outside a well.
    edge: StyleColor,
    /// The outline's width: `stroke.thin`.
    outline: f32,
    /// The frame round the active well where the pair frames it (`frame_active`).
    frame: StyleColor,
    /// The light line just inside a well's edge, so any colour reads against the surface.
    mark: StyleColor,
    /// A swatch's look, for the lines that stand out from a ring well's colour.
    swatch: SwatchStyle,
    /// The No Color well's fill: the paint chip's.
    none_fill: StyleColor,
    /// The No Color well's diagonal: the paint chip's.
    none_mark: StyleColor,
}

/// What a [`SwatchPair`]'s wells take of a dragged payload: `SwatchPair::accepts`.
pub struct DropAccepts<'a, P>(Box<AcceptsFn<'a, P>>);

/// The host's test of a payload over a well.
type AcceptsFn<'a, P> = dyn Fn(Well, &P) -> bool + 'a;

impl<'a, P> DropAccepts<'a, P> {
    /// A test that runs `accepts` on the well and the payload.
    pub fn new(accepts: impl Fn(Well, &P) -> bool + 'a) -> Self {
        DropAccepts(Box::new(accepts))
    }
}

/// A closure that sets up the picker of the popup.
pub struct PickerSetup<'a>(Box<dyn Fn(ColorPicker<'a>) -> ColorPicker<'a> + 'a>);

impl<'a> PickerSetup<'a> {
    /// A setup that runs `setup` on the popup's picker.
    pub fn new(setup: impl Fn(ColorPicker<'a>) -> ColorPicker<'a> + 'a) -> Self {
        PickerSetup(Box::new(setup))
    }
}

/// The well whose popup is open.
#[derive(Clone, Copy, Default)]
struct Opened(Option<u64>);

/// What a swatch pair reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SwatchPairOutput<P = ()> {
    /// The pair's response.
    pub response: Response,
    /// The active well after this pass: the host's, or the one the user chose.
    pub active: Well,
    /// The user made another well active in this pass.
    pub activated: bool,
    /// The well clicked in this pass, by the pointer or by Enter or Space on the focused well (with the popup on,
    /// Enter opens the popup instead). Reported whether or not the click made the well active.
    pub clicked: Option<Well>,
    /// The well double-clicked in this pass.
    pub double_clicked: Option<Well>,
    /// The edits of the picker in the popup, in order; empty while no popup is open.
    pub events: EditEvents<PickedColor>,
    /// The well `events` belong to: the one whose popup is open.
    pub well: Well,
    /// The swap button was pressed.
    pub swap: bool,
    /// The default button was pressed.
    pub defaults: bool,
    /// The picker's screen eyedropper press: the host answers through [`SwatchPair::sampled`].
    pub screen_pick: Option<ScreenPickGesture>,
    /// The well whose popup is open after this pass.
    pub open: Option<Well>,
    /// The edit of the active well: another well made active is one `Set`.
    pub choice: EditResponse<Well>,
    /// A payload released on a well that accepts it: the well and the payload.
    pub dropped: Option<(Well, P)>,
    /// The well a dragged payload that it accepts is over.
    pub over: Option<Well>,
}

impl<P> Outcome for SwatchPairOutput<P> {
    type Value = Well;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.choice.story_facts()
    }
}

/// Two overlapping colour wells, the active one in front, as PhotoCraft's foreground and background chips and
/// DesignCraft's fill and stroke proxy.
///
/// - The host holds the active well and the colours. A click on a well, or an arrow key while one has the focus,
///   makes it active and returns it as [`SwatchPairOutput::active`]; it is drawn in front with a gap in the colour the
///   pair sits on round it. The wells keep their places.
/// - [`frame_active(true)`](SwatchPair::frame_active), as Photoshop's Color panel: the primary well stays in front
///   and the active well, whichever it is, has a frame round it.
/// - A double-click, or Enter on the focused well, opens a popup with the Compact picker, which edits that well's
///   colour (`edit`, `well`, `open`); a click outside closes it.
/// - Host-picker mode, as PhotoCraft's tool rail: [`popover(false)`](SwatchPair::popover) turns the popup off, so a
///   click, or Enter or Space on the focused well, reports `clicked` and a double-click `double_clicked` for the host
///   to open its own picker. `click_activates(false)` keeps the host's active well in front whatever is clicked; the
///   arrow keys then move the focus between the wells without changing it.
/// - The optional buttons ([`swap_button`](SwatchPair::swap_button), [`default_button`](SwatchPair::default_button))
///   are icons in a row over the wells, the default one at the start and the swap one at the end, as Photoshop's.
/// - The wells are one roving group: Tab arrives at the active well; Left and Up go to the primary one, Right and
///   Down to the secondary one (mirrored right to left).
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay, content_painter, probe(SwatchPair<'static, u8>))]
pub struct SwatchPair<'a, P: Clone + Send + Sync + 'static = ()> {
    #[common]
    common: Common,
    wells: [PairWell; 2],
    active: Bind<'a, Well>,
    /// The Swap Colours button's name; without it there is no button.
    #[prop(into)]
    swap_button: Option<Label>,
    /// The Default Colours button's name; without it there is no button.
    #[prop(into)]
    default_button: Option<Label>,
    /// Whether a double-click or Enter opens the popup; off, the pair is in host-picker mode.
    #[prop]
    popover: bool,
    /// Whether a click or an arrow makes a well active.
    #[prop]
    click_activates: bool,
    /// Whether the active well is shown by a frame round it, the primary well staying in front, instead of by being
    /// in front.
    #[prop]
    frame_active: bool,
    /// The host's answer to the picker's screen eyedropper.
    #[prop]
    sampled: Option<Srgba>,
    /// Sets up the popup's picker.
    #[prop(func = "Fn(ColorPicker<'a>) -> ColorPicker<'a>")]
    picker: Option<PickerSetup<'a>>,
    /// The payload each well drags past the slop (`None`: that well drags nothing).
    drags: Option<[Option<P>; 2]>,
    /// What each well accepts of a dragged payload: the well is a drop target for the payloads this allows.
    #[prop(func = "Fn(Well, &P) -> bool")]
    accepts: Option<DropAccepts<'a, P>>,
    dropped: Option<(Well, P)>,
    over: Option<Well>,
    clicked: Option<Well>,
    double_clicked: Option<Well>,
    activated: bool,
    swap: bool,
    defaults: bool,
    edit: EditEvents<PickedColor>,
    edited: Option<Well>,
    screen_pick: Option<ScreenPickGesture>,
}

impl<'a> SwatchPair<'a, ()> {
    /// A pair named `label` of `wells`, primary first; `active` is the well in front.
    pub fn new(label: impl Into<Label>, wells: [PairWell; 2], active: Bind<'a, Well>) -> Self {
        SwatchPair {
            common: Common::new(),
            wells,
            active,
            swap_button: None,
            default_button: None,
            popover: true,
            click_activates: true,
            frame_active: false,
            sampled: None,
            picker: None,
            drags: None,
            accepts: None,
            dropped: None,
            over: None,
            clicked: None,
            double_clicked: None,
            activated: false,
            swap: false,
            defaults: false,
            edit: EditEvents::new(),
            edited: None,
            screen_pick: None,
        }
        .label(label)
    }

    /// The payload each well drags past the slop, primary first (`None`: that well drags nothing); a mixed well drags
    /// nothing whatever it is given. Set before `accepts`, which is typed by the payload.
    pub fn drags<Q: Clone + Send + Sync + 'static>(self, payloads: [Option<Q>; 2]) -> SwatchPair<'a, Q> {
        SwatchPair {
            common: self.common,
            wells: self.wells,
            active: self.active,
            swap_button: self.swap_button,
            default_button: self.default_button,
            popover: self.popover,
            click_activates: self.click_activates,
            frame_active: self.frame_active,
            sampled: self.sampled,
            picker: self.picker,
            drags: Some(payloads),
            accepts: None,
            dropped: None,
            over: None,
            clicked: self.clicked,
            double_clicked: self.double_clicked,
            activated: self.activated,
            swap: self.swap,
            defaults: self.defaults,
            edit: self.edit,
            edited: self.edited,
            screen_pick: self.screen_pick,
        }
    }
}

impl<'a, P: Clone + Send + Sync + 'static> SwatchPair<'a, P> {
    /// The payload well `i` drags: none for a well without one or a mixed one.
    fn payload(&self, i: usize) -> Option<&P> {
        self.drags.as_ref()?[i].as_ref().filter(|_| self.wells[i].paint.same().is_some())
    }

    fn has_buttons(&self) -> bool {
        self.swap_button.is_some() || self.default_button.is_some()
    }

    /// The buttons' rects in logical coordinates from the pair's start edge: default colours at the start, swap at the
    /// end.
    fn button_rects(&self, style: &SwatchPairStyle) -> [Rect; 2] {
        let side = Vec2::splat(style.buttons);
        [Rect::from_min_size(pos2(0.0, 0.0), side), Rect::from_min_size(pos2(self.wells(style).x - style.buttons, 0.0), side)]
    }

    /// The room round a well's edge: its outline, and the frame a gap outside it where the pair frames the active well.
    fn edge(&self, style: &SwatchPairStyle) -> f32 {
        if self.frame_active { 2.0 * style.outline + style.gap } else { style.outline }
    }

    /// The size of the two wells with the room round their edges.
    fn wells(&self, style: &SwatchPairStyle) -> Vec2 {
        Vec2::splat(style.well + style.step + 2.0 * self.edge(style))
    }

    /// The well drawn in front, which takes the pointer where the wells overlap.
    fn front(&self, active: Well) -> Well {
        if self.frame_active { Well::Primary } else { active }
    }

    /// The wells' rects in logical coordinates from the pair's start edge, primary first, inside their outlines.
    fn well_rects(&self, style: &SwatchPairStyle, size: Vec2) -> [Rect; 2] {
        // The buttons' squares hold their glyphs with room round them, so the wells follow without a gap.
        let edge = self.edge(style);
        let top = if self.has_buttons() { style.buttons } else { 0.0 } + edge;
        let square = Vec2::splat(style.well);
        let primary = Rect::from_min_size(pos2(edge, top), square);
        let end = size.x.min(self.wells(style).x) - edge;
        let secondary = Rect::from_min_size(pos2(end - style.well, top + style.step), square);
        [primary, secondary]
    }

    /// The wells' rects inside `rect` on screen, primary first.
    fn placed(&self, style: &SwatchPairStyle, rect: Rect, rtl: bool) -> [Rect; 2] {
        let flow = Flow::new(rect, rtl);
        self.well_rects(style, rect.size()).map(|logical| flow.to_physical(logical.translate(rect.min.to_vec2())))
    }
}

impl<P: Clone + Send + Sync + 'static> Component for SwatchPair<'_, P> {
    const ROLE: Role = Role::GROUP;

    type Style = SwatchPairStyle;
    type Output = SwatchPairOutput<P>;

    fn style(cx: &StyleCx<'_>, _: SwatchPairStyleKey<P>) -> SwatchPairStyle {
        let colors = &cx.tokens.color;
        let well = cx.tokens.size.icon.lg;
        let round = cx.tokens.shape.wells == WellShape::Round;
        let radius = if round { UiTokens::corner(well / 2.0) } else { SwatchMetrics::of(cx).radius };
        SwatchPairStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            well,
            step: cx.tokens.size.icon.sm,
            radius,
            round,
            gap: cx.tokens.space.gap.tight,
            buttons: cx.control().icon,
            edge: cx.color(colors.line.control_edge),
            outline: cx.tokens.stroke.thin,
            frame: cx.color(colors.text.muted),
            mark: cx.color(colors.text.normal),
            none_fill: cx.color(colors.surface.sunken),
            none_mark: cx.color(colors.status.bad.solid),
            swatch: Swatch::style(cx, Swatch::new(Label::default(), Srgba::BLACK).style_key()),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        let wells = self.wells(style);
        if self.has_buttons() { vec2(wells.x, wells.y + style.buttons) } else { wells }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Well>> {
        Some(self.active.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let flow = Flow::new(rect, cx.flow.is_rtl());
        let rects = self.placed(cx.style, rect, cx.flow.is_rtl());
        let active = cx.value().copied().unwrap_or_default();
        let on_top = self.front(active);
        for (i, at) in rects.into_iter().enumerate() {
            let well = Well::of(i as u64);
            let front = well == on_top;
            let sense = if self.payload(i).is_some() { Sense::CLICK | Sense::FOCUS | Sense::DRAG } else { Sense::CLICK | Sense::FOCUS };
            let mut flags = Flags::empty();
            flags.set(Flags::MIXED, self.wells[i].paint.same().is_none());
            flags.set(Flags::CHECKED, well == active);
            let shape = if cx.style.round { HitShape::circle(at.center(), at.width() / 2.0) } else { HitShape::Rect(at) };
            let part =
                cx.part(WELL.at(i as u64), shape).sense(sense).priority(HitPriority::new(HitLevel::Body, u8::from(front))).flags(flags).focusable();
            // Focus arrives at the active well, so arriving makes no other well active.
            let part = if well == active { part.selected() } else { part };
            if self.popover {
                part.pointer_only("Enter opens the picker");
            }
        }
        let buttons = self.button_rects(cx.style);
        for (key, button, shown) in [(DEFAULT, buttons[0], self.default_button.is_some()), (SWAP, buttons[1], self.swap_button.is_some())] {
            if shown {
                let at = flow.to_physical(button.translate(rect.min.to_vec2()));
                cx.part(key, at).sense(Sense::CLICK).pointer_only("The host's Default Colours and Swap Colours commands");
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // Asking registers the popup for this pass, which the overlay hook needs even while it stays closed. A pair in
        // host-picker mode has no popup, so its group offers no Expand.
        if self.popover {
            let _ = cx.popup("picker").is_open(cx);
        }
        let active = cx.value().copied().unwrap_or_default();
        for i in 0..2u64 {
            let well = Well::of(i);
            let press = cx.part(WELL.at(i)).press;
            if matches!(press.phase, PressPhase::DragStart { .. })
                && let Some(payload) = self.payload(i as usize)
            {
                cx.start_drag(payload.clone(), DragPreview::None);
            }
            if let PressPhase::Release { start, click: true, .. } = press.phase {
                self.clicked = Some(well);
                if start.clicks >= 2 {
                    self.double_clicked = Some(well);
                }
                if self.click_activates && well != active {
                    cx.set(well);
                    self.activated = true;
                }
                if self.popover && start.clicks >= 2 {
                    cx.memory::<Opened>().0 = Some(i);
                    let popup = cx.popup("picker");
                    popup.open(cx);
                }
            }
            if cx.part_activated(WELL.at(i)) == Some(FireSource::Key) {
                if self.popover {
                    cx.memory::<Opened>().0 = Some(i);
                    let popup = cx.popup("picker");
                    popup.open(cx);
                } else {
                    self.clicked = Some(well);
                }
            }
        }
        // An Expand on the pair itself reaches it as a keyed activation of its root, with no well activated: it opens
        // the active well's picker.
        let by_well = (0..2u64).any(|i| cx.part_activated(WELL.at(i)).is_some());
        if self.popover && !by_well && cx.activation().is_some_and(|activation| activation.by_key) {
            cx.memory::<Opened>().0 = Some(active.index());
            let popup = cx.popup("picker");
            popup.open(cx);
        }
        self.defaults |= cx.part_activated(DEFAULT).is_some();
        self.swap |= cx.part_activated(SWAP).is_some();
        // Focus moving onto a well (an arrow, a click) makes it active; focus resting on a well leaves the host's
        // choice of another one alone.
        if self.click_activates
            && cx.is_focused()
            && let Some(part) = cx.focus_arrived()
            && part.key == WELL
            && let Some(well) = part.index.map(Well::of)
            && well != active
        {
            cx.set(well);
            self.activated = true;
        }
        if cx.resolved.enabled
            && let Some(DropAccepts(accepts)) = &self.accepts
        {
            let target = WellsTarget {
                rects: self.placed(cx.style, cx.rect, cx.flow.is_rtl()),
                front: self.front(active),
                round: cx.style.round,
                mixed: [0, 1].map(|i| self.wells[i].paint.same().is_none()),
                accepts: &**accepts,
            };
            self.over = cx.dragging::<P>().and_then(|(payload, at)| target.takes(&payload, at));
            self.dropped = cx.drop_target(&target);
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        // Without the popover the pair registered no popup this pass.
        if !self.popover {
            return;
        }
        let popup = cx.popup("picker");
        let opened = cx.memory::<Opened>().0.map_or(Well::Primary, Well::of);
        let well = &self.wells[opened.index() as usize];
        let (color, name) = (solid_of(&well.paint), well.label.clone());
        let setup = self.picker.as_ref();
        let sampled = self.sampled;
        let shown = ui.show(popup, Overlay::popover(), |ui, _| {
            let mut picker = compact_picker(name.clone(), Bind::given(PickedColor::rgb(color)), sampled).alpha(true).eyedropper(true);
            if let Some(setup) = setup {
                picker = (setup.0)(picker);
            }
            picker.show(ui)
        });
        if let Some(picked) = shown {
            self.edit = picked.events;
            self.edited = Some(opened);
            self.screen_pick = picked.screen_pick;
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        for (i, well) in self.wells.iter().enumerate() {
            let value = match well.paint.same() {
                Some(paint) => paint_value(&cx.words::<PaintWords>(), paint),
                None => cx.words::<ChoiceWords>().swatch_mixed.as_str().to_owned(),
            };
            // The two wells are a choice of the active colour: radios, the front one checked, with the colour as value.
            access.node(WELL.at(i as u64), Role::RADIO_BUTTON).value(value);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let rects = self.placed(style, cx.rect, cx.flow.is_rtl());
        let active = cx.value().copied().unwrap_or_default();
        // The back well first, then the front one with a gap round it: being in front, or the frame, shows the active well.
        // The gap is a patch of the surface the pair sits on (a panel, a tool rail's chrome), fill only. The back well's
        // frame is painted before the front well, which covers it where they overlap.
        {
            let halo = UiTokens::corner(f32::from(style.radius.nw) + style.gap);
            let ground = ui.with_style(cx.resolved.variant, |scope| SurfaceStyle {
                radius: halo,
                edge: SurfaceEdge::None,
                shadow: None,
                ..scope.surface(cx.env.surface())
            });
            let front = self.front(active);
            for well in [front.other(), front] {
                let at = rects[well.index() as usize];
                let shown = &self.wells[well.index() as usize];
                if well == front {
                    ui.draw().surface_styled(at.expand(style.gap), &ground);
                }
                match shown.paint.same() {
                    Some(paint) => {
                        let look = WellLook {
                            none_fill: style.none_fill,
                            none_mark: style.none_mark,
                            edge: style.edge,
                            height: style.well,
                            radius: style.radius,
                            ring: shown.ring,
                            swatch: style.swatch.clone(),
                        };
                        paint_well(ui, cx, at, paint, &look);
                    }
                    None => ui.draw().swatch_well(at, style.radius, WellFill::Mixed),
                }
                // A ring well draws its own outlines.
                if shown.paint.same().is_none() || !shown.ring {
                    let draw = ui.draw();
                    draw.stroke(&PaintShape::Rect(at, style.radius), StrokeWidth::Thin, style.mark);
                    draw.stroke(&outline(at, style), StrokeWidth::Thin, style.edge);
                }
                if self.frame_active && well == active {
                    ui.draw().stroke(&frame(at, style), StrokeWidth::Thin, style.frame);
                }
            }
        }
        // The buttons' glyphs keep the control size whatever glyph role the pair's scope sets (a tool rail's).
        if let Some(name) = self.default_button.clone() {
            ui.face(DEFAULT, IconButton::new(IconSpec::glyph(&PairGlyph::Defaults)).label(name).glyphs(GlyphRole::Control));
        }
        if let Some(name) = self.swap_button.clone() {
            ui.face(SWAP, IconButton::new(IconSpec::glyph(&PairGlyph::Swap)).label(name).glyphs(GlyphRole::Control));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SwatchPairOutput<P> {
        let open = cx.recall::<Opened>().0.map(Well::of).filter(|_| !self.edit.is_empty() || self.edited.is_some());
        let (response, active) = (cx.response.clone(), cx.value().copied().unwrap_or_default());
        SwatchPairOutput {
            response,
            active,
            activated: self.activated,
            clicked: self.clicked,
            double_clicked: self.double_clicked,
            events: self.edit,
            well: self.edited.unwrap_or_default(),
            swap: self.swap,
            defaults: self.defaults,
            screen_pick: self.screen_pick,
            open,
            choice: cx.edit_response(),
            dropped: self.dropped,
            over: self.over,
        }
    }
}

/// The one drop target of a pair: it takes a payload on the well under the pointer, the front well first where they
/// overlap, unless that well is mixed or `accepts` refuses the payload.
struct WellsTarget<'t, P> {
    rects: [Rect; 2],
    front: Well,
    round: bool,
    mixed: [bool; 2],
    accepts: &'t dyn Fn(Well, &P) -> bool,
}

impl<P> WellsTarget<'_, P> {
    fn well_at(&self, at: Pos2) -> Option<Well> {
        [self.front, self.front.other()].into_iter().find(|well| {
            let rect = self.rects[well.index() as usize];
            if self.round { at.distance(rect.center()) <= rect.width() / 2.0 } else { rect.contains(at) }
        })
    }

    /// The well that takes `payload` dropped at `at`.
    fn takes(&self, payload: &P, at: Pos2) -> Option<Well> {
        let well = self.well_at(at)?;
        (!self.mixed[well.index() as usize] && (self.accepts)(well, payload)).then_some(well)
    }
}

impl<P: Clone + Send + Sync + 'static> DropTarget<P> for WellsTarget<'_, P> {
    type Drop = (Well, P);

    fn accept(&self, payload: &P, at: Pos2, _: DropOperation) -> Option<(Well, P)> {
        self.takes(payload, at).map(|well| (well, payload.clone()))
    }

    fn mark(&self, drop: &(Well, P)) -> DropMark {
        DropMark::Into(self.rects[drop.0.index() as usize])
    }
}

/// The colour a picker starts from for `paint`: a solid's colour, a gradient's first stop, else black.
fn solid_of(paint: &Mixed<Paint>) -> Srgba {
    match paint.same() {
        Some(Paint::Solid(color)) => *color,
        Some(Paint::Gradient(gradient)) => gradient.stops.first().map_or(Srgba::BLACK, |stop| stop.1),
        _ => Srgba::BLACK,
    }
}

/// A well's value for a screen reader: the hex of a solid, else the kind of paint.
fn paint_value(words: &PaintWords, paint: &Paint) -> String {
    match paint {
        Paint::None => words.no_color.as_str().to_owned(),
        Paint::Solid(color) => color.to_hex(),
        Paint::Gradient(_) => words.gradient.as_str().to_owned(),
        Paint::Pattern(_) => words.pattern.as_str().to_owned(),
        Paint::Picture(_) => words.picture.as_str().to_owned(),
    }
}

/// The frame round the active well `at`: a gap outside its outline, concentric with its corners.
fn frame(at: Rect, style: &SwatchPairStyle) -> PaintShape {
    let out = style.outline + style.gap;
    PaintShape::Rect(at.expand(out), UiTokens::corner(f32::from(style.radius.nw) + out))
}

/// The outline just outside the well `at`, concentric with its corners.
fn outline(at: Rect, style: &SwatchPairStyle) -> PaintShape {
    PaintShape::Rect(at.expand(style.outline), UiTokens::corner(f32::from(style.radius.nw) + style.outline))
}

/// The glyphs of the pair's buttons, drawn on a square grid of `GRID` units; they picture the pair, so they mirror
/// with it.
#[derive(Clone, Copy, craft_ui::contract::MemoProps)]
enum PairGlyph {
    /// Default colours: a black square over a white one, as the wells stand.
    Defaults,
    /// Swap colours: a shaft curving from the primary well's place to the secondary one's, with a head at each end.
    Swap,
}

/// The side of a pair glyph's grid, in units.
const GRID: f32 = 13.0;
/// Defaults: the black square's start-top corner and the white square's, and the side of each.
const BLACK_AT: f32 = 1.0;
const WHITE_AT: f32 = 5.0;
const SQUARE: f32 = 7.0;
/// Swap: the shaft's ends and its control point (it leaves the start across and ends going down), and each head's
/// length and half width.
const SHAFT: [(f32, f32); 3] = [(4.0, 4.0), (9.0, 4.0), (9.0, 9.0)];
const HEAD_LENGTH: f32 = 3.0;
const HEAD_HALF: f32 = 2.8;
/// The points the swap shaft is drawn through.
const SHAFT_STEPS: usize = 10;

impl Glyph for PairGlyph {
    fn directional(&self) -> bool {
        true
    }

    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, mirrored: bool) {
        // Whole pixels, so the squares' edges stay sharp.
        let rect = Rect::from_min_size(rect.min.round_to_pixels(painter.pixels_per_point()), rect.size());
        let unit = rect.width() / GRID;
        let at = |x: f32, y: f32| rect.min + vec2(if mirrored { GRID - x } else { x }, y) * unit;
        match self {
            PairGlyph::Defaults => {
                let square = |from: f32, to: f32| Rect::from_two_pos(at(from, from), at(to, to));
                let corner = CornerRadius::from(unit);
                let edge = Stroke::new(tokens.stroke.thin, color);
                let black = square(BLACK_AT, BLACK_AT + SQUARE);
                // The white square shows only outside the black one and a unit's gap round it: two strips, each
                // painted through a clip, so its outline runs along its outer sides only.
                let clear = BLACK_AT + SQUARE + 1.0;
                let white = square(WHITE_AT, WHITE_AT + SQUARE);
                let strips = [Rect::from_two_pos(at(clear, 0.0), at(GRID, GRID)), Rect::from_two_pos(at(0.0, clear), at(clear, GRID))];
                for strip in strips {
                    let painter = painter.with_clip_rect(strip);
                    painter.rect(white, corner, Color32::from(Srgba::WHITE), edge, StrokeKind::Inside);
                }
                painter.rect(black, corner, Color32::from(Srgba::BLACK), edge, StrokeKind::Inside);
            }
            PairGlyph::Swap => {
                let point = |(x, y): (f32, f32)| pos2(x, y);
                let [start, control, end] = SHAFT.map(point);
                let shaft: Vec<Pos2> = (0..=SHAFT_STEPS)
                    .map(|i| {
                        let t = i as f32 / SHAFT_STEPS as f32;
                        let (a, b) = (1.0 - t, t);
                        let p = start.to_vec2() * a * a + control.to_vec2() * 2.0 * a * b + end.to_vec2() * b * b;
                        at(p.x, p.y)
                    })
                    .collect();
                painter.add(Shape::line(shaft, Stroke::new(tokens.stroke.medium, color)));
                for (tip, out) in [(start, -(control - start).normalized()), (end, (end - control).normalized())] {
                    let side = vec2(-out.y, out.x);
                    let head = [tip + out * HEAD_LENGTH, tip + side * HEAD_HALF, tip - side * HEAD_HALF];
                    let mut head: Vec<Pos2> = head.iter().map(|p| at(p.x, p.y)).collect();
                    if mirrored {
                        head.reverse();
                    }
                    painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
                }
            }
        }
    }
}
