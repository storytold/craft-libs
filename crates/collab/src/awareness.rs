//! Awareness (y-protocols encoding): who's in the room, and where their cursors are.
//!
//! An update is `varuint(count)` then, per client, `varuint(clientId) varuint(clock)
//! varstring(state JSON)`; the state `null` means the client left. The state is the app's: the
//! Craft apps send `{user: {id, name, color}, …}` with a cursor and a selection in their own
//! document's terms. A client that hasn't renewed its state for 30 s
//! is gone (peers renew every 15 s).

use std::collections::BTreeMap;

use serde_json::Value as Json;

use crate::CollabError;

/// A peer is forgotten after this long without an update (milliseconds).
pub const TIMEOUT_MS: u64 = 30_000;
/// The local state is sent again after this long ([`Awareness::renew`]), so others don't forget us.
pub const RENEW_MS: u64 = 15_000;
/// How long a client that left or timed out is remembered by its clock, so a late update from
/// before it went doesn't bring it back.
pub const FORGET_MS: u64 = 10 * TIMEOUT_MS;

/// One client's entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub clock: u64,
    /// `None`: the client left.
    pub state: Option<Json>,
    /// When it last changed (the caller's clock, milliseconds).
    pub seen: u64,
}

/// The awareness states of a room, ours included.
#[derive(Clone, Debug, Default)]
pub struct Awareness {
    pub client_id: u64,
    pub clock: u64,
    pub local: Option<Json>,
    pub peers: BTreeMap<u64, Peer>,
}

fn write_varuint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varuint(data: &[u8], pos: &mut usize) -> Result<u64, CollabError> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let b = *data.get(*pos).ok_or_else(|| CollabError::Corrupt("awareness: truncated".into()))?;
        *pos += 1;
        // The tenth byte holds bit 63 alone: anything more doesn't fit 64 bits.
        if shift >= 64 || (shift == 63 && b & 0x7f > 1) {
            return Err(CollabError::Corrupt("awareness: number too long".into()));
        }
        v |= u64::from(b & 0x7f).checked_shl(shift).unwrap_or(0);
        if b & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
    }
}

fn read_string(data: &[u8], pos: &mut usize) -> Result<String, CollabError> {
    let len = usize::try_from(read_varuint(data, pos)?).map_err(|_| CollabError::Corrupt("awareness: length".into()))?;
    let end = pos.checked_add(len).filter(|e| *e <= data.len()).ok_or_else(|| CollabError::Corrupt("awareness: truncated".into()))?;
    let s = std::str::from_utf8(data.get(*pos..end).unwrap_or_default()).map_err(|_| CollabError::Corrupt("awareness: not UTF-8".into()))?;
    *pos = end;
    Ok(s.to_string())
}

/// Encodes entries `(clientId, clock, state)`.
pub fn encode(entries: &[(u64, u64, Option<&Json>)]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varuint(&mut out, entries.len() as u64);
    for (id, clock, state) in entries {
        write_varuint(&mut out, *id);
        write_varuint(&mut out, *clock);
        let text = state.map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        write_varuint(&mut out, text.len() as u64);
        out.extend_from_slice(text.as_bytes());
    }
    out
}

/// Decodes an update into `(clientId, clock, state)` entries.
pub fn decode(data: &[u8]) -> Result<Vec<(u64, u64, Option<Json>)>, CollabError> {
    if data.len() > crate::MAX_AWARENESS {
        return Err(CollabError::TooLarge(data.len()));
    }
    let mut pos = 0;
    let n = read_varuint(data, &mut pos)?;
    if n > 10_000 {
        return Err(CollabError::Corrupt("awareness: too many clients".into()));
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let id = read_varuint(data, &mut pos)?;
        let clock = read_varuint(data, &mut pos)?;
        let text = read_string(data, &mut pos)?;
        let state: Json = serde_json::from_str(&text).map_err(|e| CollabError::Corrupt(format!("awareness: {e}")))?;
        out.push((id, clock, (!state.is_null()).then_some(state)));
    }
    Ok(out)
}

impl Awareness {
    pub fn new(client_id: u64) -> Awareness {
        Awareness { client_id, ..Default::default() }
    }

    /// Sets our state; returns the update to send.
    pub fn set_local(&mut self, state: Option<Json>) -> Vec<u8> {
        self.clock += 1;
        self.local = state;
        self.local_update()
    }

    /// Our state again under a new clock: what to send every [`RENEW_MS`]. Peers refresh a
    /// client only for a clock they haven't seen, so sending [`Awareness::local_update`] again
    /// would not keep us present.
    pub fn renew(&mut self) -> Vec<u8> {
        self.clock += 1;
        self.local_update()
    }

    /// Our current state as an update under its current clock (for a newcomer).
    pub fn local_update(&self) -> Vec<u8> {
        encode(&[(self.client_id, self.clock, self.local.as_ref())])
    }

    /// Applies a peer's update. Returns whether anything visible changed.
    pub fn apply(&mut self, data: &[u8], now: u64) -> Result<bool, CollabError> {
        let mut changed = false;
        for (id, clock, state) in decode(data)? {
            if id == self.client_id {
                continue;
            }
            let newer = match self.peers.get(&id) {
                Some(p) => clock > p.clock || (clock == p.clock && state.is_none() && p.state.is_some()),
                None => true,
            };
            if newer {
                let same = self.peers.get(&id).is_some_and(|p| p.state == state);
                self.peers.insert(id, Peer { clock, state, seen: now });
                changed |= !same;
            } else if let Some(p) = self.peers.get_mut(&id)
                && clock == p.clock
            {
                p.seen = now;
            }
        }
        Ok(changed)
    }

