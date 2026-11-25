//! [`ColorHud`]: the colour area and hue strip in a popover at a point, driven by the host's press.

use super::strip;
use super::{ColorArea, ColorStrip};
use crate::AreaMode;
use craft_behavior::{EditEvent, EditEvents, Metrics, PartKey, Sense};
use craft_theme::Srgba;
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, EscapeEdit, MeasureCx, Outcome, POPUP_PAD, PaintUi, PartsCx,
    Pattern, Props as _, Role, Show as _, ShowContainer as _, pad_of,
};
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::pointer;
use craft_ui::style::{ComponentStyle, StyleCx, UiTokens};
use egui::{Pos2, Rect, Response, Ui, Vec2, pos2, vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The model the HUD picks in: saturation and brightness across and down, hue in the strip.
const MODE: AreaMode = AreaMode::Hue;

/// The area's part of the HUD's content: painted as the look of `ColorArea`, which senses nothing.
const AREA: PartKey = PartKey::new("area");

/// The strip's part of the HUD's content: painted as the look of `ColorStrip`.
const STRIP: PartKey = PartKey::new("strip");

/// The colour of a HUD whose binding shows none.
const BLACK: Srgba = Srgba::rgb(0.0, 0.0, 0.0);

/// The sizes of the HUD's content, from tokens, so that nothing is measured in an earlier pass: the area is a plot
/// square, the strip stands to its right.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    plot: f32,
    strip_width: f32,
    gap: f32,
}

impl Geometry {
    fn of(tokens: &UiTokens) -> Self {
        // The strip's colours are `size.icon.lg` wide with a pointer triangle's room on each side (its own padding).
        Geometry { plot: tokens.size.plot.width, strip_width: tokens.size.icon.lg + 2.0 * tokens.size.handle.sm, gap: tokens.space.gap.inline }
    }

    fn size(self) -> Vec2 {
        vec2(self.plot + self.gap + self.strip_width, self.plot)
    }

    /// The area's rect in content whose top left is `min`.
    fn area(self, min: Pos2) -> Rect {
        Rect::from_min_size(min, Vec2::splat(self.plot))
    }

    /// The strip's rect in content whose top left is `min`.
    fn strip(self, min: Pos2) -> Rect {
        Rect::from_min_size(pos2(min.x + self.plot + self.gap, min.y), vec2(self.strip_width, self.plot))
    }
}

/// Where the start colour's point stands in the area, relative to the content's top left.
fn start_point(geometry: Geometry, start: Option<Srgba>) -> Pos2 {
    let area = geometry.area(Pos2::ZERO);
    match start {
        Some(color) => {
            let values = MODE.model().from_srgb(color);
            ColorArea::new("", MODE, Bind::given(values)).point_in(area, &values)
        }
        None => area.center(),
    }
}

/// The popover's top left, so that the start colour's point in the area sits at `at`.
fn origin(tokens: &UiTokens, at: Pos2, start: Option<Srgba>) -> Pos2 {
    let point = start_point(Geometry::of(tokens), start);
    let pad = pad_of(tokens, POPUP_PAD);
    pos2(at.x - pad - point.x, at.y - pad - point.y)
}

/// The area and hue strip in a popover at `at`, with the start colour under it, driven by the host's press: the pick
/// maps the runtime's pass pointer through `ColorArea` and the strip's `value_at`, on a pass the pointer moved in
/// (a rerun pass of the frame has no move and picks nothing); no pick while the pointer is still `at`; one gesture
/// per open; Escape or `held(false)` commits and closes. The HUD keeps no state.
///
/// The host shows the HUD while its press is held and stops showing it once `closed` is reported.
#[must_use = "show the HUD with `show`"]
#[derive(Pattern)]
pub struct ColorHud<'a> {
    label: Label,
    at: Pos2,
    color: Bind<'a, Srgba>,
    start: Option<Srgba>,
    held: bool,
}

impl<'a> ColorHud<'a> {
    /// A HUD named `label` at `at` over `color`.
    pub fn new(label: impl Into<Label>, at: Pos2, color: Bind<'a, Srgba>) -> Self {
        ColorHud { label: label.into(), at, color, start: None, held: true }
    }

    /// The colour under `at` when the host's press began; the popover is placed so this colour's point sits at `at`.
    /// Without it the area's middle sits at `at`.
    pub fn start(mut self, color: Srgba) -> Self {
        self.start = Some(color);
        self
    }

    /// Whether the host's press is still held.
    pub fn held(mut self, held: bool) -> Self {
        self.held = held;
        self
    }

