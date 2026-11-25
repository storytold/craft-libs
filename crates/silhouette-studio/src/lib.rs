//! Reader for Silhouette Studio documents: `.studio` (header `silhouette04;`) and
//! `.studio3` (header `silhouette05;`).
//!
//! It reads every object's outline (open and closed paths, lines and Bézier curves, compound
//! paths, text as outlines), each outline's line colour and the page size. Coordinates are
//! millimetres from the page's top-left corner, y down, as the file stores them. Fills, layers
//! and cut settings are not read.
//!
//! The layout comes from differential analysis of documents drawn and saved in Silhouette
//! Studio: change one thing, save, compare the bytes. No code or binary was read.
//!
//! The reader is bounded and fails closed: input size and shape/point counts are capped, every
//! length is checked against the bytes that are really there, and a body that is not in the
//! plain layout known here (for example a protected store design) is rejected instead of
//! guessed at. Records it does not understand are skipped. Depends on no other Craft crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use kurbo::{BezPath, Point, Vec2};

/// Largest file read (store designs and saved samples are far smaller).
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SHAPES: usize = 100_000;
pub const MAX_POINTS: usize = 2_000_000;
/// Coordinates are millimetres; anything past a kilometre is corruption.
const MAX_COORD_MM: f32 = 1.0e6;

/// Nodes the scan may parse in all, counting candidates that turn out not to be outlines:
/// bytes that merely look like huge outlines cannot make the scan quadratic.
const MAX_SCANNED_NODES: usize = 4 * MAX_POINTS;

const MAGIC_LEN: usize = 13;
/// `.studio3` files start with a 1024-byte header holding a table of section lengths.
const V3_HEADER_LEN: usize = 1024;
const V3_TABLE_START: usize = 0x10;
const V3_TABLE_END: usize = 0x3e8;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("not a Silhouette Studio document")]
    NotStudio,
    #[error("Silhouette Studio document version {version} is not supported yet")]
    Unsupported { version: String },
    /// The container parsed but its body is not plain records. Never decrypted.
    #[error("this design is protected or encrypted (for example a store purchase)")]
    Protected,
    #[error("file exceeds the 64 MiB import limit")]
    TooLarge,
    #[error("damaged Silhouette Studio document: {0}")]
    Malformed(String),
}

/// Which container the bytes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// `.studio`: the document body alone, header `silhouette04;`.
    V2,
    /// `.studio3`: a 1024-byte section table, then the body (header `silhouette05;`),
    /// a thumbnail PNG and further sections.
    V3,
}

/// One object's outline: a subpath per contour, so a compound path stays one shape.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Page millimetres from the top-left corner, y down.
    pub path: BezPath,
    /// The object's line colour (R, G, B), when its style record was found.
    pub line_color: Option<[u8; 3]>,
}

/// A document's shapes in file order, plus notes about what was skipped.
#[derive(Debug, Clone)]
pub struct Imported {
    pub version: Version,
    /// Page width and height in millimetres; `None` when the page record is not as expected.
    pub page_mm: Option<(f64, f64)>,
    pub shapes: Vec<Shape>,
    pub warnings: Vec<String>,
}

/// Is this file name one this crate reads?
pub fn is_studio_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".studio") || lower.ends_with(".studio3")
}

/// Cheap header check, for dispatch by content rather than extension.
pub fn sniff(bytes: &[u8]) -> Option<Version> {
    match bytes.get(..MAGIC_LEN)? {
        b"silhouette04;" => Some(Version::V2),
        b"silhouette05;" => Some(Version::V3),
        _ => None,
    }
}

