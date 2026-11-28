//! Components for the overlay trials: an opener with one popup and a leaf that records what reaches it.

use craft_behavior::{Metrics, MotionMode, Sense};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::contract::{CommandSet, Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Pointer as _, Props as _, Role, Trigger};
use craft_ui::popover::Popover;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Response, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A style with no padding.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What reached a leaf.
#[derive(Default)]
#[non_exhaustive]
pub struct Seen {
    pub id: Option<Id>,
    pub presses: u32,
    pub activations: u32,
}

/// A focusable leaf that counts presses and activations.
#[derive(Component)]
pub struct Leaf<'a> {
    #[common]
    common: Common,
    size: Vec2,
    seen: &'a RefCell<Seen>,
}

impl<'a> Leaf<'a> {
    pub fn new(seen: &'a RefCell<Seen>, size: Vec2) -> Self {
        Leaf { common: Common::new(), size, seen }
    }
}

impl Component for Leaf<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: LeafStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.size
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let mut seen = self.seen.borrow_mut();
        seen.id = Some(cx.id);
        seen.presses += u32::from(cx.press().phase.is_press());
        seen.activations += u32::from(cx.activation().is_some());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// An app-scope command a modal blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "overlayTrial")]
#[non_exhaustive]
pub enum Go {
    #[command(label = "Go")]
    Go,
}

/// What an opener reports.
#[derive(Default)]
#[non_exhaustive]
pub struct Opened {
    /// The popup body ran in this pass.
    pub shown: Cell<bool>,
}

/// A control with one popup of the given preset. The test decides when it asks to open or close.
#[derive(Component)]
#[component(overlay)]
pub struct Opener<'a> {
    #[common]
    common: Common,
    trigger: Trigger,
    make: fn() -> Overlay,
    open: bool,
    close: bool,
    opened: &'a Opened,
    content: &'a RefCell<Seen>,
}

impl<'a> Opener<'a> {
    pub fn new(opened: &'a Opened, content: &'a RefCell<Seen>, trigger: Trigger, make: fn() -> Overlay) -> Self {
        Opener { common: Common::new(), trigger, make, open: false, close: false, opened, content }
    }

    pub fn open(mut self, on: bool) -> Self {
        self.open = on;
        self
    }

    pub fn close(mut self, on: bool) -> Self {
        self.close = on;
        self
    }
}

impl Component for Opener<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OpenerStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("popup");
        popup.trigger(self.trigger, cx);
        if self.open {
            popup.open(cx);
        }
        if self.close {
            popup.close(cx);
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("popup");
        let content = self.content;
        let shown = ui.show(popup, (self.make)(), |ui, _| {
            craft_ui::contract::Show::show(Leaf::new(content, Vec2::new(80.0, 30.0)).id_salt("content"), ui);
        });
        if shown.is_some() {
            self.opened.shown.set(true);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Opens a popup inside a popover, closes the popover, reopens it and reports whether the popup is open.
fn popup_open_after_popover_reopens(motion: MotionMode) -> bool {
    let (outer, inner_open, inner_wanted) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let (shown_outer, shown_inner, wanted) = (outer.clone(), inner_open.clone(), inner_wanted.clone());
    let mut fixture = Fixture::new(move |ui| {
        let opener = craft_ui::buttons::Button::new("Brush").show(ui);
        let mut is_open = shown_outer.get();
        Popover::new("Brush options", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            let (opened, content) = (Opened::default(), RefCell::new(Seen::default()));
            Opener::new(&opened, &content, Trigger::Manual, Overlay::dropdown).open(wanted.get()).id_salt("inner").show(ui);
            shown_inner.set(opened.shown.get());
        });
        shown_outer.set(is_open);
    });
    fixture.motion(motion);
    fixture.settle();
    inner_wanted.set(true);
    fixture.frame(Vec::new());
    inner_wanted.set(false);
    fixture.idle(5);
    assert!(inner_open.get(), "the popup inside the popover is open");
    outer.set(false);
    fixture.idle(60);
    outer.set(true);
    fixture.idle(10);
    inner_open.get()
}

/// With motion off the popover hides in the frame it closes, so the popup's owner does not run until the popover
/// reopens; the close waits for it.
#[trial]
fn closing_a_popover_closes_the_popup_opened_inside_it_with_motion_off() {
    assert!(!popup_open_after_popover_reopens(MotionMode::Off), "the popup closed with the popover that held it");
}
