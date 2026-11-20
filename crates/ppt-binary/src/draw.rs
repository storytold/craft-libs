//! OfficeArt drawings ([MS-ODRAW]) as `.ppt` stores them: a tree of shape group containers and
//! shape containers, each shape with its `FSP` (type, id, flags), property tables (`FOPT`),
//! anchor and client records (placeholder, text).

use crate::doc::rt;
use craft_officeart::record::MAX_DEPTH;

use crate::record::Rec;

pub use craft_officeart::props::{Props, prop};

/// An anchor rectangle in its own coordinate units: master units for a client anchor, the parent
/// group's units for a child anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// The placeholder a shape stands for (`OEPlaceholderAtom`).
#[derive(Clone, Copy, Debug)]
pub struct Placeholder {
    pub position: u32,
    /// `PlaceholderEnum`.
    pub kind: u8,
}

/// One shape of a drawing, before conversion.
#[derive(Clone, Debug)]
pub struct RawShape<'a> {
    /// `MSOSPT` shape type.
    pub shape_type: u16,
    pub spid: u32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub patriarch: bool,
    pub deleted: bool,
    pub props: Props<'a>,
    pub anchor: Option<Anchor>,
    /// For a group: the child coordinate rectangle (`FSPGR`).
    pub group_rect: Option<Anchor>,
    pub children: Vec<RawShape<'a>>,
    pub is_group: bool,
    pub placeholder: Option<Placeholder>,
    /// The records of the shape's text box (`OfficeArtClientTextbox`).
    pub text: Option<Rec<'a>>,
}

/// The top-level shapes and the background shape of a `DrawingContainer`.
pub struct Drawing<'a> {
    pub shapes: Vec<RawShape<'a>>,
    pub background: Option<RawShape<'a>>,
}

/// Read the drawing of a slide, master or notes container, reading at most `budget` records more
/// (one budget for the whole presentation, [`craft_officeart::record::MAX_RECORDS`] to start with).
pub fn drawing<'a>(container: &Rec<'a>, budget: &mut usize) -> Drawing<'a> {
    let mut out = Drawing { shapes: vec![], background: None };
    let Some(dg) = container.child(rt::DRAWING).and_then(|d| d.child(rt::DG_CONTAINER)) else { return out };
    // The background is the container's own shape (`OfficeArtDgContainer.shape`, [MS-ODRAW]
    // 2.2.13), not one found by its fBackground flag.
    if let Some(bg) = dg.child(rt::SP_CONTAINER) {
        out.background = Some(shape(&bg, budget));
    }
    if let Some(top) = dg.child(rt::SPGR_CONTAINER) {
        // The first shape of the top group is the patriarch, standing for the drawing itself.
        out.shapes = group_children(&top, 0, budget).into_iter().filter(|s| !s.patriarch).collect();
    }
    out
}

/// The shapes and groups of a group container, nested at most [`MAX_DEPTH`] deep, reading at most
/// `budget` records more (shared by the whole drawing).
fn group_children<'a>(gr: &Rec<'a>, depth: usize, budget: &mut usize) -> Vec<RawShape<'a>> {
    if depth > MAX_DEPTH {
        return vec![];
    }
    let mut out = vec![];
    for r in gr.children() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        match r.kind {
            rt::SP_CONTAINER => out.push(shape(&r, budget)),
            rt::SPGR_CONTAINER => {
                let mut kids = r.children();
                // A group container's first shape describes the group itself.
                let Some(head) = kids.next().filter(|h| h.kind == rt::SP_CONTAINER) else { continue };
                let mut g = shape(&head, budget);
                g.is_group = true;
                g.children = group_children(&r, depth + 1, budget).into_iter().skip(1).collect();
                out.push(g);
            }
            _ => {}
        }
    }
    out
}

fn rect_i32(r: &Rec) -> Option<Anchor> {
    Some(Anchor { left: f64::from(r.i32(0)?), top: f64::from(r.i32(4)?), right: f64::from(r.i32(8)?), bottom: f64::from(r.i32(12)?) })
}

