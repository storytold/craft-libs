//! MULTILEADER and MLEADERSTYLE.
//!
//! A MULTILEADER keeps most of its geometry in a context block (`300 CONTEXT_DATA{` to
//! `301 }`), which holds the leader roots (`302 LEADER{` to `303 }`), each with its leader lines
//! (`304 LEADER_LINE{` to `305 }`). The groups reuse codes at each level, so they are read by
//! level. Values are as the file has them; which of the entity's and the context's copies wins,
//! and how a landing, dogleg and attachment become a drawing, is the caller's.
//!
//! From CADCraft's `cadcraft-io` `dxf_mleader.rs` (5a6aa9b6).

use super::text::decode_contents;
use super::{Point, parse_handle};
use crate::Tag;

/// Most leader roots one multileader keeps.
pub const MAX_ROOTS: usize = 256;
/// Most leader lines one multileader keeps, over all its roots.
pub const MAX_LINES: usize = 4096;
/// Most vertices one multileader keeps, over all its lines.
pub const MAX_VERTICES: usize = 100_000;
/// Most block attribute values one multileader keeps.
pub const MAX_ATTRIBS: usize = 4096;

/// A leader root: where its lines land.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LeaderRoot {
    /// Groups 10/20/30: the landing point.
    pub landing: Option<Point>,
    /// Groups 11/21/31: the dogleg direction.
    pub direction: Option<Point>,
    /// Group 40: the dogleg length.
    pub dogleg: Option<f64>,
    /// Each leader line's vertices, from the arrow towards the landing.
    pub lines: Vec<Vec<Point>>,
}

/// A MULTILEADER.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MLeader {
    // The entity's own groups.
    /// Group 340: the MLEADERSTYLE.
    pub style: Option<u64>,
    /// Group 170: 0 none, 1 straight, 2 spline.
    pub leader_type: Option<i64>,
    /// Group 291: the dogleg is drawn.
    pub dogleg_on: Option<bool>,
    /// Group 41: the dogleg length.
    pub dogleg: Option<f64>,
    /// Group 42: the arrow size.
    pub arrow_size: Option<f64>,
    /// Group 172: the content, 0 none, 1 a block, 2 multiline text, 3 a tolerance.
    pub content_type: Option<i64>,
    /// Group 343: the text's STYLE.
    pub text_style: Option<u64>,
    /// Group 344: the block's BLOCK_RECORD.
    pub block: Option<u64>,
    /// Groups 10/20/30: the block's scale.
    pub block_scale: Option<Point>,
    /// Group 43: the block's rotation, in radians.
    pub block_rotation: Option<f64>,
    /// Groups 173 and 95: how the text's left and right sides attach to the landing.
    pub left_attachment: Option<i64>,
    pub right_attachment: Option<i64>,
    /// The block's attribute values: the ATTDEF (group 330) and the text (group 302).
    pub attribs: Vec<(u64, String)>,
    // The context block.
    /// Groups 10/20/30 of the context: the content base point.
    pub base: Option<Point>,
    /// Group 41 of the context: the text height.
    pub text_height: Option<f64>,
    /// Group 140 of the context: the arrow size.
    pub context_arrow_size: Option<f64>,
    /// Group 145 of the context: the landing gap.
    pub landing_gap: Option<f64>,
    /// Group 290 of the context: there is text.
    pub has_text: Option<bool>,
    /// Group 304 of the context: the text's contents, decoded as an MTEXT's are.
    pub text: Option<String>,
    /// Group 340 of the context: the text's STYLE.
    pub context_text_style: Option<u64>,
    /// Groups 12/22/32 and 13/23/33: the text's location and direction.
    pub text_location: Option<Point>,
    pub text_direction: Option<Point>,
    /// Groups 42, 43 and 45: the text's rotation (radians), width and line spacing factor.
    pub text_rotation: Option<f64>,
    pub text_width: Option<f64>,
    pub line_spacing: Option<f64>,
    /// Group 170 of the context: the line spacing style (2 exact).
    pub line_spacing_style: Option<i64>,
    /// Group 171: the text's alignment, 1 left, 2 centre, 3 right.
    pub text_alignment: Option<i64>,
    /// Groups 174 and 175 of the context: the left and right attachment.
    pub context_left_attachment: Option<i64>,
    pub context_right_attachment: Option<i64>,
    /// Group 296: there is a block.
    pub has_block: Option<bool>,
    /// Group 341 of the context: the block's BLOCK_RECORD.
    pub context_block: Option<u64>,
    /// Groups 15/25/35, 16/26/36 and 46: the block's location, scale and rotation (radians).
    pub block_location: Option<Point>,
    pub context_block_scale: Option<Point>,
    pub context_block_rotation: Option<f64>,
    pub roots: Vec<LeaderRoot>,
}

