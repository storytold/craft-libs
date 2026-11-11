//! `contract::tooltip`: the one step-16 function that shows every tooltip (3.4, 15.5).
//!
//! A tooltip shows after one delay while the pointer rests on the component, or on keyboard focus after the same
//! delay, and at once while a neighbour's tooltip was hiding a moment ago. A press, a menu that owns keys and a
//! running drag hide it.
//!
//! A component with no tooltip shows its label as the tooltip when its type declares `Component::SHOWS_LABEL` false
//! (or an instance does, through `Component::shows_label`): the label is then its only name and no text paints it.
//! A component that paints its label shows it whole as the tooltip when the paint cut it.

use super::cx::PartExtras;
use super::overlay::{self, Anchor, Layer, Overlay, Place};
use super::{Common, Component, Cx, Props as _, Show, TipFn, Tooltip, TooltipAt, TooltipSpec, TooltipWhen};
use crate::primitives::sensing::PartDecl;
use crate::primitives::{TextRun, Wrap};
use crate::runtime::access::scope::LabelPatch;
use crate::runtime::access::{Access, Name};
use crate::runtime::egui::memory::{self as store, Service};
use crate::runtime::egui::{areas, pass, repaint};
use crate::runtime::env::Env;
use crate::runtime::memory::{self, sub};
use crate::runtime::motion;
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
}

/// When the last tooltip of the context hid.
#[derive(Default)]
struct Grace(Option<f64>);

/// What step 16 hands the tooltip: the component's facts and the pass's `Ui`.
pub(crate) struct TipIn<'a> {
    pub ui: &'a Ui,
    pub env: &'a Env,
    /// The rich content closure of `show_with_tip`.
    pub rich: Option<TipFn<'a>>,
    /// The parts the component declared this pass, with their tooltips.
    pub parts: &'a [(PartDecl, PartExtras)],
    /// The instance's `shows_label` fact.
    pub shows_label: bool,
}

/// The text a tooltip shows for `spec`: the disabled text while disabled.
fn spec_text(env: &crate::runtime::env::EnvView<'_>, spec: &TooltipSpec, enabled: bool) -> String {
    let label = match (&spec.disabled_text, enabled) {
        (Some(disabled), false) => disabled,
        _ => &spec.text,
    };
    env.resolve(label).as_str().to_owned()
}

/// The text the component's own tooltip shows, or `None` when it has none: the tooltip's text, or its disabled text
/// while disabled, with the bound command's chord; else the full label when the paint cut it, or when the component
/// declares it paints no label (`shows_label`) and no field label names it.
fn text_of<C: Component>(cx: &Cx<'_, C>, shows_label: bool) -> Option<String> {
    let core = &cx.core;
    let base = match core.resolved.tooltip.map(|tooltip| tooltip.spec()) {
        Some(spec) => spec_text(&core.env, spec, core.resolved.enabled),
        None => {
            let paint = core.paint.borrow();
            match paint.elided.clone() {
                Some(full) => full,
                None if !shows_label && unnamed_by_field(core) => core.resolved.label.to_owned(),
                None => return None,
            }
        }
    };
    let platform = crate::runtime::windows::os_of(&core.ctx);
    Some(match core.command.as_deref().and_then(|command| command.chord.as_ref()) {
        Some(chord) => format!("{base} ({})", chord.display(platform)),
        None => base,
    })
}

/// Whether the label is non-empty and no field label shows the component's name, so the label is the only visible
/// name a tooltip can give.
fn unnamed_by_field(core: &super::CxCore<'_>) -> bool {
    !core.resolved.label.is_empty() && crate::runtime::access::scope::field_label(&core.ctx).is_none()
}

/// A part's tooltip, for the part the pointer is over or the keyboard is on.
struct PartTip {
    part: PartRef,
    spec: TooltipSpec,
    rect: Rect,
    hovered: bool,
}

