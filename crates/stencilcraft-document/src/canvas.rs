//! Canvas model beyond shapes: pages (artboards), page presets, ruler guides, grid and
//! snap settings, document metadata and groups. Everything here is plain data stored in
//! the `.stencilcraft` file; behaviour lives in the engine.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Board, DocError, Document, Shape};

/// Most pages a document may hold (the main board included).
pub const MAX_PAGES: usize = 100;
/// Most ruler guides a document may hold.
pub const MAX_GUIDES: usize = 1000;
/// Most groups a document may hold.
pub const MAX_GROUPS: usize = 100_000;
/// Deepest group nesting accepted.
pub const MAX_GROUP_DEPTH: usize = 64;
/// Longest title, page/layer/group name, colour or mat label, in bytes.
pub const MAX_NAME_BYTES: usize = 1024;
/// Longest notes text, in bytes.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;
/// Largest stored thumbnail, in bytes.
pub const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;

const INCH_MM: f64 = 25.4;

/// Labels that describe a page beyond its size: the mat it sits on, the media loaded and
/// the page colour shown on the canvas. All optional; empty means "not set".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PageSetup {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Page colour, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Cut mirrored (heat-transfer material is cut face down).
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub mirror: bool,
    /// The page is printed with registration marks for Print & Cut.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub registration_marks: bool,
}

impl PageSetup {
    pub fn is_empty(&self) -> bool {
        self == &PageSetup::default()
    }
}

/// An extra page (artboard) placed on the document plane. The main board is page 0 at the
/// origin; extra pages sit beside it, and a shape belongs to the page under its centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    #[serde(flatten)]
    pub setup: PageSetup,
}

impl Page {
    pub fn rect(&self) -> kurbo::Rect {
        kurbo::Rect::new(self.x_mm, self.y_mm, self.x_mm + self.width_mm, self.y_mm + self.height_mm)
    }
}

/// Common page and mat sizes. Sizes are portrait (width ≤ height); see [`Orientation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PagePreset {
    #[serde(rename = "letter")]
    Letter,
    #[serde(rename = "a4")]
    A4,
    #[serde(rename = "mat-12x12")]
    Mat12x12,
    #[serde(rename = "mat-12x24")]
    Mat12x24,
    #[serde(rename = "joy-mat-4.5x6.5")]
    JoyMat4_5x6_5,
    #[serde(rename = "joy-mat-4.5x12")]
    JoyMat4_5x12,
    #[serde(rename = "joy-xtra-mat-8.5x12")]
    JoyXtraMat8_5x12,
    /// Width and height are given explicitly.
    #[serde(rename = "custom")]
    Custom,
}

impl PagePreset {
    /// Portrait size in millimetres; `None` for `Custom`.
    pub fn size_mm(self) -> Option<(f64, f64)> {
        let inches = |w: f64, h: f64| Some((w * INCH_MM, h * INCH_MM));
        match self {
            PagePreset::Letter => inches(8.5, 11.0),
            PagePreset::A4 => Some((210.0, 297.0)),
            PagePreset::Mat12x12 => inches(12.0, 12.0),
            PagePreset::Mat12x24 => inches(12.0, 24.0),
            PagePreset::JoyMat4_5x6_5 => inches(4.5, 6.5),
            PagePreset::JoyMat4_5x12 => inches(4.5, 12.0),
            PagePreset::JoyXtraMat8_5x12 => inches(8.5, 12.0),
            PagePreset::Custom => None,
        }
    }

