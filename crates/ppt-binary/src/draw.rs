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
    /// Whether the record budget ran out before the drawing was read whole, so shapes (or the
    /// background) are missing.
    pub cut: bool,
}

/// Drawing records a presentation may still read, and whether one it needed was left unread.
struct Budget {
    left: usize,
    cut: bool,
}

/// Read the drawing of a slide, master or notes container, reading at most `budget` records more
/// (one budget for the whole presentation, [`craft_officeart::record::MAX_RECORDS`] to start with).
/// A shape or group the budget cuts short is left out, and so is a background it leaves unread;
/// [`Drawing::cut`] says so.
pub fn drawing<'a>(container: &Rec<'a>, budget: &mut usize) -> Drawing<'a> {
    read(container, budget, true)
}

/// [`drawing`] without the background, whose records are then not read: for a caller that uses
/// only the shapes (a notes page's text).
pub fn shapes<'a>(container: &Rec<'a>, budget: &mut usize) -> Drawing<'a> {
    read(container, budget, false)
}

fn read<'a>(container: &Rec<'a>, records: &mut usize, with_background: bool) -> Drawing<'a> {
    let mut budget = Budget { left: *records, cut: false };
    let out = read_within(container, &mut budget, with_background);
    *records = budget.left;
    Drawing { cut: budget.cut, ..out }
}

fn read_within<'a>(container: &Rec<'a>, budget: &mut Budget, with_background: bool) -> Drawing<'a> {
    let mut out = Drawing { shapes: vec![], background: None, cut: false };
    let Some(dg) = container.child(rt::DRAWING).and_then(|d| d.child(rt::DG_CONTAINER)) else { return out };
    // The background is the container's own shape (`OfficeArtDgContainer.shape`, [MS-ODRAW]
    // 2.2.13), not one found by its fBackground flag. One pass finds it and the top group, each
    // child looked at charged to the budget.
    let (mut bg, mut top) = (None, None);
    for r in dg.children() {
        if bg.is_some() && top.is_some() || !take(budget) {
            break;
        }
        match r.kind {
            rt::SP_CONTAINER if bg.is_none() => bg = Some(r),
            rt::SPGR_CONTAINER if top.is_none() => top = Some(r),
            _ => {}
        }
    }
    if let Some(bg) = bg.filter(|_| with_background) {
        out.background = shape(&bg, budget);
    }
    if let Some(top) = top {
        // The first shape of the top group is the patriarch, standing for the drawing itself.
        out.shapes = members(top.children(), 0, budget).into_iter().filter(|s| !s.patriarch).collect();
    }
    out
}

/// Take one record off `budget`; `false`, and the budget marked cut, when none are left.
fn take(budget: &mut Budget) -> bool {
    match budget.left.checked_sub(1) {
        Some(left) => {
            budget.left = left;
            true
        }
        None => {
            budget.cut = true;
            false
        }
    }
}

