//! Collaboration: presence, voting, cursor chat and joining a shared document by code
//! (N5.07–N5.09, N5.11), over a [`CollabTransport`] the app implements. The library has no
//! network: FigCraft's sessions, servers and codes stay FigCraft's; the widgets read and ask
//! through the trait.

pub mod chat;
pub mod join;
pub mod presence;
pub mod vote;

pub use chat::{CHAT_FADE_S, CHAT_HOLD_S, CursorChat, chat_alpha};
pub use join::JoinState;
pub use presence::{GLIDE, PEOPLE_COLORS, Participant, ParticipantId, glide, initials};
pub use vote::{VoteScope, VoteSession};

/// The app's side of a shared session, read by the widgets: who is here, the vote and the join
/// in progress. Implemented by the app on its session (FigCraft: over its `collab.*` state). The
/// widgets never act through it: what the user asks for comes back as [`CollabRequest`]s on
/// their outputs, which the app sends (owner rule: widgets report events, they never change the
/// app's state themselves).
///
/// ```ignore
/// impl CollabTransport for Live<'_> {
///     fn me(&self) -> ParticipantId { ParticipantId(self.session.me) }
///     fn participants(&self) -> Vec<Participant> { self.session.peers().map(to_participant).collect() }
///     fn vote(&self) -> Option<VoteSession> { self.board.vote.clone() }
///     fn join_state(&self) -> JoinState { self.join.state() }
///     fn can_remove(&self) -> bool { self.session.hosting || self.session.owns_file }
/// }
/// let out = PeopleBar::new("people", &live).show(ui, canvas);
/// for request in out.requests { live.send(request) }   // collab.follow, collab.spotlight, collab.removePerson, …
/// ```
pub trait CollabTransport {
    /// This participant's id.
    fn me(&self) -> ParticipantId;

    /// Everyone in the session, you included.
    fn participants(&self) -> Vec<Participant>;

    /// The running or last vote.
    fn vote(&self) -> Option<VoteSession>;

    /// The state of the join in progress, or the last one.
    fn join_state(&self) -> JoinState;

    /// Whether you may remove people from the session (you host it or own the file). The people
    /// bar offers Remove on a right-click only when this is true; the default is `false`, so a
    /// transport that cannot remove anyone needs no impl.
    // figcraft: crates/engine `live::can_remove_people`, used at `ui-egui/src/live.rs:485`.
    fn can_remove(&self) -> bool {
        false
    }
}

/// What a collaboration widget asks the app to send, in the order the user did it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CollabRequest {
    /// Your pointer on the canvas (document units), `None` when it left.
    Cursor(Option<[f64; 2]>),
    /// Your cursor chat text; empty closes the bubble.
    Chat(String),
    /// Follow someone's view, or stop.
    Follow(Option<ParticipantId>),
    /// Ask everyone to follow you, or stop.
    Spotlight(bool),
    /// Start a vote.
    StartVote {
        /// Votes each person may cast.
        per_person: u32,
        /// What the vote is on.
        scope: VoteScope,
    },
    /// End the running vote.
    EndVote,
    /// Show the kept results of the last vote.
    ShowResults,
    /// Join with what the user typed (a code or an invite link); the app parses it.
    Join(String),
    /// Cancel the join in progress.
    CancelJoin,
    /// Try the failed join again with what it was started with; the app keeps that (FigCraft
    /// `retry_join`).
    RetryJoin,
    /// Remove a participant from the session (FigCraft `collab.removePerson`); offered only when
    /// [`CollabTransport::can_remove`] is true.
    Remove(ParticipantId),
}
