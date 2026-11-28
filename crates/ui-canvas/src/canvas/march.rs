//! The march of the selection ants: one period and step for the `Marquee` control and the tools' own outlines.

/// Seconds the ants take to march one dash period.
pub(crate) const MARCH_SECONDS: f64 = 1.0;

/// Seconds between steps of the march (the legacy marquee's 0.1 s). Photoshop's ants step rather than glide, and
/// shown ants repaint once per step instead of every frame.
pub(crate) const MARCH_TICK: f64 = 0.1;

/// How far the ants have marched at `turn` of the march (`None`: motion is off and they stand still), in points, for
/// ants of dash length `dash`: one march covers a dash and its gap.
pub(crate) fn march_shift(turn: Option<f32>, dash: f32) -> f32 {
    turn.map_or(0.0, |turn| turn * 2.0 * dash)
}
