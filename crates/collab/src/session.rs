//! The client session: ordering, deduplication and lock bookkeeping.
//!
//! A [`Session`] holds no transport and performs no IO. Feed it [`ServerMsg`] values and it
//! returns [`Effect`] values telling the app what to do. That keeps the hard part, which is
//! ordering, pure and testable, and it means the same logic runs behind a native Socket.IO client
//! and behind the browser's.
//!
//! The ordering rule is the whole design. The relay assigns a strictly increasing `seq` to every
//! edit, and a client applies them in that order and no other. An op that arrives early is held
//! until its predecessors land. An op the client itself sent is confirmed instead of applied,
//! because the app already applied it locally when the user typed.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::proto::{ClientMsg, DocId, LockState, PeerId, Presence, Scope, ServerMsg};

/// Ops held past this many waiting for a gap to fill. Beyond it the session asks for a resync,
/// because a gap that wide means something was lost rather than reordered.
const MAX_PENDING: usize = 512;

/// What the app should do in response to relay traffic.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Run this command against the document. It came from another peer and is next in order.
    Apply { command: String, params: Value },
    /// A command this client sent has been ordered. Nothing to apply, the local copy is already
    /// correct, but any optimistic marker on it can be cleared.
    Confirmed { local: u64, seq: u64 },
    /// Replace the whole document, then continue from `seq`.
    Replace { document: Value, seq: u64 },
    /// A lock request succeeded. The app may now edit that scope.
    LockHeld { scope: Scope },
    /// A lock request failed. `holder` is who has it.
    LockRefused { scope: Scope, holder: PeerId },
    /// A scope this client held is no longer held.
    LockDropped { scope: Scope },
    /// Somebody else's lock changed. Useful for showing a region as busy.
    RemoteLocks { locks: Vec<LockState> },
    /// The roster changed.
    Roster { roster: Vec<Presence> },
    /// Tell the user something was refused.
    Notice { detail: String },
    /// Send this back to the relay. The session asks for its own resyncs this way.
    Send { msg: ClientMsg },
}

/// One document's shared-editing state.
pub struct Session {
    doc: DocId,
    me: Option<PeerId>,
    /// Highest sequence applied in order.
    applied: u64,
    /// Ops that arrived before their turn, keyed by sequence.
    pending: BTreeMap<u64, ServerMsg>,
    /// This client's own counter for optimistic edits.
    local: u64,
    /// Local counters submitted and not yet confirmed.
    inflight: HashSet<u64>,
    /// Scopes this client holds.
    held: HashSet<Scope>,
    /// Scopes other peers hold.
    remote_locks: HashMap<Scope, PeerId>,
    roster: Vec<Presence>,
}

impl Session {
    pub fn new(doc: impl Into<DocId>) -> Session {
        Session {
            doc: doc.into(),
            me: None,
            applied: 0,
            pending: BTreeMap::new(),
            local: 0,
            inflight: HashSet::new(),
            held: HashSet::new(),
            remote_locks: HashMap::new(),
            roster: Vec::new(),
        }
    }

    pub fn doc(&self) -> &str {
        &self.doc
    }

    /// This client's peer id, once the relay has assigned one.
    pub fn peer(&self) -> Option<&str> {
        self.me.as_deref()
    }

