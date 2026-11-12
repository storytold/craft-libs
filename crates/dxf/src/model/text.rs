//! Text in DXF: the caret encoding of control characters in strings, and MTEXT with its columns.
//!
//! Control characters are written as a caret and a letter (`^I` a tab, `^J` a line feed, `^ ` a
//! caret). TEXT, ATTDEF, ATTRIB and DIMENSION strings use the same encoding as MTEXT contents,
//! without line feeds: AutoCAD reads a bare `^2` in `10^2` as a control character.
//!
//! MTEXT columns are groups 75 (type), 76 (count), 78 (flow reversed), 79 (auto height), 48
//! (width), 49 (gutter) and 50 (column heights, after the column type; before it, 50 is the
//! rotation) before R2018, and an embedded object (group 101) from R2018 on.

use super::{Groups, Point, outside_app};
use crate::Tag;

/// Decode MTEXT contents: `^I` (tab), `^J` (line feed) and `^ ` (caret). Other carets stay as
/// they are (older files write stacked tolerances `\S+0.1^-0.1;` with a bare caret).
pub fn decode_contents(s: &str) -> String {
    decode(s, true)
}

/// A single-line string as read: [`decode_contents`] without line feeds (`^J` stays as it is).
pub fn decode_line(s: &str) -> String {
    decode(s, false)
}

fn decode(s: &str, line_feeds: bool) -> String {
    if !s.contains('^') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '^' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some(' ') => out.push('^'),
            Some('I') => out.push('\t'),
            Some('J') if line_feeds => out.push('\n'),
            _ => {
                out.push('^');
                continue;
            }
        }
        it.next();
    }
    out
}

/// A single-line string (TEXT, ATTDEF/ATTRIB, DIMENSION) for a DXF file: a caret as `^ ` and a
/// tab as `^I`, read back by [`decode_line`]. It must hold no line break.
pub fn encode_line(s: &str) -> String {
    s.replace('^', "^ ").replace('\t', "^I")
}