    /// Draws the HUD.
    pub fn show(self, ui: &mut Ui) -> ColorHudOutput {
        let ColorHud { label, at, mut color, start, held } = self;
        let origin = origin(&Env::of(ui).tokens, at, start);
        let shown = Popover::new(label.clone(), Bind::given(true)).at_point(origin).initial_focus(Initial::First).show(ui, |ui, body| {
            // A released press closes the HUD; the pick below commits in this same pass.
            if !held {
                body.request_close();
            }
            let live = held && body.is_open();
            let edit = HudPick::new(label.clone(), color.reborrowed(), at, live).show(ui);
            // A commit while the press is held is Escape's: the picked colour stays and the popover closes.
            if held && (&edit.events).into_iter().any(|event| matches!(event, EditEvent::Commit { .. })) {
                body.request_close_as(CloseReason::Escape);
            }
            edit
        });
        let events = match shown.inner {
            Some(edit) => edit.events,
            None => EditEvents::new(),
        };
        ColorHudOutput { response: shown.output.response, events, closed: shown.output.closed }
    }
}

/// What the HUD reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorHudOutput {
    /// The popover's response.
    pub response: Response,
    /// The edit events of the colour.
    pub events: EditEvents<Srgba>,
    /// Why it closed this pass.
    pub closed: Option<CloseReason>,
}

impl Outcome for ColorHudOutput {
    type Value = Srgba;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.response.story_facts()
    }
}

/// The style of the HUD's picking content: its sizes.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct HudPickStyle {
    #[style(metrics)]
    metrics: Metrics,
    plot: f32,
    strip_width: f32,
    gap: f32,
}

impl HudPickStyle {
    fn geometry(&self) -> Geometry {
        Geometry { plot: self.plot, strip_width: self.strip_width, gap: self.gap }
    }
}

/// The HUD's value component: it shows the area and the strip and owns the colour's gesture. The gesture slot belongs
/// to a value component, which drives it from the pass pointer and its props (the host's press) instead of a press of its own:
/// the first pick begins it, later picks update it, and the host's release or the popover's close commits it. Escape
/// commits too, so the colour stays. It is the popover's one focus stop, so Escape reaches it; the area and strip are
/// the looks of its two parts and take no input.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
// Unregistered: the component suite drives a component through its own keys, presses and assistive actions, and this one
// takes its input only from the pass pointer and `ColorHud`'s props (the host's press). `ColorHud`'s story and the HUD trials
// in `tests/it/color_parts.rs` cover it, and its faces are the registered `ColorArea` and `ColorStrip`.
#[component(unregistered)]
struct HudPick<'a> {
    #[common]
    common: Common,
    color: Bind<'a, Srgba>,
    at: Pos2,
    /// Whether the press is held and the popover open.
    live: bool,
}

impl<'a> HudPick<'a> {
    fn new(label: impl Into<Label>, color: Bind<'a, Srgba>, at: Pos2, live: bool) -> Self {
        HudPick { common: Common::new(), color, at, live }.label(label)
    }
}

impl Component for HudPick<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ESCAPE: EscapeEdit = EscapeEdit::Commit;
    const OVERLAYS: bool = false;
    type Style = HudPickStyle;
    type Output = EditResponse<Srgba>;

    fn style(cx: &StyleCx<'_>, _: HudPickStyleKey) -> HudPickStyle {
        let geometry = Geometry::of(cx.tokens);
        HudPickStyle { metrics: Metrics::new(), plot: geometry.plot, strip_width: geometry.strip_width, gap: geometry.gap }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let geometry = cx.style.geometry();
        cx.part(AREA, geometry.area(rect.min));
        cx.part(STRIP, geometry.strip(rect.min));
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Geometry::of(cx.tokens).size()
    }

    fn bind(&mut self) -> Option<BindMut<'_, Srgba>> {
        Some(self.color.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let running = cx.gesture().is_some();
        if !self.live {
            if running {
                cx.commit();
            }
            return;
        }
        // Only a pass the pointer moved in picks: a rerun pass of the frame has no move. The press that opened the HUD
        // picks nothing until the pointer leaves `at`, unless a gesture already runs.
        let moved = pointer::with_pass_pointer(&cx.ctx, |pass| pass.stirred.then_some(pass.hover).flatten());
        let Some(pointer) = moved.filter(|pointer| *pointer != self.at || running) else { return };
        let geometry = cx.style.geometry();
        let (area, strip) = (geometry.area(cx.rect.min), geometry.strip(cx.rect.min));
        let current = cx.value().copied().unwrap_or(BLACK);
        let values = MODE.model().from_srgb(current);
        let picked = if area.contains(pointer) {
            Some(ColorArea::new("", MODE, Bind::given(values)).value_at(area, &values, pointer))
        } else if strip.contains(pointer) {
            Some(strip::value_at(strip, MODE, &values, pointer))
        } else {
            None
        };
        let Some(picked) = picked else { return };
        let mut next = MODE.model().to_srgb(&picked.values);
        next.a = current.a;
        if next != current {
            cx.propose(next);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let model = MODE.model();
        let values = model.from_srgb(cx.value().copied().unwrap_or(BLACK));
        ui.face(AREA, ColorArea::new(Label::from(MODE.name()), MODE, Bind::given(values)));
        ui.face(STRIP, ColorStrip::new(Label::from(model.channels()[MODE.strip()].name.to_owned()), MODE, Bind::given(values)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Srgba> {
        cx.edit_response()
    }
}
