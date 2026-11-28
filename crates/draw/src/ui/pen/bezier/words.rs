//! The words of [`BezierOverlay`](super::BezierOverlay): the AccessKit names of its parts.

craft_ui::words! {
    /// The accessible names of a path overlay's parts, English by default.
    #[non_exhaustive]
    pub struct BezierWords {
        in_handle: "bezier.inHandle" = "In handle";
        out_handle: "bezier.outHandle" = "Out handle";
        marquee: "bezier.marquee" = "Selection";
        anchor: template "bezier.anchor" = "Anchor {i} of {n}";
    }
}
