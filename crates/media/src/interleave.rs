//! Bounded interleaving for live muxing. Muxers such as `matroska::MkvWriter` take packets in
//! file order, while two live encoders (video and audio) deliver theirs on their own schedules.
//! [`Interleaver`] holds each stream's packets and releases them in time order, waiting for a
//! stream only while it may still deliver an earlier packet, and reports a stream that falls too
//! far behind instead of holding packets without limit.

#![warn(missing_docs)]

use std::collections::VecDeque;
use std::fmt;
use std::num::NonZeroUsize;

/// A packet that [`Interleaver::pop`] released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Released<T> {
    /// The stream it was pushed to.
    pub stream: usize,
    /// Its order value, as pushed.
    pub order: i64,
    /// The packet.
    pub value: T,
}

/// Why [`Interleaver::push`], [`Interleaver::end`] or [`Interleaver::pop`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterleaveError {
    /// The stream index is not below the stream count.
    NoSuchStream(usize),
    /// The stream was ended; it takes no more packets.
    Ended(usize),
    /// The packet's order is earlier than the stream's previous packet.
    OutOfOrder {
        /// The stream.
        stream: usize,
        /// The refused packet's order.
        order: i64,
        /// The order of the stream's previous packet.
        previous: i64,
    },
    /// The interleaver already holds its maximum number of packets.
    Full,
    /// This stream has no packet queued and has not ended, and the packets held while waiting for
    /// it span more than the lag limit or reach the packet limit.
    Behind(usize),
}

impl fmt::Display for InterleaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchStream(s) => write!(f, "no stream {s}"),
            Self::Ended(s) => write!(f, "stream {s} has ended"),
            Self::OutOfOrder { stream, order, previous } => {
                write!(f, "stream {stream}: order {order} is earlier than the previous packet's {previous}")
            }
            Self::Full => write!(f, "the interleaver holds its maximum number of packets"),
            Self::Behind(s) => write!(f, "stream {s} fell too far behind"),
        }
    }
}

impl std::error::Error for InterleaveError {}

struct Stream<T> {
    queue: VecDeque<(i64, T)>,
    last: Option<i64>,
    ended: bool,
}

/// Merges packets from several streams into one sequence ordered by `(order, stream)`.
///
/// `order` is any time value shared by every stream, for example the decoding time in
/// nanoseconds. Each stream's packets must arrive with non-decreasing `order`. [`pop`] releases
/// the held packet with the smallest `(order, stream)` once no stream can still deliver one that
/// sorts before it: a stream that has ended, or whose last packet sorts after it, does not hold
/// it back. The released sequence is the same as sorting every packet by `(order, stream)`, with
/// packets of equal order from one stream kept in the order they were pushed.
///
/// While a stream that has not ended has nothing queued, the other streams' packets wait. When
/// the waiting packets span more than `max_lag` (newest order minus oldest), or their count
/// reaches the packet limit, [`pop`] returns [`InterleaveError::Behind`] naming that stream. The
/// error leaves every packet held: a caller that stops the recording calls [`end_all`] and pops
/// the rest in order.
///
/// [`pop`]: Interleaver::pop
/// [`end_all`]: Interleaver::end_all
pub struct Interleaver<T> {
    streams: Vec<Stream<T>>,
    max_lag: u64,
    max_held: usize,
    held: usize,
}

impl<T> Interleaver<T> {
    /// An interleaver for `streams` streams, numbered from 0, that reports a stream as behind
    /// when the packets waiting for it span more than `max_lag`, in the unit of `order`.
    /// The number of held packets is not limited until [`with_max_held`](Self::with_max_held).
    pub fn new(streams: usize, max_lag: u64) -> Self {
        let streams = (0..streams).map(|_| Stream { queue: VecDeque::new(), last: None, ended: false }).collect();
        Self { streams, max_lag, max_held: usize::MAX, held: 0 }
    }

