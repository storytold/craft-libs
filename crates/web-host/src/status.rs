//! The statuses a hosted app shows in its title bar (storytold/craft-libs#2079): whether the host
//! stored the document, the version it stored, how many people are in it, and whether the session
//! is read-only.
//!
//! Every input is a message of the hosting contract (storytold/craft-libs#358) or a fact the app
//! already has, so nothing here opens a network path and nothing is specific to one host:
//!
//! | Input | Where the app gets it |
//! |---|---|
//! | [`StatusInput::Edited`] | its own change tracking: the document differs from what it last posted |
//! | [`StatusInput::SavePosted`] | it posted `save`, with the `id` it put in the message |
//! | [`StatusInput::Saved`] | `saved {id}` from the host |
//! | [`StatusInput::SaveFailed`] | `failed {id, error}` for a save, or the app's own timeout for one (storytold/craft-libs#1338) |
//! | [`StatusInput::CommitPosted`] | it posted `commit` |
//! | [`StatusInput::Committed`] | `committed {version, etag}` from the host |
//! | [`StatusInput::SessionState`] | the `session-state {status, participants}` it posts |
//! | [`StatusInput::Presence`] | the `presence {participants}` it posts |
//! | [`StatusInput::ReadOnly`] | `session`'s `viewer` role, or a host-controlled read-only session (storytold/craft-libs#609) |
//!
//! [`HostStatus::apply`] keeps the state, and [`HostStatus::cluster`] turns it into what the title
//! bar draws, in reading order, leaving out what does not apply. A host that never sends the
//! collaboration messages never produces those inputs: the cluster then shows the save state alone.
//! The crate carries no display text: the app's language pack supplies it, and the English source
//! texts are noted on each variant (`Saved`, `Unsaved`, `Saving…`, `Read-only`, and a count of
//! people in the document).

use std::collections::VecDeque;

/// The sync status the title bar never shows: a synced session needs no word of its own, so only a
/// session that is not [`SYNCED`] puts one there.
pub const SYNCED: &str = "synced";

/// How many posted saves the state still tells apart. Autosave may post several before a host
/// answers, and a host that stops answering must not make the queue grow: past this many the oldest
/// posted save is forgotten, and its late answer confirms nothing.
pub const MAX_PENDING_SAVES: usize = 8;

/// How the host holds the document, as a title bar says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    /// The document has changes the host does not hold, or the host refused the last save. English
    /// source text: `Unsaved`.
    Unsaved,
    /// A save is posted and unanswered. English source text: `Saving…`.
    ///
    /// A save in flight outranks a save already confirmed: the person sees `Saving…`, not `Saved`,
    /// while any posted save waits for its answer.
    Saving,
    /// The host confirmed the last save, or committed the session: it holds the document. English
    /// source text: `Saved`.
    Saved,
}

/// The version a host stored when it committed the session (`committed {version, etag}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVersion {
    /// The session version the host stored: it counts commits, so a later commit has a higher one.
    pub version: u64,
    /// The host's `etag` for that stored content, for the app to send with its next commit.
    pub etag: String,
}