/// Reads a `.studio` / `.studio3` document.
pub fn import(bytes: &[u8]) -> Result<Imported, Error> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let bad = |what: &str| Error::Malformed(what.to_string());
    let magic = bytes.get(..MAGIC_LEN).ok_or(Error::NotStudio)?;
    if !magic.starts_with(b"silhouette") || magic.last() != Some(&b';') {
        return Err(Error::NotStudio);
    }
    let (version, body) = match magic {
        b"silhouette04;" => (Version::V2, bytes),
        // A bare body (header followed directly by the root record) is read as is.
        b"silhouette05;" if bytes.get(MAGIC_LEN) == Some(&1) => (Version::V3, bytes),
        b"silhouette05;" => (Version::V3, v3_body(bytes)?),
        _ => {
            let version = String::from_utf8_lossy(magic.get(10..12).unwrap_or_default());
            return Err(Error::Unsupported { version: version.into_owned() });
        }
    };
    let mut warnings = Vec::new();
    let root = block(body, MAGIC_LEN).ok_or_else(|| bad("root record is truncated"))?;
    let mut shapes = Vec::new();
    let mut points_total = 0usize;
    let mut cursor = root.start;
    // An object's style record comes before its outline: search from the last outline.
    let mut style_from = root.start;
    let budget = std::cell::Cell::new(MAX_SCANNED_NODES);
    while let Some((at, path)) = find_path(body, cursor, root.end, &budget) {
        cursor = path.end;
        if shapes.len() >= MAX_SHAPES {
            return Err(bad("more than 100,000 shapes"));
        }
        let nodes: usize = path.contours.iter().map(|c| c.nodes.len()).sum();
        points_total = points_total.saturating_add(nodes);
        if points_total > MAX_POINTS {
            return Err(bad("more than 2,000,000 points"));
        }
        let bez = to_bez(&path);
        let limit = f64::from(MAX_COORD_MM);
        let wild = |p: Point| !p.is_finite() || p.x.abs() > limit || p.y.abs() > limit;
        if bez.elements().iter().any(|el| el.end_point().is_some_and(wild)) {
            warnings.push(format!("skipped a path at byte {at}: coordinates out of range"));
            continue;
        }
        shapes.push(Shape { path: bez, line_color: line_color_before(body, style_from, at) });
        style_from = path.end;
    }
    if budget.get() == 0 {
        return Err(bad("too much data that only looks like outlines"));
    }
    Ok(Imported { version, page_mm: page_size(body, &root), shapes, warnings })
}

/// `.studio3`: the header is a table of little-endian u64 section lengths; the
/// sections follow the header back to back and section 0 is the document body.
fn v3_body(bytes: &[u8]) -> Result<&[u8], Error> {
    let bad = |what: &str| Error::Malformed(what.to_string());
    if bytes.len() < V3_HEADER_LEN {
        return Err(bad("header is truncated"));
    }
    let mut total: u64 = 0;
    let mut first: Option<u64> = None;
    let mut at = V3_TABLE_START;
    while at < V3_TABLE_END {
        let len = u64_at(bytes, at).ok_or_else(|| bad("section table is truncated"))?;
        first.get_or_insert(len);
        total = total.checked_add(len).ok_or_else(|| bad("section table overflows"))?;
        at += 8;
    }
    let available = (bytes.len() - V3_HEADER_LEN) as u64;
    if total > available {
        return Err(bad("sections run past the end of the file"));
    }
    let first = usize::try_from(first.unwrap_or(0)).map_err(|_| bad("section 0 is too large"))?;
    let body = bytes.get(V3_HEADER_LEN..V3_HEADER_LEN.saturating_add(first)).ok_or_else(|| bad("section 0 is truncated"))?;
    if body.get(..MAGIC_LEN) != Some(b"silhouette05;".as_slice()) || body.get(MAGIC_LEN) != Some(&1) {
        // The table parsed but the body is not the plain record layout we know:
        // the shape of an encrypted or protected design. Never try to read it.
        return Err(Error::Protected);
    }
    Ok(body)
}

/// A record: marker byte `0x01`, then a u32 length that counts itself, then payload.
/// `start..end` is the payload.
#[derive(Debug, Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
}

fn block(d: &[u8], at: usize) -> Option<Block> {
    if *d.get(at)? != 1 {
        return None;
    }
    let len = usize::try_from(u32_at(d, at + 1)?).ok()?;
    if len < 4 {
        return None;
    }
    let end = (at + 1).checked_add(len)?;
    (end <= d.len()).then_some(Block { start: at + 5, end })
}

