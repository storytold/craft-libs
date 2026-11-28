//! Joining a shared document by code or invite link (N5.11).

/// Where a join is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JoinState {
    /// No join running.
    #[default]
    Idle,
    /// Connecting with what was typed.
    Joining,
    /// In the session.
    Joined,
    /// The connection dropped and is being retried (`attempt` from 1).
    Reconnecting {
        /// The retry number, counting from 1.
        attempt: u32,
    },
    /// Couldn't join; the transport's reason, shown as it is.
    Failed(String),
}

impl JoinState {
    /// One join at a time: the Join field waits while this is true.
    pub fn pending(&self) -> bool {
        matches!(self, JoinState::Joining | JoinState::Reconnecting { .. })
    }
}