    /// The mat label a mat preset implies (`None` for paper sizes and custom).
    pub fn mat_label(self) -> Option<&'static str> {
        match self {
            PagePreset::Mat12x12 => Some("12x12"),
            PagePreset::Mat12x24 => Some("12x24"),
            PagePreset::JoyMat4_5x6_5 => Some("joy-4.5x6.5"),
            PagePreset::JoyMat4_5x12 => Some("joy-4.5x12"),
            PagePreset::JoyXtraMat8_5x12 => Some("joy-xtra-8.5x12"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    /// `(w, h)` ordered for this orientation.
    pub fn apply(self, (w, h): (f64, f64)) -> (f64, f64) {
        let (short, long) = if w <= h { (w, h) } else { (h, w) };
        match self {
            Orientation::Portrait => (short, long),
            Orientation::Landscape => (long, short),
        }
    }
}

/// Which way a ruler guide runs. A horizontal guide is the line `y = position`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuideAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: GuideAxis,
    pub position_mm: f64,
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub locked: bool,
}

/// The background grid: major lines every `spacing_mm`, split into `subdivisions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub spacing_mm: f64,
    pub subdivisions: u32,
    #[serde(default)]
    pub visible: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Self { spacing_mm: 10.0, subdivisions: 1, visible: false }
    }
}

impl Grid {
    /// Distance between snap lines: spacing over subdivisions.
    pub fn step_mm(&self) -> f64 {
        self.spacing_mm / f64::from(self.subdivisions.max(1))
    }
}

/// Which targets the snapping service considers, and how close counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapSettings {
    #[serde(default)]
    pub grid: bool,
    #[serde(default = "crate::yes")]
    pub guides: bool,
    /// Edges and centres of other shapes and of pages.
    #[serde(default = "crate::yes")]
    pub objects: bool,
    /// Smart guides: equal spacing between neighbours.
    #[serde(default = "crate::yes")]
    pub spacing: bool,
    /// Snap to point: shapes' anchor points (Illustrator's View › Snap to Point).
    #[serde(default)]
    pub points: bool,
    #[serde(default = "default_tolerance")]
    pub tolerance_mm: f64,
}

fn default_tolerance() -> f64 {
    2.0
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self { grid: false, guides: true, objects: true, spacing: true, points: false, tolerance_mm: default_tolerance() }
    }
}

/// Title, notes and a small preview, stored in the file for file browsers and recovery.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// A small standalone SVG of the design (see `Document::render_thumbnail`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_svg: Option<String>,
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        self == &Metadata::default()
    }
}

/// A member of a group: a shape or another group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupChild {
    Shape(u64),
    Group(u64),
}

/// Shapes and groups that move and select together. Grouping never changes how shapes
/// cut, nor their stacking order, which stays with their layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub children: Vec<GroupChild>,
}

/// `#rrggbb` (or `#rgb`) to RGB; `None` for anything else.
pub fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
    match hex.len() {
        6 => {
            let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        }
        3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17]),
        _ => None,
    }
}

/// Canonical lowercase `#rrggbb`.
pub fn format_hex_color([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Euclidean RGB distance (0 to about 441.7).
pub fn color_distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(y)).powi(2)).sum::<f64>().sqrt()
}

fn check_name(text: &str, what: &str) -> Result<(), DocError> {
    if text.len() > MAX_NAME_BYTES {
        return Err(DocError::Model(format!("{what}: longer than {MAX_NAME_BYTES} bytes")));
    }
    Ok(())
}

fn check_color(color: Option<&str>, what: &str) -> Result<(), DocError> {
    match color {
        Some(c) if parse_hex_color(c).is_none() => {
            Err(DocError::Model(format!("{what}: `{}` is not a #rrggbb colour", c.chars().take(16).collect::<String>())))
        }
        _ => Ok(()),
    }
}

fn check_setup(setup: &PageSetup, what: &str) -> Result<(), DocError> {
    check_name(&setup.name, what)?;
    for label in [&setup.mat, &setup.media].into_iter().flatten() {
        check_name(label, what)?;
    }
    check_color(setup.color.as_deref(), what)
}