fn shape<'a>(sp: &Rec<'a>, budget: &mut usize) -> RawShape<'a> {
    let mut s = RawShape {
        shape_type: 0,
        spid: 0,
        flip_h: false,
        flip_v: false,
        patriarch: false,
        deleted: false,
        props: Props::default(),
        anchor: None,
        group_rect: None,
        children: vec![],
        is_group: false,
        placeholder: None,
        text: None,
    };
    // Tables in specificity order: primary (the last one first), then secondary and tertiary.
    let mut primary: Vec<Rec> = vec![];
    let mut tables: Vec<Rec> = vec![];
    for r in sp.children() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        match r.kind {
            rt::FSP => {
                s.shape_type = r.instance;
                s.spid = r.u32(0).unwrap_or(0);
                let f = r.u32(4).unwrap_or(0);
                s.patriarch = f & 0x4 != 0;
                s.deleted = f & 0x8 != 0;
                s.flip_h = f & 0x40 != 0;
                s.flip_v = f & 0x80 != 0;
            }
            rt::FOPT => primary.push(r),
            rt::SECONDARY_FOPT | rt::TERTIARY_FOPT => tables.push(r),
            rt::FSPGR => s.group_rect = rect_i32(&r),
            rt::CHILD_ANCHOR => s.anchor = rect_i32(&r),
            rt::CLIENT_ANCHOR => {
                s.anchor = if r.data.len() >= 16 {
                    // RectStruct: top, left, right, bottom.
                    (|| {
                        Some(Anchor {
                            top: f64::from(r.i32(0)?),
                            left: f64::from(r.i32(4)?),
                            right: f64::from(r.i32(8)?),
                            bottom: f64::from(r.i32(12)?),
                        })
                    })()
                } else {
                    // SmallRectStruct: top, left, right, bottom.
                    (|| {
                        Some(Anchor {
                            top: f64::from(r.i16(0)?),
                            left: f64::from(r.i16(2)?),
                            right: f64::from(r.i16(4)?),
                            bottom: f64::from(r.i16(6)?),
                        })
                    })()
                };
            }
            rt::CLIENT_DATA => {
                if let Some(p) = r.child(rt::PLACEHOLDER_ATOM) {
                    s.placeholder = Some(Placeholder { position: p.u32(0).unwrap_or(0), kind: p.u8(4).unwrap_or(0) });
                }
            }
            rt::CLIENT_TEXTBOX => s.text = Some(r),
            _ => {}
        }
    }
    for t in primary.iter().rev().chain(&tables) {
        s.props.add_table(t);
    }
    s
}

