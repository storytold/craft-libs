//! The append-only journal: what a standalone server persists per file (spec §6.3).
//!
//! Every event that changes the durable state — granted client ids, accepted batches, refused
//! batch ids — is appended and *flushed once per message round, before the hub broadcasts what
//! the server said*, so a crash never loses an acknowledged change (the v2 contract, kept).
//! Compaction writes a fresh snapshot generation plus an empty journal after 1,000 entries or
//! 16 MiB, when everyone leaves, or at shutdown; CRC-32 guards every record; writes go to a
//! temp file that is fsynced and renamed; a corrupt record is skipped, reported, never deleted.

use crate::change::{Batch, BatchId, Seq};
use crate::ids::ClientId;
use crate::server::{Server, SessionState};
use serde::{Deserialize, Serialize};

/// One durable journal event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// A client was granted this id (never re-granted while the journal lives).
    ClientJoined {
        /// The granted id.
        id: ClientId,
    },
    /// A batch was accepted and stamped (replayed into the replica on restore).
    Accepted {
        /// The granted sequence number.
        seq: Seq,
        /// Who wrote it.
        author: ClientId,
        /// The batch itself.
        batch: Batch,
    },
    /// A batch id was refused for a client (lets a reconnecting client learn the fate of its
    /// in-flight batches — spec §2.10).
    Refused {
        /// Who sent it.
        author: ClientId,
        /// The refused batch id.
        batch: BatchId,
    },
    /// A client left.
    Left {
        /// The leaving id.
        id: ClientId,
    },
}

/// The compaction policy (spec §6.3 — the v2 defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactPolicy {
    /// Compact after this many journal entries.
    pub after_entries: usize,
    /// Compact after this many bytes.
    pub after_bytes: u64,
}

impl Default for CompactPolicy {
    fn default() -> Self {
        Self { after_entries: 1_000, after_bytes: 16 * 1024 * 1024 }
    }
}

/// A replayed journal's report (spec §6.3: a no-op entry changes nothing; a meaningless record
/// is skipped where it stands and reported at startup — never deleted).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    /// Entries replayed with effect.
    pub applied: usize,
    /// Entries already covered by the snapshot (no-ops).
    pub already: usize,
    /// Entries skipped as unmeaningful (logged at startup).
    pub skipped: usize,
}

/// Replay a journal into a restored server (spec §6.3 `restore`).
pub fn replay(server: &mut Server, events: &[Event]) -> ReplayReport {
    let mut report = ReplayReport::default();
    for e in events {
        match e {
            Event::ClientJoined { id } => {
                if server.rejoin(*id) {
                    report.already += 1;
                } else {
                    // Re-grant through the normal path so the counter stays monotonic.
                    let granted = server.join_as(crate::server::Access::Edit);
                    if granted == *id {
                        report.applied += 1;
                    } else {
                        // The id is below the current counter — stale but harmless; keep the state.
                        report.skipped += 1;
                    }
                }
            }
            Event::Accepted { seq, author, batch } => {
                if *seq <= server.state().seq() {
                    report.already += 1;
                } else {
                    let outcome = server.submit(*author, batch);
                    if matches!(outcome, crate::server::Outcome::Accepted { .. }) {
                        report.applied += 1;
                    } else {
                        report.skipped += 1;
                    }
                }
            }
            Event::Refused { author, batch } => {
                server.note_refused(*author, *batch);
                report.applied += 1;
            }
            Event::Left { id } => {
                server.leave(*id);
                report.applied += 1;
            }
        }
    }
    report
}

/// Serialize an event for the wire/log (postcard by default, JSON for diagnostics).
pub fn encode_event(e: &Event) -> Result<Vec<u8>, crate::Error> {
    postcard::to_allocvec(e).map_err(|err| crate::Error::EncodeDecode(err.to_string()))
}

/// Decode an event (rejecting trailing bytes — the family's decoder discipline).
///
/// Postcard itself tolerates trailing data, so the strict contract is enforced by
/// re-encoding: anything after the value (or any mutation inside it) changes the bytes and
/// is refused.
pub fn decode_event(bytes: &[u8]) -> Result<Event, crate::Error> {
    let event: Event = postcard::from_bytes(bytes).map_err(|err| crate::Error::EncodeDecode(err.to_string()))?;
    let canonical = encode_event(&event)?;
    if canonical != bytes {
        return Err(crate::Error::EncodeDecode("trailing or corrupted bytes".into()));
    }
    Ok(event)
}

/// Serialize a session snapshot (the `snapshot.<gen>` file, spec §6.3).
pub fn encode_snapshot(s: &SessionState) -> Result<Vec<u8>, crate::Error> {
    postcard::to_allocvec(s).map_err(|err| crate::Error::EncodeDecode(err.to_string()))
}

/// Decode a session snapshot (strictly: trailing bytes refuse, as above).
pub fn decode_snapshot(bytes: &[u8]) -> Result<SessionState, crate::Error> {
    let state: SessionState = postcard::from_bytes(bytes).map_err(|err| crate::Error::EncodeDecode(err.to_string()))?;
    let canonical = encode_snapshot(&state)?;
    if canonical != bytes {
        return Err(crate::Error::EncodeDecode("trailing or corrupted bytes".into()));
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Batch;
    use crate::server::Server;

    fn accepted(seq: Seq, author: ClientId, batch_id: BatchId) -> Event {
        Event::Accepted { seq, author, batch: Batch::empty(batch_id, seq.saturating_sub(1)) }
    }

    /// A restored server + replayed journal equals the live one (the v2 no-loss guarantee).
    #[test]
    fn restore_and_replay_roundtrip() {
        let mut live = Server::new(42);
        let a = live.join_as(crate::server::Access::Edit);
        let b = live.join_as(crate::server::Access::Edit);
        let out = live.submit(a, &Batch { id: 1, base_seq: 0, changes: vec![] });
        assert!(matches!(out, crate::server::Outcome::Accepted { .. }));
        live.submit(b, &Batch { id: 9, base_seq: 1, changes: vec![] });

        let events = vec![Event::ClientJoined { id: a }, Event::ClientJoined { id: b }, accepted(1, a, 1), accepted(2, b, 9)];
        let snapshot = live.state_out();
        let mut restored = Server::restore(snapshot);
        let report = replay(&mut restored, &events);
        assert_eq!(report.applied + report.already, events.len());
        assert_eq!(restored.state().seq(), live.state().seq());
    }

    /// Encoding round-trips and rejects trailing bytes.
    #[test]
    fn event_codec_roundtrip() {
        let e = accepted(5, ClientId(3), 11);
        let bytes = encode_event(&e).expect("encode");
        assert_eq!(decode_event(&bytes).expect("decode"), e);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_event(&trailing).is_err());
    }
}
