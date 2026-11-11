//! `Field` (16.3): a container that shows one control with a side label, and a help line under it.

use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, FieldLayout, FooterRegion, Gap, Props as _, Role, Show as _, SideLabel, Sizing,
};
use crate::display::{Shade, Text};
use crate::runtime::access::Access;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, TypeRole};
use craft_behavior::{Metrics, Sense};
use craft_theme::variant::Tone;
use egui::{Response, Ui};

/// The text under a field's control.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Help {
    /// Muted text: a hint or a unit.
    Text(Label),
    /// A line in the colour of a tone.
    Status(Tone, Label),
}

/// The style of a field: no look of its own; the label step and the help line carry it.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FieldRowStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// One control with a side label: beside it, above it, as a caption or after it (`Field::layout`). The label step
/// names the control by the label, runs the label's scrub and reset, and focuses the control when the label is
/// clicked. The control is the body's one child and returns its own output. `Field::help` adds a help line under
/// the control.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Field {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Shows `help` under the control while the control has no validity message.
    #[prop]
    help: Option<Help>,
    /// The layout `Field::layout` set, which `Field::side` keeps.
    layout: Option<FieldLayout>,
    /// The static text a readout field reports as its node's value, for a field whose body shows that text (a
    /// `Text` in the body closure). The value is read by assistive technology and by tests; it is not editable.
    #[prop(into)]
    value: Option<String>,
}

impl Field {
    /// A field labelled `label`, beside its control, filling the width offered.
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new().sizing(Sizing::FillWidth).footer(FooterRegion::new().gap(Gap::Tight)).label(SideLabel::new());
        Field { common: Common::new(), props, help: None, layout: None, value: None }.label(label)
    }

    /// Where the label sits relative to the control.
    #[must_use]
    pub fn layout(mut self, layout: FieldLayout) -> Self {
        self.layout = Some(layout);
        self.props.label = self.props.label.take().map(|side| side.layout(layout));
        self
    }

    /// How the label is shown and what it does: its icon, detail, scrub, whether it is visible and whether it names
    /// or describes the control. The text stays the field's label. A layout set with `Field::layout` stays, whichever
    /// is called first; without one, the layout is the side label's own.
    #[must_use]
    pub fn side(mut self, side: SideLabel) -> Self {
        self.props.label = Some(match self.layout {
            Some(layout) => side.layout(layout),
            None => side,
        });
        self
    }
}

impl Component for Field {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FieldRowStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FieldStyleKey) -> FieldRowStyle {
        FieldRowStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if let Some(text) = &self.value {
            access.value(text.clone());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Field {
    fn footer(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some((message, tone)) = cx.field_message() {
            Text::new(message.text.clone()).role(TypeRole::Caption).tone(tone).show(ui);
            return;
        }
        match &self.help {
            Some(Help::Text(text)) => Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui),
            Some(Help::Status(tone, text)) => Text::new(text.clone()).role(TypeRole::Caption).tone(*tone).show(ui),
            None => return,
        };
    }
}
