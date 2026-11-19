//! What a drawable holds before it's written: paths and groups in viewport coordinates.

use kurbo::{BezPath, Rect};

/// Straight (not premultiplied) sRGB red, green, blue and alpha, each 0..=1. Written as 8 bits a
/// component; out-of-range and non-finite components are clamped when written.
pub type Color = [f32; 4];

/// A gradient's geometry in the coordinates of the path it fills.
#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
    Linear { start: (f64, f64), end: (f64, f64) },
    Radial { center: (f64, f64), radius: f64 },
    Sweep { center: (f64, f64) },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    /// Offsets in 0..=1, ascending; colours with their opacity folded in.
    pub stops: Vec<(f64, Color)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    Gradient(Gradient),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// A stroke Android draws itself: centred, one width, one solid colour, no dashes.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    /// `strokeMiterLimit`; Android's default is 4.
    pub miter: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// `android:name`.
    pub name: Option<String>,
    pub data: BezPath,
    pub fill: Option<Fill>,
    pub stroke: Option<Stroke>,
    /// `fillType="evenOdd"` (API 24); non-zero otherwise.
    pub even_odd: bool,
}

/// A `<group>`'s own transform: scale, then rotate (degrees), then translate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupTransform {
    pub translate: (f64, f64),
    pub rotation: f64,
    pub scale: (f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// `android:name`.
    pub name: Option<String>,
    pub transform: Option<GroupTransform>,
    /// A non-zero clip outline in the group's (untransformed) coordinates.
    pub clip: Option<BezPath>,
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Path(Path),
    Group(Group),
}

/// One written element with the viewport area it paints (for [`Item::overlap`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub bbox: Rect,
}

impl Item {
    /// The areas of what paints, each a leaf (a path, or a transformed group).
    pub fn leaf_boxes(items: &[Item], out: &mut Vec<Rect>) {
        for it in items {
            match &it.kind {
                ItemKind::Group(g) if g.transform.is_none() => Self::leaf_boxes(&g.children, out),
                _ => out.push(it.bbox),
            }
        }
    }

    /// Multiply every colour's alpha by `k`: a layer's opacity folded into what it paints, since
    /// a drawable has no group opacity. The same picture only when the leaves don't overlap
    /// ([`Item::overlap`]).
    pub fn scale_alpha(items: &mut [Item], k: f32) {
        let fade = |c: &mut Color| c[3] = (c[3] * k).clamp(0.0, 1.0);
        for it in items {
            match &mut it.kind {
                ItemKind::Path(p) => {
                    match &mut p.fill {
                        Some(Fill::Solid(c)) => fade(c),
                        Some(Fill::Gradient(g)) => g.stops.iter_mut().for_each(|(_, c)| fade(c)),
                        None => {}
                    }
                    if let Some(s) = &mut p.stroke {
                        fade(&mut s.color);
                    }
                }
                ItemKind::Group(g) => Self::scale_alpha(&mut g.children, k),
            }
        }
    }

    /// Whether any two leaves' areas overlap (more than 64 leaves count as overlapping).
    pub fn overlap(items: &[Item]) -> bool {
        const MAX: usize = 64;
        let mut boxes = Vec::new();
        Self::leaf_boxes(items, &mut boxes);
        if boxes.len() > MAX {
            return true;
        }
        boxes.iter().enumerate().any(|(i, a)| {
            boxes.iter().skip(i + 1).any(|b| {
                let r = a.intersect(*b);
                r.width() > 1e-9 && r.height() > 1e-9
            })
        })
    }
}
