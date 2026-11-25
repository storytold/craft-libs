//! `contract::tooltip`: the one step-16 function that shows every tooltip (3.4, 15.5).
//!
//! A tooltip shows after one delay while the pointer rests on the component, or on keyboard focus after the same
//! delay, and at once while a neighbour's tooltip was hiding a moment ago. A press, a menu that owns keys and a
//! running drag hide it. A press on the component keeps its tooltip hidden until the pointer leaves it, so what the
//! press opens (a popover under a swatch) is not covered.
//!
//! A component with no tooltip shows its own label as the tooltip when its type declares `Component::SHOWS_LABEL`
//! false (or an instance does, through `Component::shows_label`): no text paints that name, inside a field or not. A
//! component with no label of its own is named by its field label, which is on screen, and shows none.
//! A component that paints its label shows it whole as the tooltip when the paint cut it. A canvas control shows no
//! label as its tooltip: its rect is the canvas, and its parts carry their own tooltips.

use super::cx::{Decls, PartExtras};
use super::overlay::{self, Anchor, Layer, Overlay, Place};
use super::{Common, Component, Cx, Props as _, Show, TipFn, Tooltip, TooltipAt, TooltipSpec, TooltipWhen};
use crate::primitives::sensing::PartDecl;
use crate::primitives::{TextRun, Wrap};
use crate::runtime::access::scope::LabelPatch;
use crate::runtime::access::{Access, Name};
use crate::runtime::egui::memory as store;
use crate::runtime::egui::{areas, pass, repaint};
use crate::runtime::env::Env;
use crate::runtime::memory::{self, sub};
use crate::runtime::motion;
use crate::runtime::pass_state;
use crate::runtime::{drag, frame, pointer};
use crate::style::{ComponentStyle, Flags, StyleColor, StyleCx, SurfaceKind, SurfaceLook};
use craft_behavior::PartRef;
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{Metrics, OwnerClass};
use craft_theme::Stateful;
use egui::{Rect, Ui, Vec2};

/// Seconds the pointer rests on a component before its tooltip shows.
pub const DELAY: f64 = 0.6;
/// Seconds after a tooltip hides during which the next one shows at once.
pub const GRACE: f64 = 0.3;

#[derive(Clone, Copy, Default)]
struct TipState {
    since: Option<f64>,
    shown: bool,
    /// The part the tooltip belongs to; `None` for the component's own.
    subject: Option<PartRef>,
    /// A press on the subject hid the tooltip; it stays hidden until neither the pointer nor the keyboard is on it.
    dismissed: bool,
}

/// When the last tooltip of the context hid.
#[derive(Default)]
pub(crate) struct Grace(Option<f64>);

/// What step 16 hands the tooltip: the component's facts and the pass's `Ui`.
pub(crate) struct TipIn<'a> {
    pub ui: &'a Ui,
    pub env: &'a Env,
    /// The rich content closure of `show_with_tip`.
    pub rich: Option<TipFn<'a>>,
    /// The parts the component declared this pass, with their tooltips.
    pub parts: &'a Decls,
    /// The instance's `shows_label` fact.
    pub shows_label: bool,
    /// The type is a canvas control (`Meta::CANVAS_CONTROL`): its label is never its tooltip.
    pub canvas_control: bool,
    /// The validity message no enclosing container shows as its status line (step 16's report).
    pub message: Option<String>,
}

/// The text a tooltip shows for `spec`: the disabled text while disabled.
fn spec_text(ctx: &egui::Context, env: &crate::runtime::env::EnvView<'_>, spec: &TooltipSpec, enabled: bool) -> String {
    count_text(ctx);
    resolve_spec(env, spec, enabled)
}

/// Counts a tooltip text built in this pass.
fn count_text(ctx: &egui::Context) {
    #[cfg(feature = "testing")]
    crate::runtime::measure::count_tip_text(ctx);
    #[cfg(not(feature = "testing"))]
    let _ = ctx;
}