/// Encode MTEXT contents: a tab as `^I`, a line feed as `^J`, a caret as `^ `; other control
/// characters (which draw nothing) are dropped.
pub fn encode_contents(s: &str) -> String {
    if !s.chars().any(|c| c == '^' || c.is_control()) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '^' => out.push_str("^ "),
            '\t' => out.push_str("^I"),
            '\n' => out.push_str("^J"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Apply `f` ([`encode_line`] or [`decode_line`]) to the groups of a `kind` record that hold an
/// encoded string: the text (1), and an ATTDEF's prompt (3). An embedded MTEXT (from group 101)
/// has its own and is left alone.
pub fn map_text_groups(kind: &str, tags: &mut [Tag], f: fn(&str) -> String) {
    let kind = kind.to_ascii_uppercase();
    if !matches!(kind.as_str(), "TEXT" | "ATTRIB" | "ATTDEF" | "DIMENSION") {
        return;
    }
    for t in tags.iter_mut().take_while(|t| t.code != 101) {
        if t.code == 1 || (t.code == 3 && kind == "ATTDEF") {
            *t = Tag::s(t.code, f(&t.str()));
        }
    }
}

/// A record's groups split at its embedded object (`101 Embedded Object`): its own groups and,
/// when it has one, the object's.
pub fn split_embedded(tags: &[Tag]) -> (&[Tag], Option<&[Tag]>) {
    match tags.iter().position(|t| t.code == 101) {
        Some(i) => (tags.get(..i).unwrap_or(tags), tags.get(i + 1..)),
        None => (tags, None),
    }
}

/// Upper bound on the column heights read from one MTEXT.
pub const MAX_COLUMNS: usize = 1_000;

/// The columns of an MTEXT.
#[derive(Clone, Debug, PartialEq)]
pub struct MTextColumns {
    /// Dynamic columns (type 2): the text flows into as many as it needs. Otherwise static
    /// (type 1).
    pub dynamic: bool,
    pub count: u16,
    /// One column's width (the MTEXT's own width is the total in some writers' files).
    pub width: f64,
    pub gutter: f64,
    /// The column height: the tallest for manual heights.
    pub height: f64,
    pub auto_height: bool,
    /// Each column's height, for dynamic columns with manual heights; else empty.
    pub heights: Vec<f64>,
    pub flow_reversed: bool,
}

fn pos(v: f64) -> f64 {
    v.max(0.0)
}

impl MTextColumns {
    /// The columns of an MTEXT record, from the legacy groups or the embedded object.
    pub fn read(tags: &[Tag]) -> Option<MTextColumns> {
        let (own, embedded) = split_embedded(tags);
        // An application-defined group in the record may hold any code, 50 included.
        let own_tags: Vec<&Tag> = outside_app(own).map(|(_, t)| t).collect();
        let Some(start) = own_tags.iter().position(|t| t.code == 75) else { return Self::read_embedded(own, embedded?) };
        let dynamic = match own_tags.get(start).map(|t| t.i64()) {
            Some(1) => false,
            Some(2) => true,
            _ => return None,
        };
        let cols = own_tags.get(start..).unwrap_or_default();
        let first = |code: i32| cols.iter().find(|t| t.code == code).copied();
        // The first 50 is the number of heights that follow, unless the file has the heights
        // of group 76 alone.
        let fifties = cols.iter().filter(|t| t.code == 50);
        let counted = first(76).is_some_and(|t| i64::try_from(fifties.clone().count()).is_ok_and(|n| n - 1 == t.i64()));
        let heights: Vec<f64> = fifties.skip(usize::from(counted)).take(MAX_COLUMNS).map(|t| pos(t.f64())).collect();
        let count = first(76).map_or(heights.len().max(1), |t| usize::try_from(t.i64()).unwrap_or(1)).clamp(1, MAX_COLUMNS);
        let auto_height = first(79).is_some_and(|t| t.i64() != 0);
        let defined = own_tags.iter().find(|t| t.code == 46).map_or(0.0, |t| pos(t.f64()));
        let manual = dynamic && !auto_height;
        let height =
            if manual { heights.iter().copied().fold(0.0, f64::max) } else { heights.first().copied().filter(|h| *h > 0.0).unwrap_or(defined) };
        Some(MTextColumns {
            dynamic,
            count: u16::try_from(count).unwrap_or(1),
            width: first(48).map_or(0.0, |t| pos(t.f64())),
            gutter: first(49).map_or(0.0, |t| pos(t.f64())),
            height,
            auto_height,
            heights: if manual { heights } else { Vec::new() },
            flow_reversed: first(78).is_some_and(|t| t.i64() != 0),
        })
    }

    /// The columns of an R2018 MTEXT, from its embedded object, whose groups reuse MTEXT's
    /// codes: 71 type, 72 count, 44 column width, 45 gutter, 73 auto height, 74 flow reversed,
    /// 46 heights.
    #[expect(clippy::cast_possible_truncation, reason = "the rounded count is clamped to 1..=MAX_COLUMNS before use")]
    fn read_embedded(own: &[Tag], obj: &[Tag]) -> Option<MTextColumns> {
        let first = |code: i32| obj.iter().find(|t| t.code == code);
        let dynamic = match first(71).map(Tag::i64) {
            Some(1) => false,
            Some(2) => true,
            _ => return None,
        };
        let heights: Vec<f64> = obj.iter().filter(|t| t.code == 46).take(MAX_COLUMNS).map(|t| pos(t.f64())).collect();
        let auto_height = first(73).is_some_and(|t| t.i64() != 0);
        let manual = dynamic && !auto_height;
        // The column height: the entity's own 46 (defined height), else the object's 41 or first 46.
        let own_46 = own.iter().find(|t| t.code == 46).map(|t| pos(t.f64()));
        let defined = [own_46, first(41).map(Tag::f64), heights.first().copied()].into_iter().flatten().find(|h| *h > 0.0).unwrap_or(0.0);
        // A dynamic auto-height object stores 0 columns in 72: the count follows from the total
        // width (42 = n * 44 + (n - 1) * 45).
        let [width, gutter, total] = [44, 45, 42].map(|g| first(g).map_or(0.0, Tag::f64));
        let count = match first(72).map(Tag::i64) {
            Some(n) if n > 0 => n,
            Some(_) if width + gutter > 0.0 && total > 0.0 => ((total + gutter) / (width + gutter)).round().clamp(1.0, MAX_COLUMNS as f64) as i64,
            _ => 1,
        };
        Some(MTextColumns {
            dynamic,
            count: u16::try_from(count.clamp(1, MAX_COLUMNS as i64)).unwrap_or(1),
            width: pos(width),
            gutter: pos(gutter),
            height: if manual { heights.iter().copied().fold(0.0, f64::max) } else { defined },
            auto_height,
            heights: if manual { heights } else { Vec::new() },
            flow_reversed: first(74).is_some_and(|t| t.i64() != 0),
        })
    }
}

/// Multiline text: an MTEXT entity, or the one embedded in a multiline attribute.
#[derive(Clone, Debug, PartialEq)]
pub struct MText {
    /// Group 10.
    pub insert: Point,
    /// Group 40: the text height.
    pub height: f64,
    /// Group 41: the reference rectangle's width (0: none). With columns, see
    /// [`MTextColumns::width`].
    pub width: f64,
    /// Group 71: 1–9, top left to bottom right.
    pub attach: u8,
    /// Group 72: 1 left to right, 3 top to bottom, 5 by style.
    pub drawing_direction: i64,
    /// Groups 11/21/31: the X axis direction, which wins over the rotation when given and not
    /// zero.
    pub direction: Option<Point>,
    /// Group 50, in degrees.
    pub rotation: f64,
    /// Group 7 (`Standard` when absent).
    pub style: String,
    /// The contents (the group 3 chunks, then group 1), decoded ([`decode_contents`]); the
    /// formatting codes (`\P`, `\f...;`) are left as written.
    pub contents: String,
    /// Group 44: the line spacing factor (1 when absent).
    pub line_spacing: f64,
    /// Group 73 is 2: the spacing is exact, not "at least".
    pub line_spacing_exact: bool,
    /// Group 90: the background fill flags (1 a fill colour, 2 the window's colour, 16 a
    /// frame), with the fill colour (63, 421) and the border offset factor (45).
    pub background: Option<(i64, super::Color, f64)>,
    pub columns: Option<MTextColumns>,
}

impl MText {
    /// The typed form of MTEXT groups.
    pub fn read(tags: &[Tag]) -> MText {
        let columns = MTextColumns::read(tags);
        // The MTEXT's own groups: an embedded object reuses their codes (44 is its column
        // width, not the line spacing factor).
        let (own, _) = split_embedded(tags);
        let g = Groups(own);
        let mut contents = String::new();
        for t in own.iter().filter(|t| t.code == 3) {
            contents.push_str(&t.str());
        }
        contents.push_str(&g.s(1).unwrap_or_default());
        // The direction and rotation come from before the column groups, where 50 is a height.
        let head = Groups(own.get(..own.iter().position(|t| t.code == 75).unwrap_or(own.len())).unwrap_or(own));
        let background = g.i(90).filter(|f| *f != 0).map(|f| {
            let color = match (g.i(421), g.i(63)) {
                (Some(tc), _) => super::Color::from_true(tc),
                (None, Some(v)) => super::Color::from_index(v),
                (None, None) => super::Color::ByLayer,
            };
            (f, color, g.f_or(45, 1.5))
        });
        MText {
            insert: g.point(10),
            height: g.f_or(40, 0.0),
            width: pos(g.f_or(41, 0.0)),
            attach: u8::try_from(g.i_or(71, 1).clamp(1, 9)).unwrap_or(1),
            drawing_direction: g.i_or(72, 1),
            direction: head.point_opt(11).filter(|d| d[0] != 0.0 || d[1] != 0.0 || d[2] != 0.0),
            rotation: head.f_or(50, 0.0),
            style: g.name(7).unwrap_or_else(|| "Standard".into()),
            contents: decode_contents(&contents),
            line_spacing: g.f_or(44, 1.0),
            line_spacing_exact: g.i(73) == Some(2),
            background,
            columns,
        }
    }
    /// The angle of the text's X axis in radians: of the direction when the record has one,
    /// else the rotation.
    pub fn angle(&self) -> f64 {
        match self.direction {
            Some(d) if d[0] != 0.0 || d[1] != 0.0 => d[1].atan2(d[0]),
            _ => self.rotation.to_radians(),
        }
    }
}
