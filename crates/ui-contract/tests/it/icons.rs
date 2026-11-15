//! The catalogue's drawings, not only its names: every SVG the build embeds parses, each glyph the transform and
//! layout panels add draws on the side or axis it names, and no added glyph repeats another's art as a copied file
//! does.
//!
//! `build.rs` checks that each listed file exists and that a set folder holds nothing the manifest does not list. It
//! reads no drawing, and a document the parser refuses costs an icon its drawing at run time with only a log line, so
//! the drawings themselves are checked here: their ink, and where it sits. The copy check is a regression over the
//! drawings this change adds: a file copied over another fails, which is not a proof that two different shapes mean
//! different things.

use craft_ui::primitives::{icon_names, svg_tree};
use craft_ui::testing::trial;
use resvg::usvg::{Group, Node, Tree};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The glyphs of the transform and layout panels: a flip, an axis of scale, the skew along x, the four paddings and
/// the two gaps. Each names a side or an axis of the screen, never a reading order.
const ADDED: [&str; 11] = [
    "flip_horizontal",
    "flip_vertical",
    "scale_horizontal",
    "scale_vertical",
    "shear_horizontal",
    "padding_top",
    "padding_right",
    "padding_bottom",
    "padding_left",
    "gap_horizontal",
    "gap_vertical",
];

/// The drawings an added glyph must not be drawn from: the swap arrows it stood in for, and the round-trip arrows a
/// flip and a skew are not.
const STAND_INS: [&str; 4] = ["swap_horizontal", "swap_vertical", "rotate_cw", "rotate_ccw"];

/// The sets that draw each added glyph with a file of their own. A set that is not listed falls back to the base set's
/// drawing, and a suitable pinned drawing left out of the manifest fails here rather than passing quietly.
const DRAWN_BY: [(&str, &[&str]); 11] = [
    ("flip_horizontal", &["lucide", "phosphor", "tabler"]),
    ("flip_vertical", &["lucide", "phosphor", "tabler"]),
    ("scale_horizontal", &["phosphor"]),
    ("scale_vertical", &["phosphor"]),
    ("shear_horizontal", &["phosphor", "tabler"]),
    ("padding_top", &["phosphor"]),
    ("padding_right", &["phosphor"]),
    ("padding_bottom", &["phosphor"]),
    ("padding_left", &["phosphor"]),
    ("gap_horizontal", &["lucide", "phosphor", "tabler"]),
    ("gap_vertical", &["lucide", "phosphor", "tabler"]),
];

/// The three curated horizontal/vertical pairs: the two flips, the two gaps and the two axes of scale. These are the
/// pairs a consumer can swap by mistake, and the only ones the axis trial reads.
const AXIS_PAIRS: [(&str, &str); 3] =
    [("flip_horizontal", "flip_vertical"), ("gap_horizontal", "gap_vertical"), ("scale_horizontal", "scale_vertical")];

/// The side of the drawing's centre each padding puts its filled space band on, as a sign on x and y.
const PADDING_SIDES: [(&str, i32, i32); 4] = [("padding_top", 0, -1), ("padding_right", 1, 0), ("padding_bottom", 0, 1), ("padding_left", -1, 0)];

/// The units of slack the view-box check allows, in view-box units.
const SLACK: f32 = 1.0;

/// One `[[icon]]` of the manifest: whether it mirrors under right-to-left, and the file stem each set draws it from.
struct Entry {
    directional: bool,
    stems: BTreeMap<String, String>,
}

fn icons_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("icons")
}

/// Every `.svg` under `dir`.
fn svgs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(found) = std::fs::read_dir(dir) else { return };
    for entry in found.flatten() {
        let path = entry.path();
        if path.is_dir() {
            svgs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "svg") {
            out.push(path);
        }
    }
}

/// The quoted string `rest` starts with, if it starts with one.
fn quoted(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// A line of a table as its key and the text after `=`, for the keys a table holds: a comment or a bare word is none.
fn table_line(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.trim().split_once('=')?;
    let key = key.trim();
    if key.starts_with('#') || !key.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return None;
    }
    Some((key, rest.trim()))
}

/// The manifest's `[[set]]` tables: the set ids in manifest order, and the one set the others fall back to.
fn declared_sets(manifest: &str) -> (Vec<String>, String) {
    let mut ids = Vec::new();
    let mut base = None;
    for block in manifest.split("[[set]]").skip(1) {
        let (mut id, mut is_base) = (None, false);
        for (key, rest) in block.lines().filter_map(table_line) {
            match key {
                "id" => id = quoted(rest).map(str::to_owned),
                "base" => is_base = rest.starts_with("true"),
                _ => {}
            }
        }
        if let Some(id) = id {
            if is_base {
                assert!(base.is_none(), "icons.toml declares more than one base set");
                base = Some(id.clone());
            }
            ids.push(id);
        }
    }
    assert!(!ids.is_empty(), "icons.toml declares no set");
    (ids, base.expect("icons.toml has no set with `base = true`"))
}

