//! Counts of work done, for tests to assert on.
//!
//! A test of a bound counts what an operation did (windings tested, candidates read, samples taken)
//! and not how long it took. Outside tests [`add`] does nothing and compiles away.

/// What is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Windings of a path tested at a point (the last check of an interior point).
    Winding,
    /// Rectangles a lookup in a `RectIndex` (or runs of points of one x) read.
    IndexCandidates,
    /// Nodes of a `RectIndex` a lookup visited.
    IndexNodes,
    /// Work charged to a `join`.
    Join,
    /// Points sampled from curves to fit.
    FitSamples,
    /// Work charged to a refit (points sampled, smoothed and fitted).
    Fit,
    /// Joints read by `remove_redundant_points`.
    Redundant,
    /// Elements of stroke outline made.
    StrokeOutput,
    /// Elements of a path read, copied or swept (what an operation refused for its size does not do).
    Visited,
    /// Sweeps built.
    Sweeps,
    /// Segments of runs the tidy tried to merge into one (a run read again at each piece added).
    TidyMerge,
    /// Lines a curve was flattened into to find the way an edge leaves a vertex.
    FlattenLines,
    /// Flags set to give a predicate the coverage of a face (one per input, each time).
    MaskFlags,
    /// Fills a point was tested against to find the inputs that cover a face.
    FillTests,
    /// Points a polyline reduction read.
    RdpReads,
    /// Elements of contours made by a sweep.
    ContourElements,
    /// Joints of a run checked one by one against the line that replaces it.
    RunChecks,
    /// Heights an interior point was looked for at.
    InteriorHeights,
    /// Curves whose length was measured.
    Measured,
    /// Entries of the windings walked to list the inputs that cover each side of an edge.
    MaskEntries,
    /// Entries the windings of an arrangement of several inputs were counted to hold.
    CoverageEntries,
    /// Segments of the sweep that a contour pass walks, for each pass of the batches of a classified walk.
    PassSegments,
}

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::RefCell<[usize; 22]> = const { std::cell::RefCell::new([0; 22]) };
}

/// Counts `n` more of `what` on this thread (tests only).
#[inline]
pub(crate) fn add(what: Work, n: usize) {
    #[cfg(test)]
    COUNTS.with(|c| {
        if let Some(slot) = c.borrow_mut().get_mut(what as usize) {
            *slot = slot.saturating_add(n);
        }
    });
    #[cfg(not(test))]
    let _ = (what, n);
}

/// The count of `what` on this thread since the last [`take`], and resets it.
#[cfg(test)]
pub(crate) fn take(what: Work) -> usize {
    COUNTS.with(|c| c.borrow_mut().get_mut(what as usize).map(std::mem::take).unwrap_or(0))
}

/// Resets every count on this thread.
#[cfg(test)]
pub(crate) fn reset() {
    COUNTS.with(|c| *c.borrow_mut() = [0; 22]);
}
