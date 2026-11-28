//! Drawings of versions 1 and 2: the file before RIFF, which starts with `WL` (R40 to R44). A
//! header of offsets, the settings with the page's box, then one flat list of records: 28 bytes
//! for a group or a layer, and a longer record for each drawn object, which holds its matrix, fill,
//! outline and shape. The reader maps the rectangles, ellipses and curves onto one page with one
//! layer and keeps no groups.

use crate::bytes::{Rd, u32_at};
use crate::geometry;
use crate::layout::{self, Profile};
use crate::limits::Budget;
use crate::model::{Cap, Drawing, Flags, Geometry, Join, Layer, LayerKind, Matrix, Object, Outline, Page, Shape};
use crate::records::{self, coord, fixed};
use crate::riff::push;
use crate::{Error, Warning};

/// The header: the signature, a marker byte, a zero and five u32 offsets (R40).
const HEADER: usize = 28;
/// The marker bytes read after `WL` (R40): `e` for version 1, `l` and `m` for version 2.
const MARKERS: [u8; 3] = *b"elm";
/// Where the page's box lies after the start of the settings (R41).
const PAGE_BOX: usize = 2;
/// The bytes of a group or layer record (R42).
const GROUP: usize = 28;
/// The u32 that starts every record (R42).
const RECORD_START: u32 = 24;
/// The flag bit, in byte 26, of a drawn object (R42).
const DRAWN: u8 = 0x40;
/// The bytes after a drawn object's body size (R42).
const AFTER_BODY: usize = 32;
/// Where a drawn object's fields lie (R43, R44).
const POINT: usize = 32;
const KIND: usize = 60;
const OUTLINED: usize = 67;
const OUTLINE_WIDTH: usize = 68;
const OUTLINE_COLOR: usize = 74;

/// Where a drawn object's fill and shape lie, which differs between the markers (R43, R44).
#[derive(Clone, Copy)]
struct Fields {
    fill_kind: usize,
    shape: usize,
}

impl Fields {
    fn of(marker: u8) -> Self {
        if marker == b'e' { Fields { fill_kind: 94, shape: 114 } } else { Fields { fill_kind: 105, shape: 136 } }
    }

    /// The least a drawn object takes: its fixed fields and the 4 bytes of a shape's size.
    fn least(self) -> usize {
        self.shape + 4
    }
}
/// Object types (R43).
const RECTANGLE: u8 = 0;
const ELLIPSE: u8 = 1;
const CURVE: u8 = 2;
const TEXT: u8 = 3;

/// Where the settings start, and where the records start and end, when `b` has a `WL` header
/// whose offsets fit the file.
fn header(b: &[u8]) -> Option<(usize, usize, usize)> {
    if b.get(..2)? != b"WL" || !MARKERS.contains(b.get(2)?) || *b.get(3)? != 0 {
        return None;
    }
    let (settings, start, end) = (u32_at(b, 4)? as usize, u32_at(b, 8)? as usize, u32_at(b, 12)? as usize);
    (settings >= HEADER && settings <= start && start <= end && end <= b.len()).then_some((settings, start, end))
}

/// Does `b` start like a drawing of version 1 or 2? The two letters alone are not enough: the
/// marker byte, the zero after it and the record offsets must fit too.
pub(crate) fn is_wl(b: &[u8]) -> bool {
    header(b).is_some()
}