/// An object's outline in page coordinates, as every drawable object (shape, line,
/// compound path, text) stores it after its drawing-frame nodes and matrix:
///
/// ```text
/// 01 L  u32 contours-1  contours × contour
/// contour: u32 Lc  u8 closed  u32 nodes-1  nodes × node      (Lc counts itself)
/// node:    f32 x  f32 y  in-handle  out-handle
/// handle:  00 (none) | 01 f32 dx f32 dy (relative to the node)
/// ```
/// Every length and count must agree before a match is accepted.
struct PathRecord {
    contours: Vec<Contour>,
    end: usize,
}

struct Contour {
    closed: bool,
    nodes: Vec<Node>,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    at: Point,
    handle_in: Option<Vec2>,
    handle_out: Option<Vec2>,
}

fn find_path(d: &[u8], from: usize, to: usize, budget: &std::cell::Cell<usize>) -> Option<(usize, PathRecord)> {
    let window = d.get(from..to)?;
    let mut offset = 0;
    while let Some(rel) = window.get(offset..)?.iter().position(|&b| b == 1) {
        let at = from + offset + rel;
        if let Some(path) = path_at(d, at, to, budget) {
            return Some((at, path));
        }
        offset += rel + 1;
    }
    None
}

fn path_at(d: &[u8], at: usize, limit: usize, budget: &std::cell::Cell<usize>) -> Option<PathRecord> {
    let outer = block(d, at)?;
    if outer.end > limit {
        return None;
    }
    let count = usize::try_from(u32_at(d, outer.start)?).ok()?.checked_add(1)?;
    let mut contours = Vec::new();
    let mut p = outer.start + 4;
    // Points one outline may still hold: no record keeps more than `MAX_POINTS` nodes in memory.
    let mut room = MAX_POINTS;
    while p < outer.end {
        if contours.len() >= count {
            return None;
        }
        let (contour, end) = contour_at(d, p, outer.end, room, budget)?;
        room -= contour.nodes.len();
        contours.push(contour);
        p = end;
    }
    (p == outer.end && contours.len() == count).then_some(PathRecord { contours, end: outer.end })
}

fn contour_at(d: &[u8], at: usize, limit: usize, room: usize, budget: &std::cell::Cell<usize>) -> Option<(Contour, usize)> {
    let end = at.checked_add(usize::try_from(u32_at(d, at)?).ok()?)?;
    let closed = match *d.get(at + 4)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let count = usize::try_from(u32_at(d, at + 5)?).ok()?.checked_add(1)?;
    // Each node takes at least 10 bytes: refuse counts the bytes cannot hold.
    if end > limit || count > (end - at) / 10 {
        return None;
    }
    // Checked before allocating. Zero marks the budget spent: every later candidate fails.
    let left = if count > room { None } else { budget.get().checked_sub(count) };
    budget.set(left.unwrap_or(0));
    left?;
    let mut nodes = Vec::with_capacity(count);
    let mut p = at + 9;
    for _ in 0..count {
        let point = point_at(d, p)?;
        p += 8;
        let mut handle = || -> Option<Option<Vec2>> {
            let flag = *d.get(p)?;
            p += 1;
            match flag {
                0 => Some(None),
                1 => {
                    let v = point_at(d, p)?.to_vec2();
                    p += 8;
                    Some(Some(v))
                }
                _ => None,
            }
        };
        let handle_in = handle()?;
        let handle_out = handle()?;
        nodes.push(Node { at: point, handle_in, handle_out });
    }
    (p == end).then_some((Contour { closed, nodes }, end))
}

fn point_at(d: &[u8], at: usize) -> Option<Point> {
    let (x, y) = (f32_at(d, at)?, f32_at(d, at + 4)?);
    let ok = |v: f32| v.is_finite() && v.abs() <= MAX_COORD_MM;
    (ok(x) && ok(y)).then(|| Point::new(f64::from(x), f64::from(y)))
}