/// [`spec_text`] without the count, for a builder that counted its text already, or one that is no tooltip (a
/// component's `tooltip_text`).
pub(crate) fn resolve_spec(env: &crate::runtime::env::EnvView<'_>, spec: &TooltipSpec, enabled: bool) -> String {
    let label = match (&spec.disabled_text, enabled) {
        (Some(disabled), false) => disabled,
        _ => &spec.text,
    };
    env.resolve(label).as_str().to_owned()
}

/// The text the component's own tooltip shows, or `None` when it has none: its validity message when no container
/// shows it (`message`); else a base text with the bound command's chord, and the bound command's hint on a line
/// after it; else, with no base text, the hint alone with the chord. The base is the tooltip's text, or its disabled
/// text while disabled; else the full label when the paint cut it, or its own label when the component declares it
/// paints no label (`shows_label`), its face dropped the label (`PaintRecord::label_dropped`) or its command has a
/// hint that is not shown alone (`CommandState::hint_alone`).
///
/// A canvas control (`canvas_control`) never shows its label: its rect is the canvas it is shown over, not a face of
/// its own, so the label would show wherever the pointer rests on the page ("Rulers margins" over a whole page). Its
/// parts name themselves with their own tooltips (a margin edge's "Left Margin").
fn text_of(core: &super::CxCore<'_>, shows_label: bool, canvas_control: bool, message: Option<&str>) -> Option<String> {
    count_text(&core.ctx);
    if let Some(message) = message {
        return Some(message.to_owned());
    }
    let hint = core.command.as_deref().and_then(|command| command.hint.as_deref()).filter(|hint| !hint.is_empty());
    let base = match core.resolved.tooltip.map(|tooltip| tooltip.spec()) {
        Some(spec) => Some(resolve_spec(&core.env, spec, core.resolved.enabled)),
        None if canvas_control => return None,
        None => {
            let paint = core.paint.borrow();
            // A hint shown alone (`CommandState::hint_alone`) does not bring the label that shows whole with it.
            let alone = hint.is_some() && core.command.as_deref().is_some_and(|command| command.hint_alone);
            let unpainted = !shows_label || paint.label_dropped || (hint.is_some() && !alone);
            paint.elided.clone().or_else(|| (unpainted && !core.resolved.label.is_empty()).then(|| core.resolved.label.to_owned()))
        }
    };
    let command = core.command.as_deref();
    let platform = crate::runtime::windows::os_of(&core.ctx);
    let chorded = |text: &str| match command.and_then(|command| command.chord.as_ref()) {
        Some(chord) => with_chord(text, chord, platform),
        None => text.to_owned(),
    };
    match (base, hint) {
        (Some(base), Some(hint)) => Some(format!("{}\n{hint}", chorded(&base))),
        (Some(base), None) => Some(chorded(&base)),
        (None, Some(hint)) => Some(chorded(hint)),
        (None, None) => None,
    }
}

/// A tooltip's text followed by the chord that does the same, as every tooltip shows a key: "Hide Layers (⌘,)".
pub fn with_chord(base: &str, chord: &craft_keymap::Chord, os: craft_keymap::Platform) -> String {
    format!("{base} ({})", chord.display(os))
}

/// The tooltip of a member of a collection's run (`Collection::MEMBERS`) the pointer rests on: shown by the
/// collection's step 16 as a part's, under the collection's timer, with the member's part as its subject.
pub(crate) struct MemberTip {
    pub part: PartRef,
    pub spec: TooltipSpec,
    pub rect: Rect,
    pub pressed: bool,
}

/// A part's tooltip, for the part the pointer is over or the keyboard is on.
struct PartTip<'a> {
    part: PartRef,
    spec: &'a TooltipSpec,
    rect: Rect,
    hovered: bool,
}

