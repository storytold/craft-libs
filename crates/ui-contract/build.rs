//! Generates `primitives::icon::IconSet` and the embedded SVG catalog from `icons/icons.toml`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use toml::{Table, Value};

struct Set {
    id: String,
    title: String,
    licence: String,
    stroke_width: Option<f64>,
    /// The base set: every icon has a file in it, and the other sets fall back to it.
    base: bool,
    /// The set `IconSet::default()` names.
    default: bool,
}

struct Entry {
    name: String,
    directional: bool,
    /// One file stem per set, in set order; `None` where a non-base set has no file.
    files: Vec<Option<String>>,
}

fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dir = Path::new(&root).join("icons");
    println!("cargo::rerun-if-changed=icons");
    let text = std::fs::read_to_string(dir.join("icons.toml")).expect("read icons/icons.toml");
    let manifest: Table = text.parse().unwrap_or_else(|e| panic!("icons/icons.toml: {e}"));
    let sets = sets(&manifest);
    let icons = icons(&manifest, &sets);
    check_files(&dir, &sets, &icons);
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    std::fs::write(Path::new(&out).join("icons.rs"), generate(&root, &sets, &icons)).expect("write icons.rs");
    deflate_egui_fonts(Path::new(&out));
}

/// Deflate egui's default fonts and the suite's interface face into `OUT_DIR/egui_fonts/` for `fonts::deflated_default_definitions`:
/// egui's `default_fonts` feature embeds the same files uncompressed (1.4 MB), more than a web
/// build's size budget allows.
fn deflate_egui_fonts(out: &Path) {
    use std::io::Write as _;
    let dir = out.join("egui_fonts");
    let fonts = [
        ("Hack-Regular", epaint_default_fonts::HACK_REGULAR),
        ("egui-icons", epaint_default_fonts::EGUI_ICONS),
        ("Ubuntu-Light", epaint_default_fonts::UBUNTU_LIGHT),
    ];
    // The suite's interface face (Inter, subset to Latin, Greek and Cyrillic; fonts/inter/OFL.txt).
    println!("cargo::rerun-if-changed=fonts/inter");
    let inter: Vec<(&str, Vec<u8>)> = ["Inter-Regular", "Inter-SemiBold"]
        .into_iter()
        .map(|name| (name, std::fs::read(format!("fonts/inter/{name}.ttf")).unwrap_or_else(|e| panic!("fonts/inter/{name}.ttf: {e}"))))
        .collect();
    std::fs::create_dir_all(&dir).expect("create egui_fonts");
    for (name, bytes) in fonts.into_iter().chain(inter.iter().map(|(name, bytes)| (*name, bytes.as_slice()))) {
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(bytes).expect("deflate");
        std::fs::write(dir.join(format!("{name}.ttf.deflate")), z.finish().expect("deflate")).expect("write egui font");
    }
}

fn str_field<'a>(table: &'a Table, key: &str, what: &str) -> &'a str {
    table.get(key).and_then(Value::as_str).unwrap_or_else(|| panic!("icons/icons.toml: {what} has no string `{key}`"))
}

fn sets(manifest: &Table) -> Vec<Set> {
    let list = manifest.get("set").and_then(Value::as_array).expect("icons/icons.toml: no [[set]] tables");
    let sets: Vec<Set> = list
        .iter()
        .map(|value| {
            let table = value.as_table().expect("icons/icons.toml: [[set]] is not a table");
            let id = str_field(table, "id", "a [[set]]").to_owned();
            assert!(is_slug(&id, '_'), "icons/icons.toml: set id `{id}` is not lower-case ascii");
            let what = format!("set `{id}`");
            let stroke_width = match str_field(table, "kind", &what) {
                "stroke" => Some(
                    table.get("stroke_width").and_then(Value::as_float).unwrap_or_else(|| panic!("icons/icons.toml: {what} has no `stroke_width`")),
                ),
                "fill" => None,
                other => panic!("icons/icons.toml: {what} has kind `{other}`, not `stroke` or `fill`"),
            };
            Set {
                title: str_field(table, "title", &what).to_owned(),
                licence: str_field(table, "licence", &what).to_owned(),
                stroke_width,
                base: table.get("base").and_then(Value::as_bool).unwrap_or(false),
                default: table.get("default").and_then(Value::as_bool).unwrap_or(false),
                id,
            }
        })
        .collect();
    assert!(!sets.is_empty(), "icons/icons.toml: no sets");
    assert_eq!(sets.iter().filter(|s| s.base).count(), 1, "icons/icons.toml: exactly one [[set]] must have `base = true`");
    assert_eq!(sets.iter().filter(|s| s.default).count(), 1, "icons/icons.toml: exactly one [[set]] must have `default = true`");
    sets
}

