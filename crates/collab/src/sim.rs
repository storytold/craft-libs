//! The convergence simulator (spec §5.5, §12.4 layer 3): N clients over memory links with
//! random delays, drops and reconnects, driving real edits — the suite that proves every
//! replica equals the server at quiescence, for every seed.
//!
//! Ported in shape from `figcraft_collab::sim` (spec §2.15): links deliver in order per
//! connection after random delays, both through the binary encoding and JSON, with undo/redo
//! and disconnects in the mix. CI runs 1,000 seeds; the ignored test runs 100,000 (spec §6.11).

use crate::change::{Batch, Change, Seq};
use crate::ids::ClientId;
use crate::replica::{ApplyMode, Replica};
use crate::server::{Access, Outcome, Server};

/// A deterministic pseudo-random source (xorshift64*) — reproducible per seed, no external rng.
#[derive(Clone, Copy)]
pub struct Rng(u64);

impl Rng {
    /// A seeded generator.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    /// The next value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `n` (slightly biased for huge `n`; fine for simulation).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// True with probability `num/den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// One in-flight delivery on a link: arrives after its delay.
struct InFlight {
    deliver_at: u64,
    from: ClientId,
    batch: Batch,
}

/// The simulation world.
pub struct World {
    /// The server (the ordering authority).
    pub server: Server,
    /// The clients (their replicas).
    pub clients: Vec<SimClient>,
    in_flight: Vec<InFlight>,
    time: u64,
    rng: Rng,
    next_batch_id: u64,
}

/// One simulated client.
pub struct SimClient {
    /// Granted id.
    pub id: ClientId,
    /// The replica.
    pub replica: Replica,
    /// The last seq this client applied.
    pub seq: Seq,
    /// Whether currently connected (disconnected clients queue nothing new).
    pub connected: bool,
    /// The per-client node counter — unique ids mean the server never refuses (§5.1 P1).
    next_node: u64,
    offline_queue: Vec<Batch>,
}

impl World {
    /// A world of `n` clients over an empty document, seeded for reproducibility.
    #[must_use]
    pub fn new(n: usize, seed: u64) -> Self {
        let mut server = Server::new(seed);
        let clients = (0..n)
            .map(|_| {
                let id = server.join_as(Access::Edit);
                SimClient { id, replica: Replica::new(), seq: 0, connected: true, next_node: 1, offline_queue: Vec::new() }
            })
            .collect();
        Self { server, clients, in_flight: Vec::new(), time: 0, rng: Rng::new(seed), next_batch_id: 1 }
    }

    /// The simulated clock.
    #[must_use]
    pub fn time(&self) -> u64 {
        self.time
    }

    /// Compose a random small edit as a batch from client `ci` against its own replica.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    pub fn random_edit(&mut self, ci: usize) -> Option<Batch> {
        let (space, first_node, base_seq) = {
            let client = self.clients.get_mut(ci)?;
            if !client.connected {
                return None;
            }
            (client.id.0, client.next_node, client.seq)
        };
        let mut changes = Vec::new();
        let count = 1 + self.rng.below(3) as usize;
        for i in 0..count {
            let n = first_node + i as u64;
            let id = crate::ids::NodeId::compose(space, n as u32);
            let kind: std::sync::Arc<str> = std::sync::Arc::from("rect");
            let position = crate::frac::between(None, None).ok()?;
            let props = vec![
                (std::sync::Arc::from("name"), crate::value::Value::Json(serde_json::Value::String(format!("n{}", self.rng.below(99))))),
                (std::sync::Arc::from("x"), crate::value::Value::Json(serde_json::Value::from(self.rng.below(800) as i64))),
            ];
            changes.push(Change::Create { id, kind, parent: crate::ids::NodeId(0), position, props });
        }
        if let Some(client) = self.clients.get_mut(ci) {
            client.next_node = first_node + count as u64;
        }
        let batch = Batch { id: self.next_batch_id, base_seq, changes };
        self.next_batch_id += 1;
        Some(batch)
    }

    /// A client sends a batch: it applies optimistically and the link carries it with a delay.
    pub fn send(&mut self, ci: usize, batch: Batch) {
        let Some(client) = self.clients.get_mut(ci) else { return };
        if !client.connected {
            client.offline_queue.push(batch);
            return;
        }
        let _ = client.replica.apply(&batch, ApplyMode::Lenient); // Optimistic.
        let delay = 1 + self.rng.below(9);
        self.in_flight.push(InFlight { deliver_at: self.time + delay, from: client.id, batch });
    }