/// Each `[[icon]]` of `icons.toml`: its reading direction, and the stem each *declared set* draws it from. A key that is
/// not a set id (a freeform `note`, a future field) names no file and cannot become one. The trial compares the names
/// this yields with the compiled `icon_names()`, so a line this reader stops understanding fails instead of passing
/// unnoticed.
fn entries(manifest: &str, sets: &[String]) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    for block in manifest.split("[[icon]]").skip(1) {
        let (mut name, mut directional) = (None, None);
        let mut stems = BTreeMap::new();
        for (key, rest) in block.lines().filter_map(table_line) {
            match (key, quoted(rest)) {
                ("name", Some(found)) => name = Some(found.to_owned()),
                (set, Some(stem)) if sets.iter().any(|declared| declared == set) => {
                    stems.insert(set.to_owned(), stem.to_owned());
                }
                ("directional", None) => directional = Some(rest.starts_with("true")),
                _ => {}
            }
        }
        if let Some(name) = name {
            assert!(
                out.insert(name.clone(), Entry { directional: directional.unwrap_or(false), stems }).is_none(),
                "icons.toml lists `{name}` twice"
            );
        }
    }
    out
}

/// The markers the pinned npm copy of Lucide's and Tabler's files carry, and a copy taken from anywhere else does not:
/// Lucide's licence comment and its class, Tabler's class.
const LUCIDE_MARKERS: [&str; 2] = ["@license lucide-static v", "class=\"lucide "];
const TABLER_MARKERS: [&str; 1] = ["class=\"icon icon-tabler"];

fn pinned_markers(set: &str) -> &'static [&'static str] {
    match set {
        "lucide" => &LUCIDE_MARKERS,
        "tabler" => &TABLER_MARKERS,
        _ => &[],
    }
}

/// The sets that must draw `name` themselves.
fn required_sets(name: &str) -> &'static [&'static str] {
    DRAWN_BY.iter().find(|(glyph, _)| *glyph == name).map_or(&[], |(_, sets)| *sets)
}

/// The parsed drawing `stem` of `set`, with the path it came from for the failure message.
fn drawing(set: &str, stem: &str) -> (PathBuf, Tree) {
    let path = icons_dir().join(set).join(format!("{stem}.svg"));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let tree = svg_tree::parse(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    (path, tree)
}

/// One painted path as these trials read it: the geometry the parser gives (every segment, in path order), the transform
/// the path lands with, its absolute box including the stroke, and whether it is filled. The geometry is what tells two
/// drawings apart when their boxes match: Lucide's `flip-horizontal` and `gap-horizontal` have the same multiset of path
/// boxes and different brackets, one opening out and one in. The stroke is part of the box because a drawing's ink is
/// what the eye sees, and a stroke past the view box is ink the icon loses.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Shape {
    /// Every segment of the path, as the parser types them, in path order.
    segments: String,
    /// The path's absolute transform.
    transform: String,
    /// The absolute box including the stroke, in hundredths of a view-box unit: left, top, right, bottom.
    bounds: [i64; 4],
    /// Whether the path is painted with a fill.
    filled: bool,
}

impl Shape {
    /// The centre of the path's box, in hundredths of a view-box unit.
    fn centre(&self) -> (i64, i64) {
        ((self.bounds[0] + self.bounds[2]) / 2, (self.bounds[1] + self.bounds[3]) / 2)
    }

    fn width(&self) -> i64 {
        self.bounds[2] - self.bounds[0]
    }

    fn height(&self) -> i64 {
        self.bounds[3] - self.bounds[1]
    }

    /// The box's area in hundredths squared, which ranks a drawing's parts by how much of it they paint.
    fn area(&self) -> i64 {
        self.width().max(0) * self.height().max(0)
    }
}