    /// Highest sequence applied in order.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    pub fn roster(&self) -> &[Presence] {
        &self.roster
    }

    /// Whether this client holds `scope`.
    pub fn holds(&self, scope: &str) -> bool {
        self.held.contains(scope)
    }

    /// Who holds `scope`, when somebody else does.
    pub fn blocked_by(&self, scope: &str) -> Option<&str> {
        self.remote_locks.get(scope).map(String::as_str)
    }

    /// Whether this client may edit `scope` right now.
    ///
    /// An unlocked scope is editable. The optimistic case matters: a scope nobody has claimed is
    /// fair game, and the relay is the arbiter if two peers claim it at once.
    pub fn may_edit(&self, scope: &str) -> bool {
        !self.remote_locks.contains_key(scope)
    }

    /// The message to send when entering the document.
    pub fn join(&self, display: impl Into<String>) -> ClientMsg {
        ClientMsg::Join { doc: self.doc.clone(), display: display.into() }
    }

    /// Ask for a scope.
    pub fn request_lock(&self, scope: impl Into<Scope>) -> ClientMsg {
        ClientMsg::Lock { doc: self.doc.clone(), scope: scope.into() }
    }

    /// Give a scope back. Dropping it locally is immediate, because a release cannot be refused.
    pub fn release_lock(&mut self, scope: impl Into<Scope>) -> ClientMsg {
        let scope = scope.into();
        self.held.remove(&scope);
        ClientMsg::Unlock { doc: self.doc.clone(), scope }
    }

    /// Offer a local edit for ordering.
    ///
    /// The app applies the command locally first and calls this to broadcast it, which is what
    /// makes typing feel immediate. The returned message carries a local counter that comes back
    /// as [`Effect::Confirmed`].
    pub fn submit(&mut self, command: impl Into<String>, params: Value, scope: Option<Scope>) -> ClientMsg {
        self.local += 1;
        self.inflight.insert(self.local);
        ClientMsg::Submit { doc: self.doc.clone(), local: self.local, command: command.into(), params, scope }
    }

    /// How many local edits are awaiting confirmation.
    pub fn inflight(&self) -> usize {
        self.inflight.len()
    }

    /// Feed one relay message in, get the app's work out.
    ///
    /// A message for another document is ignored, so one handler can serve several sessions.
    pub fn handle(&mut self, msg: ServerMsg) -> Vec<Effect> {
        if msg.doc() != self.doc {
            log::debug!("session {} ignoring a message for {}", self.doc, msg.doc());
            return Vec::new();
        }
        match msg {
            ServerMsg::Joined { peer, seq, roster, locks, .. } => {
                self.me = Some(peer);
                self.applied = seq;
                self.pending.clear();
                self.roster = roster.clone();
                self.remote_locks.clear();
                self.held.clear();
                let me = self.me.clone().unwrap_or_default();
                for l in &locks {
                    if l.holder == me {
                        self.held.insert(l.scope.clone());
                    } else {
                        self.remote_locks.insert(l.scope.clone(), l.holder.clone());
                    }
                }
                vec![Effect::Roster { roster }, Effect::RemoteLocks { locks }]
            }

            op @ ServerMsg::Op { .. } => self.take_op(op),

            ServerMsg::Snapshot { document, seq, .. } => {
                self.applied = seq;
                self.pending.clear();
                // Local edits cannot be reconciled against a document we did not derive, so they
                // are dropped. The snapshot already reflects whatever the relay accepted.
                self.inflight.clear();
                vec![Effect::Replace { document, seq }]
            }

            ServerMsg::LockGranted { scope, .. } => {
                self.remote_locks.remove(&scope);
                self.held.insert(scope.clone());
                vec![Effect::LockHeld { scope }]
            }

            ServerMsg::LockDenied { scope, holder, .. } => {
                self.held.remove(&scope);
                self.remote_locks.insert(scope.clone(), holder.clone());
                vec![Effect::LockRefused { scope, holder }]
            }

            ServerMsg::LockReleased { scope, .. } => {
                self.held.remove(&scope);
                self.remote_locks.remove(&scope);
                vec![Effect::LockDropped { scope }]
            }

            ServerMsg::Presence { roster, .. } => {
                self.roster = roster.clone();
                vec![Effect::Roster { roster }]
            }

            ServerMsg::Error { detail, .. } => vec![Effect::Notice { detail }],
        }
    }

    /// Order one op, draining anything it unblocks.
    fn take_op(&mut self, op: ServerMsg) -> Vec<Effect> {
        let Some(seq) = op_seq(&op) else { return Vec::new() };

        // Already seen. The relay re-sends after a reconnect, so this is normal.
        if seq <= self.applied {
            return Vec::new();
        }

        if seq > self.applied + 1 {
            // Out of order. Hold it and wait.
            self.pending.insert(seq, op);
            if self.pending.len() > MAX_PENDING {
                log::warn!("doc {}: {} ops held, asking for a resync", self.doc, self.pending.len());
                self.pending.clear();
                return vec![Effect::Send { msg: ClientMsg::Resync { doc: self.doc.clone(), have: self.applied } }];
            }
            return Vec::new();
        }

        let mut out = Vec::new();
        self.consume(op, &mut out);
        // Drain the run that is now contiguous.
        while let Some(next) = self.pending.remove(&(self.applied + 1)) {
            self.consume(next, &mut out);
        }
        out
    }

    /// Apply or confirm one in-order op.
    fn consume(&mut self, op: ServerMsg, out: &mut Vec<Effect>) {
        let ServerMsg::Op { seq, origin, local, command, params, .. } = op else { return };
        self.applied = seq;

        let mine = self.me.as_deref() == Some(origin.as_str());
        match (mine, local) {
            // Our own edit, coming back ordered. The document already has it.
            (true, Some(local)) => {
                self.inflight.remove(&local);
                out.push(Effect::Confirmed { local, seq });
            }
            // Ours but with no counter, which should not happen. Applying it would duplicate the
            // edit, so it is dropped with a note in the log.
            (true, None) => log::warn!("doc {}: own op at seq {seq} carried no local counter", self.doc),
            (false, _) => out.push(Effect::Apply { command, params }),
        }
    }
}

fn op_seq(op: &ServerMsg) -> Option<u64> {
    match op {
        ServerMsg::Op { seq, .. } => Some(*seq),
        _ => None,
    }
}