/// The icons sorted by name, so the catalog looks one up by binary search.
fn icons(manifest: &Table, sets: &[Set]) -> Vec<Entry> {
    let list = manifest.get("icon").and_then(Value::as_array).expect("icons/icons.toml: no [[icon]] tables");
    let mut seen = BTreeSet::new();
    let mut entries: Vec<Entry> = list
        .iter()
        .map(|value| {
            let table = value.as_table().expect("icons/icons.toml: [[icon]] is not a table");
            let name = str_field(table, "name", "an [[icon]]").to_owned();
            assert!(is_slug(&name, '_'), "icons/icons.toml: icon name `{name}` is not lower-case ascii with underscores");
            assert!(seen.insert(name.clone()), "icons/icons.toml: icon `{name}` is listed twice");
            let directional = table
                .get("directional")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| panic!("icons/icons.toml: icon `{name}` has no `directional = true|false`"));
            let files = sets
                .iter()
                .map(|set| {
                    let file = table.get(&set.id).and_then(Value::as_str);
                    assert!(file.is_some() || !set.base, "icon `{name}` has no file in the base set `{}`", set.id);
                    if let Some(file) = file {
                        assert!(is_slug(file, '-'), "icon `{name}`: file `{file}` in set `{}` is not a plain file stem", set.id);
                    }
                    file.map(str::to_owned)
                })
                .collect();
            Entry { name, directional, files }
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Every listed file exists, and every set folder holds only listed files and its licence.
fn check_files(dir: &Path, sets: &[Set], icons: &[Entry]) {
    for (index, set) in sets.iter().enumerate() {
        let folder = dir.join(&set.id);
        for icon in icons {
            if let Some(file) = &icon.files[index] {
                let path = folder.join(format!("{file}.svg"));
                assert!(path.is_file(), "icon `{}` is missing from set `{}`: no file {}", icon.name, set.id, path.display());
            }
        }
        let listed: BTreeSet<String> = icons.iter().filter_map(|icon| icon.files[index].as_ref()).map(|file| format!("{file}.svg")).collect();
        let entries = std::fs::read_dir(&folder).unwrap_or_else(|e| panic!("read {}: {e}", folder.display()));
        for entry in entries {
            let file = entry.expect("read icon folder entry").file_name().to_string_lossy().into_owned();
            assert!(file == "LICENSE" || listed.contains(&file), "set `{}` holds {file}, which icons.toml does not list", set.id);
        }
        assert!(folder.join("LICENSE").is_file(), "set `{}` has no LICENSE file", set.id);
    }
}

fn is_slug(s: &str, separator: char) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == separator)
}

fn camel(s: &str) -> String {
    s.split(['_', '-']).map(|word| word[..1].to_ascii_uppercase() + &word[1..]).collect()
}