    /// Limits the held packets to `max_held`: [`push`](Self::push) refuses a packet beyond it
    /// with [`InterleaveError::Full`], and [`pop`](Self::pop) reports the stream it waits for as
    /// behind once the limit is reached.
    #[must_use]
    pub fn with_max_held(mut self, max_held: NonZeroUsize) -> Self {
        self.max_held = max_held.get();
        self
    }

    /// The number of streams.
    pub fn streams(&self) -> usize {
        self.streams.len()
    }

    /// The number of packets held.
    pub fn held(&self) -> usize {
        self.held
    }

    /// True when every stream has ended and no packet is held.
    pub fn is_finished(&self) -> bool {
        self.held == 0 && self.streams.iter().all(|s| s.ended)
    }

    /// Queues a packet on `stream` with the given `order`.
    pub fn push(&mut self, stream: usize, order: i64, value: T) -> Result<(), InterleaveError> {
        if self.held >= self.max_held {
            return Err(InterleaveError::Full);
        }
        let s = self.streams.get_mut(stream).ok_or(InterleaveError::NoSuchStream(stream))?;
        if s.ended {
            return Err(InterleaveError::Ended(stream));
        }
        if let Some(previous) = s.last
            && order < previous
        {
            return Err(InterleaveError::OutOfOrder { stream, order, previous });
        }
        s.last = Some(order);
        s.queue.push_back((order, value));
        self.held += 1;
        Ok(())
    }

    /// Marks `stream` as ended: it takes no more packets and no longer holds the others back.
    /// Ending a stream twice is not an error.
    pub fn end(&mut self, stream: usize) -> Result<(), InterleaveError> {
        let s = self.streams.get_mut(stream).ok_or(InterleaveError::NoSuchStream(stream))?;
        s.ended = true;
        Ok(())
    }

    /// Ends every stream, so [`pop`](Self::pop) releases every held packet in order.
    pub fn end_all(&mut self) {
        for s in &mut self.streams {
            s.ended = true;
        }
    }

    /// The next packet in order, `Ok(None)` while it must wait for a stream (or nothing is held),
    /// or [`InterleaveError::Behind`] when the wait has exceeded a limit.
    pub fn pop(&mut self) -> Result<Option<Released<T>>, InterleaveError> {
        let mut next: Option<(usize, i64)> = None;
        for (i, s) in self.streams.iter().enumerate() {
            if let Some(&(order, _)) = s.queue.front()
                && next.is_none_or(|(_, best)| order < best)
            {
                next = Some((i, order));
            }
        }
        let Some((pick, order)) = next else { return Ok(None) };
        // A stream with nothing queued that has not ended can still deliver a packet at its last
        // order, or at i64::MIN when it has sent nothing; it holds (order, pick) back only when
        // that packet would sort before it.
        let waiting =
            self.streams.iter().enumerate().find(|&(i, s)| s.queue.is_empty() && !s.ended && (s.last.unwrap_or(i64::MIN), i) < (order, pick));
        if let Some((behind, _)) = waiting {
            let newest = self.streams.iter().filter_map(|s| s.queue.back().map(|&(o, _)| o)).max().unwrap_or(order);
            if i128::from(newest) - i128::from(order) > i128::from(self.max_lag) || self.held >= self.max_held {
                return Err(InterleaveError::Behind(behind));
            }
            return Ok(None);
        }
        let Some((order, value)) = self.streams.get_mut(pick).and_then(|s| s.queue.pop_front()) else {
            return Ok(None);
        };
        self.held -= 1;
        Ok(Some(Released { stream: pick, order, value }))
    }
}

