//! Components for the label-step trials: a scrubbing value control, a field container and a control that fills its
//! row.

use craft_behavior::{Metrics, Sense, ValueDrag};
use craft_ui::contract::{
    Accessible as _, Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, FieldLabel, MeasureCx, PaintUi,
    Props as _, Role, Scrub, SideLabel,
};
use craft_ui::primitives::TextRun;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Id, Rect, Response, Vec2};
use std::cell::RefCell;

/// A style with no padding.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a dial saw of its label scope and itself.
#[non_exhaustive]
pub struct Seen {
    pub id: Option<Id>,
    pub rect: Rect,
    pub label: Option<FieldLabel>,
}

impl Default for Seen {
    fn default() -> Self {
        Seen { id: None, rect: Rect::NOTHING, label: None }
    }
}

pub const SIZE: Vec2 = Vec2::new(100.0, 20.0);
/// The value change per point of label drag.
const RATE: f64 = 1.0;
/// The value a reset restores.
pub const RESET: f64 = 7.0;
const RANGE: std::ops::RangeInclusive<f64> = 0.0..=1000.0;

/// A number control that scrubs by its label and paints its value as text, so it has a baseline.
#[derive(Component)]
pub struct Dial<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f64>,
    seen: &'a RefCell<Seen>,
    warping: bool,
}

impl<'a> Dial<'a> {
    pub fn new(value: &'a mut f64, seen: &'a RefCell<Seen>) -> Self {
        Dial { common: Common::new(), value: Bind::bound(value).reset(RESET), seen, warping: false }
    }

    /// The label scrub warps the pointer at the window edge and hides it.
    pub fn warping(mut self) -> Self {
        self.warping = true;
        self
    }
}

impl Component for Dial<'_> {
    const ROLE: Role = Role::SLIDER;
    type Style = PlainStyle;
    type Output = EditResponse<f64>;

    fn style(_: &StyleCx<'_>, _: DialStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        SIZE
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    fn scrub(&self) -> Option<Scrub<f64>> {
        let drag = ValueDrag::relative(RATE, RANGE);
        let drag = if self.warping { drag.warp(true).hide_cursor(true) } else { drag };
        Some(Scrub::new(drag, |value| *value, |value| value))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let mut seen = self.seen.borrow_mut();
        seen.rect = cx.rect;
        seen.id = Some(cx.id);
        seen.label = cx.field_label();
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let text = format!("{}", cx.value().copied().unwrap_or(0.0));
        ui.draw().text(cx.rect, &TextRun::new(text));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}

/// A container with a side label whose body is the caller's.
#[derive(Component)]
#[component(container)]
pub struct Field {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Field {
    pub fn new(label: &'static str, side: SideLabel) -> Self {
        Field { common: Common::new(), props: ContainerProps::new().label(side) }.label(label)
    }
}

impl Component for Field {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FieldStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Field {}

/// A control that fills the width it is offered and can shrink to `WIDE_MIN`.
#[derive(Component)]
pub struct Wide {
    #[common]
    common: Common,
}

pub const WIDE_MIN: f32 = 40.0;

impl Wide {
    pub fn new() -> Self {
        Wide { common: Common::new() }
    }
}

impl Component for Wide {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: WideStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        match cx.proposal {
            craft_behavior::Proposal::Min => Vec2::new(WIDE_MIN, 20.0),
            _ => Vec2::new(cx.room.unwrap_or(WIDE_MIN), 20.0),
        }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