/// The drawing-wide default properties (`OfficeArtDggContainer`'s `FOPT`).
pub fn default_props<'a>(dgg: Option<Rec<'a>>) -> Props<'a> {
    let mut p = Props::default();
    if let Some(dgg) = dgg {
        for t in dgg.children().filter(|r| matches!(r.kind, rt::FOPT | rt::SECONDARY_FOPT | rt::TERTIARY_FOPT)) {
            p.add_table(&t);
        }
    }
    p
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::record::build::*;
    use craft_officeart::props::MAX_PROPS;
    use craft_officeart::record::MAX_RECORDS;

    pub fn fopt(entries: &[(u16, u32)], complex: &[u8]) -> Vec<u8> {
        let mut b = vec![];
        for (id, op) in entries {
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&op.to_le_bytes());
        }
        b.extend_from_slice(complex);
        atom(rt::FOPT, entries.len() as u16, &b)
    }

    /// A table naming more than [`MAX_PROPS`] properties keeps that many, and still reads the
    /// complex data after them and the records after it.
    #[test]
    fn a_property_set_keeps_at_most_max_props() {
        let mut entries: Vec<(u16, u32)> = (0..MAX_PROPS as u16 + 100).map(|i| (0x1000 + i, u32::from(i))).collect();
        let name: Vec<u8> = "Kept\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        entries.insert(0, (0x8000 | prop::NAME, name.len() as u32));
        let fsp = atom(rt::FSP, 1, &[1025u32.to_le_bytes(), 0u32.to_le_bytes()].concat());
        let anchor = atom(rt::CHILD_ANCHOR, 0, &[1i32, 2, 3, 4].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
        let sp = container(rt::SP_CONTAINER, 0, &[fsp, fopt(&entries, &name), anchor]);
        let s = shape(&Rec::at(&sp, 0).unwrap(), &mut MAX_RECORDS.clone());
        assert_eq!(s.props.len(), MAX_PROPS);
        assert_eq!(s.props.string(prop::NAME).as_deref(), Some("Kept"));
        assert_eq!(s.props.get(0x1000), Some(0));
        assert_eq!(s.props.get(0x1000 + MAX_PROPS as u16), None, "past the limit");
        assert_eq!(s.anchor, Some(Anchor { left: 1.0, top: 2.0, right: 3.0, bottom: 4.0 }));
        // Inheriting can't pass the limit either.
        let mut p = Props::default();
        p.set(prop::FILL_COLOR, 1);
        p.inherit(&s.props);
        assert_eq!(p.len(), MAX_PROPS);
    }

    /// A shape's later primary property table wins over its earlier one, and both over its
    /// secondary table.
    #[test]
    fn the_last_primary_table_wins() {
        let fsp = atom(rt::FSP, 1, &[1025u32.to_le_bytes(), 0u32.to_le_bytes()].concat());
        let secondary = {
            let mut b = vec![];
            for (id, op) in [(prop::FILL_COLOR, 3u32), (prop::LINE_COLOR, 3)] {
                b.extend_from_slice(&id.to_le_bytes());
                b.extend_from_slice(&op.to_le_bytes());
            }
            atom(rt::SECONDARY_FOPT, 2, &b)
        };
        let sp = container(
            rt::SP_CONTAINER,
            0,
            &[fsp, secondary, fopt(&[(prop::FILL_COLOR, 1), (prop::LINE_WIDTH, 1)], &[]), fopt(&[(prop::FILL_COLOR, 2)], &[])],
        );
        let s = shape(&Rec::at(&sp, 0).unwrap(), &mut MAX_RECORDS.clone());
        assert_eq!((s.props.get(prop::FILL_COLOR), s.props.get(prop::LINE_WIDTH), s.props.get(prop::LINE_COLOR)), (Some(2), Some(1), Some(3)));
    }

    /// A slide whose drawing's top group holds `members` after its patriarch.
    fn slide_of(members: Vec<Vec<u8>>) -> Vec<u8> {
        let patriarch = container(rt::SP_CONTAINER, 0, &[atom(rt::FSP, 0, &[0, 4, 0, 0, 5, 0, 0, 0])]);
        let top = container(rt::SPGR_CONTAINER, 0, &[vec![patriarch], members].concat());
        container(rt::SLIDE, 0, &[container(rt::DRAWING, 0, &[container(rt::DG_CONTAINER, 0, &[top])])])
    }

    /// Groups nest at most [`MAX_DEPTH`] deep: a deeper group is left out, its parents kept.
    #[test]
    fn groups_nest_at_most_max_depth() {
        let leaf = || container(rt::SP_CONTAINER, 0, &[atom(rt::FSP, 1, &[0; 8])]);
        let nested = |levels: usize| (0..levels).fold(leaf(), |inner, _| container(rt::SPGR_CONTAINER, 0, &[leaf(), inner]));
        let depth = |s: &RawShape| {
            let mut n = 0;
            let mut at = s;
            while let Some(child) = at.children.first() {
                n += 1;
                at = child;
            }
            n
        };
        let read = |levels: usize| {
            let slide = slide_of(vec![nested(levels)]);
            depth(&super::drawing(&Rec::at(&slide, 0).unwrap(), &mut MAX_RECORDS.clone()).shapes[0])
        };
        let limit = craft_officeart::record::MAX_DEPTH;
        assert_eq!(read(limit), limit);
        assert_eq!(read(limit + 8), limit, "the groups past the limit are left out");
    }

    /// A presentation's drawings read at most [`MAX_RECORDS`] records together, however many shapes
    /// each holds: a slide after the budget is spent reads none.
    #[test]
    fn drawings_read_at_most_max_records_together() {
        let shapes = vec![container(rt::SP_CONTAINER, 0, &[]); MAX_RECORDS / 2 + 5];
        let slide = slide_of(shapes);
        let mut budget = MAX_RECORDS;
        let first = super::drawing(&Rec::at(&slide, 0).unwrap(), &mut budget).shapes.len();
        let second = super::drawing(&Rec::at(&slide, 0).unwrap(), &mut budget).shapes.len();
        assert!(first > MAX_RECORDS / 2 && second < MAX_RECORDS / 2, "{first} then {second}");
        assert!(super::drawing(&Rec::at(&slide, 0).unwrap(), &mut budget).shapes.is_empty());
    }

    #[test]
    fn reads_groups_anchors_and_placeholders() {
        let fsp = |t: u16, id: u32, flags: u32| atom(rt::FSP, t, &[id.to_le_bytes(), flags.to_le_bytes()].concat());
        let small =
            |t: i16, l: i16, r: i16, b: i16| atom(rt::CLIENT_ANCHOR, 0, &[t, l, r, b].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
        let child =
            |l: i32, t: i32, r: i32, b: i32| atom(rt::CHILD_ANCHOR, 0, &[l, t, r, b].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
        let ph = container(rt::CLIENT_DATA, 0, &[atom(rt::PLACEHOLDER_ATOM, 0, &[0, 0, 0, 0, 0x0D, 0, 0, 0])]);
        let patriarch = container(rt::SP_CONTAINER, 0, &[atom(rt::FSPGR, 1, &[0; 16]), fsp(0, 1024, 0x5)]);
        let title = container(rt::SP_CONTAINER, 0, &[fsp(1, 1025, 0xA00), small(10, 20, 300, 80), ph]);
        let group = container(
            rt::SPGR_CONTAINER,
            0,
            &[
                container(
                    rt::SP_CONTAINER,
                    0,
                    &[
                        atom(rt::FSPGR, 1, &[0i32, 0, 100, 100].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()),
                        fsp(0, 1026, 0x201),
                        small(100, 100, 200, 200),
                    ],
                ),
                container(rt::SP_CONTAINER, 0, &[fsp(3, 1027, 0x40), child(0, 0, 50, 50)]),
            ],
        );
        let drawing = container(rt::DRAWING, 0, &[container(rt::DG_CONTAINER, 0, &[container(rt::SPGR_CONTAINER, 0, &[patriarch, title, group])])]);
        let slide = container(rt::SLIDE, 0, &[drawing]);
        let d = super::drawing(&Rec::at(&slide, 0).unwrap(), &mut MAX_RECORDS.clone());
        assert_eq!(d.shapes.len(), 2);
        let t = &d.shapes[0];
        assert_eq!(t.placeholder.unwrap().kind, 0x0D);
        assert_eq!(t.anchor, Some(Anchor { left: 20.0, top: 10.0, right: 300.0, bottom: 80.0 }));
        let g = &d.shapes[1];
        assert!(g.is_group);
        assert_eq!(g.children.len(), 1);
        assert!(g.children[0].flip_h);
        assert_eq!(g.group_rect.unwrap().right, 100.0);
    }
}
