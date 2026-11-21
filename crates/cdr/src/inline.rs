//! What a version 3 object holds as arguments where later versions name records: its transforms
//! (argument 100, R33), its fill (argument 20, R34) and its outline (argument 10, R35).

use crate::bytes::Rd;
use crate::layout::Profile;
use crate::model::{Cap, Color, Join, Matrix, Outline};
use crate::records::{self, angle, coord};
use crate::riff::push;
use crate::{Limits, Warning};

/// Transform entry types (R33): a matrix, and an envelope.
const MATRIX: u16 = 0x8c;
const ENVELOPE: u16 = 0x14;
/// The bytes of one transform entry (R33).
const ENTRY: usize = 10;
/// The bytes an outline keeps for its dashes, one byte each (R35).
const DASH_AREA: usize = 10;

/// The transforms of one object or group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    /// The matrices composed in file order, in points.
    pub(crate) matrix: Matrix,
    /// An envelope bends the shape. The reader doesn't apply envelopes.
    pub(crate) envelope: bool,
}

/// The transforms argument (R33): a u16, the entry count, then 10-byte entries of a type and, at
/// byte 6, where the entry's data starts in the argument. The count is held to the entries the
/// argument holds and to the arguments a record may have. Entries of other types are passed over.
pub(crate) fn transforms(b: &[u8], p: &Profile, limits: &Limits, warn: &mut Vec<Warning>) -> Option<Placement> {
    let mut r = Rd::new(b);
    r.skip(2)?;
    let stored = usize::from(r.u16()?);
    let count = stored.min(r.left() / ENTRY).min(limits.arguments);
    if count < stored {
        push(warn, Warning::Damaged);
    }
    let mut out = Placement { matrix: Matrix::IDENTITY, envelope: false };
    for _ in 0..count {
        let mut e = Rd::new(r.bytes(ENTRY)?);
        let kind = e.u16()?;
        e.skip(4)?;
        let at = usize::from(e.u16()?);
        match kind {
            MATRIX => out.matrix = records::fixed_matrix(&mut Rd::at(b, at), p.unit())?.after(&out.matrix),
            ENVELOPE => out.envelope = true,
            _ => {}
        }
    }
    Some(out)
}

