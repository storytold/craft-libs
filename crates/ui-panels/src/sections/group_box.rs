//! `GroupBox` (16.1): a framed container whose header is a `TitleRow`, with a check box that gates the body when it
//! has a gate.

use super::title_row::{HeaderStrength, TitleRow};
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, HeaderRegion, Pad, Props as _, Role, Show as _,
    Sizing, WordSetCx as _,
};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, Sense};
use egui::Ui;

#[cfg(feature = "testing")]
mod stories;

/// The heading level AccessKit reports for a section header.
pub(super) const SECTION_LEVEL: usize = 3;

/// The style of a group box: the frame comes from the container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GroupBoxStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A frame around a body, titled by the container's label. A gate makes the header's check box edit the gate and
/// disables the body while the gate is off. The frame fills the width offered; `fill(false)` makes it as wide as its
/// body and title, so boxes in a row sit side by side. `GroupBox::apply` configures it as a bulk-edit row: no frame
/// or padding, and an inline header holding the gate, so the check and the caller's field share one line.
#[derive(crate::contract::Component)]
#[component(container)]
pub struct GroupBox<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    gate: Option<Bind<'a, bool>>,
    strength: HeaderStrength,
    header_fill: bool,
}

impl<'a> GroupBox<'a> {
    /// A group box titled `title`, gated by `gate` when it has one.
    pub fn new(title: impl Into<crate::runtime::env::Label>, gate: Option<Bind<'a, bool>>) -> Self {
        let props = ContainerProps::new().surface(SurfaceKind::Frame).pad(Pad::Regular).sizing(Sizing::FillWidth).header(HeaderRegion::new()).gates();
        GroupBox { common: Common::new(), props, gate, strength: HeaderStrength::Section, header_fill: true }.label(title)
    }

    /// A bulk-edit row labelled `label`: the check box edits `apply`, and the body, the caller's field bound to a
    /// `Mixed` value, is enabled only while `apply` is on. Opening a dialog or visiting a tab never turns mixed values
    /// into the first item's value: the caller starts `apply` off.
    pub fn apply(label: impl Into<crate::runtime::env::Label>, apply: Bind<'a, bool>) -> Self {
        let props = ContainerProps::new().pad(Pad::None).header(HeaderRegion::new().inline(true)).gates();
        GroupBox { common: Common::new(), props, gate: Some(apply), strength: HeaderStrength::Normal, header_fill: false }.label(label)
    }

    /// Whether the frame fills the width offered (the default) or is as wide as its content.
    #[must_use]
    pub fn fill(mut self, fill: bool) -> Self {
        self.props = self.props.sizing(if fill { Sizing::FillWidth } else { Sizing::Content });
        self
    }
}

impl Component for GroupBox<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = GroupBoxStyle;
    type Output = EditResponse<bool>;

    fn style(_: &StyleCx<'_>, _: GroupBoxStyleKey) -> GroupBoxStyle {
        GroupBoxStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        self.gate.as_mut().map(Bind::reborrow)
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}

impl Container for GroupBox<'_> {
    /// The gate's shown copy goes to the check box; the container applies a change at step 15.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let row = TitleRow::new(cx.label()).strength(self.strength).fill(self.header_fill).level(SECTION_LEVEL);
        let mixed = cx.gate_is_mixed();
        match cx.gate() {
            Some(gate) => row.gate(Bind::bound(gate)).gate_mixed(mixed).show(ui),
            None => row.show(ui),
        };
    }
}