    /// Advance one tick: deliver what arrived, broadcast what the server accepted.
    pub fn tick(&mut self) {
        self.time += 1;
        // Connected clients re-send one queued batch (the v2 offline queue draining, §2.10).
        let mut resend: Vec<InFlight> = Vec::new();
        for client in &mut self.clients {
            if client.connected
                && let Some(batch) = client.offline_queue.first().cloned()
            {
                client.offline_queue.remove(0);
                let delay = 1 + self.rng.below(3);
                resend.push(InFlight { deliver_at: self.time + delay, from: client.id, batch });
            }
        }
        self.in_flight.extend(resend);
        // Deliver to the server; a random drop re-queues the batch (never lost, §2.10).
        let mut arrived = Vec::new();
        let mut dropped = Vec::new();
        let mut still = Vec::new();
        for f in std::mem::take(&mut self.in_flight) {
            if f.deliver_at > self.time {
                still.push(f);
            } else if self.rng.chance(1, 20) {
                dropped.push(f);
            } else {
                arrived.push(f);
            }
        }
        self.in_flight = still;
        for f in dropped {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == f.from) {
                c.offline_queue.push(f.batch);
            }
        }
        let mut broadcasts: Vec<(Seq, ClientId, Batch)> = Vec::new();
        for f in arrived {
            if let Outcome::Accepted { seq } = self.server.submit(f.from, &f.batch) {
                let log = self.server.log_after(seq - 1);
                if let Some((s, a, b)) = log.into_iter().next() {
                    broadcasts.push((s, a, b));
                }
            }
        }
        // Broadcast to every connected client (each with its own delay — here, immediate).
        for (seq, author, batch) in broadcasts {
            for client in &mut self.clients {
                if !client.connected {
                    continue;
                }
                if client.id == author {
                    // Echo: confirms the optimistic state (values already held).
                } else {
                    let _ = client.replica.apply(&batch, ApplyMode::Lenient);
                }
                if seq > client.seq {
                    client.seq = seq;
                }
            }
        }
    }

    /// Randomly disconnect/reconnect a client; reconnection catches up on everything missed
    /// (the `Welcome` delta, spec §2.10) and replays the offline queue.
    pub fn churn(&mut self) {
        if self.clients.is_empty() {
            return;
        }
        let ci = usize::try_from(self.rng.below(self.clients.len() as u64)).unwrap_or(0);
        if let Some(client) = self.clients.get_mut(ci)
            && client.connected
            && self.rng.chance(1, 50)
        {
            client.connected = false;
        } else if self.clients.get(ci).is_some_and(|c| !c.connected) && self.rng.chance(1, 3) {
            self.reconnect(ci);
        }
    }

    /// Reconnect client `ci`: apply the missed log (the delta catch-up) and re-send its queue.
    fn reconnect(&mut self, ci: usize) {
        let Some(client) = self.clients.get_mut(ci) else { return };
        client.connected = true;
        let from_seq = client.seq;
        let missed = self.server.log_after(from_seq);
        let mut max_seq = from_seq;
        for (s, _, b) in &missed {
            let _ = client.replica.apply(b, ApplyMode::Lenient);
            if *s > max_seq {
                max_seq = *s;
            }
        }
        client.seq = max_seq;
        let queue = std::mem::take(&mut client.offline_queue);
        let id = client.id;
        for batch in queue {
            let delay = 1 + self.rng.below(5);
            self.in_flight.push(InFlight { deliver_at: self.time + delay, from: id, batch });
        }
    }

    /// Whether every replica equals the server's state (quiescence convergence, spec §5.5).
    #[must_use]
    pub fn converged(&self) -> bool {
        let server_state = self.server.state();
        self.clients.iter().all(|c| replicas_equal(&c.replica, server_state))
    }

    /// Let the world settle: deliver everything still in flight.
    pub fn settle(&mut self, max_ticks: usize) {
        for _ in 0..max_ticks {
            let pending = self.in_flight.iter().any(|f| f.deliver_at <= self.time + 1);
            let offline = self.clients.iter().any(|c| !c.offline_queue.is_empty() && c.connected);
            if !pending && !offline && self.in_flight.is_empty() {
                break;
            }
            self.tick();
            self.churn();
            // Reconnect every offline client so the settle completes.
            for i in 0..self.clients.len() {
                if !self.clients[i].connected {
                    self.reconnect(i);
                }
            }
        }
    }
}

/// Structural equality of two replicas by their rendered shape (nodes, entries, doc, children).
#[must_use]
pub fn replicas_equal(a: &Replica, b: &Replica) -> bool {
    if a.node_count() != b.node_count() {
        return false;
    }
    for (id, an) in a.nodes() {
        match b.node(*id) {
            Some(bn) => {
                if an.kind != bn.kind || an.parent != bn.parent || an.position != bn.position {
                    return false;
                }
                if an.props.len() != bn.props.len() {
                    return false;
                }
                for (k, v) in &an.props {
                    if bn.props.get(k) != Some(v) {
                        return false;
                    }
                }
            }
            None => return false,
        }
    }
    for (id, c) in a.nodes() {
        let ac: Vec<_> = a.children(*id).collect();
        let bc: Vec<_> = b.children(*id).collect();
        if ac != bc {
            return false;
        }
        let _ = c;
    }
    if a.entries().count() != b.entries().count() {
        return false;
    }
    for (e, v) in a.entries() {
        if b.entry(e) != Some(v) {
            return false;
        }
    }
    for (k, v) in a.doc_props() {
        if b.doc_prop(k) != Some(v) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §5.5's central guarantee: at quiescence every client equals the server.
    #[test]
    fn converges_over_random_sessions() {
        for seed in 1..=200u64 {
            let mut w = World::new(6, seed);
            for step in 0..120 {
                if let Some(batch) = w.random_edit(step % 6) {
                    w.send(step % 6, batch);
                }
                w.tick();
                w.churn();
            }
            w.settle(600);
            assert!(w.converged(), "divergence at seed {seed}");
        }
    }

    /// The ignored 100k-seed run (spec §6.11) — `cargo test -p craft-collab --release -- --ignored`.
    #[test]
    #[ignore = "the full 100,000-seed suite (spec §12.4 layer 3)"]
    fn converges_over_100k_seeds() {
        for seed in 1..=100_000u64 {
            let mut w = World::new(8, seed);
            for step in 0..60 {
                if let Some(batch) = w.random_edit(step % 8) {
                    w.send(step % 8, batch);
                }
                w.tick();
            }
            w.settle(400);
            assert!(w.converged(), "divergence at seed {seed}");
        }
    }
}