/// The part whose tooltip shows: the hovered one with a tooltip, else the keyboard's active part.
fn part_tip<'a>(core: &super::CxCore<'_>, parts: &'a Decls) -> Option<PartTip<'a>> {
    let tip = |decl: &PartDecl, extras: &'a PartExtras, hovered: bool| {
        let part = PartRef { key: decl.key, index: decl.index };
        let rect = core.results.get(&(part.key, part.index))?.rect;
        Some(PartTip { part, spec: extras.tooltip.as_ref()?.spec(), rect, hovered })
    };
    let hovered = parts.iter().find_map(|(decl, extras)| {
        let result = core.results.get(&(decl.key, decl.index))?;
        if result.hovered { tip(decl, extras, true) } else { None }
    });
    hovered.or_else(|| {
        let active = core.active.filter(|_| core.state.flags.contains(Flags::FOCUS_VISIBLE))?;
        parts.iter().find(|(decl, _)| decl.key == active.key && decl.index == active.index).and_then(|(decl, extras)| tip(decl, extras, false))
    })
}

/// Step 16, after the component's report: advances the component's tooltip timer and shows its tooltip, or the
/// tooltip of the part the pointer or the keyboard is on.
pub(crate) fn run(core: &super::CxCore<'_>, input: TipIn<'_>) {
    // A hovered part's tooltip, else the hovered member's, else the keyboard's active part's.
    let part = match (part_tip(core, input.parts), &core.member_tip) {
        (Some(tip), _) if tip.hovered => Some(tip),
        (_, Some(member)) => Some(PartTip { part: member.part, spec: &member.spec, rect: member.rect, hovered: true }),
        (tip, None) => tip,
    };
    let member_pressed =
        core.member_tip.as_ref().filter(|member| part.as_ref().is_some_and(|tip| tip.part == member.part)).map(|member| member.pressed);
    let keyboard = core.state.flags.contains(Flags::FOCUS_VISIBLE);
    let (spec, rect, hovered, subject) = match &part {
        Some(part) => (Some(part.spec), part.rect, part.hovered || keyboard, Some(part.part)),
        None => (core.resolved.tooltip.map(Tooltip::spec), core.rect, core.response.hovered() || keyboard, None),
    };
    let (shows_label, canvas_control, message) = (input.shows_label, input.canvas_control, input.message.as_deref());
    let text = || match &part {
        Some(part) => Some(spec_text(&core.ctx, &core.env, part.spec, core.resolved.enabled)),
        None => text_of(core, shows_label, canvas_control, message),
    };
    let call = TipCall {
        ctx: &core.ctx,
        owner: core.memory.owner(),
        id: core.id,
        env: input.env,
        layer: input.ui.layer_id(),
        rect,
        text: &text,
        rich: if part.is_some() { None } else { input.rich },
        spec,
        at_pointer: false,
        wants_input: hovered,
        pressed: member_pressed.unwrap_or_else(|| core.press.phase.is_press()),
        subject,
        describes: subject.is_none().then_some(core.id),
    };
    show_tip(call);
}

/// Shows `tooltip` for a raw egui response during the app migration: the same delay, grace and placement as a
/// component's tooltip, none of its pipeline.
pub fn attach(response: &egui::Response, tooltip: &Tooltip) {
    let ctx = &response.ctx;
    let Some(env) = Env::root_of(ctx) else { return };
    memory::register_owner(ctx, response.id);
    let spec = tooltip.spec();
    let text = || Some(spec_text(ctx, &env.view(), spec, response.enabled()));
    let call = TipCall {
        ctx,
        owner: response.id,
        id: response.id,
        env: &env,
        layer: response.layer_id,
        rect: response.rect,
        text: &text,
        rich: None,
        spec: Some(spec),
        at_pointer: false,
        wants_input: response.hovered() || response.has_focus(),
        pressed: crate::runtime::egui::response::is_pressed(response),
        subject: None,
        describes: Some(response.id),
    };
    show_tip(call);
}

