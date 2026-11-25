//! Placeholder inheritance: which layout and master placeholders a placeholder takes its
//! position, formatting and text styles from (`p:ph` `type` and `idx`, ECMA-376 part 1, 19.3.1.36).
//!
//! Only the matching lives here. What a caller does with the parents (effective position, fill,
//! text style) is its own business.

use crate::{Layout, Master, PhType, Shape};

/// Where a shape lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Slide,
    Layout,
    Master,
}

/// Can a slide placeholder of type `slide` inherit from a layout placeholder of type `other`?
fn compatible(slide: PhType, other: PhType) -> bool {
    use PhType::*;
    match slide {
        Title | CtrTitle => matches!(other, Title | CtrTitle),
        SubTitle | Body | Obj | Chart | Table | ClipArt | Diagram | Media | Picture => {
            !matches!(other, Title | CtrTitle | Date | Footer | SlideNum | Header)
        }
        k => k == other,
    }
}

/// The placeholder in `shapes` that `shape` matches: by `idx` (when `by_idx`) and a compatible
/// type, else the same type, else a compatible one.
pub fn find_ph<'s>(shapes: &'s [Shape], shape: &Shape, by_idx: bool) -> Option<&'s Shape> {
    let ph = shape.ph.as_ref()?;
    if by_idx
        && ph.idx != 0
        && let Some(s) = shapes.iter().find(|s| s.ph.as_ref().is_some_and(|p| p.idx == ph.idx && compatible(ph.kind, p.kind)))
    {
        return Some(s);
    }
    // Same type first, then a compatible one.
    shapes
        .iter()
        .find(|s| s.ph.as_ref().is_some_and(|p| p.kind == ph.kind && (!by_idx || ph.idx == 0 || p.idx == ph.idx || ph.kind.is_title())))
        .or_else(|| shapes.iter().find(|s| s.ph.as_ref().is_some_and(|p| p.kind == ph.kind)))
        .or_else(|| shapes.iter().find(|s| s.ph.as_ref().is_some_and(|p| compatible(ph.kind, p.kind))))
}

/// The master placeholder a placeholder of type `kind` inherits from: the title, the matching
/// footer-row placeholder, or the body.
pub fn master_ph(master: &Master, kind: PhType) -> Option<&Shape> {
    let want = match kind {
        PhType::Title | PhType::CtrTitle => PhType::Title,
        PhType::Date | PhType::Footer | PhType::SlideNum | PhType::Header => kind,
        _ => PhType::Body,
    };
    master.shapes.iter().find(|s| s.ph_type() == Some(want))
}

/// The layout and master placeholders `shape` inherits from. A slide's placeholder matches one on
/// `layout`, and the master placeholder follows the layout one's type; a layout's placeholder
/// inherits from the master only; a master's from nothing.
pub fn parents<'a>(master: &'a Master, layout: Option<&'a Layout>, owner: Owner, shape: &Shape) -> (Option<&'a Shape>, Option<&'a Shape>) {
    let Some(ph) = shape.ph.as_ref() else { return (None, None) };
    match owner {
        Owner::Slide => {
            let lp = layout.and_then(|l| find_ph(&l.shapes, shape, true));
            let kind = lp.and_then(Shape::ph_type).unwrap_or(ph.kind);
            (lp, master_ph(master, kind))
        }
        Owner::Layout => (None, master_ph(master, ph.kind)),
        Owner::Master => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Placeholder, ShapeId};

    fn ph(id: u32, kind: PhType, idx: u32) -> Shape {
        Shape { id: ShapeId(id), ph: Some(Placeholder { kind, idx, ..Default::default() }), ..Default::default() }
    }

    #[test]
    fn slide_placeholders_match_by_index_then_type() {
        let master = Master { shapes: vec![ph(1, PhType::Title, 0), ph(2, PhType::Body, 1), ph(3, PhType::SlideNum, 12)], ..Default::default() };
        let layout = Layout { shapes: vec![ph(10, PhType::Title, 0), ph(11, PhType::Body, 1), ph(12, PhType::Obj, 2)], ..Default::default() };
        let id = |s: Option<&Shape>| s.map(|s| s.id.0);
        // By idx: idx 2 is the layout's obj, whose master parent is the body.
        let (l, m) = parents(&master, Some(&layout), Owner::Slide, &ph(20, PhType::Body, 2));
        assert_eq!((id(l), id(m)), (Some(12), Some(2)));
        // A centred title takes the layout's title and the master's title.
        let (l, m) = parents(&master, Some(&layout), Owner::Slide, &ph(21, PhType::CtrTitle, 0));
        assert_eq!((id(l), id(m)), (Some(10), Some(1)));
        // An unknown idx falls back to the same type.
        let (l, _) = parents(&master, Some(&layout), Owner::Slide, &ph(22, PhType::Body, 9));
        assert_eq!(id(l), Some(11));
        // A title never inherits from a body, nor a slide number from a title.
        assert_eq!(find_ph(&layout.shapes[1..], &ph(23, PhType::Title, 0), true).map(|s| s.id.0), None);
        let (l, m) = parents(&master, Some(&layout), Owner::Slide, &ph(24, PhType::SlideNum, 12));
        assert_eq!((id(l), id(m)), (None, Some(3)));
        // Layouts inherit from the master only, masters from nothing, other shapes from nothing.
        assert_eq!(parents(&master, None, Owner::Layout, &ph(25, PhType::SubTitle, 1)), (None, master.shapes.get(1)));
        assert_eq!(parents(&master, None, Owner::Master, &ph(26, PhType::Title, 0)), (None, None));
        assert_eq!(parents(&master, Some(&layout), Owner::Slide, &Shape::default()), (None, None));
    }
}