/// The fill argument (R34): a type byte, 0 for none and 1 for a uniform fill with its color. Fills
/// of other types read as none with a warning. The outer `None` is an argument that can't be read.
pub(crate) fn fill(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> Option<Option<Color>> {
    let mut r = Rd::new(b);
    match r.u8()? {
        0 => Some(None),
        1 => Some(Some(records::color(&mut r, p, warn)?)),
        _ => {
            push(warn, Warning::UnsupportedFills);
            Some(None)
        }
    }
}

/// The outline argument (R35): a flags byte whose bit 0 is set when the object is outlined, the
/// width, the stretch, the nib angle, the color, 7 bytes, the dash count and 10 bytes that hold the
/// dashes, the join and the caps (u16 each), then the two arrowhead ids. An argument that ends
/// after its color gives a solid line with butt caps and miter joins. The outer `None` is an
/// argument that can't be read.
pub(crate) fn outline(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> Option<Option<Outline>> {
    let mut r = Rd::new(b);
    if r.u8()? & 1 == 0 {
        return Some(None);
    }
    let width = coord(&mut r, p)?.abs();
    let _stretch = r.u16()?;
    let _nib = angle(&mut r, p)?;
    let color = records::color(&mut r, p, warn)?;
    let mut out = Outline { width, color, cap: Cap::Butt, join: Join::Miter, dashes: Vec::new(), behind_fill: false, scale_with: false };
    let Some(stored) = r.skip(7).and_then(|()| r.u8()).map(usize::from) else { return Some(Some(out)) };
    let Some(area) = r.bytes(DASH_AREA) else { return Some(Some(out)) };
    if stored > DASH_AREA {
        push(warn, Warning::Damaged);
    }
    let dashes: Vec<f64> = area.iter().take(stored).map(|d| f64::from(*d)).collect();
    if dashes.iter().any(|d| *d > 0.0) {
        out.dashes = dashes;
    }
    out.join = match r.u16() {
        Some(1) => Join::Round,
        Some(2) => Join::Bevel,
        _ => Join::Miter,
    };
    out.cap = match r.u16() {
        Some(1) => Cap::Round,
        Some(2) => Cap::Square,
        _ => Cap::Butt,
    };
    if matches!((r.u32(), r.u32()), (Some(s), Some(e)) if s != 0 || e != 0) {
        push(warn, Warning::Arrowheads);
    }
    Some(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p3() -> Profile {
        crate::layout::for_version(300).unwrap()
    }

    /// A transforms argument of `entries` (type, data offset), then `data` from byte 4 + 10 for
    /// each entry.
    fn arg(count: u16, entries: &[(u16, u16)], data: &[i32]) -> Vec<u8> {
        let mut b = vec![0, 0];
        b.extend(count.to_le_bytes());
        for (kind, at) in entries {
            b.extend(kind.to_le_bytes());
            b.extend([0; 4]);
            b.extend(at.to_le_bytes());
            b.extend([0; 2]);
        }
        b.extend(data.iter().flat_map(|v| v.to_le_bytes()));
        b
    }

    #[test]
    fn matrices_compose_in_file_order_and_other_entries_are_passed_over() {
        // Scale by 2, then move 1 inch right: entry types 0x8c, an unknown one and 0x8c.
        let scale = [131072, 0, 0, 0, 131072, 0];
        let shift = [65536, 0, 1000, 0, 65536, 0];
        let b = arg(3, &[(MATRIX, 34), (0x99, 0), (MATRIX, 58)], &[scale, shift].concat());
        let mut w = vec![];
        let t = transforms(&b, &p3(), &Limits::default(), &mut w).unwrap();
        assert_eq!((t.matrix, t.envelope), (Matrix([2.0, 0.0, 0.0, 2.0, 72.0, 0.0]), false));
        assert!(w.is_empty());
        let b = arg(2, &[(MATRIX, 24), (ENVELOPE, 0)], &scale);
        assert!(transforms(&b, &p3(), &Limits::default(), &mut vec![]).unwrap().envelope);
    }

    #[test]
    fn hostile_transform_arguments_do_not_read_past_their_bytes() {
        let scale = [131072, 0, 0, 0, 131072, 0];
        // A count of 65,535 over one entry reads that entry, with the damage warning.
        let b = arg(u16::MAX, &[(MATRIX, 14)], &scale);
        let mut w = vec![];
        assert_eq!(transforms(&b, &p3(), &Limits::default(), &mut w).unwrap().matrix.0[0], 2.0);
        assert_eq!(w, vec![Warning::Damaged]);
        // A data offset past the argument, and one that leaves less than a matrix.
        for at in [5000, 14 + 4] {
            assert!(transforms(&arg(1, &[(MATRIX, at)], &scale), &p3(), &Limits::default(), &mut vec![]).is_none(), "{at}");
        }
        // More entries than a record may have arguments.
        let many: Vec<(u16, u16)> = vec![(0x99, 0); 100];
        let limits = Limits { arguments: 8, ..Limits::default() };
        let mut w = vec![];
        assert!(transforms(&arg(100, &many, &[]), &p3(), &limits, &mut w).is_some());
        assert_eq!(w, vec![Warning::Damaged]);
        for len in 0..4 {
            assert!(transforms(&b[..len], &p3(), &Limits::default(), &mut vec![]).is_none());
        }
    }

    /// An outline argument: flags, width in 1/1000 inch, black, `dashes`, join, caps, arrowheads.
    fn outline_arg(flags: u8, count: u8, dashes: &[u8], join: u16, cap: u16, heads: [u32; 2]) -> Vec<u8> {
        let mut b = vec![flags];
        b.extend(28i16.to_le_bytes());
        b.extend([100, 0, 0, 0]);
        b.extend([1, 0, 0, 0, 100]);
        b.extend([0; 7]);
        b.push(count);
        let mut area = dashes.to_vec();
        area.resize(DASH_AREA, 0);
        b.extend(area);
        b.extend(join.to_le_bytes());
        b.extend(cap.to_le_bytes());
        b.extend(heads.iter().flat_map(|h| h.to_le_bytes()));
        b
    }

    #[test]
    fn an_outline_reads_its_width_color_dashes_join_and_caps() {
        let mut w = vec![];
        let o = outline(&outline_arg(1, 2, &[3, 1], 1, 2, [0, 0]), &p3(), &mut w).unwrap().unwrap();
        assert!((o.width - 2.016).abs() < 1e-9, "{}", o.width);
        assert_eq!((o.color, o.join, o.cap, o.dashes), (Color::Cmyk([0.0, 0.0, 0.0, 1.0]), Join::Round, Cap::Square, vec![3.0, 1.0]));
        assert!(w.is_empty());
        // Bit 0 clear: no outline, whatever follows.
        assert_eq!(outline(&outline_arg(0, 2, &[3, 1], 1, 2, [7, 0]), &p3(), &mut w), Some(None));
        assert_eq!(outline(&[0], &p3(), &mut w), Some(None));
        assert!(w.is_empty());
        outline(&outline_arg(1, 0, &[], 0, 0, [0, 9]), &p3(), &mut w).unwrap().unwrap();
        assert_eq!(w, vec![Warning::Arrowheads]);
    }

    #[test]
    fn a_dash_count_past_the_dash_bytes_is_held_to_them() {
        let mut w = vec![];
        let o = outline(&outline_arg(1, 255, &[2; 10], 0, 0, [0, 0]), &p3(), &mut w).unwrap().unwrap();
        assert_eq!(o.dashes.len(), DASH_AREA);
        assert_eq!(w, vec![Warning::Damaged]);
        // An argument that ends inside the dashes keeps a solid line; one cut before the color is
        // unreadable.
        let full = outline_arg(1, 255, &[2; 10], 0, 0, [0, 0]);
        assert_eq!(outline(&full[..25], &p3(), &mut vec![]).unwrap().unwrap().dashes, Vec::<f64>::new());
        assert_eq!(outline(&full[..12], &p3(), &mut vec![]).unwrap().unwrap().cap, Cap::Butt);
        for len in 0..12 {
            assert!(outline(&full[..len], &p3(), &mut vec![]).is_none(), "{len}");
        }
    }

    #[test]
    fn fills_read_none_uniform_and_other_types() {
        let mut w = vec![];
        assert_eq!(fill(&[0], &p3(), &mut w), Some(None));
        assert_eq!(fill(&[1, 1, 100, 0, 0, 0], &p3(), &mut w), Some(Some(Color::Cmyk([1.0, 0.0, 0.0, 0.0]))));
        assert!(w.is_empty());
        for kind in [2, 4, 6, 7, 10] {
            let mut w = vec![];
            assert_eq!(fill(&[kind, 0, 0], &p3(), &mut w), Some(None));
            assert_eq!(w, vec![Warning::UnsupportedFills], "{kind}");
        }
        assert_eq!(fill(&[], &p3(), &mut w), None);
        assert_eq!(fill(&[1, 1, 100], &p3(), &mut w), None);
    }
}