impl Document {
    /// Pages, guides, grid, snapping, colours, metadata and groups: finite, within caps,
    /// and groups form a forest (unknown shape members are allowed: they are pruned).
    pub(crate) fn validate_canvas(&self) -> Result<(), DocError> {
        let bad = |message: String| Err(DocError::Model(message));
        if self.pages.len() >= MAX_PAGES {
            return bad(format!("pages: more than {MAX_PAGES}"));
        }
        check_setup(&self.board_setup, "board")?;
        for (i, page) in self.pages.iter().enumerate() {
            let finite = [page.x_mm, page.y_mm, page.width_mm, page.height_mm].iter().all(|v| v.is_finite() && v.abs() < 1e7);
            if !finite || page.width_mm <= 0.0 || page.height_mm <= 0.0 {
                return bad(format!("page {}: position and size must be finite, size positive", i + 1));
            }
            check_setup(&page.setup, "page")?;
        }
        if self.guides.len() > MAX_GUIDES {
            return bad(format!("guides: more than {MAX_GUIDES}"));
        }
        if self.guides.iter().any(|g| !g.position_mm.is_finite()) {
            return bad("guides: positions must be finite".to_string());
        }
        if !(self.grid.spacing_mm.is_finite() && (0.01..=10_000.0).contains(&self.grid.spacing_mm) && (1..=100).contains(&self.grid.subdivisions)) {
            return bad("grid: spacing must be 0.01-10000 mm, subdivisions 1-100".to_string());
        }
        if !(0.0..=1000.0).contains(&self.snap.tolerance_mm) {
            return bad("snap: tolerance must be 0-1000 mm".to_string());
        }
        for layer in &self.layers {
            check_name(&layer.name, "layer name")?;
            check_color(layer.color.as_deref(), "layer colour")?;
        }
        check_name(&self.meta.title, "title")?;
        if self.meta.notes.len() > MAX_NOTES_BYTES {
            return bad(format!("notes: longer than {MAX_NOTES_BYTES} bytes"));
        }
        if self.meta.thumbnail_svg.as_ref().is_some_and(|t| t.len() > MAX_THUMBNAIL_BYTES) {
            return bad(format!("thumbnail: larger than {MAX_THUMBNAIL_BYTES} bytes"));
        }
        self.validate_groups()
    }

    fn validate_groups(&self) -> Result<(), DocError> {
        let bad = |message: String| Err(DocError::Model(message));
        if self.groups.len() > MAX_GROUPS {
            return bad(format!("groups: more than {MAX_GROUPS}"));
        }
        let ids: BTreeSet<u64> = self.groups.iter().map(|g| g.id).collect();
        if ids.len() != self.groups.len() {
            return bad("groups: ids must be unique".to_string());
        }
        let mut parents: BTreeMap<GroupChild, u64> = BTreeMap::new();
        for group in &self.groups {
            check_name(&group.name, "group name")?;
            for child in &group.children {
                if let GroupChild::Group(id) = child
                    && !ids.contains(id)
                {
                    return bad(format!("group {}: member group {id} does not exist", group.id));
                }
                if parents.insert(*child, group.id).is_some() {
                    return bad(format!("groups: {child:?} belongs to two groups"));
                }
            }
        }
        // With one parent each, a cycle is a walk up that never reaches a root.
        for id in &ids {
            let mut current = GroupChild::Group(*id);
            let mut depth = 0;
            while let Some(parent) = parents.get(&current) {
                depth += 1;
                if depth > MAX_GROUP_DEPTH {
                    return bad(format!("groups: nesting deeper than {MAX_GROUP_DEPTH}, or a cycle, at group {id}"));
                }
                current = GroupChild::Group(*parent);
            }
        }
        Ok(())
    }

    /// Whether shape `id` may not be edited: the shape or its layer is locked.
    pub fn shape_locked(&self, id: u64) -> bool {
        self.layers.iter().any(|layer| layer.shapes.iter().any(|s| s.id == id && (s.locked || layer.locked)))
    }

