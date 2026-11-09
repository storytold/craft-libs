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
    /// Points of a stretch the search for the direction at a split was charged for (all of them, whatever it reads), part of [`Work::Fit`].
    SplitScans,
    /// Segments the estimate of which way to sweep read (counted, and the sampled ones boxed).
    TurnReads,
    /// Segment boxes the estimate of which way to sweep built (at most the sample).
    TurnBoxes,
    /// Operations run turned a quarter turn, the estimate finding the turned sweep clearly cheaper.
    Turns,
    /// Face outlines made into paths by a walk of the faces that may stop early (`regions_within`), the empty ones too.
    FacePaths,
    /// Entries of the windings walked to list the inputs that cover a face for a predicate (the entries of the
    /// winding and one more, for each distinct winding).
    CoverEntries,
    /// Fills a point was tested against to find the inputs that cover a face.
    FillTests,
    /// Points a polyline reduction read.
    RdpReads,
    /// Elements of contours made by a sweep.
    ContourElements,
    /// Joints of a run checked one by one against the line that replaces it.
    RunChecks,
    /// Points of a run that a cubic replacing it was refitted from.
    Witnesses,
    /// Tables of the corners of a subpath made (for the strides of a refit).
    CornerTables,
    /// Curves put in the tidy's index of the input curves a result's boundary is rebuilt from.
    TidyCurves,
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
    /// Entries of windings hashed to find the remembered answer of a predicate.
    WindingKeys,
    /// Lookups of the answer for a winding that found none stored (the predicate was asked).
    AnswerMisses,
    /// Pairs of line bounds walked to find where lines cross.
    CrossingPairs,
    /// Subpaths of a fill whose winding a point was tested against (those whose bounds hold the point).
    FillSubpaths,
    /// Segments on the boundary of a sweep, counted before its contours are made.
    BoundarySegments,
    /// Frames tried for a planar map (the paths copied, sheared and snapped to the sweep's tolerance).
    FrameTries,
    /// Segments a planar map charged to wind points against, to find the fills of its faces and where its holes are.
    FillElements,
    /// Segments of the subpaths of fills that a planar map wound points against, counted where each is wound.
    FillWound,
    /// Segments of the faces that a planar map wound the points of holes against, counted where each is wound.
    HoleWound,
    /// Steps charged to the budget of the searches after a sweep (`Limits::search_work`).
    SearchSteps,
    /// Nearest points of a curve to a point that a search after a sweep found.
    Nearest,
    /// Blocks of junctions, and junctions in them, that a lookup of the junctions in a box looked at.
    JunctionSteps,
    /// Chains of an Outline that were compared with a chain found beside them, for edges that coincide.
    ChainCompares,
    /// Places where segments were found to cross, by testing the pairs whose bounds overlap (the check of a sweep's crossings).
    Crossings,
    /// Faces of an offset whose point deep inside was looked for among the crossings of the curves themselves, the
    /// flattened path having none.
    ExactSpans,
    /// Evaluations of the height of a piece of a cubic (Newton steps and halvings), to find where it crosses the height of a point whose
    /// winding is counted.
    WindingSteps,
    /// Nanoseconds of the sweep priced for the ends of segments at the points where they meet (`Limits::ends_work`).
    PricedEnds,
    /// Pairs of curves tried for running along one another (the work of curves that coincide).
    CoincidenceTests,
    /// Nanoseconds of sweep a metered dry sweep counted, up to the point it stopped (`Limits::sweep_work`).
    MeterNanos,
    /// Dry sweeps run (inputs past the gate, within a limit that is not lifted).
    MeteredSweeps,
    /// The fourth powers of the segments that start (or end) together, summed, as the meter counts them before its sweep.
    MeterGroups,
}

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::RefCell<[usize; 64]> = const { std::cell::RefCell::new([0; 64]) };
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
    COUNTS.with(|c| *c.borrow_mut() = [0; 64]);
}