/// A point read group by group (x, then y and z).
#[derive(Clone, Copy, Default)]
struct P {
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
}

impl P {
    /// Take group `code` when it is coordinate `base`, `base + 10` or `base + 20`.
    fn take(&mut self, base: i32, code: i32, v: f64) {
        match code - base {
            0 => self.x = Some(v),
            10 => self.y = Some(v),
            20 => self.z = Some(v),
            _ => {}
        }
    }
    fn get(&self) -> Option<Point> {
        self.x.map(|x| [x, self.y.unwrap_or(0.0), self.z.unwrap_or(0.0)])
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Entity,
    Context,
    Root,
    Line,
}

fn handle(t: &Tag) -> Option<u64> {
    parse_handle(&t.str()).filter(|h| *h != 0)
}

#[derive(Default)]
struct Points {
    block_scale: P,
    base: P,
    text_location: P,
    text_direction: P,
    block_location: P,
    context_block_scale: P,
    landing: P,
    direction: P,
}

impl MLeader {
    /// The typed form of a MULTILEADER (or MLEADER) record. Its own groups follow the
    /// `AcDbMLeader` marker (or, without one, the last marker). Leader roots, lines, vertices and
    /// attribute values past [`MAX_ROOTS`], [`MAX_LINES`], [`MAX_VERTICES`] and [`MAX_ATTRIBS`]
    /// are left out.
    pub fn read(tags: &[Tag]) -> MLeader {
        let is = |t: &Tag, s: &str| t.str_ref().is_some_and(|v| v.trim_start().starts_with(s));
        let start = tags
            .iter()
            .position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbMLeader"))
            .or_else(|| tags.iter().rposition(|t| t.code == 100))
            .map_or(0, |i| i + 1);
        let mut m = MLeader::default();
        let mut pts = Points::default();
        let mut lvl = Level::Entity;
        // Whether the current root and line are kept (under the caps), and the last vertex.
        let (mut root_kept, mut line_kept, mut vertex_kept) = (false, false, false);
        let (mut lines, mut vertices) = (0usize, 0usize);
        let mut attrib: Option<u64> = None;
        let close_root = |m: &mut MLeader, pts: &mut Points, kept: bool| {
            if let Some(r) = m.roots.last_mut().filter(|_| kept) {
                r.landing = pts.landing.get();
                r.direction = pts.direction.get();
            }
            pts.landing = P::default();
            pts.direction = P::default();
        };
        for t in tags.get(start..).unwrap_or(&[]) {
            let (c, f) = (t.code, t.f64());
            match lvl {
                Level::Entity => match c {
                    300 if is(t, "CONTEXT_DATA") => lvl = Level::Context,
                    340 => m.style = handle(t),
                    170 => m.leader_type = Some(t.i64()),
                    291 => m.dogleg_on = Some(t.i64() != 0),
                    41 => m.dogleg = Some(f),
                    42 => m.arrow_size = Some(f),
                    172 => m.content_type = Some(t.i64()),
                    343 => m.text_style = handle(t),
                    344 => m.block = handle(t),
                    10 | 20 | 30 => pts.block_scale.take(10, c, f),
                    43 => m.block_rotation = Some(f),
                    173 => m.left_attachment = m.left_attachment.or(Some(t.i64())),
                    95 => m.right_attachment = m.right_attachment.or(Some(t.i64())),
                    // Block attribute values: ATTDEF handle (330), index, width, then the text (302).
                    330 => attrib = handle(t),
                    302 => {
                        if let Some(h) = attrib.take()
                            && m.attribs.len() < MAX_ATTRIBS
                        {
                            m.attribs.push((h, t.str()));
                        }
                    }
                    _ => {}
                },
                Level::Context => match c {
                    301 => lvl = Level::Entity,
                    302 if is(t, "LEADER") => {
                        root_kept = m.roots.len() < MAX_ROOTS;
                        if root_kept {
                            m.roots.push(LeaderRoot::default());
                        }
                        lvl = Level::Root;
                    }
                    10 | 20 | 30 => pts.base.take(10, c, f),
                    41 => m.text_height = Some(f),
                    140 => m.context_arrow_size = Some(f),
                    145 => m.landing_gap = Some(f),
                    290 => m.has_text = Some(t.i64() != 0),
                    304 => m.text = Some(decode_contents(&t.str())),
                    340 => m.context_text_style = handle(t),
                    12 | 22 | 32 => pts.text_location.take(12, c, f),
                    13 | 23 | 33 => pts.text_direction.take(13, c, f),
                    42 => m.text_rotation = Some(f),
                    43 => m.text_width = Some(f),
                    45 => m.line_spacing = Some(f),
                    170 => m.line_spacing_style = Some(t.i64()),
                    171 => m.text_alignment = Some(t.i64()),
                    174 => m.context_left_attachment = Some(t.i64()),
                    175 => m.context_right_attachment = Some(t.i64()),
                    296 => m.has_block = Some(t.i64() != 0),
                    341 => m.context_block = handle(t),
                    15 | 25 | 35 => pts.block_location.take(15, c, f),
                    16 | 26 | 36 => pts.context_block_scale.take(16, c, f),
                    46 => m.context_block_rotation = Some(f),
                    _ => {}
                },
                Level::Root => match c {
                    303 | 301 => {
                        close_root(&mut m, &mut pts, root_kept);
                        lvl = if c == 303 { Level::Context } else { Level::Entity };
                    }
                    304 if is(t, "LEADER_LINE") => {
                        line_kept = root_kept && lines < MAX_LINES;
                        if line_kept && let Some(r) = m.roots.last_mut() {
                            r.lines.push(Vec::new());
                            lines += 1;
                        }
                        vertex_kept = false;
                        lvl = Level::Line;
                    }
                    10 | 20 | 30 => pts.landing.take(10, c, f),
                    11 | 21 | 31 => pts.direction.take(11, c, f),
                    40 => {
                        if let Some(r) = m.roots.last_mut().filter(|_| root_kept) {
                            r.dogleg = Some(f);
                        }
                    }
                    _ => {}
                },
                Level::Line => match c {
                    305 => lvl = Level::Root,
                    303 | 301 => {
                        close_root(&mut m, &mut pts, root_kept);
                        lvl = if c == 303 { Level::Context } else { Level::Entity };
                    }
                    10 => {
                        vertex_kept = line_kept && vertices < MAX_VERTICES;
                        if vertex_kept && let Some(l) = m.roots.last_mut().and_then(|r| r.lines.last_mut()) {
                            l.push([f, 0.0, 0.0]);
                            vertices += 1;
                        }
                    }
                    20 | 30 if vertex_kept => {
                        if let Some(v) = m.roots.last_mut().and_then(|r| r.lines.last_mut()).and_then(|l| l.last_mut()) {
                            v[if c == 20 { 1 } else { 2 }] = f;
                        }
                    }
                    _ => {}
                },
            }
        }
        // A file that ends inside a root still has its landing.
        if matches!(lvl, Level::Root | Level::Line) {
            close_root(&mut m, &mut pts, root_kept);
        }
        m.block_scale = pts.block_scale.get();
        m.base = pts.base.get();
        m.text_location = pts.text_location.get();
        m.text_direction = pts.text_direction.get();
        m.block_location = pts.block_location.get();
        m.context_block_scale = pts.context_block_scale.get();
        m
    }
}

/// An MLEADERSTYLE.
#[derive(Clone, Debug, PartialEq)]
pub struct MLeaderStyle {
    pub handle: Option<u64>,
    /// Group 3.
    pub description: String,
    /// Group 170: the content type (as a MULTILEADER's 172).
    pub content_type: i64,
    /// Group 173: the leader type (as a MULTILEADER's 170).
    pub leader_type: i64,
    /// Group 42: the landing gap.
    pub landing_gap: f64,
    /// Group 43: the dogleg length.
    pub dogleg: f64,
    /// Group 44: the arrow size.
    pub arrow_size: f64,
    /// Group 45: the text height.
    pub text_height: f64,
    /// Group 342: the text's STYLE.
    pub text_style: Option<u64>,
    /// Group 343: the block's BLOCK_RECORD.
    pub block: Option<u64>,
    /// Group 296 is not 0.
    pub annotative: bool,
}

impl MLeaderStyle {
    pub fn read(r: &super::Record<'_>) -> MLeaderStyle {
        let g = r.groups();
        let own = g.subclass("AcDbMLeaderStyle");
        let g = if own.0.is_empty() { g } else { own };
        MLeaderStyle {
            handle: r.handle(),
            description: g.s(3).unwrap_or_default(),
            content_type: g.i_or(170, 2),
            leader_type: g.i_or(173, 1),
            landing_gap: g.f_or(42, 0.0),
            dogleg: g.f_or(43, 0.0),
            arrow_size: g.f_or(44, 0.0),
            text_height: g.f_or(45, 0.0),
            text_style: g.handle(342).filter(|h| *h != 0),
            block: g.handle(343).filter(|h| *h != 0),
            annotative: g.i_or(296, 0) != 0,
        }
    }
}
