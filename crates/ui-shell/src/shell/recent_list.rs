//! `RecentList`: the recent files as rows with their age and thumbnail.

use super::recent::{RecentFile, ThumbKey};
use super::start::StartOutput;
use crate::primitives::{ThumbSource, Thumbnails};
use crate::runtime::env::Label;
use egui::Ui;

/// The recent files as rows, with the commands of a recent file and its context menu.
#[must_use = "show the list with `show`"]
pub struct RecentList<'a> {
    recent: &'a [RecentFile],
    max_recent: usize,
    empty: Option<Label>,
    now: Option<i64>,
    thumbnails: Option<(&'a Thumbnails<ThumbKey>, &'a dyn ThumbSource<ThumbKey>)>,
}

impl<'a> RecentList<'a> {
    /// A list of `recent` files.
    pub fn new(recent: &'a [RecentFile]) -> Self {
        RecentList { recent, max_recent: usize::MAX, empty: None, now: None, thumbnails: None }
    }

    /// At most this many recent files show.
    #[must_use]
    pub fn max_recent(mut self, max_recent: usize) -> Self {
        self.max_recent = max_recent;
        self
    }

    /// The text shown when there are no recent files.
    #[must_use]
    pub fn empty(mut self, empty: impl Into<Label>) -> Self {
        self.empty = Some(empty.into());
        self
    }

    /// The time the age lines count from, in seconds since the Unix epoch. Without it the rows show no age.
    #[must_use]
    pub fn now(mut self, now: Option<i64>) -> Self {
        self.now = now;
        self
    }

    /// Rows ask `thumbnails.state(..)` for what they show; `source` gives each key's revision.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<ThumbKey>, source: &'a dyn ThumbSource<ThumbKey>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }

    /// Shows the rows and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> StartOutput {
        let _ = (self.recent, self.max_recent, self.empty, self.now, self.thumbnails, ui);

        todo!("ch.C7")
    }
}
