//! [`SwatchPair`]: two overlapping colour wells, foreground over background or fill over stroke.

use craft_behavior::{EditEvents, Flow, HitLevel, HitPriority, IconId, Metrics, Mixed, NavShape, PartKey, PressPhase, Roving, Sense};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::choice::{ChoiceWords, Swatch, SwatchMetrics};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Outcome, Overlay, OverlayUi, Overlays as _,
    PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, Flags, FocusLook, StyleColor, StyleCx};
use egui::{Color32, Rect, Response, Vec2, pos2, vec2};

use super::popover::compact_picker;
use super::{ColorPicker, PickedColor, ScreenPickGesture};

#[cfg(feature = "screen-testing")]
mod stories;

/// One of the wells, `WELL.at(0)` the primary and `WELL.at(1)` the secondary.
const WELL: PartKey = PartKey::new("well");
/// The Default Colours button.
const DEFAULT: PartKey = PartKey::new("default");
/// The Swap Colours button.
const SWAP: PartKey = PartKey::new("swap");
/// A ring well's band, as a share of the well's side.
const RING_SHARE: f32 = 0.3;

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
    color: Mixed<Srgba>,
    ring: bool,
}

impl PairWell {
    /// A well named `label` (its accessible name and tooltip, translated: "Foreground colour") of the host's `color`;
    /// `Mixed::Mixed` for a selection whose fills differ, shown as the mixed well.
    pub fn new(label: impl Into<Label>, color: impl Into<Mixed<Srgba>>) -> Self {
        Self { label: label.into(), color: color.into(), ring: false }
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
    /// The gap in the panel's colour round the front well: `space.gap.tight`.
    gap: f32,
    /// The height of the button row.
    buttons: f32,
    /// The colour the pair sits on, for the gap.
    ground: StyleColor,
    /// The edge of a ring well.
    edge: StyleColor,
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
pub struct SwatchPairOutput {
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
}

impl Outcome for SwatchPairOutput {
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
#[component(overlay)]
pub struct SwatchPair<'a> {
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
    /// The host's answer to the picker's screen eyedropper.
    #[prop]
    sampled: Option<Srgba>,
    /// Sets up the popup's picker.
    #[prop(func = "Fn(ColorPicker<'a>) -> ColorPicker<'a>")]
    picker: Option<PickerSetup<'a>>,
    clicked: Option<Well>,
    double_clicked: Option<Well>,
    activated: bool,
    swap: bool,
    defaults: bool,
    edit: EditEvents<PickedColor>,
    edited: Option<Well>,
    screen_pick: Option<ScreenPickGesture>,
}

impl<'a> SwatchPair<'a> {
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
            sampled: None,
            picker: None,
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

    fn has_buttons(&self) -> bool {
        self.swap_button.is_some() || self.default_button.is_some()
    }

    /// The buttons' rects in logical coordinates from the pair's start edge: default colours at the start, swap at the
    /// end.
    fn button_rects(style: &SwatchPairStyle) -> [Rect; 2] {
        let side = Vec2::splat(style.buttons);
        [Rect::from_min_size(pos2(0.0, 0.0), side), Rect::from_min_size(pos2(style.well + style.step - style.buttons, 0.0), side)]
    }

    /// The wells' rects in logical coordinates from the pair's start edge, primary first.
    fn well_rects(&self, style: &SwatchPairStyle, size: Vec2) -> [Rect; 2] {
        let top = if self.has_buttons() { style.buttons + style.metrics.gap } else { 0.0 };
        let square = Vec2::splat(style.well);
        let primary = Rect::from_min_size(pos2(0.0, top), square);
        let secondary = Rect::from_min_size(pos2(size.x.min(style.well + style.step) - style.well, top + style.step), square);
        [primary, secondary]
    }
}

impl Component for SwatchPair<'_> {
    const ROLE: Role = Role::GROUP;

    type Style = SwatchPairStyle;
    type Output = SwatchPairOutput;

    fn style(cx: &StyleCx<'_>, _: SwatchPairStyleKey) -> SwatchPairStyle {
        let colors = &cx.tokens.color;
        SwatchPairStyle {
            metrics: Metrics::new().gap(cx.control().gap),
            focus: cx.focus_look(),
            well: cx.tokens.size.icon.lg,
            step: cx.tokens.size.icon.sm,
            gap: cx.tokens.space.gap.tight,
            buttons: SwatchMetrics::of(cx).side,
            ground: cx.color(colors.surface.panel),
            edge: cx.color(colors.line.control_edge),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        let wells = Vec2::splat(style.well + style.step);
        if self.has_buttons() { vec2(wells.x, wells.y + style.buttons + style.metrics.gap) } else { wells }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Well>> {
        Some(self.active.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let flow = Flow::new(rect, cx.flow.is_rtl());
        let rects = self.well_rects(cx.style, rect.size());
        let active = cx.value().copied().unwrap_or_default();
        for (i, logical) in rects.into_iter().enumerate() {
            let front = Well::of(i as u64) == active;
            let at = flow.to_physical(logical.translate(rect.min.to_vec2()));
            let mut flags = Flags::empty();
            flags.set(Flags::MIXED, self.wells[i].color.same().is_none());
            flags.set(Flags::CHECKED, front);
            let part = cx
                .part(WELL.at(i as u64), at)
                .sense(Sense::CLICK | Sense::FOCUS)
                .priority(HitPriority::new(HitLevel::Body, u8::from(front)))
                .flags(flags)
                .focusable();
            if self.popover {
                part.pointer_only("Enter opens the picker");
            }
        }
        let buttons = Self::button_rects(cx.style);
        for (key, button, shown) in [(DEFAULT, buttons[0], self.default_button.is_some()), (SWAP, buttons[1], self.swap_button.is_some())] {
            if shown {
                let at = flow.to_physical(button.translate(rect.min.to_vec2()));
                cx.part(key, at).sense(Sense::CLICK).pointer_only("The host's Default Colours and Swap Colours commands");
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // Asking registers the popup for this pass, which the overlay hook needs even while it stays closed.
        let _ = cx.popup("picker").is_open(cx);
        let active = cx.value().copied().unwrap_or_default();
        for i in 0..2u64 {
            let well = Well::of(i);
            let press = cx.part(WELL.at(i)).press;
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
        self.defaults |= cx.part_activated(DEFAULT).is_some();
        self.swap |= cx.part_activated(SWAP).is_some();
        if self.click_activates
            && cx.is_focused()
            && let Some(part) = cx.active_part()
            && part.key == WELL
            && let Some(well) = part.index.map(Well::of)
            && well != active
        {
            cx.set(well);
            self.activated = true;
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("picker");
        let opened = cx.memory::<Opened>().0.map_or(Well::Primary, Well::of);
        let well = &self.wells[opened.index() as usize];
        let (color, name) = (*well.color.or(&Srgba::BLACK), well.label.clone());
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
            let value = match well.color.same() {
                Some(color) => color.to_hex(),
                None => cx.words::<ChoiceWords>().swatch_mixed.as_str().to_owned(),
            };
            // The two wells are a choice of the active colour: radios, the front one checked, with the colour as value.
            access.node(WELL.at(i as u64), Role::RADIO_BUTTON).value(value);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let flow = Flow::new(cx.rect, cx.flow.is_rtl());
        let rects = self.well_rects(style, cx.rect.size());
        let active = cx.value().copied().unwrap_or_default();
        let place = |logical: Rect| flow.to_physical(logical.translate(cx.rect.min.to_vec2()));
        // The back well first, then the front one with a gap of the ground colour round it.
        for well in [active.other(), active] {
            let at = place(rects[well.index() as usize]);
            let shown = &self.wells[well.index() as usize];
            if well == active {
                ui.draw().fill(&PaintShape::Rect(at.expand(style.gap), egui::CornerRadius::ZERO), style.ground);
            }
            match shown.color.same() {
                Some(color) if shown.ring => paint_ring(ui, at, *color, style),
                _ => ui.face(WELL.at(well.index()), Swatch::new(shown.label.clone(), shown.color).selected(well == active)),
            }
        }
        if let Some(name) = self.default_button.clone() {
            ui.face(DEFAULT, IconButton::new(IconId::new("default_colors")).label(name));
        }
        if let Some(name) = self.swap_button.clone() {
            ui.face(SWAP, IconButton::new(IconId::new("swap_colors")).label(name));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SwatchPairOutput {
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
        }
    }
}

/// A ring well: a band of the colour round the edge, the middle open to what is behind.
fn paint_ring(ui: &mut PaintUi<'_>, at: Rect, color: Srgba, style: &SwatchPairStyle) {
    let band = at.width() * RING_SHARE;
    let fill = DataColor::new(Color32::from(color));
    let draw = ui.draw();
    let bands = [
        Rect::from_min_max(at.min, pos2(at.max.x, at.min.y + band)),
        Rect::from_min_max(pos2(at.min.x, at.max.y - band), at.max),
        Rect::from_min_max(pos2(at.min.x, at.min.y + band), pos2(at.min.x + band, at.max.y - band)),
        Rect::from_min_max(pos2(at.max.x - band, at.min.y + band), pos2(at.max.x, at.max.y - band)),
    ];
    for part in bands {
        draw.data_fill(&PaintShape::Rect(part, egui::CornerRadius::ZERO), fill);
    }
    draw.stroke(&PaintShape::Rect(at, egui::CornerRadius::ZERO), StrokeWidth::Thin, style.edge);
}
