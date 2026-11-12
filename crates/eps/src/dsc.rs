//! The header comments of a PostScript file (Document Structuring Conventions 3.0).

/// Bytes of a file's start read for its header comments.
pub const MAX_HEADER: usize = 64 << 10;
/// Bytes of a file's end read for comments its header defers there with `(atend)`.
pub const MAX_TRAILER: usize = 64 << 10;

/// What the header comments of a PostScript program say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comments {
    /// `%%BoundingBox`: lower left x and y, upper right x and y, in points.
    pub bounding_box: Option<[f64; 4]>,
    /// `%%HiResBoundingBox`, the same box with fractions.
    pub hires_bounding_box: Option<[f64; 4]>,
    /// `%%Pages`.
    pub pages: Option<u32>,
    /// `%%LanguageLevel`.
    pub language_level: Option<u32>,
    /// `%%Creator`, without the parentheses some writers put around it.
    pub creator: Option<String>,
    /// `%%Title`, likewise.
    pub title: Option<String>,
    /// In the Illustrator format: the header has `%AI…` or `%%AI…` comments, or names Illustrator
    /// as its creator.
    pub illustrator: bool,
}

/// Comments that end the header, besides the first line that is no comment.
const END_OF_HEADER: [&str; 8] =
    ["%%EndComments", "%%BeginDocument", "%%BeginProlog", "%%BeginSetup", "%%BeginData", "%%BeginBinary", "%%BeginResource", "%%Page:"];

impl Comments {
    /// The header comments of `ps` (the PostScript section of a file, see [`crate::sections`]).
    /// A bounding box the header defers with `(atend)` is the last one of the trailer, outside
    /// any document the file embeds.
    pub fn read(ps: &[u8]) -> Self {
        let head = String::from_utf8_lossy(ps.get(..MAX_HEADER).unwrap_or(ps));
        let mut c = Comments::default();
        let (mut box_atend, mut hires_atend) = (false, false);
        // Lines may end in `\r` alone (classic Mac files).
        for line in head.split(['\n', '\r']) {
            if !(line.starts_with('%') || line.trim().is_empty()) || END_OF_HEADER.iter().any(|k| line.starts_with(k)) {
                break;
            }
            let value = |key: &str| line.strip_prefix(key).map(str::trim);
            if line.starts_with("%AI") || line.starts_with("%%AI") {
                c.illustrator = true;
            }
            // The first of a comment given twice counts (DSC 3.0, section 4.6).
            if let Some(v) = value("%%HiResBoundingBox:") {
                hires_atend |= v.starts_with("(atend)");
                c.hires_bounding_box = c.hires_bounding_box.or_else(|| parse_box(v));
            } else if let Some(v) = value("%%BoundingBox:") {
                box_atend |= v.starts_with("(atend)");
                c.bounding_box = c.bounding_box.or_else(|| parse_box(v));
            } else if let Some(v) = value("%%Pages:") {
                c.pages = c.pages.or_else(|| v.split_whitespace().next().and_then(|n| n.parse().ok()));
            } else if let Some(v) = value("%%LanguageLevel:") {
                c.language_level = c.language_level.or_else(|| v.split_whitespace().next().and_then(|n| n.parse().ok()));
            } else if let Some(v) = value("%%Creator:") {
                c.illustrator |= v.contains("Illustrator");
                c.creator = c.creator.or_else(|| Some(text(v)));
            } else if let Some(v) = value("%%Title:") {
                c.title = c.title.or_else(|| Some(text(v)));
            }
        }
        // An outer header box is never replaced by the trailer comments of an embedded document:
        // the trailer is read only for what the outer header defers there.
        if box_atend || hires_atend {
            let tail = String::from_utf8_lossy(ps.get(ps.len().saturating_sub(MAX_TRAILER)..).unwrap_or(ps));
            if hires_atend {
                c.hires_bounding_box = c.hires_bounding_box.or_else(|| trailer_box(&tail, "%%HiResBoundingBox:"));
            }
            if box_atend {
                c.bounding_box = c.bounding_box.or_else(|| trailer_box(&tail, "%%BoundingBox:"));
            }
        }
        c
    }

    /// The box the file draws in: the high-resolution one when it has one.
    pub fn page_box(&self) -> Option<[f64; 4]> {
        self.hires_bounding_box.or(self.bounding_box)
    }
}

/// A text value, without the parentheses of a PostScript string around it.
fn text(v: &str) -> String {
    v.strip_prefix('(').and_then(|v| v.strip_suffix(')')).unwrap_or(v).to_string()
}

/// The four numbers of a bounding box comment, when they are four finite numbers that make a box
/// of a finite size (subtracting extreme corners can overflow to infinity).
fn parse_box(value: &str) -> Option<[f64; 4]> {
    let mut words = value.split_whitespace();
    let mut bb = [0.0f64; 4];
    for n in &mut bb {
        *n = words.next()?.parse().ok()?;
    }
    let [x0, y0, x1, y1] = bb;
    let finite = [x0, y0, x1, y1, x1 - x0, y1 - y0].iter().all(|n| n.is_finite());
    (words.next().is_none() && finite && x1 > x0 && y1 > y0).then_some(bb)
}

/// The last `key` box of the trailer that is the outer document's own: the lines are walked
/// backwards from the end of the file, so the nesting is known even when the window starts inside
/// an embedded document. Data and resource blocks are skipped the same way.
fn trailer_box(tail: &str, key: &str) -> Option<[f64; 4]> {
    let mut nested = 0usize;
    for line in tail.split(['\n', '\r']).rev() {
        if ["%%EndDocument", "%%EndData", "%%EndBinary", "%%EndResource"].iter().any(|k| line.starts_with(k)) {
            nested = nested.checked_add(1)?;
        } else if ["%%BeginDocument", "%%BeginData", "%%BeginBinary", "%%BeginResource"].iter().any(|k| line.starts_with(k)) {
            nested = nested.checked_sub(1)?;
        } else if nested == 0
            && let Some(bb) = line.strip_prefix(key).and_then(parse_box)
        {
            return Some(bb);
        }
    }
    None
}
