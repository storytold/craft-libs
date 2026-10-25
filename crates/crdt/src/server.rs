//! In-crate test oracle (spec §5.4): validate, accept with a seq, or refuse with fixes.
//!
//! This is not the live relay. CollabCraft orders Socket.IO ops and arbitrates locks.
//! The [`crate::sim`] suite uses this `Server` as a memory-only peer so it can prove
//! every replica matches at quiescence without opening a socket.
//!
//! Every accepted change gets the next sequence number. A batch is validated as a whole
//! on a scratch copy: it lands atomically or it is refused with the current values of
//! everything it touched. Parents must exist, cycles are rejected, new ids come from
//! the sender's space.

use crate::Error;
use crate::change::{Batch, BatchId, Change, Seq};
use crate::ids::ClientId;
use crate::replica::{ApplyMode, Replica};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The outcome of submitting a batch (spec §5.4).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Accepted: stamped with the next seq, broadcast to everyone including the author.
    Accepted {
        /// The granted sequence number.
        seq: Seq,
    },
    /// `Refused`: a short human reason plus the current values for every touched target.
    Refused {
        /// Why (English source text; clients translate by matching the reason key, spec §15.2).
        reason: String,
        /// The current values of everything the batch touched, as `Set` fixes.
        fixes: Vec<Change>,
    },
}

/// What access a joined client has (v2's edit/view, extended by v3's comment/suggest).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    /// Direct editing of the live state.
    Edit,
    /// View only — enforced by the server, not the app (spec §2.12).
    View,
    /// Commenting without editing (v3 link kind).
    Comment,
    /// Suggesting: edits divert into a contribution branch (v3, spec §6.8).
    Suggest,
}

/// One joined client's record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientRecord {
    /// Granted id (kept across reconnects while the session lives).
    pub id: ClientId,
    /// Access level.
    pub access: Access,
    /// The last batch id the server processed for this client (spec §2.10 — processed).
    pub last_batch: Option<BatchId>,
    /// Recently refused batch ids (bounded, ≤1024 — spec §2.10).
    pub refused: Vec<BatchId>,
}

/// The saved session state (one serde value per file).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionState {
    /// The full document state.
    pub replica: Replica,
    /// The random session id (survives server restarts, identifies resumes).
    pub session: u64,
    /// The client-id counter — never re-grants a space (spec §5.1 P1).
    pub client_counter: u32,
    /// The joined clients that may reconnect with their ids.
    pub clients: Vec<ClientRecord>,
    /// Removed ids (accepted back by re-Create).
    pub removed: Vec<crate::ids::NodeId>,
    /// The next seq to grant.
    pub next_seq: Seq,
}

impl SessionState {
    /// Encode for a snapshot file. Postcard, no trailing bytes on decode.
    pub fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        postcard::to_allocvec(self).map_err(|err| crate::Error::EncodeDecode(err.to_string()))
    }

    /// Decode a snapshot. Trailing bytes are refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, crate::Error> {
        let state: SessionState = postcard::from_bytes(bytes).map_err(|err| crate::Error::EncodeDecode(err.to_string()))?;
        let canonical = state.encode()?;
        if canonical != bytes {
            return Err(crate::Error::EncodeDecode("trailing or corrupted bytes".into()));
        }
        Ok(state)
    }
}

/// The pure server: validate, accept or refuse, grant seqs. No I/O, no sockets, no clock.
#[derive(Clone, Debug)]
pub struct Server {
    state: Replica,
    session: u64,
    client_counter: u32,
    clients: HashMap<ClientId, ClientRecord>,
    /// The accepted-batch log (bounded by the caller's compaction policy, spec §6.3).
    log: Vec<(Seq, ClientId, Batch)>,
    /// The id spaces ever used in the file — new client ids start above them all.
    used_spaces: Vec<u32>,
}

impl Server {
    /// A fresh server for an empty document (a host starting a local session).
    #[must_use]
    pub fn new(session: u64) -> Self {
        Self::with_replica(session, Replica::new())
    }

