//! The size of a split in a persisted dock layout.

/// A transient sizing request. Applications may adapt their own persisted sizing enum.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SplitSize {
    /// Fraction of the space left after the divider is removed, assigned to the first child.
    Ratio(f32),
    /// First child size in logical points.
    FixedFirst(f32),
    /// Second child size in logical points.
    FixedSecond(f32),
}