    /// Index of the layer holding shape `id`.
    pub fn layer_of_shape(&self, id: u64) -> Option<usize> {
        self.layers.iter().position(|layer| layer.shapes.iter().any(|s| s.id == id))
    }

    pub fn group(&self, id: u64) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The group directly holding `child`.
    pub fn group_parent(&self, child: GroupChild) -> Option<u64> {
        self.groups.iter().find(|g| g.children.contains(&child)).map(|g| g.id)
    }

    /// Each group member's direct parent group; build once for many [`group_chain_in`] calls.
    pub fn group_parent_map(&self) -> BTreeMap<GroupChild, u64> {
        self.groups.iter().flat_map(|g| g.children.iter().map(move |c| (*c, g.id))).collect()
    }

    /// Groups holding `child`, outermost first.
    pub fn group_chain(&self, child: GroupChild) -> Vec<u64> {
        group_chain_in(&self.group_parent_map(), child)
    }

    /// Every existing shape inside group `id`, nested groups included, in document order.
    pub fn group_shapes(&self, id: u64) -> Vec<u64> {
        let mut members = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(gid) = stack.pop() {
            if !seen.insert(gid) {
                continue;
            }
            for child in self.group(gid).map_or(&[][..], |g| &g.children) {
                match child {
                    GroupChild::Shape(s) => {
                        members.insert(*s);
                    }
                    GroupChild::Group(g) => stack.push(*g),
                }
            }
        }
        self.shapes().map(|(_, s)| s.id).filter(|s| members.contains(s)).collect()
    }

    /// The groups without members that no longer exist; groups left empty go too.
    pub fn pruned_groups(&self) -> Vec<Group> {
        let shapes: BTreeSet<u64> = self.shapes().map(|(_, s)| s.id).collect();
        let mut groups = self.groups.clone();
        loop {
            let before: usize = groups.iter().map(|g| g.children.len() + 1).sum();
            let live: BTreeSet<u64> = groups.iter().map(|g| g.id).collect();
            for group in &mut groups {
                group.children.retain(|child| match child {
                    GroupChild::Shape(id) => shapes.contains(id),
                    GroupChild::Group(id) => live.contains(id),
                });
            }
            groups.retain(|g| !g.children.is_empty());
            let after: usize = groups.iter().map(|g| g.children.len() + 1).sum();
            if after == before {
                return groups;
            }
        }
    }

    /// A free group id.
    pub fn next_group_id(&self) -> u64 {
        self.groups.iter().map(|g| g.id).max().map_or(1, |m| m.saturating_add(1))
    }

    /// All pages: the main board (index 0, at the origin) then the extra pages.
    pub fn all_pages(&self) -> Vec<Page> {
        let mut pages =
            vec![Page { x_mm: 0.0, y_mm: 0.0, width_mm: self.board.width_mm, height_mm: self.board.height_mm, setup: self.board_setup.clone() }];
        pages.extend(self.pages.iter().cloned());
        pages
    }

    /// The page a shape belongs to: the first extra page under its bounds centre, else 0.
    pub fn page_of_shape(&self, shape: &Shape) -> usize {
        let Some(center) = shape.bounds().map(|b| b.center()) else {
            return 0;
        };
        self.pages.iter().position(|p| p.rect().contains(center)).map_or(0, |i| i + 1)
    }

    /// One page as its own document, ready to export or cut: the board is the page, and
    /// the shapes on that page are moved to its origin. `None` for an unknown page.
    pub fn page_document(&self, index: usize) -> Option<Document> {
        let page = self.all_pages().get(index)?.clone();
        let mut out = self.clone();
        out.board = Board { width_mm: page.width_mm, height_mm: page.height_mm };
        out.board_setup = page.setup.clone();
        out.pages.clear();
        out.guides.clear();
        let shift = kurbo::Affine::translate((-page.x_mm, -page.y_mm));
        for layer in &mut out.layers {
            layer.shapes.retain(|s| self.page_of_shape(s) == index);
            for shape in &mut layer.shapes {
                shape.path.apply_affine(shift);
            }
        }
        out.groups = out.pruned_groups();
        Some(out)
    }