/// Every painted path of `group`, depth first. A path with neither a fill nor a stroke paints nothing (Tabler ships one
/// as a placeholder) and is left out: its box would otherwise outrank the drawing's real parts.
fn shapes(group: &Group) -> Vec<Shape> {
    fn walk(group: &Group, out: &mut Vec<Shape>) {
        for node in group.children() {
            match node {
                Node::Group(inner) => walk(inner, out),
                Node::Path(path) if path.is_visible() && (path.fill().is_some() || path.stroke().is_some()) => {
                    let bounds = path.abs_stroke_bounding_box();
                    let hundredths = |value: f32| (value * 100.0).round() as i64;
                    out.push(Shape {
                        segments: format!("{:?}", path.data().segments().collect::<Vec<_>>()),
                        transform: format!("{:?}", path.abs_transform()),
                        bounds: [hundredths(bounds.left()), hundredths(bounds.top()), hundredths(bounds.right()), hundredths(bounds.bottom())],
                        filled: path.fill().is_some(),
                    });
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(group, &mut out);
    out
}

/// How far a drawing spreads along x and along y, in hundredths: the separation of its two largest painted paths, or
/// the box of its one path. A curated pair's two drawings each have one of those shapes, so a swapped pair turns the
/// larger number into the other axis.
fn spread(painted: &[Shape]) -> (i64, i64) {
    let mut ranked: Vec<&Shape> = painted.iter().collect();
    ranked.sort_by_key(|shape| std::cmp::Reverse(shape.area()));
    match ranked.as_slice() {
        [first, second, ..] => {
            let ((ax, ay), (bx, by)) = (first.centre(), second.centre());
            ((ax - bx).abs(), (ay - by).abs())
        }
        [only] => (only.width(), only.height()),
        [] => (0, 0),
    }
}

#[trial]
fn every_icon_drawing_in_the_catalogue_parses() {
    let dir = icons_dir();
    let mut files = Vec::new();
    svgs(&dir, &mut files);
    assert!(files.len() > 250, "the catalogue holds {} drawings: the walk no longer finds them", files.len());
    let mut refused = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        if let Err(error) = svg_tree::parse(&bytes) {
            refused.push(format!("{}: {error}", path.strip_prefix(&dir).unwrap_or(path).display()));
        }
    }
    assert!(refused.is_empty(), "drawings the parser refuses, which their icon would paint as nothing:\n{}", refused.join("\n"));
}

#[trial]
fn each_added_glyph_names_a_physical_axis_and_is_drawn_by_the_packs_that_have_it() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let (sets, base) = declared_sets(&manifest);
    let catalogue = entries(&manifest, &sets);
    assert_eq!(catalogue.keys().map(String::as_str).collect::<Vec<_>>(), icon_names(), "the manifest and the compiled catalog list different icons");

    // A block may carry a freeform key without that key becoming a set, so metadata cannot reach for a file that is
    // not there. Read through the same reader the rest of the trial uses.
    let sample = "[[set]]\nid = \"phosphor\"\nbase = true\n\n[[icon]]\nname = \"flip_horizontal\"\ndirectional = false\nphosphor = \"flip-horizontal\"\nnote = \"a remark\"\n";
    let (sample_sets, sample_base) = declared_sets(sample);
    let synthetic = entries(sample, &sample_sets);
    assert_eq!(sample_base, "phosphor");
    assert_eq!(synthetic["flip_horizontal"].stems.len(), 1, "a freeform `note` key became a set: {:?}", synthetic["flip_horizontal"].stems);

    for name in ADDED {
        let entry = catalogue.get(name).unwrap_or_else(|| panic!("icons.toml lists no icon `{name}`"));
        assert!(!entry.directional, "`{name}` names a side or an axis of the screen: it must not mirror under right-to-left");
        assert!(entry.stems.contains_key(&base), "`{name}` has no drawing in the base set `{base}`, so a pack without one paints nothing");
        let drawn: Vec<&str> = entry.stems.keys().map(String::as_str).collect();
        for set in required_sets(name) {
            assert!(drawn.contains(set), "`{name}` has no drawing in `{set}`, which draws the same meaning: {drawn:?}");
        }
        for (set, stem) in &entry.stems {
            let stand_in = STAND_INS.iter().any(|other| catalogue.get(*other).is_some_and(|entry| entry.stems.get(set) == Some(stem)));
            assert!(!stand_in, "`{name}` in `{set}` names `{stem}`, the drawing of a stand-in");
            let path = icons_dir().join(set).join(format!("{stem}.svg"));
            let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))).into_owned();
            for marker in pinned_markers(set) {
                assert!(
                    text.contains(*marker),
                    "`{name}` in `{set}` is not the pinned npm copy of `{stem}`, which carries {marker:?}: {}",
                    path.display()
                );
            }
        }
    }
}

#[trial]
fn the_added_drawings_paint_their_own_pictures() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let (sets, _) = declared_sets(&manifest);
    let catalogue = entries(&manifest, &sets);
    for set in &sets {
        // The art, not the file stem: every visible path's box, whether it is filled, and the order they come in. A
        // file copied over another, or a drawing pasted under a new name, repeats that list and fails here. The
        // fingerprint is a copy regression over the drawings this change adds, not a proof that two shapes which share
        // every box mean different things.
        let mut seen: BTreeMap<Vec<Shape>, String> = BTreeMap::new();
        for name in ADDED.into_iter().chain(STAND_INS) {
            let Some(stem) = catalogue.get(name).and_then(|entry| entry.stems.get(set)) else { continue };
            let (path, tree) = drawing(set, stem);
            let ink = shapes(tree.root());
            assert!(!ink.is_empty(), "`{name}` in `{set}` paints nothing: {}", path.display());
            if let Some(other) = seen.insert(ink, format!("`{name}`")) {
                panic!(
                    "{other} and `{name}` in `{set}` repeat one another's ink boxes, fills and order: one file is a copy of the other's art, or was redrawn from it"
                );
            }
        }
    }
}