    /// Drops the state of peers not heard of for [`TIMEOUT_MS`]. Returns whether any went. A
    /// peer that left or timed out keeps its clock for [`FORGET_MS`]: it comes back only with a
    /// newer one.
    pub fn prune(&mut self, now: u64) -> bool {
        let mut went = false;
        for p in self.peers.values_mut() {
            if p.state.is_some() && now.saturating_sub(p.seen) >= TIMEOUT_MS {
                p.state = None;
                p.seen = now;
                went = true;
            }
        }
        self.peers.retain(|_, p| p.state.is_some() || now.saturating_sub(p.seen) < FORGET_MS);
        went
    }

    /// The peers present now, with their state.
    pub fn present(&self) -> impl Iterator<Item = (u64, &Json)> {
        self.peers.iter().filter_map(|(id, p)| p.state.as_ref().map(|s| (*id, s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encode_decode_and_apply() {
        let mut a = Awareness::new(1);
        let mut b = Awareness::new(2);
        let up = a.set_local(Some(json!({"user": {"id": "u1", "name": "Ann", "color": "#3B7DD8"}})));
        assert!(b.apply(&up, 0).expect("apply"));
        assert_eq!(b.present().count(), 1);
        // A replay of the same clock changes nothing; an older one is ignored.
        assert!(!b.apply(&up, 10).expect("apply"));
        let up2 = a.set_local(Some(json!({"user": {"name": "Ann B"}})));
        assert!(b.apply(&up2, 20).expect("apply"));
        assert!(!b.apply(&up, 30).expect("apply"));
        assert_eq!(b.peers[&1].state.as_ref().map(|s| s["user"]["name"].clone()), Some(json!("Ann B")));
        // Leaving.
        let bye = a.set_local(None);
        assert!(b.apply(&bye, 40).expect("apply"));
        b.prune(40);
        assert_eq!(b.present().count(), 0);
        // Our own entry is never a peer.
        assert!(!a.apply(&up2, 0).expect("apply"));
    }

    #[test]
    fn timeouts_and_garbage() {
        let mut a = Awareness::new(1);
        let mut b = Awareness::new(2);
        b.apply(&a.set_local(Some(json!({"x": 1}))), 0).expect("apply");
        assert!(!b.prune(TIMEOUT_MS - 1));
        assert!(b.prune(TIMEOUT_MS));
        assert!(decode(&[5]).is_err());
        assert!(decode(&[1, 1, 1, 9, b'{']).is_err());
        assert!(decode(&[0xff; 20]).is_err());
        assert_eq!(decode(&encode(&[(300, 70000, None)])).expect("decode"), vec![(300, 70000, None)]);
    }

    /// The y-protocols layout, byte for byte: one client, id 300, clock 2, state `{"a":1}`.
    #[test]
    fn the_wire_layout_is_y_protocols() {
        let state = json!({"a": 1});
        let bytes = encode(&[(300, 2, Some(&state))]);
        assert_eq!(bytes, [&[1, 0xac, 0x02, 2, 7][..], br#"{"a":1}"#].concat());
        assert_eq!(decode(&bytes).expect("decode"), vec![(300, 2, Some(state))]);
    }

    #[test]
    fn an_update_over_the_limit_is_refused_before_it_is_read() {
        let big = vec![0u8; crate::MAX_AWARENESS + 1];
        assert_eq!(decode(&big), Err(CollabError::TooLarge(big.len())));
    }

    #[test]
    fn a_renewal_has_a_new_clock() {
        let mut a = Awareness::new(1);
        let first = decode(&a.set_local(Some(json!({"x": 1})))).expect("decode");
        let again = decode(&a.local_update()).expect("decode");
        let renewed = decode(&a.renew()).expect("decode");
        assert_eq!(again, first, "a snapshot for a newcomer changes nothing");
        assert!(renewed[0].1 > first[0].1, "a renewal is newer");
        assert_eq!(renewed[0].2, first[0].2, "with the same state");
    }

    #[test]
    fn a_late_update_does_not_bring_back_a_peer_that_went() {
        let mut a = Awareness::new(1);
        let mut b = Awareness::new(2);
        let here = a.set_local(Some(json!({"x": 1})));
        let bye = a.set_local(None);
        b.apply(&here, 0).expect("apply");
        b.apply(&bye, 10).expect("apply");
        b.prune(20);
        assert!(!b.apply(&here, 30).expect("apply"), "the older state is still older after pruning");
        assert_eq!(b.present().count(), 0);

        // The same after a timeout; a newer clock brings the peer back.
        let mut c = Awareness::new(3);
        let mut d = Awareness::new(4);
        let here = c.set_local(Some(json!({"x": 1})));
        d.apply(&here, 0).expect("apply");
        assert!(d.prune(TIMEOUT_MS));
        assert!(!d.apply(&here, TIMEOUT_MS + 1).expect("apply"));
        assert_eq!(d.present().count(), 0);
        assert!(d.apply(&c.renew(), TIMEOUT_MS + 2).expect("apply"));
        assert_eq!(d.present().count(), 1);
        // Clocks aren't kept for ever.
        d.prune(TIMEOUT_MS * 2 + 2);
        d.prune(TIMEOUT_MS * 2 + 2 + FORGET_MS);
        assert!(d.peers.is_empty());
    }

    #[test]
    fn a_number_past_64_bits_is_refused() {
        // 2^64 + 1 as a client id would read as 1.
        let over = [1, 0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02, 1, 4, b'n', b'u', b'l', b'l'];
        assert!(decode(&over).is_err());
        let most = encode(&[(u64::MAX, u64::MAX, None)]);
        assert_eq!(decode(&most).expect("decode"), vec![(u64::MAX, u64::MAX, None)]);
    }
}
