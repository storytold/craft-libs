//! `FieldGroup` (16.3): a container whose `Field`s share one label column.

use crate::contract::{Common, Component, Container, ContainerProps, Cx, Gap, LabelScheme, Props as _, Role, Sizing};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::Response;

/// The style of a field group: no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FieldGroupStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A stack of `Field`s (and `LinkedFields`) named `label`, filling the width offered. Every beside label in the
/// body gets the width of the widest one, so the controls line up.
#[derive(crate::contract::Component)]
#[component(container)]
pub struct FieldGroup {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl FieldGroup {
    /// A group named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new().sizing(Sizing::FillWidth).label_column(true);
        FieldGroup { common: Common::new(), props }.label(label)
    }

    /// The fields under the group show their labels in `scheme`: a dialog's form by default, an inspector's rows with
    /// `LabelScheme::Inspector`. Nested groups and containers keep it unless they set their own.
    #[must_use]
    pub fn labels(mut self, scheme: LabelScheme) -> Self {
        self.props = self.props.labels(scheme);
        self
    }

    /// The spacing between the fields: `Gap::Row` by default, `Gap::Tight` for a dense panel's channel rows.
    #[must_use]
    pub fn gap(mut self, gap: Gap) -> Self {
        self.props = self.props.body_gap(gap);
        self
    }
}

impl Component for FieldGroup {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FieldGroupStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FieldGroupStyleKey) -> FieldGroupStyle {
        FieldGroupStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for FieldGroup {}