pub(crate) fn drawing(b: &[u8], budget: &mut Budget) -> Result<Drawing, Error> {
    let (settings, start, end) = header(b).ok_or(Error::NotCdr)?;
    let marker = b.get(2).copied().ok_or(Error::NotCdr)?;
    let fields = Fields::of(marker);
    // Lengths are i16 in 1/1000 inch, as in version 3 (R5).
    let p = layout::for_version(layout::OLDEST_RIFF).ok_or(Error::NotCdr)?;
    let mut warn = Vec::new();
    let mut objects = Vec::new();
    let mut at = start;
    // Every step takes at least a group record's bytes, so the file's size bounds the steps.
    while end - at >= GROUP {
        budget.take_chunk()?;
        // A record that doesn't start as records do is past one whose length isn't known.
        if u32_at(b, at) != Some(RECORD_START) {
            break;
        }
        if b.get(at + 26).is_none_or(|flags| flags & DRAWN == 0) {
            at += GROUP;
            continue;
        }
        // A drawn object of another type than rectangle, ellipse, curve and text doesn't say its
        // length in the same way, so the records after it can't be found.
        if b.get(at + KIND).is_none_or(|kind| *kind > TEXT) {
            push(&mut warn, Warning::UnsupportedObjects);
            at = end;
            break;
        }
        let len = Rd::at(b, at + 28).u16().map_or(0, |body| usize::from(body) + AFTER_BODY);
        let Some(rec) = b.get(at..at + len).filter(|_| len >= fields.least() && at + len <= end) else {
            push(&mut warn, Warning::Damaged);
            at = end;
            break;
        };
        at += len;
        if !budget.take_object() {
            push(&mut warn, Warning::TooDeep);
            at = end;
            break;
        }
        objects.extend(object(rec, fields, &p, budget, &mut warn));
    }
    if at != end {
        push(&mut warn, Warning::Damaged);
    }
    let version = if marker == b'e' { 100 } else { 200 };
    let layer = Layer { name: String::new(), kind: LayerKind::Drawing, flags: Flags::default(), objects, guides: Vec::new() };
    let pages = vec![Page { size: None, layers: vec![layer] }];
    Ok(Drawing { version, page_size: page_box(b, settings, &p), pages, master: Vec::new(), guides: Vec::new(), warnings: warn })
}

