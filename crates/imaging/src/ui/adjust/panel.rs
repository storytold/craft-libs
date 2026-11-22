//! [`AdjustmentsPanel`]: the grid of adjustment buttons a document panel offers for adding an adjustment layer.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::{Metrics, NavShape, Roving, Sense};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    BodyLayout, Common, Component, Container, ContainerProps, Cx, Gap, NoValue, Outcome, Pad, Pattern, Props as _, Role, Show as _,
    ShowContainer as _, Sizing, clicked,
};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Response, Ui};

use super::{AdjustWords, AdjustmentKind};

/// The gap between the buttons, in the grid's layout and in the column count.
const BUTTON_GAP: Gap = Gap::Tight;

/// What the Adjustments panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct AdjustmentsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The kind whose button was activated this pass.
    pub picked: Option<AdjustmentKind>,
}

impl Outcome for AdjustmentsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A grid of icon buttons in one roving group; the app picks which kinds it offers (default all).
///
/// - Each button shows the kind's icon and is named, and has its tooltip, by the kind's title. The buttons keep the
///   order of [`AdjustmentKind::ALL`], or of the subset given to [`kinds`](Self::kinds), and wrap to the width the
///   panel is given.
/// - The arrow keys move between the buttons by the grid they form; Tab leaves the group.
/// - The app names what the buttons are for (its "Add an adjustment" caption) and runs the command of the picked kind.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct AdjustmentsPanel<'a> {
    label: Label,
    kinds: &'a [AdjustmentKind],
}

impl<'a> AdjustmentsPanel<'a> {
    /// A panel named `label` offering every kind.
    pub fn new(label: impl Into<Label>) -> Self {
        AdjustmentsPanel { label: label.into(), kinds: &AdjustmentKind::ALL }
    }

    /// Offers only `kinds`, in that order.
    pub fn kinds(mut self, kinds: &'a [AdjustmentKind]) -> Self {
        self.kinds = kinds;
        self
    }

    /// Draws the grid and reports the picked kind.
    pub fn show(self, ui: &mut Ui) -> AdjustmentsPanelOutput {
        let AdjustmentsPanel { label, kinds } = self;
        let env = Env::of(ui);
        let words = env.words::<AdjustWords>();
        let gap = BUTTON_GAP.points(ui);
        let columns = ButtonGrid::columns(&StyleCx::of_env(ui.ctx(), &env), ui.available_width(), gap, kinds.len());
        let mut picked = None;
        let shown = ButtonGrid::new(label, columns).show(ui, |ui, _| {
            for &kind in kinds {
                let name = kind.label(&words);
                let button = IconButton::new(kind.icon()).id_salt(kind.key()).label(name.clone()).tooltip(name).show(ui);
                if clicked(&button) {
                    picked = Some(kind);
                }
            }
        });
        AdjustmentsPanelOutput { response: shown.output, picked }
    }
}

/// The style of the button grid: it paints nothing of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct ButtonGridStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The wrapping row of buttons, one roving group. A container's group steps over its members, which carry no row
/// lengths, so the group is a wrapped grid of the column count the caller works out from the width it gives.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(container, unregistered)]
struct ButtonGrid {
    #[common]
    common: Common,
    #[container()]
    props: ContainerProps,
}

impl ButtonGrid {
    /// How many of `count` buttons fit across `width` with `gap` between them. An icon button is square at the
    /// control height.
    fn columns(cx: &StyleCx<'_>, width: f32, gap: f32, count: usize) -> usize {
        let side = cx.tokens.size.control.md;
        (((width + gap) / (side + gap)).floor() as usize).clamp(1, count.max(1))
    }

    fn new(label: Label, columns: usize) -> Self {
        let props = ContainerProps::new()
            .pad(Pad::None)
            .sizing(Sizing::FillWidth)
            .layout(BodyLayout::row().wrap(true))
            .body_gap(BUTTON_GAP)
            .roving(Roving::new(NavShape::wrapped_grid(columns)));
        ButtonGrid { common: Common::new(), props }.label(label)
    }
}

impl Component for ButtonGrid {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = ButtonGridStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ButtonGridStyleKey) -> ButtonGridStyle {
        ButtonGridStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for ButtonGrid {}
