//! `DropVeil` (8.4, 12.4): a container over its body that takes files dragged in from the OS. While files hover it
//! dims the body, frames it and shows a card with the label; a drop is reported in the output.

use crate::contract::{
    Access, Accessible as _, Common, Component, Container, ContainerProps, Cx, DropMark, DropOperation, MeasureCx, NoValue, OsFiles, Outcome, Pad,
    PaintUi, PartsCx, Pointer as _, Politeness, Props as _, Remember as _, Role, Sizing, WindowDropTarget, WordSetCx as _,
};
use crate::display::Text;
use crate::runtime::drag;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Corners, Insets, Stateful, StyleCx, SurfaceKind, SurfaceLook, TypeRole, UiTokens};
use craft_behavior::{Metrics, Proposal, Sense};
use egui::{Rect, Response};

#[cfg(feature = "testing")]
mod stories;

/// The style of a drop veil: the scrim comes from the surface kind; the card is a child.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DropVeilStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The style of the veil's card: a popup surface with the dialog padding around its text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct VeilCardStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
}

/// The card of a veil: a popup surface around the label's `Text`, as wide and high as the text with its padding.
#[derive(crate::contract::Component)]
#[component(sample = VeilCard::sample)]
struct VeilCard {
    #[common]
    common: Common,
}

impl VeilCard {
    #[cfg(feature = "testing")]
    fn sample() -> Self {
        VeilCard::new("Drop files here")
    }

    fn new(label: impl Into<Label>) -> Self {
        VeilCard { common: Common::new() }.label(label)
    }
}

impl Component for VeilCard {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const OVERLAYS: bool = false;
    type Style = VeilCardStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: VeilCardStyleKey) -> VeilCardStyle {
        let radius = UiTokens::corner(cx.tokens.shape.radius.surface);
        let surface =
            SurfaceLook::new(Stateful::new(cx.color(cx.tokens.color.surface.panel))).kind(SurfaceKind::Popup).radius(radius).corners(Corners::All);
        VeilCardStyle { surface, metrics: Metrics::new().padding(Insets::uniform(cx.tokens.space.pad.roomy)) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> egui::Vec2 {
        let padding = cx.style.metrics().padding;
        let proposal = match cx.proposal {
            Proposal::Width(width) => Proposal::Width((width - padding.start - padding.end).max(0.0)),
            other => other,
        };
        cx.child("text", &mut card_text(cx.resolved.label.to_owned()), proposal)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.child("text", cx.content_rect, card_text(cx.resolved.label.to_owned()));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// What a drop veil reports: its response and the files dropped on it in this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct DropVeilOutput {
    /// The veil's response.
    pub response: Response,
    /// The files dropped anywhere in the window in this pass.
    pub dropped: Option<OsFiles>,
}

impl Outcome for DropVeilOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The accepting side of a veil: any files, anywhere in the window, by any operation.
struct WholeWindow {
    rect: Rect,
}

impl WindowDropTarget<OsFiles> for WholeWindow {
    type Drop = OsFiles;

    fn accept(&self, payload: &OsFiles, _op: DropOperation) -> Option<OsFiles> {
        (!payload.files.is_empty()).then(|| payload.clone())
    }

    fn mark(&self, _drop: &OsFiles) -> DropMark {
        DropMark::Into(self.rect)
    }
}

/// Whether the hover of the current files has been announced.
#[derive(Clone, Default)]
struct Announced(bool);

/// A body that dims and shows `label` on a card while the OS drags files over the window, and reports the files
/// dropped on the window.
#[derive(crate::contract::Component)]
#[component(container)]
pub struct DropVeil {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    hovering: bool,
    dropped: Option<OsFiles>,
    /// The card's rect, from the card child's measured size.
    card: Option<Rect>,
}

impl DropVeil {
    /// A veil saying `label` ("Import 3 photos") while files hover.
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).pad(Pad::None);
        DropVeil { common: Common::new(), props, hovering: false, dropped: None, card: None }.label(label)
    }
}

/// The card's text.
fn card_text(label: impl Into<Label>) -> Text {
    Text::new(label).role(TypeRole::Title)
}

impl Component for DropVeil {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = DropVeilStyle;
    type Output = DropVeilOutput;

    fn style(_: &StyleCx<'_>, _: DropVeilStyleKey) -> DropVeilStyle {
        DropVeilStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let held = drag::active::<OsFiles>(cx.ui.ctx()).is_some_and(|files| !files.files.is_empty());
        self.card = held.then(|| {
            let size = cx.child("card", &mut VeilCard::new(cx.label()), Proposal::Width(rect.width()));
            Rect::from_center_size(rect.center(), size)
        });
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let held = drag::active::<OsFiles>(&cx.ctx).is_some_and(|files| !files.files.is_empty());
        let target = WholeWindow { rect: cx.rect };
        let (id, ordinal) = (cx.id, drag::next_ordinal(&cx.ctx, cx.id));
        self.dropped = cx.window_drop_target::<OsFiles, _>(&target);
        // The veil shows only while it is the target that would take the drop.
        self.hovering = held && self.dropped.is_none() && drag::winning(&cx.ctx, id, ordinal);
        let announced = cx.memory::<Announced>();
        let first = self.hovering && !announced.0;
        announced.0 = self.hovering;
        if first {
            let text = cx.env.resolve(&cx.label()).as_str().to_owned();
            cx.announce(text, Politeness::Polite);
        }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if !self.hovering {
            return;
        }
        let draw = ui.draw();
        draw.surface(cx.rect, SurfaceKind::Scrim, Corners::None);
        if let Some(card) = self.card {
            ui.child("card", card, VeilCard::new(cx.label()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> DropVeilOutput {
        DropVeilOutput { response: cx.response.clone(), dropped: self.dropped }
    }
}

impl Container for DropVeil {}