/// What the app tells [`HostStatus`]: a contract message it received or posted, its own change
/// tracking, or the session's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusInput<'a> {
    /// The document changed: what the host holds is no longer what the app shows.
    Edited,
    /// The app posted a `save`. `id` is the message's `id`, or `None` when it carries none: the
    /// contract has a host answer only a save with an `id`, so one without is stored once posted. An
    /// empty `id` counts as none.
    SavePosted { id: Option<&'a str> },
    /// The host stored the save with this `id` (`saved`).
    Saved { id: &'a str },
    /// A save of this `id` was refused, and this is why (`failed`). An empty `error` still says the
    /// save did not happen.
    SaveFailed { id: Option<&'a str>, error: &'a str },
    /// The app asked the host to commit the session (it posted `commit`): the document it sent is
    /// the document as it is now.
    CommitPosted,
    /// The host committed the session and now holds it as this version (`committed`).
    Committed { version: u64, etag: &'a str },
    /// The app's own sync word and who is in the session, as it posted in `session-state`.
    SessionState { status: &'a str, participants: usize },
    /// Who is in the session, as the app posted in `presence`.
    Presence { participants: usize },
    /// The session is read-only: the host's decision, not the app's (storytold/craft-libs#609).
    ReadOnly(bool),
}

/// Where the cluster starts: the session's mode, or how the host holds the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headline {
    /// Read-only: the app cannot change the document. Drawn instead of a save state, so the cluster
    /// cannot promise a save the person may not make. English source text: `Read-only`.
    ReadOnly,
    /// Whether the host holds the document.
    Save(SaveState),
}

/// One thing the cluster draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterPart<'a> {
    /// The read-only notice or the save state; always drawn.
    Headline(Headline),
    /// How many people are in the document, this app included: a count, never the list. English
    /// source text: a plural count of people in the document.
    Participants(usize),
    /// The app's own sync word (`pending`, `offline`, `error`), shown as the app spells it.
    Sync(&'a str),
    /// The version the host stored at the last commit.
    Version(u64),
}

/// What the title bar's cluster draws, built by [`HostStatus::cluster`]. The app supplies the words
/// and the layout; this says which parts are there and in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster<'a> {
    /// The first part: `Read-only`, or the save state.
    pub headline: Headline,
    /// People in the document, this app included; `0` when no session reports any.
    pub participants: usize,
    /// The app's own sync word, and only while it is not [`SYNCED`].
    pub sync: Option<&'a str>,
    /// The stored version after a commit.
    pub version: Option<u64>,
    /// Why the last save was refused, while the document still shows as unsaved.
    ///
    /// It is not a [`ClusterPart`]: it belongs to the headline, so the app draws it where it draws
    /// the failure (a tooltip on the label, or beside it).
    pub error: Option<&'a str>,
}

impl<'a> Cluster<'a> {
    /// The parts in the order the title bar draws them, leaving out what does not apply: no
    /// participant count without a session, no sync word while `synced`, no version before a commit.
    pub fn parts(self) -> impl Iterator<Item = ClusterPart<'a>> {
        let headline = Some(ClusterPart::Headline(self.headline));
        let participants = (self.participants > 0).then_some(ClusterPart::Participants(self.participants));
        let sync = self.sync.map(ClusterPart::Sync);
        let version = self.version.map(ClusterPart::Version);
        [headline, participants, sync, version].into_iter().flatten()
    }
}

/// The host statuses of one document, from the messages its app posts and receives.
///
/// Plain data: it builds everywhere, so it is tested natively. The bridge (`web`, wasm32 only) turns
/// a host's messages into [`StatusInput`] values; the app turns its own tracking into the rest.
#[derive(Debug, Clone)]
pub struct HostStatus {
    save: SaveState,
    /// The `id`s of posted saves the host has not answered for, oldest first.
    pending: VecDeque<String>,
    /// The app asked for a commit and the host has not answered yet.
    committing: bool,
    /// The document has changes the last posted save did not carry.
    dirty: bool,
    /// Why the last save was refused, until another attempt is posted or one succeeds.
    error: Option<String>,
    stored: Option<StoredVersion>,
    /// The app's own sync word, kept as it was given.
    sync: Option<String>,
    participants: usize,
    read_only: bool,
}

impl Default for HostStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl HostStatus {
    /// A document the host opened and has not changed: `Saved`, synced, with no participants and no
    /// committed version. The app reports its first change with [`StatusInput::Edited`].
    pub fn new() -> Self {
        Self {
            save: SaveState::Saved,
            pending: VecDeque::new(),
            committing: false,
            dirty: false,
            error: None,
            stored: None,
            sync: None,
            participants: 0,
            read_only: false,
        }
    }

    /// Take one input. Out of order they are safe: a save in flight stays in flight, an edit made
    /// while it was posted keeps the document unsaved after its confirmation, and an answer for a
    /// save that is no longer told apart confirms nothing.
    pub fn apply(&mut self, input: StatusInput<'_>) {
        match input {
            StatusInput::Edited => self.edited(),
            StatusInput::SavePosted { id } => self.posted(id.filter(|id| !id.is_empty())),
            StatusInput::Saved { id } => self.confirmed(id),
            StatusInput::SaveFailed { id, error } => self.refused(id.filter(|id| !id.is_empty()), error),
            StatusInput::CommitPosted => self.commit_posted(),
            StatusInput::Committed { version, etag } => self.committed(version, etag),
            StatusInput::SessionState { status, participants } => {
                self.sync = Some(status.to_string());
                self.participants = participants;
            }
            StatusInput::Presence { participants } => self.participants = participants,
            StatusInput::ReadOnly(read_only) => self.read_only = read_only,
        }
    }