/// Shows `tip` at the pointer for a surface whose tips have no part of their own (a canvas tool's hover tip): the same
/// delay, grace, look and hiding as a component's tooltip, placed at the pointer whatever the tip's `at` says.
///
/// `owner` is the component whose memory keeps the timer. `at` is where the pointer rests in `ui`'s layer, `None` once
/// it left; `pressed` is a press or drag that hides the tip. With no tip, a tip shown hides.
pub fn at_pointer(ui: &Ui, owner: egui::Id, tip: Option<&Tooltip>, at: Option<egui::Pos2>, pressed: bool) {
    let ctx = ui.ctx();
    let env = Env::of(ui);
    let spec = tip.map(Tooltip::spec);
    let text = || spec.map(|spec| spec_text(ctx, &env.view(), spec, true));
    let call = TipCall {
        ctx,
        owner,
        id: sub(owner, "pointer tip"),
        env: &env,
        layer: ui.layer_id(),
        rect: Rect::from_center_size(at.unwrap_or_default(), Vec2::ZERO),
        text: &text,
        rich: None,
        spec,
        at_pointer: true,
        wants_input: at.is_some(),
        pressed,
        subject: None,
        describes: Some(owner),
    };
    show_tip(call);
}

/// What `show_tip` shows a tooltip from.
struct TipCall<'a> {
    ctx: &'a egui::Context,
    /// Whose memory holds the timer: the component, or the raw response.
    owner: egui::Id,
    id: egui::Id,
    env: &'a Env,
    /// The layer the rect is in.
    layer: areas::Layer,
    /// The rect the tooltip hangs from, in the layer's space.
    rect: Rect,
    /// Builds the text; `show_tip` calls it only for a tooltip that is wanted or shown.
    text: &'a dyn Fn() -> Option<String>,
    rich: Option<TipFn<'a>>,
    spec: Option<&'a TooltipSpec>,
    /// The tooltip shows at the pointer whatever its spec's `at` says. Such a tip has no subject a press could
    /// dismiss it from: it hides only while the press lasts.
    at_pointer: bool,
    /// The pointer rests on the subject or the keyboard is on it.
    wants_input: bool,
    pressed: bool,
    subject: Option<PartRef>,
    /// The node the tooltip describes; `None` for a part's tooltip.
    describes: Option<egui::Id>,
}