fn generate(root: &str, sets: &[Set], icons: &[Entry]) -> String {
    let mut out = String::new();
    let n = icons.len();
    let base = sets.iter().find(|s| s.base).expect("a base set");

    let _ = writeln!(out, "/// The names of the catalog's icons, sorted: the manifest's `[[icon]]` names.");
    let _ =
        writeln!(out, "pub(super) const NAMES: [&str; {n}] = [{}];\n", icons.iter().map(|i| format!("{:?}", i.name)).collect::<Vec<_>>().join(", "));
    let _ = writeln!(
        out,
        "/// Whether each icon points along the reading direction, by index in `NAMES`.\npub(super) const DIRECTIONAL: [bool; {n}] = [{}];\n",
        icons.iter().map(|i| i.directional.to_string()).collect::<Vec<_>>().join(", ")
    );

    out.push_str("/// An icon set. The base set draws every icon; the others are packs that draw the icons they have and fall back to the base set for the rest.\n");
    out.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]\n#[non_exhaustive]\npub enum IconSet {\n");
    for set in sets {
        if set.default {
            out.push_str("    #[default]\n");
        }
        let _ = writeln!(out, "    /// {}, {}.\n    {},", set.title, set.licence, camel(&set.id));
    }
    out.push_str("}\n\nimpl IconSet {\n    /// Every set, in manifest order.\n");
    let _ = writeln!(
        out,
        "    pub const ALL: [IconSet; {}] = [{}];\n",
        sets.len(),
        sets.iter().map(|s| format!("IconSet::{}", camel(&s.id))).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(
        out,
        "    /// The base set: it draws every icon, and the other sets fall back to it.\n    pub const BASE: IconSet = IconSet::{};\n",
        camel(&base.id)
    );
    set_fn(&mut out, sets, "The folder name under `crates/ui-contract/icons/`.", "id", "&'static str", |s| format!("{:?}", s.id));
    set_fn(&mut out, sets, "The set's name for display.", "title", "&'static str", |s| format!("{:?}", s.title));
    set_fn(&mut out, sets, "The set's own stroke width in view-box units; `None` for a set drawn with fills.", "stroke_width", "Option<f32>", |s| {
        s.stroke_width.map_or("None".to_owned(), |w| format!("Some({w:?})"))
    });
    out.push_str("}\n\n");

    out.push_str("/// The SVG document of the icon at `index` in `NAMES` for `set`, and whether the set draws it from its own file.\n");
    out.push_str("pub(super) const fn svg(index: usize, set: IconSet) -> (&'static [u8], bool) {\n    match set {\n");
    for set in sets {
        let upper = set.id.to_ascii_uppercase();
        if set.base {
            let _ = writeln!(out, "        IconSet::{} => ({upper}[index], true),", camel(&set.id));
        } else {
            let _ = writeln!(
                out,
                "        IconSet::{} => match {upper}[index] {{\n            Some(bytes) => (bytes, true),\n            None => ({}[index], false),\n        }},",
                camel(&set.id),
                base.id.to_ascii_uppercase()
            );
        }
    }
    out.push_str("    }\n}\n");

    for (index, set) in sets.iter().enumerate() {
        if set.base {
            let _ = writeln!(out, "\nconst {}: [&[u8]; {n}] = [", set.id.to_ascii_uppercase());
        } else {
            let _ = writeln!(out, "\nconst {}: [Option<&[u8]>; {n}] = [", set.id.to_ascii_uppercase());
        }
        for icon in icons {
            match (&icon.files[index], set.base) {
                (Some(file), true) => {
                    let _ = writeln!(out, "    include_bytes!({:?}),", format!("{root}/icons/{}/{file}.svg", set.id));
                }
                (Some(file), false) => {
                    let _ = writeln!(out, "    Some(include_bytes!({:?})),", format!("{root}/icons/{}/{file}.svg", set.id));
                }
                (None, _) => out.push_str("    None,\n"),
            }
        }
        out.push_str("];\n");
    }
    out
}

/// A `const fn` of `IconSet` with one match arm per set.
fn set_fn(out: &mut String, sets: &[Set], doc: &str, func: &str, ty: &str, value: fn(&Set) -> String) {
    let _ = writeln!(out, "    /// {doc}\n    pub const fn {func}(self) -> {ty} {{\n        match self {{");
    for set in sets {
        let _ = writeln!(out, "            IconSet::{} => {},", camel(&set.id), value(set));
    }
    out.push_str("        }\n    }\n\n");
}
