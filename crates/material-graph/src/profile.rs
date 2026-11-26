// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where evaluation spends its time, for a host that wants to know: named spans around nodes and stages, handed to the
//! [`Profiler`] of the current [`crate::context::EvalContext`]. Without one a span measures nothing. Nothing here
//! touches what is computed.

use std::any::Any;

/// What measures the spans: given to an evaluation in its [`crate::context::EvalContext`].
pub trait Profiler: Send + Sync {
    /// Whether spans are measured now: when not, evaluation skips the work only a measurement needs.
    fn enabled(&self) -> bool;
    /// Opens a span of `name`; dropping the returned value closes it.
    fn span(&self, name: &str) -> Box<dyn Any + Send>;
    /// Observes a node once its inputs are evaluated, before they are resized and the node is computed (the time of
    /// the inputs is not in it), even when span timers are disabled.
    fn node(&self, _id: u32, _kind: &str, _size: (usize, usize)) -> Option<Box<dyn Any + Send>> {
        None
    }
    /// Observes a node with its bound settings after parameter expressions have been applied.
    fn node_variant(&self, id: u32, node: &crate::graph::Node, size: (usize, usize)) -> Option<Box<dyn Any + Send>> {
        self.node(id, &node.kind, size)
    }
    /// Observes a result supplied by an accelerator, including one reused from its cache.
    fn accelerated_node(&self, _id: u32, _kind: &str) {}
    /// Observes a result still held on the accelerator and readable by compatible nodes.
    fn resident_node(&self, id: u32, kind: &str) {
        self.accelerated_node(id, kind);
    }
    /// Observes resident grayscale pixels fetched for a consumer that needs host pixels.
    fn resident_readback(&self, _id: u32, _bytes: usize) {}
}

/// A timed stretch of work: runs from [`span`] until dropped.
#[must_use = "a span measures until it is dropped"]
pub struct Span {
    _guard: Option<Box<dyn Any + Send>>,
}

/// True when spans are being measured on this thread.
pub fn enabled() -> bool {
    crate::context::with_current(|ec| ec.and_then(|ec| ec.profiler.as_ref()).is_some_and(|p| p.enabled()))
}

/// Opens a span of `name`. Spans may nest and overlap.
pub fn span(name: &str) -> Span {
    Span { _guard: crate::context::with_current(|ec| ec.and_then(|ec| ec.profiler.as_ref()).filter(|p| p.enabled()).map(|p| p.span(name))) }
}