/// The shapes and groups among a group container's records, nested at most [`MAX_DEPTH`] groups
/// deep (a deeper group is left out, shapes beside it kept), reading at most `budget` records
/// more (shared by the whole presentation). A group whose members the budget cuts short is left
/// out, as a shape is, rather than kept without them.
fn members<'a>(records: crate::record::Records<'a>, depth: usize, budget: &mut Budget) -> Vec<RawShape<'a>> {
    let mut out = vec![];
    for r in records {
        if !take(budget) {
            break;
        }
        match r.kind {
            rt::SP_CONTAINER => out.extend(shape(&r, budget)),
            rt::SPGR_CONTAINER if depth < MAX_DEPTH => {
                let mut kids = r.children();
                // A group container's first shape describes the group itself; it is read once.
                let Some(head) = kids.next().filter(|h| h.kind == rt::SP_CONTAINER) else { continue };
                if !take(budget) {
                    break;
                }
                let Some(mut g) = shape(&head, budget) else { break };
                g.is_group = true;
                g.children = members(kids, depth + 1, budget);
                // Once the budget is cut it stays spent, so nothing after this group is read.
                if budget.cut {
                    break;
                }
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

/// One shape container, or `None` when the budget runs out before all its records are read.
fn shape<'a>(sp: &Rec<'a>, budget: &mut Budget) -> Option<RawShape<'a>> {
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
        if !take(budget) {
            return None;
        }
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
                // Each record looked at in the client data is charged too.
                for c in r.children() {
                    if !take(budget) {
                        return None;
                    }
                    if c.kind == rt::PLACEHOLDER_ATOM {
                        s.placeholder = Some(Placeholder { position: c.u32(0).unwrap_or(0), kind: c.u8(4).unwrap_or(0) });
                        break;
                    }
                }
            }
            rt::CLIENT_TEXTBOX => {
                // The text box's records are read later, by the text reader: charged here.
                for _ in r.children() {
                    if !take(budget) {
                        return None;
                    }
                }
                s.text = Some(r);
            }
            _ => {}
        }
    }
    for t in primary.iter().rev().chain(&tables) {
        s.props.add_table(t);
    }
    Some(s)
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
        let s = shape(&Rec::at(&sp, 0).unwrap(), &mut Budget { left: MAX_RECORDS, cut: false }).unwrap();
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
        let s = shape(&Rec::at(&sp, 0).unwrap(), &mut Budget { left: MAX_RECORDS, cut: false }).unwrap();
        assert_eq!((s.props.get(prop::FILL_COLOR), s.props.get(prop::LINE_WIDTH), s.props.get(prop::LINE_COLOR)), (Some(2), Some(1), Some(3)));
    }

    /// A slide whose drawing's top group holds `members` after its patriarch.
    fn slide_of(members: Vec<Vec<u8>>) -> Vec<u8> {
        let patriarch = container(rt::SP_CONTAINER, 0, &[atom(rt::FSP, 0, &[0, 4, 0, 0, 5, 0, 0, 0])]);
        let top = container(rt::SPGR_CONTAINER, 0, &[vec![patriarch], members].concat());
        container(rt::SLIDE, 0, &[container(rt::DRAWING, 0, &[container(rt::DG_CONTAINER, 0, &[top])])])
    }

    /// Groups nest at most [`MAX_DEPTH`] deep: a deeper group is left out, its parents and the
    /// shapes beside it kept.
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
        // `limit` groups, the innermost holding a shape: all read.
        assert_eq!(read(limit), limit);
        // A group at the next level is left out, not read as an empty group.
        assert_eq!(read(limit + 1), limit - 1, "the group past the limit is left out");
        assert_eq!(read(limit + 8), limit - 1, "the groups past the limit are left out");
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

    /// A budget of exactly a drawing's records reads all of it: each record is charged once, a
    /// group's first shape included. One record fewer leaves out the shape it cuts short, never
    /// keeping it half read, and a background the budget doesn't reach is left out rather than
    /// read as an empty (white) one.
    #[test]
    fn the_record_budget_reads_whole_shapes() {
        let fsp = || atom(rt::FSP, 1, &[0; 8]);
        let anchor = atom(rt::CLIENT_ANCHOR, 0, &[0; 8]);
        let ph = container(rt::CLIENT_DATA, 0, &[atom(rt::PLACEHOLDER_ATOM, 0, &[0, 0, 0, 0, 0x0D, 0, 0, 0])]);
        // The text box's own records count too.
        let text = container(rt::CLIENT_TEXTBOX, 0, &[atom(rt::TEXT_HEADER, 0, &[0; 4])]);
        let title = container(rt::SP_CONTAINER, 0, &[fsp(), anchor, ph, text]);
        let group = container(
            rt::SPGR_CONTAINER,
            0,
            &[container(rt::SP_CONTAINER, 0, &[fsp(), atom(rt::FSPGR, 1, &[0; 16])]), container(rt::SP_CONTAINER, 0, &[fsp()])],
        );
        let background = container(rt::SP_CONTAINER, 0, &[fsp()]);
        let patriarch = container(rt::SP_CONTAINER, 0, &[atom(rt::FSP, 0, &[0, 4, 0, 0, 5, 0, 0, 0])]);
        let top = container(rt::SPGR_CONTAINER, 0, &[patriarch, group, title]);
        let dg = container(rt::DG_CONTAINER, 0, &[background, top]);
        let slide = container(rt::SLIDE, 0, &[container(rt::DRAWING, 0, std::slice::from_ref(&dg))]);
        // Every record below the drawing container, nested ones included.
        fn count(b: &[u8]) -> usize {
            crate::record::Records::new(b).map(|r| 1 + if r.is_container() { count(r.data) } else { 0 }).sum()
        }
        let all = count(Rec::at(&dg, 0).unwrap().data);
        let read = |mut budget: usize| super::drawing(&Rec::at(&slide, 0).unwrap(), &mut budget);
        let d = read(all);
        assert!(d.background.is_some());
        assert_eq!(d.shapes.len(), 2, "the group and the title");
        assert_eq!(d.shapes[0].children.len(), 1, "the group's one member");
        assert!(d.shapes[1].placeholder.is_some() && d.shapes[1].text.is_some());
        assert!(!d.cut, "a budget used up by the last record cuts nothing");
        let short = read(all - 1);
        assert_eq!(short.shapes.len(), 1, "the title, cut short, is left out");
        assert!(short.cut, "and the drawing says so");
        assert!(read(2).background.is_none(), "the background is left out, not made empty");
        // At no budget is the group kept without its member.
        for budget in 0..all {
            assert!(read(budget).shapes.iter().all(|s| !s.is_group || !s.children.is_empty()), "budget {budget}");
        }
        // Without the background, its records aren't read.
        let mut left = all;
        let d = super::shapes(&Rec::at(&slide, 0).unwrap(), &mut left);
        assert!(d.background.is_none() && d.shapes.len() == 2 && left > 0);
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
