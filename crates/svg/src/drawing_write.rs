//! SVG polyline and triangle path attributes for display-list exporters.
use crate::write::num;
use std::fmt::{self, Display};

/// A fixed-precision polyline attribute streamed into an SVG writer.
pub struct Points<I> {
    iter: I,
    decimals: usize,
}
/// Format points without allocating an intermediate attribute string. Precision is capped at 32.
pub fn points<I: Iterator<Item = [f64; 2]> + Clone>(iter: I, decimals: usize) -> Points<I> {
    Points { iter, decimals: decimals.min(32) }
}
impl<I: Iterator<Item = [f64; 2]> + Clone> Display for Points<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for [x, y] in self.iter.clone() {
            write!(f, "{},{} ", num(x, self.decimals), num(y, self.decimals))?;
        }
        Ok(())
    }
}
/// A fixed-precision sequence of closed triangle subpaths.
pub struct Triangles<I> {
    iter: I,
    decimals: usize,
}
/// Format independent triangles without allocating an intermediate attribute string. Precision is capped at 32.
pub fn triangles<I: Iterator<Item = [[f64; 2]; 3]> + Clone>(iter: I, decimals: usize) -> Triangles<I> {
    Triangles { iter, decimals: decimals.min(32) }
}
impl<I: Iterator<Item = [[f64; 2]; 3]> + Clone> Display for Triangles<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for [[ax, ay], [bx, by], [cx, cy]] in self.iter.clone() {
            write!(
                f,
                "M{} {}L{} {}L{} {}Z",
                num(ax, self.decimals),
                num(ay, self.decimals),
                num(bx, self.decimals),
                num(by, self.decimals),
                num(cx, self.decimals),
                num(cy, self.decimals)
            )?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_precision_attributes_keep_independent_triangles() {
        assert_eq!(points([[1.0, 2.0], [3.0, 4.0]].into_iter(), 2).to_string(), "1.00,2.00 3.00,4.00 ");
        assert_eq!(triangles([[[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]].into_iter(), 1).to_string(), "M1.0 2.0L3.0 4.0L5.0 6.0Z");
    }
}