/// One subpath per contour. A segment is cubic where either end has a handle,
/// straight otherwise; a closed contour also joins its last node to its first.
fn to_bez(path: &PathRecord) -> BezPath {
    let mut bez = BezPath::new();
    for contour in &path.contours {
        let Some(first) = contour.nodes.first() else {
            continue;
        };
        bez.move_to(first.at);
        let closing = contour.closed.then_some(first);
        let next = contour.nodes.iter().skip(1).chain(closing);
        let last = contour.nodes.len() - 1;
        for (i, (from, to)) in contour.nodes.iter().zip(next).enumerate() {
            match (from.handle_out, to.handle_in) {
                // A straight closing edge is the close itself.
                (None, None) if contour.closed && i == last => {}
                (None, None) => bez.line_to(to.at),
                (out, inn) => bez.curve_to(from.at + out.unwrap_or_default(), to.at + inn.unwrap_or_default(), to.at),
            }
        }
        if contour.closed {
            bez.close_path();
        }
    }
    bez
}

/// The line colour of the last object style in `from..to`. A style holds a cut-action
/// record (`01 L`, then a name string starting "cut": "cut", "cutunion", …); 25 bytes
/// before it sit a flag byte `01` and the line colour as four f32 in B, G, R, A order.
fn line_color_before(d: &[u8], from: usize, to: usize) -> Option<[u8; 3]> {
    let window = d.get(from..to)?;
    let mut found = None;
    let mut offset = 0;
    while let Some(rel) = window.get(offset..)?.windows(3).position(|w| w == b"cut") {
        let name = from + offset + rel;
        offset += rel + 1;
        let Some(record) = name.checked_sub(9) else {
            continue;
        };
        let name_len = u32_at(d, name - 4).and_then(|n| usize::try_from(n).ok());
        let plausible =
            d.get(record) == Some(&1) && name_len.is_some_and(|n| (3..=32).contains(&n)) && record >= from + 25 && d.get(record - 25) == Some(&1);
        if !plausible {
            continue;
        }
        let channel = |i: usize| {
            let v = f32_at(d, record - 24 + 4 * i)?;
            (v.is_finite() && (0.0..=1.0).contains(&v)).then(|| (v * 255.0).round() as u8)
        };
        if let (Some(b), Some(g), Some(r)) = (channel(0), channel(1), channel(2)) {
            found = Some([r, g, b]);
        }
    }
    found
}