#[trial]
fn each_added_glyph_draws_inside_its_view_box() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let (sets, _) = declared_sets(&manifest);
    let catalogue = entries(&manifest, &sets);
    let mut drawings = 0usize;
    for name in ADDED {
        for (set, stem) in &catalogue[name].stems {
            let (path, tree) = drawing(set, stem);
            let view = tree.size();
            // The ink, stroke and caps included: a cap past the view box is a cropped glyph.
            let ink = tree.root().abs_stroke_bounding_box();
            assert!(view.width() > 0.0 && view.height() > 0.0, "{}: the drawing has no view box", path.display());
            assert!(ink.width() > 0.0 && ink.height() > 0.0, "`{name}` draws nothing in `{set}`: {}", path.display());
            assert!(
                ink.left() >= -SLACK && ink.top() >= -SLACK && ink.right() <= view.width() + SLACK && ink.bottom() <= view.height() + SLACK,
                "`{name}` in `{set}` paints outside its view box: {ink:?} of {view:?} in {}",
                path.display()
            );
            drawings += 1;
        }
    }
    assert!(drawings >= ADDED.len(), "the added glyphs use {drawings} drawings, fewer than the base set's {}", ADDED.len());
}

#[trial]
fn each_padding_drawing_puts_its_space_on_its_side() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let (sets, _) = declared_sets(&manifest);
    let catalogue = entries(&manifest, &sets);
    for (name, dx, dy) in PADDING_SIDES {
        for (set, stem) in &catalogue[name].stems {
            let (path, tree) = drawing(set, stem);
            let view = tree.size();
            let bands: Vec<Shape> = shapes(tree.root()).into_iter().filter(|shape| shape.filled).collect();
            assert_eq!(bands.len(), 1, "`{name}` in `{set}` fills {} parts: the space it names is one solid band ({})", bands.len(), path.display());
            let band = &bands[0];
            assert!(band.width() > 0 && band.height() > 0, "`{name}` in `{set}` fills an empty band: {}", path.display());
            let (x, y) = band.centre();
            let (across, along) = ((view.width() * 100.0) as i64 / 2, (view.height() * 100.0) as i64 / 2);
            // A tenth of the drawing, so a band nudged off centre cannot pass, and a twentieth the other way, so the
            // band is squarely on one side rather than in a corner.
            let reach = across.min(along) / 5;
            let slack = across.min(along) / 20;
            let offset = if dx != 0 { (x - across) * i64::from(dx) } else { (y - along) * i64::from(dy) };
            let side_ways = if dx != 0 { (y - along).abs() } else { (x - across).abs() };
            assert!(
                offset >= reach,
                "`{name}` in `{set}` puts its band {offset} hundredths off centre towards its side, not the {reach} a tenth of the drawing needs: {}",
                path.display()
            );
            assert!(side_ways <= slack, "`{name}` in `{set}` puts its band {side_ways} hundredths off the other axis: {}", path.display());
        }
    }
}

/// The curated pairs run along the axis their names give: a flip and a gap spread between their two largest painted
/// paths (Lucide's horizontal flip is taller than it is wide, so the drawing's overall shape says nothing), and an
/// axis of scale is one painted path that is wider than tall or the reverse. Swapping one pair's drawings therefore
/// fails here. This is a check on three named pairs, not a reading of what an arbitrary SVG means.
#[trial]
fn each_axis_pair_runs_along_its_axis() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let (sets, _) = declared_sets(&manifest);
    let catalogue = entries(&manifest, &sets);
    for (horizontal, vertical) in AXIS_PAIRS {
        for (name, along_x) in [(horizontal, true), (vertical, false)] {
            for (set, stem) in &catalogue[name].stems {
                let (path, tree) = drawing(set, stem);
                let view = tree.size();
                let (dx, dy) = spread(&shapes(tree.root()));
                let (along, across) = if along_x { (dx, dy) } else { (dy, dx) };
                let axis = if along_x { "x" } else { "y" };
                let tenth = (view.width().max(view.height()) * 100.0) as i64 / 10;
                assert!(
                    along > across,
                    "`{name}` in `{set}` spreads {dx} hundredths along x and {dy} along y, so it does not run along {axis}: {}",
                    path.display()
                );
                assert!(
                    along >= tenth,
                    "`{name}` in `{set}` spreads only {along} hundredths along {axis}, under a tenth of the drawing: {}",
                    path.display()
                );
            }
        }
    }
}