    /// How the host holds the document, ignoring the session's mode.
    pub fn save_state(&self) -> SaveState {
        self.save
    }

    /// Why the last save was refused, while no save has succeeded since.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The version the host stored at its last commit.
    pub fn stored_version(&self) -> Option<&StoredVersion> {
        self.stored.as_ref()
    }

    /// The app's own sync word as it was given, `synced` included.
    pub fn sync_status(&self) -> Option<&str> {
        self.sync.as_deref()
    }

    /// People in the document, this app included; `0` when no session reported any.
    pub fn participants(&self) -> usize {
        self.participants
    }

    /// The `id`s of posted saves the host has not answered for, oldest first.
    ///
    /// What an app's own timeout turns into [`StatusInput::SaveFailed`] (storytold/craft-libs#1338):
    /// the state remembers which ids still wait, including the ones it had to forget, so the app does
    /// not keep a second list that drifts from this one.
    pub fn unanswered_saves(&self) -> impl Iterator<Item = &str> {
        self.pending.iter().map(String::as_str)
    }

    /// Whether the host made the session read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// What the title bar draws from this state.
    pub fn cluster(&self) -> Cluster<'_> {
        Cluster {
            headline: if self.read_only { Headline::ReadOnly } else { Headline::Save(self.save) },
            participants: self.participants,
            sync: self.sync.as_deref().filter(|status| *status != SYNCED),
            version: self.stored.as_ref().map(|stored| stored.version),
            error: if self.save == SaveState::Unsaved && !self.read_only { self.error.as_deref() } else { None },
        }
    }

    fn edited(&mut self) {
        self.dirty = true;
        self.settle();
    }

    fn posted(&mut self, id: Option<&str>) {
        // The bytes just posted carry every change so far; what is edited from now on they do not.
        self.dirty = false;
        // A new attempt replaces the last failure's message; if it fails too, its own arrives.
        self.error = None;
        let Some(id) = id else {
            // Nothing will answer it: the contract has a host answer only a save with an `id`.
            self.settle();
            return;
        };
        if self.pending.len() >= MAX_PENDING_SAVES {
            self.pending.pop_front();
        }
        self.pending.push_back(id.to_string());
        self.save = SaveState::Saving;
    }

    fn confirmed(&mut self, id: &str) {
        // A duplicate `id` removes both, so a repeated answer cannot leave one behind for ever.
        self.pending.retain(|queued| queued != id);
        self.settle();
    }

    fn refused(&mut self, id: Option<&str>, error: &str) {
        if let Some(id) = id {
            self.pending.retain(|queued| queued != id);
        }
        self.error = Some(error.to_string());
        if self.pending.is_empty() && !self.committing {
            // The bytes posted most recently are not stored, whatever the app has changed since.
            self.save = SaveState::Unsaved;
        }
    }

    fn commit_posted(&mut self) {
        self.committing = true;
        self.dirty = false;
        self.error = None;
        self.save = SaveState::Saving;
    }

    fn committed(&mut self, version: u64, etag: &str) {
        self.committing = false;
        // The commit covers the document the app sent, like a save: an edit made after it was posted
        // is not in the stored version, and `settle` keeps the document unsaved for it.
        self.stored = Some(StoredVersion { version, etag: etag.to_string() });
        self.settle();
    }

    /// What an answer leaves the save state as: `Saving…` while a posted save or a commit waits,
    /// otherwise `Unsaved` when changes came after the last post, otherwise `Saved`.
    fn settle(&mut self) {
        if !self.pending.is_empty() || self.committing {
            self.save = SaveState::Saving;
            return;
        }
        if self.dirty {
            self.save = SaveState::Unsaved;
            return;
        }
        self.save = SaveState::Saved;
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(status: &HostStatus) -> Vec<ClusterPart<'_>> {
        status.cluster().parts().collect()
    }

    #[test]
    fn a_posted_save_is_saving_and_its_answer_is_saved() {
        let mut status = HostStatus::new();
        assert_eq!(status.save_state(), SaveState::Saved);

        status.apply(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Unsaved))]);

        status.apply(StatusInput::SavePosted { id: Some("a") });
        assert_eq!(status.save_state(), SaveState::Saving);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saving))]);

        status.apply(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))]);
    }

    #[test]
    fn a_save_in_flight_outranks_a_confirmed_one() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.save_state(), SaveState::Saving);

        // The first save's answer must not turn the cluster to `Saved` while the second waits.
        status.apply(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saving);
        status.apply(StatusInput::Saved { id: "b" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn a_refused_save_returns_to_unsaved_and_keeps_the_error() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SaveFailed { id: Some("a"), error: "disk full" });

        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(status.error(), Some("disk full"));
        let cluster = status.cluster();
        assert_eq!(cluster.headline, Headline::Save(SaveState::Unsaved));
        assert_eq!(cluster.error, Some("disk full"), "the app can say why it is unsaved");

        // A new attempt starts without the old message; success forgets it for good.
        status.apply(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.error(), None);
        status.apply(StatusInput::Saved { id: "b" });
        assert_eq!(status.error(), None);
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn a_refusal_of_a_save_without_an_id_is_still_unsaved() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: None });
        assert_eq!(status.save_state(), SaveState::Saved, "no id: stored once posted");

        status.apply(StatusInput::SaveFailed { id: None, error: "no write access" });
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(status.error(), Some("no write access"));
    }

    #[test]
    fn an_edit_made_while_a_save_was_in_flight_keeps_it_unsaved() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::Edited);
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Saving, "the save in flight still runs");

        status.apply(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit came after the posted bytes");
    }

    #[test]
    fn a_commit_shows_saving_until_the_host_answers_with_its_version() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);

        status.apply(StatusInput::CommitPosted);
        assert_eq!(status.save_state(), SaveState::Saving, "the host has the commit, not the answer");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saving))]);

        status.apply(StatusInput::Committed { version: 7, etag: "\"v7\"" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(status.stored_version().map(|stored| (stored.version, stored.etag.as_str())), Some((7, "\"v7\"")));
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Version(7)]);

        // A later commit replaces the version: the newest stored one is the one to show.
        status.apply(StatusInput::Committed { version: 9, etag: "\"v9\"" });
        assert_eq!(status.cluster().version, Some(9));
    }

    #[test]
    fn an_edit_made_while_a_commit_was_in_flight_keeps_it_unsaved() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::Edited);
        status.apply(StatusInput::CommitPosted);
        status.apply(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Saving, "the commit still waits");

        status.apply(StatusInput::Committed { version: 4, etag: "\"v4\"" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit came after the committed bytes");
        assert_eq!(status.stored_version().map(|stored| stored.version), Some(4), "the stored version is still shown");
    }

    #[test]
    fn an_edit_after_a_commit_is_unsaved_again() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::Committed { version: 3, etag: "\"v3\"" });
        assert_eq!(status.save_state(), SaveState::Saved);

        status.apply(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(
            status.stored_version().map(|stored| stored.version),
            Some(3),
            "the stored version is what was committed, not what is being edited"
        );
    }

    #[test]
    fn the_participant_count_comes_from_session_state_and_presence() {
        let mut status = HostStatus::new();
        assert_eq!(status.participants(), 0);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))], "no session, no count");

        status.apply(StatusInput::SessionState { status: SYNCED, participants: 3 });
        assert_eq!(status.participants(), 3);
        status.apply(StatusInput::Presence { participants: 2 });
        assert_eq!(status.participants(), 2, "someone left");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Participants(2)]);

        status.apply(StatusInput::Presence { participants: 1 });
        assert_eq!(parts(&status)[1], ClusterPart::Participants(1));

        // The last person leaving takes the count with it, leaving the save state alone.
        status.apply(StatusInput::Presence { participants: 0 });
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))]);
    }

    #[test]
    fn two_clients_in_one_room_show_the_same_count() {
        let mut left = HostStatus::new();
        let mut right = HostStatus::new();
        for status in [&mut left, &mut right] {
            status.apply(StatusInput::SessionState { status: SYNCED, participants: 2 });
        }
        assert_eq!(left.cluster().participants, right.cluster().participants);

        // The left client sees the other leave; the count it shows is the session's, not its own.
        left.apply(StatusInput::Presence { participants: 1 });
        assert_eq!(left.cluster().participants, 1);
        assert_eq!(right.cluster().participants, 2);
    }

    #[test]
    fn the_sync_word_is_shown_only_when_it_is_not_synced() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SessionState { status: SYNCED, participants: 0 });
        assert_eq!(status.cluster().sync, None, "synced says nothing");

        status.apply(StatusInput::SessionState { status: "pending", participants: 0 });
        assert_eq!(status.cluster().sync, Some("pending"));
        status.apply(StatusInput::SessionState { status: "offline", participants: 0 });
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Sync("offline")]);

        // The word is the app's own: it is kept as given, not translated here.
        status.apply(StatusInput::SessionState { status: "error", participants: 0 });
        assert_eq!(status.sync_status(), Some("error"));
        status.apply(StatusInput::SessionState { status: SYNCED, participants: 0 });
        assert_eq!(status.sync_status(), Some(SYNCED), "the word is kept");
        assert_eq!(status.cluster().sync, None, "and hidden");
    }

    #[test]
    fn a_read_only_session_says_so_instead_of_a_save_state() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::ReadOnly(true));
        status.apply(StatusInput::Edited);
        status.apply(StatusInput::SaveFailed { id: None, error: "viewers cannot save" });

        assert_eq!(status.save_state(), SaveState::Unsaved, "the state is still tracked");
        let cluster = status.cluster();
        assert_eq!(cluster.headline, Headline::ReadOnly);
        assert_eq!(cluster.error, None, "a read-only session shows no save failure");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::ReadOnly)]);

        // Leaving read-only shows the save state that was tracked meanwhile.
        status.apply(StatusInput::ReadOnly(false));
        assert_eq!(status.cluster().headline, Headline::Save(SaveState::Unsaved));
        assert_eq!(status.cluster().error, Some("viewers cannot save"));
    }

    #[test]
    fn the_cluster_draws_the_parts_in_one_order() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SessionState { status: "pending", participants: 4 });
        status.apply(StatusInput::Committed { version: 12, etag: "\"v12\"" });

        assert_eq!(
            parts(&status),
            [
                ClusterPart::Headline(Headline::Save(SaveState::Saving)),
                ClusterPart::Participants(4),
                ClusterPart::Sync("pending"),
                ClusterPart::Version(12),
            ],
            "headline, people, sync, version: the same order in every app"
        );
    }

    #[test]
    fn an_answer_for_a_forgotten_save_confirms_nothing_and_the_queue_is_bounded() {
        let mut status = HostStatus::new();
        for id in 0..MAX_PENDING_SAVES {
            status.apply(StatusInput::SavePosted { id: Some(&id.to_string()) });
        }
        // More saves than the queue tells apart: the first is forgotten, and its answer changes nothing.
        status.apply(StatusInput::SavePosted { id: Some("extra") });
        status.apply(StatusInput::Saved { id: "0" });
        assert_eq!(status.save_state(), SaveState::Saving, "the newer saves still wait");

        // Answering the saves it remembers settles it, one at a time.
        for id in 1..MAX_PENDING_SAVES {
            status.apply(StatusInput::Saved { id: &id.to_string() });
        }
        assert_eq!(status.save_state(), SaveState::Saving);
        status.apply(StatusInput::Saved { id: "extra" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn an_answer_for_an_id_that_was_never_posted_confirms_the_document_it_covers() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::Edited);
        // A host that answers a save the app posted before this state existed: nothing is in flight,
        // so the answer is for the bytes as they are now.
        status.apply(StatusInput::Saved { id: "from-before" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit is still not stored");

        status.apply(StatusInput::SavePosted { id: Some("from-before") });
        status.apply(StatusInput::Saved { id: "from-before" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn the_unanswered_saves_are_the_ids_an_answer_has_not_removed() {
        let mut status = HostStatus::new();
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());

        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["a", "b"], "oldest first");

        status.apply(StatusInput::Saved { id: "a" });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["b"]);
        status.apply(StatusInput::SaveFailed { id: Some("b"), error: "timeout" });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());

        // A save without an id is never answered, so it never waits.
        status.apply(StatusInput::SavePosted { id: None });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());
    }

    #[test]
    fn a_repeated_answer_does_not_leave_a_save_in_flight() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saved);

        status.apply(StatusInput::Edited);
        status.apply(StatusInput::SavePosted { id: Some("") });
        assert_eq!(status.save_state(), SaveState::Saved, "an empty id is no id: stored once posted");
    }

    #[test]
    fn a_refusal_while_another_save_waits_does_not_end_the_saving() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") });
        status.apply(StatusInput::SavePosted { id: Some("b") });
        status.apply(StatusInput::SaveFailed { id: Some("a"), error: "conflict" });

        assert_eq!(status.save_state(), SaveState::Saving, "b was not answered yet");
        assert_eq!(status.error(), Some("conflict"));

        status.apply(StatusInput::Saved { id: "b" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(status.error(), None, "the newest save is stored");
    }
}
