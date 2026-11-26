//! Voting sessions on a board (N5.08).

/// What a vote is on (FigCraft `board_facilitate.rs:21`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VoteScope {
    #[default]
    /// Every object on the board.
    All,
    /// Sticky notes only.
    Stickies,
    /// The objects currently selected.
    Selection,
}

impl VoteScope {
    /// Every scope, in menu order.
    pub const ALL: [VoteScope; 3] = [VoteScope::All, VoteScope::Stickies, VoteScope::Selection];
}

/// A running or ended vote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoteSession {
    /// Votes each person may cast.
    pub per_person: u32,
    /// What the vote is on.
    pub scope: VoteScope,
    /// True once the vote has ended.
    pub ended: bool,
    /// Your votes used.
    pub used: u32,
    /// Votes per object (the app's object ids), most first, for the results.
    pub tallies: Vec<(u64, u32)>,
    /// Earlier votes' results are kept.
    pub has_history: bool,
}

impl VoteSession {
    /// Votes you have left.
    pub fn left(&self) -> u32 {
        self.per_person.saturating_sub(self.used)
    }
}