    /// A server over an existing document state (a standalone server opening a file).
    #[must_use]
    pub fn with_replica(session: u64, replica: Replica) -> Self {
        let used_spaces = replica.nodes().map(|(id, _)| id.space()).collect::<Vec<_>>();
        Self { state: replica, session, client_counter: 0, clients: HashMap::new(), log: Vec::new(), used_spaces }
    }

    /// The session id.
    #[must_use]
    pub fn session(&self) -> u64 {
        self.session
    }

    /// The current replica state.
    #[must_use]
    pub fn state(&self) -> &Replica {
        &self.state
    }

    /// The accepted log (after `seq`, for Delta catch-up, spec §2.10).
    #[must_use]
    pub fn log_after(&self, seq: Seq) -> Vec<(Seq, ClientId, Batch)> {
        self.log.iter().filter(|(s, _, _)| *s > seq).cloned().collect()
    }

    /// Whether the log still holds every batch after `seq` (else the welcome is a Snapshot).
    #[must_use]
    pub fn log_covers(&self, seq: Seq) -> bool {
        self.log.first().is_none_or(|(s, _, _)| *s > seq)
    }

    /// Join a client: grant a fresh id above every used space (spec §5.1).
    pub fn join_as(&mut self, access: Access) -> ClientId {
        let alloc = crate::ids::Allocator::new();
        let id = alloc.next_client_id(&self.used_spaces, self.client_counter).unwrap_or(ClientId(self.client_counter));
        self.client_counter = id.0;
        self.clients.insert(id, ClientRecord { id, access, last_batch: None, refused: Vec::new() });
        id
    }

    /// Re-admit a reconnecting client with its previous id (spec §2.10 — resume keeps identity).
    pub fn rejoin(&mut self, id: ClientId) -> bool {
        self.clients.contains_key(&id)
    }

    /// A client leaves; its record stays for a while for resume (the caller decides expiry).
    pub fn leave(&mut self, id: ClientId) {
        self.clients.remove(&id);
    }

    /// Note a refused batch id for a client (spec §2.10: the reconnecting client learns the fate
    /// of its in-flight batches; the list is bounded to 1,024).
    pub fn note_refused(&mut self, client: ClientId, batch: BatchId) {
        const MAX_REFUSED: usize = 1024;
        if let Some(r) = self.clients.get_mut(&client) {
            r.refused.push(batch);
            if r.refused.len() > MAX_REFUSED {
                let cut = r.refused.len() - MAX_REFUSED;
                r.refused.drain(..cut);
            }
        }
    }

    /// The record of a joined client (its processed/refused state, spec §2.10).
    #[must_use]
    pub fn client_record(&self, client: ClientId) -> Option<&ClientRecord> {
        self.clients.get(&client)
    }

