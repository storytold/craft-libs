//! `Section` (16.5): an inline container whose header is a `TitleRow` with a disclosure bound to the container's
//! open state, and an optional trailing region for the app's controls. The body shows while the section is open.

use super::group_box::SECTION_LEVEL;
use super::stack::StackSlot;
use super::title_row::{HeaderAction, HeaderStrength, MenuRows, TitleRow};
use super::words::SectionWords;
use crate::contract::overlay::Popup;
use crate::contract::{
    Bind, BindMut, BodyCx, Collapse, Common, Component, Container, ContainerProps, Cx, DragBand, Gap, HeaderRegion, NoValue, Outcome, Pointer as _,
    Props as _, REORDER_BAND, Role, SaveKey, Show as _, Sizing, WordSetCx as _, reorder,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, PointerButton, PressPhase, Sense};
use egui::{Id, Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// The style of a section: no surface or padding of its own; the header and body carry the look.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SectionStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a section reports: its response, the header action taken this pass and what its reorder band did. The open
/// state is the caller's binding, or the container's own memory (saved across runs under `remember`).
#[derive(Debug)]
#[non_exhaustive]
pub struct SectionOutput {
    /// The section's response.
    pub response: Response,
    /// The header action part taken this pass.
    pub action: Option<HeaderAction>,
    /// The header band was press-dragged past the drag threshold this pass.
    pub grabbed: bool,
    /// Alt+Up or Alt+Down was pressed with focus in the section: `true` toward the end.
    pub nudged: Option<bool>,
}

impl Outcome for SectionOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The closure that fills a section's trailing header region.
type Controls<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// An inline section of a panel that fills the width offered: a rule over a section-strength title row over a body. The open state is the container's own (it starts open), the caller's binding, or remembered across runs
/// under a key; a fixed section is always open and shows the title alone.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Section<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Binds the open state to `open`.
    #[prop]
    open: Option<Bind<'a, bool>>,
    collapsible: bool,
    /// The header's weight: `Section` by default, `Caption` for a quiet heading.
    #[prop]
    strength: HeaderStrength,
    /// The rows of the menu the header's Menu action, a right click, the Menu key and Shift+F10 open.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    menu: Option<MenuRows<'a>>,
    actions: Vec<HeaderAction>,
    controls: Option<Controls<'a>>,
    taken: Option<HeaderAction>,
    menu_opener: Option<Id>,
    nudged: Option<bool>,
}

impl<'a> Section<'a> {
    /// A collapsible section titled `title`. It starts open.
    pub fn new(title: impl Into<Label>) -> Self {
        Section::build(title, true, ContainerProps::new().collapse(Collapse::new()).sizing(Sizing::FillWidth))
    }

    /// A section that is always open: the title alone, with no disclosure.
    pub fn fixed(title: impl Into<Label>) -> Self {
        Section::build(title, false, ContainerProps::new().sizing(Sizing::FillWidth))
    }

    fn build(title: impl Into<Label>, collapsible: bool, props: ContainerProps) -> Self {
        let props = props.header(HeaderRegion::new().rule_above(true).gap(Gap::Tight));
        Section {
            common: Common::new(),
            props,
            open: None,
            collapsible,
            strength: HeaderStrength::Section,
            menu: None,
            actions: Vec::new(),
            controls: None,
            taken: None,
            menu_opener: None,
            nudged: None,
        }
        .label(title)
    }

    /// Whether the section starts open (the default). The state lives in the container unless `open` binds it.
    #[must_use]
    pub fn initially_open(mut self, on: bool) -> Self {
        if let Some(collapse) = self.props.collapse {
            self.props = self.props.collapse(collapse.initially_open(on));
        }
        self
    }

    /// Keeps the open state across runs in the slot `key`.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }

    /// Whether dragging the header reorders the section among its siblings.
    #[must_use]
    pub fn reorder(mut self, on: bool) -> Self {
        let band = if on { DragBand::Reorder } else { DragBand::None };
        self.update_header(|header| header.band(band));
        self
    }

    /// The section is a member of a `SectionStack`: its header reorders it and it reports its place among the
    /// stack's sections.
    #[must_use]
    pub fn in_stack(mut self, slot: StackSlot) -> Self {
        self.props = self.props.position(slot.position());
        self.reorder(true).id_salt(slot.salt())
    }

    /// Fills the trailing region of the header with the app's controls (a Segmented, a Select, an icon button, count
    /// text). They sit at the end edge, after the title row and its fixed actions, and take their own clicks.
    #[must_use]
    pub fn controls(mut self, controls: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.update_header(|header| header.trailing(true));
        self.controls = Some(Box::new(controls));
        self
    }

    /// The space between the header and the body when the section is open (`Gap::Tight` by default).
    #[must_use]
    pub fn header_gap(mut self, gap: Gap) -> Self {
        self.update_header(|header| header.gap(gap));
        self
    }

    fn update_header(&mut self, change: impl FnOnce(HeaderRegion) -> HeaderRegion) {
        let header = change(self.props.header.unwrap_or_default());
        self.props.header = Some(header);
    }

    /// The action parts at the end of the header, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }
}

impl Component for Section<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = SectionStyle;
    type Output = SectionOutput;

    fn style(_: &StyleCx<'_>, _: SectionStyleKey) -> SectionStyle {
        SectionStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.props.header.is_some_and(|header| header.band == DragBand::Reorder) {
            self.nudged = reorder::take_move_item(&mut cx.core.intents);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> SectionOutput {
        if let Some(opener) = self.menu_opener {
            Popup::toggle(opener, &mut cx);
        }
        let grabbed = cx
            .core
            .part_if_declared(REORDER_BAND)
            .is_some_and(|band| matches!(band.press.phase, PressPhase::DragStart { start, .. } if start.button == PointerButton::Primary));
        SectionOutput { response: cx.response.clone(), action: self.taken, grabbed, nudged: self.nudged }
    }
}

impl Container for Section<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn controls(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some(controls) = self.controls.as_mut() {
            controls(ui, cx);
        }
    }

    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let row = TitleRow::new(cx.label()).strength(self.strength).level(SECTION_LEVEL).actions(self.actions.iter().copied());
        let row = if self.collapsible { row.disclosure(Bind::bound(cx.open())) } else { row };
        let shown = row.show(ui);
        if let Some(action) = shown.action {
            self.taken = Some(action);
        }
        if let Some(rows) = self.menu.as_mut() {
            self.menu_opener = rows.hang(ui, &shown, cx.words::<SectionWords>().menu.clone());
        }
    }
}