/// The part whose tooltip shows: the hovered one with a tooltip, else the keyboard's active part.
fn part_tip<C: Component>(cx: &Cx<'_, C>, parts: &[(PartDecl, PartExtras)]) -> Option<PartTip> {
    let core = &cx.core;
    let tip = |decl: &PartDecl, extras: &PartExtras, hovered: bool| {
        let part = PartRef { key: decl.key, index: decl.index };
        let rect = core.results.get(&(part.key, part.index))?.rect;
        Some(PartTip { part, spec: extras.tooltip.as_ref()?.spec().clone(), rect, hovered })
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
pub(crate) fn run<C: Component>(cx: &Cx<'_, C>, input: TipIn<'_>) {
    let core = &cx.core;
    let part = part_tip(cx, input.parts);
    let keyboard = core.state.flags.contains(Flags::FOCUS_VISIBLE);
    let own_spec = core.resolved.tooltip.map(|tooltip| tooltip.spec().clone());
    let (text, spec, rect, hovered, subject) = match &part {
        Some(part) => (
            Some(spec_text(&core.env, &part.spec, core.resolved.enabled)),
            Some(part.spec.clone()),
            part.rect,
            part.hovered || keyboard,
            Some(part.part),
        ),
        None => (text_of(cx, input.shows_label), own_spec, core.rect, core.response.hovered() || keyboard, None),
    };
    let call = TipCall {
        ctx: &core.ctx,
        owner: core.memory.owner(),
        id: core.id,
        env: input.env,
        layer: input.ui.layer_id(),
        rect,
        text,
        rich: if part.is_some() { None } else { input.rich },
        spec,
        wants_input: hovered,
        pressed: core.press.phase.is_press(),
        subject,
    };
    show_tip(call);
}

/// Shows `tooltip` for a raw egui response during the app migration: the same delay, grace and placement as a
/// component's tooltip, none of its pipeline.
pub fn attach(response: &egui::Response, tooltip: &Tooltip) {
    let ctx = &response.ctx;
    let Some(env) = Env::root_of(ctx) else { return };
    memory::register_owner(ctx, response.id);
    let spec = tooltip.spec().clone();
    let text = spec_text(&env.view(), &spec, response.enabled());
    let call = TipCall {
        ctx,
        owner: response.id,
        id: response.id,
        env: &env,
        layer: response.layer_id,
        rect: response.rect,
        text: Some(text),
        rich: None,
        spec: Some(spec),
        wants_input: response.hovered() || response.has_focus(),
        pressed: crate::runtime::egui::response::is_pressed(response),
        subject: None,
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
    text: Option<String>,
    rich: Option<TipFn<'a>>,
    spec: Option<TooltipSpec>,
    /// The pointer rests on the subject or the keyboard is on it.
    wants_input: bool,
    pressed: bool,
    subject: Option<PartRef>,
}

fn show_tip(call: TipCall<'_>) {
    let TipCall { ctx, owner, id, env, layer: rect_layer, rect, text, rich, spec, wants_input, pressed, subject } = call;
    let state_id = sub(id, "tip_state");
    let has_content = text.is_some() || rich.is_some();
    // A component with no tooltip content and no timer or fading layer left from before has nothing to advance.
    if !(has_content && wants_input) && !store::exists::<TipState>(ctx, state_id) {
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
    let when_now = spec.as_ref().is_some_and(|spec| spec.when == TooltipWhen::Now);
    let wants = has_content && wants_input;
    let suppressed = pressed || overlay_blocks(ctx) || pointer::press_captured(ctx) || drag::running(ctx);
    if wants {
        state.since.get_or_insert(now);
    } else {
        state.since = None;
    }
    let due = state.since.map(|since| since + if when_now { 0.0 } else { DELAY });
    let in_grace = Service::<Grace>::run(ctx, |grace| grace.0.is_some_and(|hid| now - hid < GRACE));
    let show = wants && !suppressed && due.is_some_and(|due| now >= due || in_grace);
    if let (Some(due), true, false) = (due, wants, show)
        && !suppressed
        && due > now
    {
        repaint::request_after(ctx, pass::current_viewport(ctx), due - now);
    }
    if state.shown && !show {
        Service::<Grace>::run(ctx, |grace| grace.0 = Some(now));
    }
    state.shown = show;
    let tip_id = sub(id, "tip");
    let presence = motion::layer_presence(ctx, tip_id, owner, env.motion, show, overlay_motion());
    if state.shown || state.since.is_some() || presence.visible {
        store::put(ctx, state_id, state);
    }
    if !presence.visible {
        overlay::forget_layer(ctx, tip_id, owner);
        return;
    }
    let at_pointer = spec.as_ref().is_some_and(|spec| spec.at == TooltipAt::Pointer);
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
            if let Some(text) = &text {
                let tip = TipText { common: Common::default(), text }.id_salt(tip_id).show(ui);
                if subject.is_none() {
                    // The described control's node was written earlier in the pass; the relation joins it.
                    let patch = LabelPatch::DescribedBy { node: id, label: tip.id };
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
fn overlay_blocks(ctx: &egui::Context) -> bool {
    crate::runtime::overlay::class_open(ctx, OwnerClass::Menu)
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
    #[cfg(feature = "testing")]
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