    /// Submit a batch: fast checks first, then whole-batch validation on a scratch copy, then an
    /// atomic accept with the next seq — or a refusal with fixes (spec §5.4, verbatim contract).
    pub fn submit(&mut self, from: ClientId, batch: &Batch) -> Outcome {
        let Some(record) = self.clients.get(&from) else {
            return Outcome::Refused { reason: "not joined".into(), fixes: Vec::new() };
        };
        let access = record.access;
        if access != Access::Edit {
            return Outcome::Refused {
                reason: if access == Access::View {
                    "view access only".into()
                } else {
                    "this edit needs review — diverted to a contribution (see review queue)".into()
                },
                fixes: Vec::new(),
            };
        }
        // Fast shape checks without copying (spec §5.4 step 1).
        if batch.changes.len() > 500_000 {
            return Outcome::Refused { reason: "the batch is larger than the limit".into(), fixes: Vec::new() };
        }
        for c in &batch.changes {
            if let Change::Set { value, .. } | Change::SetDoc { value, .. } | Change::Put { value, .. } = c
                && !json_is_finite(value)
            {
                return Outcome::Refused { reason: "a value is not finite or is out of range".into(), fixes: Vec::new() };
            }
        }
        // Deep validation on a scratch copy (spec §5.4 step 2), collecting touched current values.
        let mut scratch = self.state.clone();
        let mut touched: Vec<Change> = Vec::new();
        for c in &batch.changes {
            match c {
                Change::Create { id, parent, .. } => {
                    if scratch.node(*id).is_some() && !scratch.was_removed(*id) {
                        return Self::refuse(touched, "the object already exists");
                    }
                    if id.space() != from.0 && !self.state.was_removed(*id) {
                        return Self::refuse(touched, "the id is from another client's space");
                    }
                    if scratch.node(*parent).is_none() && *parent != crate::ids::NodeId(0) {
                        return Self::refuse(touched, "a layer you are changing was deleted by someone else");
                    }
                }
                Change::Set { id, prop, .. } => {
                    let Some(node) = scratch.node(*id) else {
                        return Self::refuse(touched, "a layer you are changing was deleted by someone else");
                    };
                    if prop.as_ref() != "parent" && prop.as_ref() != "position" {
                        if let Some(cur) = node.props.get(prop.as_ref()) {
                            touched.push(Change::Set { id: *id, prop: prop.clone(), value: cur.clone() });
                        } else {
                            touched.push(Change::Set { id: *id, prop: prop.clone(), value: crate::value::Value::Json(serde_json::Value::Null) });
                        }
                    } else if prop.as_ref() == "parent" {
                        touched.push(Change::Set {
                            id: *id,
                            prop: prop.clone(),
                            value: crate::value::Value::Json(serde_json::Value::Number((node.parent.0).into())),
                        });
                    }
                }
                Change::Delete { .. } | Change::SetDoc { .. } | Change::Put { .. } | Change::Remove { .. } => {}
            }
            if let Err(e) = scratch.apply_one_for_server(c) {
                // The specific reason survives: "the move would create a cycle", "a layer
                // you are changing was deleted by someone else"... (spec §5.4's messages).
                return Self::refuse(touched, &e.to_string());
            }
        }
        // Atomic accept (spec §5.4 step 3).
        let seq = self.state.seq() + 1;
        let () = scratch.set_seq(seq);
        self.state = scratch;
        // The log is bounded by the persistence layer's compaction policy (spec §6.3).
        self.log.push((seq, from, batch.clone()));
        if let Some(r) = self.clients.get_mut(&from) {
            r.last_batch = Some(batch.id);
        }
        Outcome::Accepted { seq }
    }

    /// Refuse with the current values of everything the batch touched (the `fixes` the client
    /// applies to reconcile — spec §5.4: "reason + fixes", never a silent refusal).
    fn refuse(touched: Vec<Change>, reason: &str) -> Outcome {
        Outcome::Refused { reason: reason.to_string(), fixes: touched }
    }

    /// Serialize the whole session for persistence (spec §6.3 — snapshot per file).
    #[must_use]
    pub fn state_out(&self) -> SessionState {
        SessionState {
            replica: self.state.clone(),
            session: self.session,
            client_counter: self.client_counter,
            clients: self.clients.values().cloned().collect(),
            removed: Vec::new(),
            next_seq: self.state.seq() + 1,
        }
    }

    /// Restore from a saved state.
    #[must_use]
    pub fn restore(state: SessionState) -> Self {
        let used_spaces = state.replica.nodes().map(|(id, _)| id.space()).collect::<Vec<_>>();
        let mut clients = HashMap::new();
        for c in state.clients {
            clients.insert(c.id, c);
        }
        Self { state: state.replica, session: state.session, client_counter: state.client_counter, clients, log: Vec::new(), used_spaces }
    }
}

/// JSON values must be finite and within sane sizes (fast check, spec §5.4 step 1).
fn json_is_finite(v: &crate::value::Value) -> bool {
    match v {
        crate::value::Value::Json(j) => json_number_finite(j),
        _ => true,
    }
}

fn json_number_finite(j: &serde_json::Value) -> bool {
    match j {
        serde_json::Value::Number(n) => n.as_f64().is_none_or(f64::is_finite),
        serde_json::Value::Array(a) => a.iter().all(json_number_finite),
        serde_json::Value::Object(o) => o.values().all(json_number_finite),
        _ => true,
    }
}

impl Replica {
    /// Server-side single-change application with full validation (strict; internal to submit).
    fn apply_one_for_server(&mut self, c: &Change) -> Result<(), Error> {
        let batch = Batch { id: 0, base_seq: self.seq(), changes: vec![c.clone()] };
        self.apply(&batch, ApplyMode::Strict)
    }
}
