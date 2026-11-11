//! The revisions panel: a compared copy's changes to accept or reject.

use craft_theme::variant::{Tone, Variant};
use craft_ui::buttons::{Button, Link};
use craft_ui::contract::{NoValue, Outcome, Pattern, Props, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

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
/// card per change with its kind, the heading as a link (goes there), Accept and Reject, and up to eight detail
/// lines with bullets. With no changes: "No differences left." and End Review. Every link and button is a Tab stop
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
                let (tone, kind) = match revision.kind {
                    RevisionKind::Inserted => (Tone::Good, &words.inserted),
                    RevisionKind::Deleted => (Tone::Bad, &words.deleted),
                    RevisionKind::Changed => (Tone::Info, &words.changed),
                };
                Stack::new().show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        Text::new(kind.clone()).role(TypeRole::Caption).tone(tone).show(ui);
                        if clicked(&Link::new(revision.heading.clone()).show(ui)) {
                            actions.push(RevisionAction::Go(index));
                        }
                        if clicked(&Button::new(words.accept.clone()).variant(Variant::PRIMARY).show(ui)) {
                            actions.push(RevisionAction::Accept(index));
                        }
                        if clicked(&Button::new(words.reject.clone()).show(ui)) {
                            actions.push(RevisionAction::Reject(index));
                        }
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