fn show_tip(call: TipCall<'_>) {
    let TipCall { ctx, owner, id, env, layer: rect_layer, rect, text: build, rich, spec, at_pointer, wants_input, pressed, subject, describes } =
        call;
    // The host turned tooltips off: none shows, and one that was showing goes as it does when the pointer leaves.
    let wants_input = wants_input && env.tooltips;
    let state_id = sub(id, "tip_state");
    let built = std::cell::OnceCell::new();
    let text = || built.get_or_init(build).as_deref();
    let wants = wants_input && (rich.is_some() || text().is_some());
    // A component no pointer or keyboard wants a tooltip of, with no timer or fading layer left from before, has
    // nothing to advance.
    if !wants && !store::exists::<TipState>(ctx, state_id) {
        return;
    }
    let now = frame::clock(ctx);
    let mut state = store::take::<TipState>(ctx, state_id).unwrap_or_else(|| {
        memory::adopt::<TipState>(ctx, owner, state_id);
        TipState::default()
    });
    if state.subject != subject {
        state = TipState { subject, ..TipState::default() };
    }
    let when_now = spec.is_some_and(|spec| spec.when == TooltipWhen::Now);
    let screen = areas::layer_to_screen(ctx, rect_layer) * rect;
    if !wants_input {
        state.dismissed = false;
    } else if pressed && !at_pointer {
        state.dismissed = true;
    }
    let suppressed = pressed || state.dismissed || overlay_blocks(ctx, screen.center()) || pointer::press_captured(ctx) || drag::running(ctx);
    if wants {
        state.since.get_or_insert(now);
    } else {
        state.since = None;
    }
    let due = state.since.map(|since| since + if when_now { 0.0 } else { DELAY });
    let in_grace = pass_state::with::<Grace, _>(ctx, |grace| grace.0.is_some_and(|hid| now - hid < GRACE));
    let show = wants && !suppressed && due.is_some_and(|due| now >= due || in_grace);
    if let (Some(due), true, false) = (due, wants, show)
        && !suppressed
        && due > now
    {
        repaint::request_after(ctx, pass::current_viewport(ctx), due - now);
    }
    if state.shown && !show {
        pass_state::with::<Grace, _>(ctx, |grace| grace.0 = Some(now));
    }
    state.shown = show;
    let tip_id = sub(id, "tip");
    let presence = motion::layer_presence(ctx, tip_id, owner, env.motion, show, overlay_motion());
    if state.shown || state.since.is_some() || state.dismissed || presence.visible {
        store::put(ctx, state_id, state);
    }
    if !presence.visible {
        overlay::forget_layer(ctx, tip_id, owner);
        return;
    }
    let at_pointer = at_pointer || spec.is_some_and(|spec| spec.at == TooltipAt::Pointer);
    let place = if at_pointer {
        Place::anchored(Anchor::Pointer, Side::Below, Align::Start)
    } else {
        Place::anchored(Anchor::Rect(rect), Side::Below, Align::Start)
    };
    let tooltip = Overlay::tooltip().place(place);
    let layer = Layer {
        id: tip_id,
        owner,
        overlay: &tooltip,
        anchor: areas::layer_to_screen(ctx, rect_layer) * rect,
        env,
        exiting: !show,
        fade: presence.t,
        size: None,
        region: None,
    };
    let mut rich = rich;
    overlay::host(ctx, &layer, &mut |ui| match rich.take() {
        Some(content) => content(ui),
        None => {
            if let Some(text) = text() {
                let tip = TipText { common: Common::default(), text }.id_salt(tip_id).show(ui);
                if let Some(node) = describes {
                    // The described control's node was written earlier in the pass; the relation joins it.
                    let patch = LabelPatch::DescribedBy { node, label: tip.id };
                    crate::runtime::egui::access::write_label_patch(ctx, &patch);
                }
            }
        }
    });
}

fn overlay_motion() -> motion::MotionSpec {
    motion::spec::OPEN_FADE
}

/// Whether a menu owns the keys: tooltips stay hidden then.
/// An open menu blocks the tooltips of what lies outside it; its own rows keep theirs.
fn overlay_blocks(ctx: &egui::Context, at: egui::Pos2) -> bool {
    crate::runtime::overlay::class_open_away_from(ctx, OwnerClass::Menu, at)
}

/// The look of a tooltip card.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TipStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
}

/// The text of a tooltip, on a popup surface.
#[derive(crate::contract::Component)]
#[component(sample = TipText::sample)]
pub(crate) struct TipText<'a> {
    #[common]
    common: Common,
    text: &'a str,
}

impl TipText<'static> {
    fn sample() -> Self {
        TipText { common: Common::default(), text: "Hides the layer" }
    }
}

impl Component for TipText<'_> {
    const ROLE: super::Role = super::Role::TOOLTIP;
    const PARTS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    type Style = TipStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: TipTextStyleKey) -> TipStyle {
        let fill = StyleColor(cx.surface(SurfaceKind::Popup).fill);
        TipStyle { surface: SurfaceLook::new(Stateful::new(fill)).kind(SurfaceKind::Popup), metrics: cx.control().metrics() }
    }

    fn measure(&mut self, cx: &super::MeasureCx<'_, Self>) -> Vec2 {
        cx.layout(&TextRun::new(self.text).wrap(Wrap::Wrap)).size()
    }

    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::empty()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.name(Name::Text(self.text.to_owned()));
    }

    fn paint(&mut self, ui: &mut super::PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect: Rect = cx.content_rect;
        ui.draw().text(rect, &TextRun::new(self.text).wrap(Wrap::Wrap));
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
