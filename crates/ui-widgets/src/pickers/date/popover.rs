//! `CalendarPopover`: the calendar of a `DatePicker` hung from any rect.

use super::{DatePrecision, PickedDate};
use crate::contract::{Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Overlay, Role};
use crate::popover::PopoverOutput;
use crate::runtime::env::Label;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::Response;

/// The style of a calendar popover: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CalendarPopoverStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A calendar hung from any rect that shares `DatePicker`'s overlay body: the precision switch, Today, Clear and the
/// hover preview, so there is one calendar body.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
#[allow(dead_code)] // P3(pdf-forms-w4): FM-PF14
pub struct CalendarPopover<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'a, bool>,
    closed: Option<CloseReason>,
    value: Bind<'a, Option<PickedDate>>,
    /// The precisions offered; the first is the popup's page when the value has none of them.
    #[prop]
    precisions: &'a [DatePrecision],
    /// The earliest value allowed.
    #[prop]
    min: Option<PickedDate>,
    /// The latest value allowed.
    #[prop]
    max: Option<PickedDate>,
    /// What today is: the calendar rings its cell and the Today link picks it.
    #[prop]
    today: Option<PickedDate>,
}

impl<'a> CalendarPopover<'a> {
    /// A calendar popover named `label`, open while `open` is true, reading and writing `value`.
    pub fn new(label: impl Into<Label>, open: Bind<'a, bool>, value: Bind<'a, Option<PickedDate>>) -> Self {
        let _ = (label, open, value);
        todo!("FM-PF14")
    }

    /// The rect the calendar hangs from; the popover closes when the opener is no longer shown.
    #[must_use]
    pub fn opened_by(self, opener: &Response) -> Self {
        let _ = opener;
        todo!("FM-PF14")
    }

    /// Changes the popover's overlay as `Popover::overlay` does.
    #[must_use]
    pub fn overlay(self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        let _ = change;
        todo!("FM-PF14")
    }
}

impl Component for CalendarPopover<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = CalendarPopoverStyle;
    type Output = PopoverOutput;

    fn style(_: &StyleCx<'_>, _: CalendarPopoverStyleKey) -> CalendarPopoverStyle {
        CalendarPopoverStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> PopoverOutput {
        let _ = cx;
        todo!("FM-PF14")
    }
}

impl Container for CalendarPopover<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
