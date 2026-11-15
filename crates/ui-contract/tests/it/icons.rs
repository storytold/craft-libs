//! The catalogue's drawings, not only its names: every SVG the build embeds parses, and the glyphs the transform and
//! layout panels add keep a physical meaning and their own art.
//!
//! `build.rs` checks that each listed file exists and that a set folder holds nothing the manifest does not list. It
//! reads no drawing, and a document the parser refuses costs an icon its drawing at run time with only a log line, so
//! the drawings themselves are checked here.

use craft_ui::primitives::{icon_names, svg_tree};
use craft_ui::testing::trial;
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

/// The drawings an added glyph must not reuse: the swap arrows it stood in for, and the round-trip arrows a flip and
/// a skew are not.
const STAND_INS: [&str; 4] = ["swap_horizontal", "swap_vertical", "rotate_cw", "rotate_ccw"];

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
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
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

/// Each `[[icon]]` of `icons.toml`: its reading direction, and the stem each set draws it from. The manifest is the
/// source of both facts, and the trial compares the names it yields with the compiled `icon_names()`, so a line this
/// reader stops understanding fails instead of passing unnoticed.
fn entries(manifest: &str) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    for block in manifest.split("[[icon]]").skip(1) {
        let (mut name, mut directional) = (None, None);
        let mut stems = BTreeMap::new();
        for (key, rest) in block.lines().filter_map(table_line) {
            match (key, quoted(rest)) {
                ("name", Some(found)) => name = Some(found.to_owned()),
                (_, Some(stem)) => {
                    stems.insert(key.to_owned(), stem.to_owned());
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

/// The set a pack without a drawing falls back to: the manifest's one `base = true` set.
fn base_set(manifest: &str) -> String {
    for block in manifest.split("[[set]]").skip(1) {
        let (mut id, mut base) = (None, false);
        for (key, rest) in block.lines().filter_map(table_line) {
            match key {
                "id" => id = quoted(rest).map(str::to_owned),
                "base" => base = rest.starts_with("true"),
                _ => {}
            }
        }
        if base {
            return id.expect("icons.toml: the base set has no `id`");
        }
    }
    panic!("icons.toml has no set with `base = true`");
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
fn each_added_glyph_names_a_physical_axis_and_has_its_own_art() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let entries = entries(&manifest);
    assert_eq!(entries.keys().map(String::as_str).collect::<Vec<_>>(), icon_names(), "the manifest and the compiled catalog list different icons");
    let base = base_set(&manifest);
    for name in ADDED {
        let entry = entries.get(name).unwrap_or_else(|| panic!("icons.toml lists no icon `{name}`"));
        assert!(!entry.directional, "`{name}` names a side or an axis of the screen: it must not mirror under right-to-left");
        assert!(entry.stems.contains_key(&base), "`{name}` has no drawing in the base set `{base}`, so a pack without one paints nothing");
        for (set, stem) in &entry.stems {
            for other in ADDED.into_iter().filter(|other| *other != name) {
                let theirs = entries.get(other).and_then(|entry| entry.stems.get(set));
                assert!(theirs != Some(stem), "`{name}` and `{other}` share the drawing `{stem}` in `{set}`, so one of them is a stand-in");
            }
            for stand_in in STAND_INS {
                let theirs = entries.get(stand_in).and_then(|entry| entry.stems.get(set));
                assert!(
                    theirs != Some(stem),
                    "`{name}` in `{set}` reuses `{stand_in}`'s drawing `{stem}`: a flip, a padding, a gap and a skew each have their own"
                );
            }
        }
    }
}

#[trial]
fn each_added_glyph_draws_inside_its_view_box() {
    let manifest = std::fs::read_to_string(icons_dir().join("icons.toml")).unwrap();
    let entries = entries(&manifest);
    let dir = icons_dir();
    let mut drawings = 0;
    for name in ADDED {
        let entry = &entries[name];
        for (set, stem) in &entry.stems {
            let path = dir.join(set).join(format!("{stem}.svg"));
            let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let tree = svg_tree::parse(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let view = tree.size();
            let ink = tree.root().abs_bounding_box();
            assert!(view.width() > 0.0 && view.height() > 0.0, "{}: the drawing has no view box", path.display());
            assert!(ink.width() > 0.0 && ink.height() > 0.0, "`{name}` draws nothing in `{set}`: {}", path.display());
            assert!(
                ink.left() >= -1.0 && ink.top() >= -1.0 && ink.right() <= view.width() + 1.0 && ink.bottom() <= view.height() + 1.0,
                "`{name}` in `{set}` draws outside its view box: {ink:?} of {view:?} in {}",
                path.display()
            );
            drawings += 1;
        }
    }
    assert!(drawings >= ADDED.len(), "the added glyphs use {drawings} drawings, fewer than the base set's {}", ADDED.len());
}
