//! `CommentBalloons`: comment and revision cards in the margin of the page view, with leaders to
//! their anchors.

use std::marker::PhantomData;

use craft_pages::ui::PlacedPage;
use craft_ui::runtime::env::Label;
use egui::{Pos2, Rect, Response, Ui};

use super::comments::{CommentEdit, CommentsPanelState};
use crate::comments::{CommentThread, ThreadId};
use crate::revisions::Revision;

craft_ui::words! {
    /// The words of the comment balloons.
    pub struct BalloonWords {
        add: "balloons.addComment" = "Add a comment";
    }
}

/// One card of the balloons: a comment thread or a revision, on a page at an anchor.
#[derive(Clone, Copy, Debug)]
pub enum BalloonItem<'a> {
    /// A comment thread.
    Thread {
        /// The thread.
        thread: &'a CommentThread,
        /// The page it is on.
        page: usize,
        /// The point it is anchored at.
        anchor: Pos2,
    },
    /// A tracked change.
    Revision {
        /// The change.
        revision: &'a Revision,
        /// The page it is on.
        page: usize,
        /// The point it is anchored at.
        anchor: Pos2,
    },
}

/// Where the balloons sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BalloonPlace {
    /// In a column beside the pages.
    #[default]
    Column,
    /// Floating over the pages.
    Floating,
}

/// Layout and leaders of comment and revision cards. Each card is the panel's own comment row, fed
/// as the panel feeds it; the events the cards report go through the panel's `apply` and
/// `apply_menu`, so the panel and the balloons share one draft model and one event mapping.
#[must_use = "show the balloons with `show`"]
pub struct CommentBalloons<'a> {
    stub: PhantomData<(&'a PlacedPage, &'a mut CommentsPanelState)>,
}

impl<'a> CommentBalloons<'a> {
    /// Balloons named `label` for `items` on `pages`, with the card state in `state`.
    pub fn new(label: impl Into<Label>, pages: &'a [PlacedPage], items: &'a [BalloonItem<'a>], state: &'a mut CommentsPanelState) -> Self {
        let _ = (label.into(), pages, items, state);
        todo!("cv.K39")
    }

    /// Where the balloons sit.
    pub fn place(self, place: BalloonPlace) -> Self {
        let _ = (self, place);
        todo!("cv.K39")
    }

    /// The width of a card, in points.
    pub fn width(self, width: f32) -> Self {
        let _ = (self, width);
        todo!("cv.K39")
    }

    /// The name of the viewer, whose messages the cards mark as theirs.
    pub fn me(self, me: &'a str) -> Self {
        let _ = (self, me);
        todo!("cv.K39")
    }

    /// The time the cards' ages count from.
    pub fn now(self, now: Option<i64>) -> Self {
        let _ = (self, now);
        todo!("cv.K39")
    }

    /// Shows the balloons.
    pub fn show(self, ui: &mut Ui) -> CommentBalloonsOutput {
        let _ = (self, ui);
        todo!("cv.K39")
    }
}

/// What the balloons report for a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CommentBalloonsOutput {
    /// The balloons' response.
    pub response: Response,
    /// What the user asked of the comments, as the panel reports it.
    pub edits: Vec<CommentEdit>,
    /// Each thread's card and the rect it took this pass.
    pub cards: Vec<(ThreadId, Rect)>,
}
