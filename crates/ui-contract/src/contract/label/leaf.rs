//! The side label as a leaf component: its node, press state, reset dot and colon.

use super::props::{FieldLayout, Glyph, LabelKind};
use super::step::LabelIn;
use crate::contract::{Component, Props};
use crate::contract::{Cx, MeasureCx, PaintUi, PartsCx, Styled as _};
use crate::primitives::{IconSpec, TextRun, Wrap};
use crate::runtime::env::{Env, Label};
use crate::runtime::words::CommonWords;
use crate::style::{ComponentStyle, ResetMarkLook, Stateful, StyleColor, StyleCx, TextLook, TypeRole};
use craft_behavior::{Flow, Metrics, PartKey, PressState, Sense};
use craft_theme::States;
use egui::{Rect, Response, Vec2};
use std::cell::Cell;

/// The part key of the reset dot.
const DOT: PartKey = PartKey::new("reset");

/// The style of the label leaf: no padding, and the look of the reset dot.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(super) struct LabelStyle {
    #[style(metrics)]
    metrics: Metrics,
    reset: ResetMarkLook,
    /// The side of the icon before the text.
    icon: f32,
    /// The gap between the icon, the text and the detail.
    gap: f32,
    /// The detail: `text.muted`, `text.disabled` with a disabled control.
    detail: Stateful<StyleColor>,
    /// The name: `text.muted`, dimmed with a disabled control.
    #[style(text)]
    name: TextLook,
    /// The glyph and icon: their own ink (`None`), `text.disabled` with a disabled control.
    glyph: Stateful<Option<StyleColor>>,
}

/// What the label leaf reports.
pub(super) struct LabelOutput {
    response: Response,
    pub(super) press: PressState,
    pub(super) double_click: bool,
    pub(super) dot_click: bool,
    pub(super) activated: bool,
}

impl crate::contract::Outcome for LabelOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The side label as a leaf: its own node, press state and reset dot.
#[derive(crate::contract::Component)]
#[component(sample = LabelLeaf::sample)]
pub(super) struct LabelLeaf {
    #[common]
    common: crate::contract::Common,
    pub(super) scrub: bool,
    /// The control behind the label has a reset value.
    pub(super) resettable: bool,
    /// The dot shows: the value differs from the reset value.
    pub(super) differs: bool,
    /// The width the dot's slot takes at the end of the label, 0 without one.
    slot: f32,
    pub(super) icon: Option<IconSpec<'static>>,
    /// A glyph shown in place of the name, icon and detail.
    pub(super) glyph: Option<Glyph>,
    /// The detail, resolved in the scope's language.
    pub(super) detail: Option<Label>,
    /// The colon drawn after the name, in the scope's language; a separate run, so the name stays the one text a
    /// narrow room elides.
    pub(super) colon: Option<String>,
    pub(super) baseline: Cell<f32>,
    double_click: bool,
    dot_click: bool,
    activated: bool,
}

impl LabelLeaf {
    fn detail_run<'a>(&self, detail: &'a Label, ink: StyleColor) -> TextRun<'a> {
        TextRun::new(detail.as_str()).color(ink).wrap(Wrap::Wrap)
    }

    #[cfg(feature = "testing")]
    fn sample() -> Self {
        LabelLeaf::new(Label::from("Opacity"), 0.0)
    }

    pub(super) fn new(text: Label, slot: f32) -> Self {
        LabelLeaf {
            common: crate::contract::Common::new(),
            scrub: false,
            resettable: false,
            differs: false,
            slot,
            icon: None,
            glyph: None,
            detail: None,
            colon: None,
            baseline: Cell::new(0.0),
            double_click: false,
            dot_click: false,
            activated: false,
        }
        .label(text)
    }
}

impl Component for LabelLeaf {
    const ROLE: crate::contract::Role = crate::contract::Role::LABEL;
    const OVERLAYS: bool = false;
    // The label is no key target: Enter and Space go to its control, which a click on the label focuses.
    const KEYS: crate::runtime::focus::KeyUse =
        crate::runtime::focus::KeyUse::of(Self::INPUT).with_activate(crate::runtime::focus::ActivateKeys::Neither);
    type Style = LabelStyle;
    type Output = LabelOutput;

