//! The revisions panel: a compared copy's changes to accept or reject.

use craft_behavior::{Metrics, Sense};
use craft_theme::variant::Variant;
use craft_ui::buttons::{Button, Link};
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, MeasureCx, NoValue, Outcome, Pad, PaintUi, Pattern, Props, Role, RowAlign, Show as _,
    ShowContainer as _, Sizing, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::PaintShape;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx, SurfaceKind, TypeRole};
use egui::{CornerRadius, Response, Ui, Vec2};

use crate::revisions::{Revision, RevisionAction, RevisionKind};

#[cfg(feature = "screen-testing")]
mod stories;

/// The detail lines a card shows.
const DETAIL_LINES: usize = 8;

craft_ui::words! {
    /// The panel's words, English by default.
    #[non_exhaustive]
    pub struct RevisionsWords {
        compared_with: template "revisions.comparedWith" = "Compared with {name}";
        by_position: "revisions.byPosition" = "Not a copy of this presentation: slides are matched by position.";
        accept_all: count "revisions.acceptAll" = "Accept All ({n})" | "Accept All ({n})";
        reject_all: count "revisions.rejectAll" = "Reject All ({n})" | "Reject All ({n})";
        inserted: "revisions.inserted" = "Inserted";
        deleted: "revisions.deleted" = "Deleted";
        changed: "revisions.changed" = "Changed";
        /// A detail line of a change, after its bullet.
        detail_line: template "revisions.detailLine" = "\u{2022} {detail}";
        accept: "revisions.accept" = "Accept";
        reject: "revisions.reject" = "Reject";
        none_left: "revisions.noneLeft" = "No differences left.";
        end_review: "revisions.endReview" = "End Review";
    }
}

/// A compare pane: "Compared with {name}" (and the by-position note), Accept All (n) and Reject All (n), then a
/// raised card per change with a bar in the kind's status colour (named by the kind), the heading as a link (goes
/// there), Accept and Reject at the end edge, and up to eight detail lines with bullets. With no changes: "No differences left." and End Review. Every link and button is a Tab stop
/// in reading order.
#[derive(Pattern, Props)]
pub struct RevisionsPanel<'a> {
    label: Label,
    compared_with: &'a str,
    revisions: &'a [Revision],
    /// The copy did not match by identity: changes are by position.
    #[prop]
    by_position: bool,
}

impl<'a> RevisionsPanel<'a> {
    /// A panel named `label` listing `revisions` found by comparing with `compared_with` (the other copy's name).
    pub fn new(label: impl Into<Label>, compared_with: &'a str, revisions: &'a [Revision]) -> Self {
        RevisionsPanel { label: label.into(), compared_with, revisions, by_position: false }
    }

    /// Shows the panel and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> RevisionsPanelOutput {
        let RevisionsPanel { label, compared_with, revisions, by_position } = self;
        let env = Env::of(ui);
        let words = env.words::<RevisionsWords>();
        let mut actions = Vec::new();
        let shown = Stack::new().label(label).show(ui, |ui, _| {
            Text::new(words.compared_with.fill(&[("name", compared_with)])).shade(Shade::Muted).show(ui);
            if by_position {
                Text::new(words.by_position.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            if revisions.is_empty() {
                Text::new(words.none_left.clone()).show(ui);
                if clicked(&Button::new(words.end_review.clone()).show(ui)) {
                    actions.push(RevisionAction::EndReview);
                }
                return;
            }
            let count = u64::try_from(revisions.len()).unwrap_or(u64::MAX);
            let digits = env.format().integer(count);
            Row::new().show(ui, |ui, _| {
                if clicked(&Button::new(words.accept_all.text(count, &digits)).show(ui)) {
                    actions.push(RevisionAction::AcceptAll);
                }
                if clicked(&Button::new(words.reject_all.text(count, &digits)).show(ui)) {
                    actions.push(RevisionAction::RejectAll);
                }
            });
            for (index, revision) in revisions.iter().enumerate() {
                let kind = match revision.kind {
                    RevisionKind::Inserted => &words.inserted,
                    RevisionKind::Deleted => &words.deleted,
                    RevisionKind::Changed => &words.changed,
                };
                RevisionCard::new().show(ui, |ui, _| {
                    Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            ChangeBar::new(kind.clone(), revision.kind).show(ui);
                            if clicked(&Link::new(revision.heading.clone()).show(ui)) {
                                actions.push(RevisionAction::Go(index));
                            }
                        });
                        Row::new().show(ui, |ui, _| {
                            if clicked(&Button::new(words.accept.clone()).variant(Variant::PRIMARY).show(ui)) {
                                actions.push(RevisionAction::Accept(index));
                            }
                            if clicked(&Button::new(words.reject.clone()).show(ui)) {
                                actions.push(RevisionAction::Reject(index));
                            }
                        });
                    });
                    for detail in revision.details.iter().take(DETAIL_LINES) {
                        Text::new(words.detail_line.fill(&[("detail", detail.as_str())])).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    }
                });
            }
        });
        RevisionsPanelOutput { response: shown.output, actions }
    }
}

/// What the Revisions panel did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct RevisionsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// In the order they happened this pass.
    pub actions: Vec<RevisionAction>,
}

impl Outcome for RevisionsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One change's card: a raised surface with control padding around its rows.
#[derive(craft_ui::contract::Component)]
#[component(container, unregistered)]
struct RevisionCard {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl RevisionCard {
    fn new() -> Self {
        RevisionCard { common: Common::new(), props: ContainerProps::new().surface(SurfaceKind::Raised).pad(Pad::Control).sizing(Sizing::FillWidth) }
    }
}

/// The style of a revision card: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct RevisionCardStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for RevisionCard {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = RevisionCardStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RevisionCardStyleKey) -> RevisionCardStyle {
        RevisionCardStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for RevisionCard {}

/// The style of a change's bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct ChangeBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    inserted: StyleColor,
    deleted: StyleColor,
    changed: StyleColor,
    size: Vec2,
}

/// The bar at the start of a card in the status colour of its kind, named by the kind's word.
#[derive(craft_ui::contract::Component)]
#[component(unregistered)]
struct ChangeBar {
    #[common]
    common: Common,
    kind: RevisionKind,
}

impl ChangeBar {
    fn new(name: Label, kind: RevisionKind) -> Self {
        ChangeBar { common: Common::new(), kind }.label(name)
    }
}

impl Component for ChangeBar {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    type Style = ChangeBarStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ChangeBarStyleKey) -> ChangeBarStyle {
        ChangeBarStyle {
            metrics: Metrics::new(),
            inserted: cx.tone(craft_theme::variant::Tone::Good),
            deleted: cx.tone(craft_theme::variant::Tone::Bad),
            changed: cx.tone(craft_theme::variant::Tone::Info),
            size: Vec2::new(cx.tokens.stroke.thick, cx.tokens.size.icon.sm),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.size
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let color = match self.kind {
            RevisionKind::Inserted => cx.style.inserted,
            RevisionKind::Deleted => cx.style.deleted,
            RevisionKind::Changed => cx.style.changed,
        };
        let bar = egui::Rect::from_center_size(cx.content_rect.center(), cx.style.size.min(cx.content_rect.size()));
        ui.draw().fill(&PaintShape::Rect(bar, CornerRadius::ZERO), color);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