    /// A small standalone SVG preview of the board's output shapes, at most `max_bytes`
    /// long: shapes past the budget are left out. Coordinates keep two decimals.
    pub fn render_thumbnail(&self, max_bytes: usize) -> String {
        let (w, h) = (self.board.width_mm, self.board.height_mm);
        // Documents are y-up; flip into SVG's y-down space. The shared SVG writer owns markup,
        // escaping, numeric formatting, and path-data formatting.
        let mut svg = thumbnail_writer(w, h);
        let mut empty = svg.clone();
        empty.line(1);
        empty.close();
        empty.line(0);
        empty.close();
        let empty_len = empty.finish().len();
        if empty_len > max_bytes {
            return String::new();
        }
        let mut remaining = max_bytes - empty_len;
        'layers: for layer in self.layers.iter().filter(|l| l.is_output()) {
            for shape in layer.shapes.iter().filter(|s| !s.hidden) {
                let mut element = craft_svg::write::SvgWriter::fragment();
                element.line(2);
                element.tag("path").attr("d", craft_svg::write::path_data_fixed(&shape.path, 2)).empty();
                let element = element.finish();
                if element.len() > remaining {
                    break 'layers;
                }
                remaining -= element.len();
                svg.raw(&element);
            }
        }
        svg.line(1);
        svg.close();
        svg.line(0);
        svg.close();
        svg.finish()
    }
}

fn thumbnail_writer(width_mm: f64, height_mm: f64) -> craft_svg::write::SvgWriter {
    use craft_svg::write::{SvgWriter, num};

    let mut writer = SvgWriter::fragment();
    writer.root().attr("viewBox", format!("0 0 {} {}", num(width_mm, 2), num(height_mm, 2))).open();
    writer
        .tag("g")
        .attr("transform", format!("matrix(1 0 0 -1 0 {})", num(height_mm, 2)))
        .attr("fill", "none")
        .attr("stroke", "#000")
        .attr("stroke-width", num((width_mm.max(height_mm) / 200.0).max(0.01), 2))
        .open();
    writer
}

/// Groups holding `child` according to `parents` ([`Document::group_parent_map`]),
/// outermost first. Stops at a cycle or past the depth cap.
pub fn group_chain_in(parents: &BTreeMap<GroupChild, u64>, child: GroupChild) -> Vec<u64> {
    let mut chain = Vec::new();
    let mut current = child;
    while let Some(parent) = parents.get(&current) {
        if chain.contains(parent) || chain.len() > MAX_GROUP_DEPTH {
            break;
        }
        chain.push(*parent);
        current = GroupChild::Group(*parent);
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_orientation() {
        let (w, h) = PagePreset::Letter.size_mm().unwrap();
        assert!((w - 215.9).abs() < 1e-9 && (h - 279.4).abs() < 1e-9);
        assert_eq!(Orientation::Landscape.apply((210.0, 297.0)), (297.0, 210.0));
        assert_eq!(Orientation::Portrait.apply((297.0, 210.0)), (210.0, 297.0));
        assert!(PagePreset::Custom.size_mm().is_none());
        let json = serde_json::to_string(&PagePreset::JoyMat4_5x6_5).unwrap();
        assert_eq!(json, "\"joy-mat-4.5x6.5\"");
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#FF0080"), Some([255, 0, 128]));
        assert_eq!(parse_hex_color("#f08"), Some([255, 0, 136]));
        assert_eq!(parse_hex_color("ff0080"), None);
        assert_eq!(parse_hex_color("#ff00"), None);
        assert_eq!(parse_hex_color("#é0000"), None);
        assert_eq!(format_hex_color([1, 2, 255]), "#0102ff");
        assert!(color_distance([0, 0, 0], [3, 4, 0]) - 5.0 < 1e-9);
    }
}