impl<T> fmt::Debug for Interleaver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Interleaver")
            .field("streams", &self.streams.len())
            .field("held", &self.held)
            .field("max_lag", &self.max_lag)
            .field("max_held", &self.max_held)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn drain<T>(il: &mut Interleaver<T>) -> Vec<(usize, i64, T)> {
        let mut out = Vec::new();
        while let Ok(Some(r)) = il.pop() {
            out.push((r.stream, r.order, r.value));
        }
        out
    }

    #[test]
    fn waits_for_a_stream_that_may_still_send_an_earlier_packet() {
        let mut il = Interleaver::new(2, 1_000);
        il.push(0, 0, "v0").ok();
        il.push(0, 40, "v1").ok();
        // Stream 1 has sent nothing yet, so v0 must wait.
        assert_eq!(il.pop(), Ok(None));
        il.push(1, 20, "a0").ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.value), Some("v0"));
        assert_eq!(il.pop().ok().flatten().map(|r| r.value), Some("a0"));
        // v1 (40) waits: stream 1's last packet (20) is earlier.
        assert_eq!(il.pop(), Ok(None));
        il.push(1, 60, "a1").ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.value), Some("v1"));
        // a1 waits for stream 0, whose last packet (40) is earlier.
        assert_eq!(il.pop(), Ok(None));
        assert_eq!(il.held(), 1);
    }

    #[test]
    fn a_stream_whose_last_packet_is_later_does_not_hold_others_back() {
        let mut il = Interleaver::new(2, 1_000);
        il.push(1, 100, ()).ok();
        il.push(0, 10, ()).ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.stream), Some(0));
        // Stream 0 is empty but its last packet (10) is before 100, so 100 waits.
        assert_eq!(il.pop(), Ok(None));
        il.push(0, 150, ()).ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.order), Some(100));
    }

    #[test]
    fn equal_orders_go_lower_stream_first_and_wait_for_it() {
        let mut il = Interleaver::new(2, 1_000);
        il.push(1, 0, "a").ok();
        // Stream 0 could still send order 0, which sorts first.
        assert_eq!(il.pop(), Ok(None));
        il.push(0, 0, "v").ok();
        assert_eq!(drain(&mut il).iter().map(|e| e.2).collect::<Vec<_>>(), ["v"]);
        // "a" now waits: stream 0's last packet (0, 0) sorts before (0, 1) and it may send more at 0.
        il.end(0).ok();
        assert_eq!(drain(&mut il).iter().map(|e| e.2).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn ended_streams_release_everything_in_order() {
        let mut il = Interleaver::new(3, 0);
        il.push(2, 5, 'c').ok();
        il.push(0, 7, 'a').ok();
        il.push(0, 9, 'b').ok();
        assert!(il.end(1).is_ok());
        assert_eq!(il.end(1), Ok(()));
        il.end_all();
        assert_eq!(drain(&mut il).iter().map(|e| e.2).collect::<String>(), "cab");
        assert!(il.is_finished());
        assert_eq!(il.pop(), Ok(None));
    }

    #[test]
    fn a_stream_is_behind_only_when_the_wait_exceeds_the_lag() {
        let mut il = Interleaver::new(2, 100);
        il.push(0, 0, ()).ok();
        il.push(0, 100, ()).ok();
        // The held span is exactly the limit.
        assert_eq!(il.pop(), Ok(None));
        il.push(0, 101, ()).ok();
        assert_eq!(il.pop(), Err(InterleaveError::Behind(1)));
        // The error keeps the packets; ending the streams releases them in order.
        assert_eq!(il.held(), 3);
        il.end_all();
        assert_eq!(drain(&mut il).iter().map(|e| e.1).collect::<Vec<_>>(), [0, 100, 101]);
    }

    #[test]
    fn the_packet_limit_refuses_pushes_and_reports_the_waited_stream() {
        let mut il = Interleaver::new(2, u64::MAX).with_max_held(NonZeroUsize::MIN.saturating_add(1));
        assert_eq!(il.push(1, 0, ()), Ok(()));
        assert_eq!(il.push(1, 0, ()), Ok(()));
        assert_eq!(il.push(1, 1, ()), Err(InterleaveError::Full));
        assert_eq!(il.pop(), Err(InterleaveError::Behind(0)));
        il.end(0).ok();
        assert_eq!(drain(&mut il).len(), 2);
        assert_eq!(il.push(1, 1, ()), Ok(()));
    }

    #[test]
    fn push_refuses_bad_packets() {
        let mut il = Interleaver::new(1, 0);
        assert_eq!(il.push(1, 0, ()), Err(InterleaveError::NoSuchStream(1)));
        assert_eq!(il.end(3), Err(InterleaveError::NoSuchStream(3)));
        assert_eq!(il.push(0, 10, ()), Ok(()));
        assert_eq!(il.push(0, 10, ()), Ok(()));
        assert_eq!(il.push(0, 9, ()), Err(InterleaveError::OutOfOrder { stream: 0, order: 9, previous: 10 }));
        il.end(0).ok();
        assert_eq!(il.push(0, 11, ()), Err(InterleaveError::Ended(0)));
        assert_eq!(il.held(), 2);
    }

    #[test]
    fn extreme_orders_do_not_overflow() {
        let mut il = Interleaver::new(2, 10);
        il.push(1, i64::MIN, ()).ok();
        il.push(1, i64::MAX, ()).ok();
        assert_eq!(il.pop(), Err(InterleaveError::Behind(0)));
        il.end_all();
        assert_eq!(drain(&mut il).iter().map(|e| e.1).collect::<Vec<_>>(), [i64::MIN, i64::MAX]);
        // The span from i64::MIN to i64::MAX is u64::MAX: behind just below that limit, not at it.
        for (lag, expected) in [(u64::MAX - 1, Err(InterleaveError::Behind(0))), (u64::MAX, Ok(None))] {
            let mut il = Interleaver::new(2, lag);
            il.push(1, i64::MIN, ()).ok();
            il.push(1, i64::MAX, ()).ok();
            assert_eq!(il.pop(), expected);
        }
    }

    #[test]
    fn a_silent_higher_stream_does_not_hold_back_the_earliest_order() {
        // Stream 1 has sent nothing, but nothing it sends can sort before (i64::MIN, 0).
        let mut il = Interleaver::new(2, 0).with_max_held(NonZeroUsize::MIN);
        il.push(0, i64::MIN, "v").ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.value), Some("v"));
        // A silent lower stream still holds back a packet at i64::MIN on a higher stream.
        let mut il = Interleaver::new(2, 0);
        il.push(1, i64::MIN, "a").ok();
        assert_eq!(il.pop(), Ok(None));
        il.end(0).ok();
        assert_eq!(il.pop().ok().flatten().map(|r| r.value), Some("a"));
    }

    #[test]
    fn no_streams() {
        let mut il: Interleaver<()> = Interleaver::new(0, 0);
        assert!(il.is_finished());
        assert_eq!(il.pop(), Ok(None));
        assert_eq!(il.push(0, 0, ()), Err(InterleaveError::NoSuchStream(0)));
    }

    proptest! {
        /// Popping after every push gives the same sequence as a stable sort of all packets by
        /// (order, stream), whichever streams end early.
        #[test]
        fn output_is_the_merge_of_the_streams(
            steps in proptest::collection::vec((0usize..3, 0i64..5), 0..200),
            ends in proptest::collection::vec(0usize..200, 3),
        ) {
            let mut il = Interleaver::new(3, u64::MAX);
            let mut last = [0i64; 3];
            let mut pushed = Vec::new();
            let mut out = Vec::new();
            for (n, &(stream, step)) in steps.iter().enumerate() {
                for (s, &at) in ends.iter().enumerate() {
                    if at == n {
                        il.end(s).ok();
                    }
                }
                last[stream] += step;
                let id = pushed.len();
                if il.push(stream, last[stream], id).is_ok() {
                    pushed.push((last[stream], stream, id));
                }
                while let Some(r) = il.pop().ok().flatten() {
                    out.push((r.order, r.stream, r.value));
                }
            }
            il.end_all();
            while let Some(r) = il.pop().ok().flatten() {
                out.push((r.order, r.stream, r.value));
            }
            let mut sorted = pushed.clone();
            sorted.sort_by_key(|&(order, stream, _)| (order, stream));
            prop_assert_eq!(out, sorted);
            prop_assert!(il.is_finished());
        }
    }
}