    fn style(cx: &StyleCx<'_>, _: LabelLeafStyleKey) -> LabelStyle {
        LabelStyle {
            metrics: Metrics::new(),
            reset: cx.reset_mark(),
            icon: cx.control().icon,
            gap: cx.tokens.space.gap.inline,
            detail: {
                let (muted, off) = (cx.color(cx.tokens.color.text.muted), cx.color(cx.tokens.color.text.disabled));
                Stateful::new(muted).off(States::new(muted, muted, muted, off))
            },
            name: {
                let (muted, off) = (cx.color(cx.tokens.color.text.muted), cx.color(cx.tokens.color.text.disabled));
                TextLook::new(TypeRole::Label, Stateful::new(muted).off(States::new(muted, muted, muted, off)))
            },
            glyph: {
                let off = Some(cx.color(cx.tokens.color.text.disabled));
                Stateful::new(None).off(States::new(None, None, None, off))
            },
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let line = cx.layout(&TextRun::new(cx.resolved.label));
        let name = line.size();
        if let Some(glyph) = &self.glyph {
            let (shown, baseline) = match glyph {
                Glyph::Icon(_) => {
                    let height = cx.style.icon.max(name.y);
                    (Vec2::new(cx.style.icon, height), line.first_baseline + (height - name.y) / 2.0)
                }
                Glyph::Text(text) => {
                    let run = cx.layout(&TextRun::new(text.as_str()));
                    (run.size(), run.first_baseline)
                }
            };
            self.baseline.set(baseline);
            return Vec2::new(shown.x + self.slot, shown.y);
        }
        let colon = self.colon.as_deref().map_or(0.0, |colon| cx.layout(&TextRun::new(colon)).size().x);
        let (mut width, mut height) = (name.x + colon, name.y);
        if self.icon.is_some() {
            width += cx.style.icon + cx.style.gap;
            height = height.max(cx.style.icon);
        }
        if let Some(detail) = &self.detail {
            // The detail wraps in the room the name and the gap leave.
            let room = match cx.proposal {
                craft_behavior::Proposal::Width(width) => Some(width),
                _ => cx.room,
            };
            let left = room.map_or(f32::INFINITY, |room| (room - self.slot - width - cx.style.gap).max(0.0));
            let detail = cx.layout_at(&self.detail_run(detail, cx.style.detail.off.rest), left).size();
            width += cx.style.gap + detail.x;
            height = height.max(detail.y);
        }
        // The text sits at the row's vertical centre, so an icon taller than the text moves the baseline down.
        self.baseline.set(line.first_baseline + (height - name.y) / 2.0);
        Vec2::new(width + self.slot, height)
    }

    /// A click on the label focuses its control (the scope's `FieldLabel::activated`), so every label takes clicks.
    fn sense(&self) -> Sense {
        if self.scrub { Sense::CLICK | Sense::DRAG } else { Sense::CLICK }
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if self.slot > 0.0 && self.differs {
            let (slot, _) = cx.flow.take_end(self.slot);
            cx.part(DOT, slot).sense(Sense::CLICK).pointer_only("the Reset command");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        use crate::contract::Pointer as _;
        self.double_click = self.resettable && cx.press().double_pressed();
        self.dot_click = self.slot > 0.0 && self.differs && cx.part(DOT).press.phase.is_click();
        self.activated = cx.activation().is_some();
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let flow = Flow::new(cx.rect, cx.core.env.direction().is_rtl());
        let (slot, mut rest) = flow.take_end(self.slot);
        let draw = ui.draw();
        let ink = cx.resolve(&cx.style.glyph);
        let icon_of = |icon: &IconSpec<'static>| IconSpec { color: ink.or(icon.color), ..icon.clone() };
        let middle = cx.rect.center().y;
        let centred = |rect: Rect, height: f32| Rect::from_min_size(egui::pos2(rect.min.x, middle - height / 2.0), Vec2::new(rect.width(), height));
        if let Some(glyph) = &self.glyph {
            match glyph {
                Glyph::Icon(icon) => {
                    draw.icon(centred(rest.take_start(cx.style.icon).0, cx.style.icon), &icon_of(icon));
                }
                Glyph::Text(text) => {
                    let run = ink.map_or_else(|| TextRun::new(text.as_str()), |ink| TextRun::new(text.as_str()).color(ink));
                    let size = draw.layout(&run, rest.rect().width()).size();
                    draw.text(centred(rest.take_start(size.x).0, size.y), &run);
                }
            }
        } else {
            if let Some(icon) = &self.icon {
                let (cell, after) = rest.take_start(cx.style.icon);
                draw.icon(centred(cell, cx.style.icon), &icon_of(icon));
                rest = after.take_start(cx.style.gap).1;
            }
            // A name wider than its room elides; the full text stays the accessible name.
            let name_ink = cx.resolve(&cx.style.name.color);
            let colon = self.colon.as_deref().map(|colon| TextRun::new(colon).color(name_ink));
            let colon_width = colon.as_ref().map_or(0.0, |run| draw.layout(run, f32::INFINITY).size().x);
            let run = TextRun::new(cx.core.resolved.label).color(name_ink).elide();
            let name = draw.layout(&run, (rest.rect().width() - colon_width).max(0.0)).size();
            let (name_rect, after) = rest.take_start(name.x);
            draw.text(centred(name_rect, name.y), &run);
            let after = match colon {
                Some(run) => {
                    let (colon_rect, after) = after.take_start(colon_width);
                    draw.text(centred(colon_rect, name.y), &run);
                    after
                }
                None => after,
            };
            if let Some(detail) = &self.detail {
                let run = self.detail_run(detail, cx.resolve(&cx.style.detail));
                let (_, detail_flow) = after.take_start(cx.style.gap);
                let size = draw.layout(&run, detail_flow.rect().width()).size();
                draw.text(centred(detail_flow.rect(), size.y), &run);
            }
        }
        if self.differs && self.slot > 0.0 {
            draw.reset_mark(&cx.style.reset, slot.center());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> LabelOutput {
        LabelOutput {
            response: cx.core.response.clone(),
            press: cx.core.press,
            double_click: self.double_click,
            dot_click: self.dot_click,
            activated: self.activated,
        }
    }
}

/// The colon a beside text label draws after its name, unless the name ends in one. The label's node and the
/// control keep the name as given.
pub(super) fn colon(env: &Env, input: &LabelIn<'_>) -> Option<String> {
    let text = env.view().resolve(input.text);
    let beside_text = input.side.layout == FieldLayout::Beside && input.side.kind == LabelKind::Text;
    if !beside_text || text.as_str().is_empty() || text.as_str().ends_with([':', '\u{ff1a}']) {
        return None;
    }
    Some(colon_of(&env.words::<CommonWords>()))
}

/// The colon of the scope's language that follows a caption's name.
#[doc(hidden)]
pub fn colon_of(words: &CommonWords) -> String {
    words.caption.fill(&[("caption", "")])
}