/// Page width and height (mm): the root's second record, 0x29 bytes into its payload.
/// Seen in one page size only (12 × 12 in), so it is checked hard and optional.
fn page_size(d: &[u8], root: &Block) -> Option<(f64, f64)> {
    let first = block(d, root.start)?;
    let second = block(d, first.end)?;
    let at = second.start + 0x29;
    if at + 8 > second.end {
        return None;
    }
    let (w, h) = (f32_at(d, at)?, f32_at(d, at + 4)?);
    let sane = |v: f32| v.is_finite() && (1.0..=10_000.0).contains(&v);
    // Stored as f32: round to a micrometre so 304.8 stays 304.8.
    let um = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    (sane(w) && sane(h)).then_some((um(w), um(h)))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

fn f32_at(d: &[u8], at: usize) -> Option<f32> {
    u32_at(d, at).map(f32::from_bits)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn block(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![1];
        out.extend_from_slice(&(payload.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// One object outline in the page-coordinate (compact) layout the reader reads.
    fn path_record(contours: &[(bool, &[(f32, f32)])]) -> Vec<u8> {
        let mut outer = (contours.len() as u32 - 1).to_le_bytes().to_vec();
        for &(closed, points) in contours {
            let mut c = vec![u8::from(closed)];
            c.extend_from_slice(&(points.len() as u32 - 1).to_le_bytes());
            for &(x, y) in points {
                c.extend_from_slice(&x.to_le_bytes());
                c.extend_from_slice(&y.to_le_bytes());
                c.extend_from_slice(&[0, 0]);
            }
            outer.extend_from_slice(&(c.len() as u32 + 4).to_le_bytes());
            outer.extend(c);
        }
        block(&outer)
    }

    /// A v2 body: header, root record holding a filler record, the page record and paths.
    fn v2(page: (f32, f32), paths: &[&[(f32, f32)]]) -> Vec<u8> {
        let records: Vec<_> = paths.iter().map(|p| path_record(&[(true, p)])).collect();
        v2_records(page, &records)
    }

    fn v2_records(page: (f32, f32), records: &[Vec<u8>]) -> Vec<u8> {
        let mut page_payload = vec![0u8; 0x29];
        page_payload.extend_from_slice(&page.0.to_le_bytes());
        page_payload.extend_from_slice(&page.1.to_le_bytes());
        let mut root = block(&[0u8; 8]);
        root.extend(block(&page_payload));
        for r in records {
            root.extend_from_slice(r);
        }
        let mut out = b"silhouette04;".to_vec();
        out.extend(block(&root));
        out
    }

    fn v3(body_v2: &[u8]) -> Vec<u8> {
        let mut body = body_v2.to_vec();
        body[..13].copy_from_slice(b"silhouette05;");
        let thumb = b"not-a-real-thumbnail";
        let mut out = b"silhouette05;".to_vec();
        out.resize(1024, 0);
        out[0x10..0x18].copy_from_slice(&(body.len() as u64).to_le_bytes());
        out[0x18..0x20].copy_from_slice(&(thumb.len() as u64).to_le_bytes());
        out.extend(body);
        out.extend_from_slice(thumb);
        out
    }

    const RECT: &[(f32, f32)] = &[(62.38, 58.21), (62.38, 169.35), (201.3, 169.35), (201.3, 58.21)];

    /// An object style as Silhouette writes it: flag, line colour as B, G, R, A, 8 bytes,
    /// then the cut-action record (`01 L`, name "cut").
    fn style(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = vec![1];
        for c in [rgb[2], rgb[1], rgb[0], 255] {
            out.extend_from_slice(&(f32::from(c) / 255.0).to_le_bytes());
        }
        out.extend_from_slice(&[0; 8]);
        let mut action = 3u32.to_le_bytes().to_vec();
        action.extend_from_slice(b"cut");
        out.extend(block(&action));
        out
    }

    #[test]
    fn v2_rectangle_decodes_in_page_millimetres_y_down() {
        let got = import(&v2((304.8, 304.8), &[RECT])).unwrap();
        assert_eq!(got.version, Version::V2);
        assert_eq!(got.page_mm, Some((304.8, 304.8)));
        let r = kurbo::Shape::bounding_box(&got.shapes[0].path);
        assert!((r.x0 - 62.38).abs() < 1e-3 && (r.y0 - 58.21).abs() < 1e-3, "{r:?}");
        assert!((r.x1 - 201.3).abs() < 1e-3 && (r.y1 - 169.35).abs() < 1e-3, "{r:?}");
        assert_eq!(got.shapes[0].line_color, None);
    }

    #[test]
    fn v3_container_reads_section_zero() {
        let got = import(&v3(&v2((215.9, 279.4), &[RECT, &[(1.0, 1.0), (2.0, 1.0), (2.0, 2.0)]]))).unwrap();
        assert_eq!((got.version, got.shapes.len(), got.page_mm), (Version::V3, 2, Some((215.9, 279.4))));
    }

    #[test]
    fn open_paths_stay_open_and_compound_contours_stay_one_shape() {
        let line = path_record(&[(false, &[(25.4, 25.4), (76.2, 50.8)])]);
        let compound = path_record(&[(true, RECT), (true, &[(1.0, 1.0), (2.0, 1.0), (2.0, 2.0)])]);
        let got = import(&v2_records((304.8, 304.8), &[line, compound])).unwrap();
        let closes = |i: usize| got.shapes[i].path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::ClosePath)).count();
        assert_eq!(got.shapes.len(), 2);
        assert_eq!(closes(0), 0, "a line is open");
        assert_eq!(closes(1), 2, "both contours of the compound path close");
    }

    #[test]
    fn each_outline_takes_the_line_colour_of_the_style_before_it() {
        let ochre = [0xb8, 0x7c, 0x1b];
        let blue = [0, 0, 0xff];
        let mut records = Vec::new();
        for rgb in [ochre, blue] {
            let mut object = style(rgb);
            object.extend(path_record(&[(true, RECT)]));
            records.push(object);
        }
        records.push(path_record(&[(true, RECT)]));
        let got = import(&v2_records((304.8, 304.8), &records)).unwrap();
        let colours: Vec<_> = got.shapes.iter().map(|s| s.line_color).collect();
        assert_eq!(colours, [Some(ochre), Some(blue), None]);
    }

    #[test]
    fn protected_foreign_and_unsupported_files_are_rejected() {
        let mut file = v3(&v2((304.8, 304.8), &[RECT]));
        for b in &mut file[1024..1024 + 40] {
            *b ^= 0x5a;
        }
        assert_eq!(import(&file).unwrap_err(), Error::Protected);
        assert_eq!(import(b"<svg/>").unwrap_err(), Error::NotStudio);
        assert_eq!(import(b"silhouette03;\x01\x04\0\0\0").unwrap_err(), Error::Unsupported { version: "03".to_string() });
        assert_eq!(sniff(b"silhouette05;"), Some(Version::V3));
        assert!(is_studio_path("A.STUDIO3") && !is_studio_path("a.svg"));
    }

    #[test]
    fn a_non_finite_coordinate_is_never_imported() {
        let got = import(&v2((304.8, 304.8), &[&[(f32::NAN, 0.0), (1.0, 1.0)], RECT])).unwrap();
        assert_eq!(got.shapes.len(), 1);
    }

    /// Every 20 bytes starts a candidate outline of 18,432 nodes that parses to its end and
    /// then fails its contour count: without the scan budget this is billions of node reads.
    #[test]
    fn overlapping_near_outlines_are_refused_not_scanned_quadratically() {
        let n = 18_432u32;
        let mut period = vec![1];
        period.extend_from_slice(&(17 + 10 * n).to_le_bytes());
        period.extend_from_slice(&1u32.to_le_bytes());
        period.extend_from_slice(&(9 + 10 * n).to_le_bytes());
        period.push(0);
        period.extend_from_slice(&(n - 1).to_le_bytes());
        period.extend_from_slice(&[0, 0]);
        assert_eq!(period.len(), 20);
        let file = v2_records((304.8, 304.8), &[period.repeat(100_000)]);
        assert!(matches!(import(&file), Err(Error::Malformed(what)) if what.contains("looks like outlines")));
    }

    /// One contour claiming more nodes than `MAX_POINTS`, with the bytes to back the claim.
    #[test]
    fn a_contour_past_the_point_limit_is_refused_before_its_nodes_are_read() {
        let n = MAX_POINTS as u32 + 1;
        let mut outer = 0u32.to_le_bytes().to_vec();
        outer.extend_from_slice(&(9 + 10 * n).to_le_bytes());
        outer.push(1);
        outer.extend_from_slice(&(n - 1).to_le_bytes());
        outer.resize(outer.len() + 10 * n as usize, 0);
        let file = v2_records((304.8, 304.8), &[block(&outer)]);
        assert!(matches!(import(&file), Err(Error::Malformed(_))));
    }

    #[test]
    fn truncations_and_bit_flips_never_panic() {
        let mut state: u64 = 0x5111_0E77;
        for seed in [v2((304.8, 304.8), &[RECT]), v3(&v2((304.8, 304.8), &[RECT, RECT]))] {
            for cut in 0..seed.len() {
                let _ = import(&seed[..cut]);
            }
            for pos in 0..seed.len() {
                let mut mutant = seed.clone();
                mutant[pos] ^= 1 << (pos % 8);
                let _ = import(&mutant);
            }
            for _ in 0..4000 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let mut mutant = seed.clone();
                let pos = (state as usize) % mutant.len();
                mutant[pos] ^= 1 << (state >> 61);
                let _ = import(&mutant);
            }
        }
    }
}
