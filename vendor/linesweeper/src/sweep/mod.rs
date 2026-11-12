//! The sweep-line implementation.
//!
//! The details of this implementation are described in the `docs` directory.
//! The main entry point is [`Sweeper`], which computes all the intersection
//! points between a collection of line segments, and makes them available
//! sweep-line by sweep-line.

mod output_event;
mod range;
mod sweep_line;

pub use output_event::OutputEvent;
pub use range::{SegmentsConnectedAtX, SweepLineRange, SweepLineRangeBuffers};
pub use sweep_line::{ChangedInterval, Correctness, SweepLine, SweepLineBuffers, Sweeper};

use crate::Segments;

/// Runs the sweep-line algorithm, calling the provided callback on every output point.
pub fn sweep<C: FnMut(f64, OutputEvent)>(segments: &Segments, eps: f64, mut callback: C) {
    let mut state = Sweeper::new(segments, eps);
    let mut range_bufs = SweepLineRangeBuffers::default();
    let mut line_bufs = SweepLineBuffers::default();
    while let Some(mut line) = state.next_line(&mut line_bufs) {
        let y = line.y();
        while let Some(mut range) = line.next_range(&mut range_bufs, segments) {
            while let Some(events) = range.events() {
                for ev in events {
                    callback(y, ev);
                }
            }
        }
    }
}