/// The page's size from its box (R41): left, top, right and bottom around the page's centre.
fn page_box(b: &[u8], settings: usize, p: &Profile) -> Option<(f64, f64)> {
    let mut r = Rd::at(b, settings + PAGE_BOX);
    let (left, top, right, bottom) = (coord(&mut r, p)?, coord(&mut r, p)?, coord(&mut r, p)?, coord(&mut r, p)?);
    let (w, h) = ((right - left).abs(), (top - bottom).abs());
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// One drawn object's record (R43, R44): a shape, or `None` with a warning for text, for an object
/// of another type and for a shape that can't be read.
fn object(rec: &[u8], fields: Fields, p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Object> {
    // A curve left out at the coordinate limit is not damage.
    budget.coordinates_refused();
    let read = shape(rec, fields, p, budget, warn);
    if read.is_none() && !budget.coordinates_refused() {
        push(warn, Warning::Damaged);
    }
    read?.map(|s| Object::Shape(Box::new(s)))
}

/// The shape of a record. The outer `None` is a record that can't be read; the inner one an object
/// the reader leaves out, with its warning.
fn shape(rec: &[u8], fields: Fields, p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Option<Shape>> {
    let k = p.unit();
    let mut r = Rd::at(rec, POINT);
    let (x, y) = (f64::from(r.i16()?), f64::from(r.i16()?));
    // x' = a x + b y + tx + x of the point, y' = c x + d y + ty + y of the point.
    let (a, b, tx, c, d, ty) = (fixed(&mut r)?, fixed(&mut r)?, fixed(&mut r)?, fixed(&mut r)?, fixed(&mut r)?, fixed(&mut r)?);
    let matrix = Matrix([a, c, b, d, (tx + x) * k, (ty + y) * k]);
    let body = rec.get(fields.shape..)?;
    let geometry = match *rec.get(KIND)? {
        RECTANGLE => rect(body, p)?,
        ELLIPSE => {
            let mut s = Rd::new(body);
            let (w, h) = (coord(&mut s, p)?, coord(&mut s, p)?);
            // Two angles follow. No source confirms how they make an arc, so an ellipse whose
            // angles differ is left out; one whose angles are missing is damage.
            let (from, to) = (s.i16()?, s.i16()?);
            if from != to {
                push(warn, Warning::UnsupportedObjects);
                return Some(None);
            }
            Geometry::Ellipse { w, h }
        }
        CURVE => Geometry::Path(geometry::point_list(body, p, budget, warn)?),
        _ => {
            push(warn, Warning::UnsupportedObjects);
            return Some(None);
        }
    };
    let color_at = |at: usize, warn: &mut Vec<Warning>| Some(records::color5(rec.get(at..at + 5)?.try_into().ok()?, warn));
    let fill = match *rec.get(fields.fill_kind)? {
        0 => None,
        1 => Some(color_at(fields.fill_kind + 1, warn)?),
        _ => {
            push(warn, Warning::UnsupportedFills);
            None
        }
    };
    let outline = if *rec.get(OUTLINED)? != 0 {
        let width = f64::from(Rd::at(rec, OUTLINE_WIDTH).u16()?) * k;
        Some(Outline {
            width,
            color: color_at(OUTLINE_COLOR, warn)?,
            cap: Cap::Butt,
            join: Join::Miter,
            dashes: Vec::new(),
            behind_fill: false,
            scale_with: false,
        })
    } else {
        None
    };
    Some(Some(Shape::new(geometry, matrix, fill, outline)))
}

/// A rectangle (R44): its width and height, then a corner radius, which is kept when it is at most
/// half the shorter side.
fn rect(body: &[u8], p: &Profile) -> Option<Geometry> {
    let mut r = Rd::new(body);
    let (w, h) = (coord(&mut r, p)?, coord(&mut r, p)?);
    let radius = coord(&mut r, p).filter(|v| *v > 0.0 && *v <= w.abs().min(h.abs()) / 2.0).unwrap_or(0.0);
    Some(Geometry::Rect { w, h, radii: [radius; 4] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use crate::model::Color;
    use crate::synth::{self, WlShape};

    fn read(b: &[u8]) -> Result<Drawing, Error> {
        drawing(b, &mut Budget::new(Limits::default()))
    }

    #[test]
    fn the_header_is_taken_only_when_its_offsets_fit() {
        let file = synth::wl_file(b'e', (612.0, 792.0), &[synth::wl_group()]);
        assert!(is_wl(&file));
        // The two letters, a marker and a zero are not enough.
        assert!(!is_wl(b"WLe\0"));
        assert!(!is_wl(b"WLe\0 and then some text that is long enough to hold offsets"));
        let with = |at: usize, v: u32| {
            let mut f = file.clone();
            f[at..at + 4].copy_from_slice(&v.to_le_bytes());
            f
        };
        // Records that start inside the header, start after their end, or end past the file.
        assert!(!is_wl(&with(8, 27)));
        assert!(!is_wl(&with(8, file.len() as u32)));
        assert!(!is_wl(&with(12, file.len() as u32 + 1)));
        // Settings that start inside the header or after the records.
        assert!(!is_wl(&with(4, 27)));
        assert!(!is_wl(&with(4, 65)));
        let mut other = file.clone();
        other[2] = b'x';
        assert!(!is_wl(&other));
        // No sample shows the marker `d`, so it is not taken.
        other[2] = b'd';
        assert!(!is_wl(&other));
        for marker in *b"lm" {
            assert!(is_wl(&synth::wl_file(marker, (612.0, 792.0), &[synth::wl_group()])));
        }
        other[2] = b'e';
        other[3] = 1;
        assert!(!is_wl(&other));
    }

    #[test]
    fn a_version_2_file_keeps_its_page_box_fill_and_shape_where_that_version_puts_them() {
        let (red, black) = ([1, 0, 100, 100, 0], [1, 0, 0, 0, 100]);
        let rect = synth::wl_object(WlShape::Rect { w: 72.0, h: 36.0, radius: 9.0 }, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], Some(red), Some((2.0, black)));
        for marker in *b"elm" {
            let file = synth::wl_file(marker, (720.0, 504.0), std::slice::from_ref(&rect));
            // Version 2 files start their settings after a picture, not after the header.
            assert_eq!(u32_at(&file, 4) == Some(28), marker == b'e');
            let d = read(&file).unwrap();
            let near = |a: f64, b: f64| (a - b).abs() < 0.02;
            assert_eq!((d.version, d.warnings.clone()), (if marker == b'e' { 100 } else { 200 }, vec![]));
            assert!(d.page_size.is_some_and(|(w, h)| near(w, 720.0) && near(h, 504.0)), "{:?}", d.page_size);
            let Object::Shape(s) = &d.pages[0].layers[0].objects[0] else { panic!("group") };
            assert!(
                matches!(s.geometry, Geometry::Rect { w, h, radii } if near(w, 72.0) && near(h, 36.0) && near(radii[0], 9.0)),
                "{}",
                marker as char
            );
            assert_eq!(s.fill, Some(Color::Cmyk([0.0, 1.0, 1.0, 0.0])), "{}", marker as char);
            assert!(s.outline.as_ref().is_some_and(|o| near(o.width, 2.0) && o.color == Color::Cmyk([0.0, 0.0, 0.0, 1.0])), "{}", marker as char);
        }
    }

    #[test]
    fn a_drawn_object_is_marked_by_its_flag_bit() {
        let rect = synth::wl_object(WlShape::Rect { w: 72.0, h: 36.0, radius: 0.0 }, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], None, None);
        let mut file = synth::wl_file(b'l', (612.0, 792.0), &[rect]);
        // Files with the marker `l` set another flag bit beside it.
        file[64 + 26] = 0xc0;
        let d = read(&file).unwrap();
        assert_eq!((d.pages[0].layers[0].objects.len(), d.warnings), (1, vec![]));
    }

    #[test]
    fn the_walk_stops_where_records_can_no_longer_be_found() {
        let unit = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let rect = || synth::wl_object(WlShape::Rect { w: 72.0, h: 36.0, radius: 0.0 }, unit, None, None);
        // An object of another type doesn't give its length in the same way.
        let mut other = rect();
        other[KIND] = 0xff;
        let d = read(&synth::wl_file(b'm', (612.0, 792.0), &[rect(), other, rect()])).unwrap();
        assert_eq!((d.pages[0].layers[0].objects.len(), d.warnings), (1, vec![Warning::UnsupportedObjects]));
        // A record that doesn't start as records do.
        let mut lost = rect();
        lost[0] = 0x84;
        let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[rect(), lost, rect()])).unwrap();
        assert_eq!((d.pages[0].layers[0].objects.len(), d.warnings), (1, vec![Warning::Damaged]));
    }

    #[test]
    fn a_rectangle_keeps_a_radius_of_at_most_half_its_shorter_side() {
        let p = layout::for_version(300).unwrap();
        let body = |radius: i16| [1000i16, 500, radius].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let radius_of = |b: &[u8]| match rect(b, &p) {
            Some(Geometry::Rect { w, h, radii }) if (w - 72.0).abs() < 1e-9 && (h - 36.0).abs() < 1e-9 => radii[0],
            other => panic!("{other:?}"),
        };
        assert!((radius_of(&body(200)) - 14.4).abs() < 1e-9);
        for radius in [300, -5, 0] {
            assert_eq!(radius_of(&body(radius)), 0.0, "{radius}");
        }
        // A record that ends after the size has square corners.
        assert_eq!(radius_of(&body(200)[..4]), 0.0);
        assert_eq!(rect(&body(200)[..3], &p), None);
    }

    #[test]
    fn a_record_shorter_than_its_fields_ends_the_walk_as_damage() {
        let rect = synth::wl_object(WlShape::Rect { w: 72.0, h: 36.0, radius: 0.0 }, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], Some([1, 0, 0, 0, 100]), None);
        let mut short = rect.clone();
        // A body size that leaves less than the fixed fields.
        short[28..30].copy_from_slice(&((Fields::of(b'e').least() - AFTER_BODY - 1) as u16).to_le_bytes());
        let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[rect.clone(), short, rect])).unwrap();
        assert_eq!(d.pages[0].layers[0].objects.len(), 1);
        assert_eq!(d.warnings, vec![Warning::Damaged]);
    }

    #[test]
    fn an_ellipse_without_its_angles_is_damage() {
        let ellipse =
            synth::wl_object(WlShape::Ellipse { w: 72.0, h: 36.0, angles: (0, 0) }, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], Some([1, 0, 0, 0, 100]), None);
        let mut cut = ellipse.clone();
        // A body size that ends the record after the ellipse's width and height.
        cut[28..30].copy_from_slice(&((Fields::of(b'e').least() - AFTER_BODY) as u16).to_le_bytes());
        cut.truncate(Fields::of(b'e').least());
        let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[ellipse, cut])).unwrap();
        assert_eq!(d.pages[0].layers[0].objects.len(), 1);
        assert_eq!(d.warnings, vec![Warning::Damaged]);
    }

    #[test]
    fn fill_types_other_than_uniform_and_unknown_inks_are_named() {
        let mut textured =
            synth::wl_object(WlShape::Rect { w: 72.0, h: 36.0, radius: 0.0 }, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], Some([0, 9, 0, 50, 0]), None);
        let d = read(&synth::wl_file(b'm', (612.0, 792.0), std::slice::from_ref(&textured))).unwrap();
        let Object::Shape(s) = &d.pages[0].layers[0].objects[0] else { panic!("group") };
        assert_eq!(s.fill, Some(Color::Spot { name: "Spot 0:9".into(), tint: 0.5, look: None }));
        assert_eq!((d.version, d.warnings), (200, vec![Warning::SpotLooks]));
        textured[Fields::of(b'e').fill_kind] = 5;
        let d = read(&synth::wl_file(b'm', (612.0, 792.0), &[textured])).unwrap();
        let Object::Shape(s) = &d.pages[0].layers[0].objects[0] else { panic!("group") };
        assert_eq!((s.fill.clone(), d.warnings), (None, vec![Warning::UnsupportedFills]));
    }
}
