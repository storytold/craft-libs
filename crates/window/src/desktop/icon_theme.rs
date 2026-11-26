//! Linux: icons from the desktop's freedesktop icon theme (the Icon Theme Specification), for the
//! caption buttons' glyphs. A theme is searched, then the themes it inherits, then `hicolor`;
//! within each, its directories from the best fit for a 16 px icon down. Only SVG files are found:
//! symbolic icons are drawn as a coverage mask in the button's ink colour.

use std::path::{Path, PathBuf};

/// Bounds on what a theme's files can make us do: themes in an inheritance chain, directories a
/// theme lists, and the bytes read from one file.
const MAX_THEMES: usize = 32;
const MAX_DIRS: usize = 4096;
/// How deep an index's unlisted directories are looked for (`symbolic/ui`, `16x16@2/actions`).
const SCAN_DEPTH: usize = 3;
const MAX_FILE: u64 = 1 << 20;
/// The size the caption glyphs are drawn at, in the spec's units.
const ICON_SIZE: i64 = 16;

/// A theme or icon name that stays inside its directory.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// At most 1 MiB of a file: settings, theme indexes and icons are small, and a hostile theme
/// cannot make us read more.
pub fn read_small(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path).ok()?.take(MAX_FILE).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Where themes live, in the spec's order: `~/.icons`, `$XDG_DATA_HOME/icons`, then each
/// `$XDG_DATA_DIRS/icons`. Relative paths in the variables are ignored, as the XDG spec says.
pub fn base_dirs(var: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let absolute = |p: &String| Path::new(p).is_absolute();
    let home = var("HOME").filter(absolute).map(PathBuf::from);
    let mut dirs = vec![];
    if let Some(home) = &home {
        dirs.push(home.join(".icons"));
    }
    if let Some(data) = var("XDG_DATA_HOME").filter(absolute).map(PathBuf::from).or_else(|| home.map(|h| h.join(".local").join("share"))) {
        dirs.push(data.join("icons"));
    }
    let data_dirs = var("XDG_DATA_DIRS").filter(|d| !d.trim().is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(data_dirs.split(':').filter(|d| Path::new(d).is_absolute()).map(|d| Path::new(d).join("icons")));
    dirs
}

/// Every `[section]` of an `index.theme` as (key, value) pairs, in one pass; the first section
/// of a name wins. Looking each directory's section up by rescanning the file would multiply its
/// size by the number of directories.
fn sections(index: &str) -> std::collections::HashMap<&str, Vec<(&str, &str)>> {
    let mut all = std::collections::HashMap::new();
    let mut current: Option<Vec<(&str, &str)>> = None;
    let mut name = "";
    for line in index.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some(done) = current.take() {
                all.entry(name).or_insert(done);
            }
            name = s;
            current = Some(vec![]);
        } else if let (Some(pairs), Some((k, v))) = (current.as_mut(), line.split_once('=')) {
            pairs.push((k.trim(), v.trim()));
        }
    }
    if let Some(done) = current {
        all.entry(name).or_insert(done);
    }
    all
}

fn list(value: Option<&str>) -> impl Iterator<Item = &str> {
    value.unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty())
}

/// How far a theme directory is from a 16 px icon at scale 1 (the spec's
/// `DirectorySizeDistance`), then its scale: 0 is an exact fit.
fn distance(keys: &[(&str, &str)]) -> (i64, i64) {
    let get = |k: &str| keys.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
    let num = |k: &str, default: i64| get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(default).clamp(0, 1 << 16);
    let size = num("Size", 0);
    let scale = num("Scale", 1).max(1);
    let d = match get("Type").unwrap_or("Threshold") {
        "Fixed" => (size * scale - ICON_SIZE).abs(),
        "Scalable" => {
            let (min, max) = (num("MinSize", size) * scale, num("MaxSize", size) * scale);
            (min - ICON_SIZE).max(ICON_SIZE - max).max(0)
        }
        _ => ((size * scale - ICON_SIZE).abs() - num("Threshold", 2) * scale).max(0),
    };
    (d, scale)
}

/// Every directory under `root`, at most [`SCAN_DEPTH`] levels down and [`MAX_DIRS`] in all, in
/// path order: scalable and symbolic ones first, as icons there fit any size.
fn subdirectories(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut level = vec![root.to_path_buf()];
    for _ in 0..SCAN_DEPTH {
        let mut next = vec![];
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten().take(MAX_DIRS) {
                let path = entry.path();
                if found.len() + next.len() < MAX_DIRS && path.is_dir() {
                    next.push(path);
                }
            }
        }
        found.extend(next.iter().cloned());
        level = next;
    }
    let vector = |p: &PathBuf| !p.components().any(|c| matches!(c.as_os_str().to_str(), Some("scalable" | "symbolic")));
    found.sort_by_key(|p| (vector(p), p.clone()));
    found
}

/// One theme of a chain: its listed directories, best fit first, in every base directory that has
/// the theme, and (looked for only when an icon isn't in those) the directories its index forgot.
struct Theme {
    roots: Vec<PathBuf>,
    listed: Vec<PathBuf>,
    unlisted: std::cell::OnceCell<Vec<PathBuf>>,
}

impl Theme {
    /// The directories the index forgot to list: GTK still finds icons there, and custom themes
    /// often add `symbolic/ui` without listing it.
    fn unlisted(&self) -> &[PathBuf] {
        self.unlisted.get_or_init(|| {
            let listed: std::collections::HashSet<&PathBuf> = self.listed.iter().collect();
            self.roots.iter().flat_map(|r| subdirectories(r)).filter(|d| !listed.contains(d)).collect()
        })
    }
}

/// A theme and the themes it inherits, in search order.
pub struct IconTheme {
    themes: Vec<Theme>,
}

impl IconTheme {
    /// `name` and the themes it inherits, then `hicolor`, looked for in `bases` (see [`base_dirs`]).
    /// `None` when no theme in the chain has an `index.theme`.
    pub fn load(name: &str, bases: &[PathBuf]) -> Option<Self> {
        let mut chain: Vec<String> = vec![name.to_string()];
        let mut themes = vec![];
        let mut i = 0;
        loop {
            let Some(theme) = chain.get(i).cloned() else {
                // Every chain ends in hicolor, whether or not the themes before it exist.
                if chain.iter().any(|t| t == "hicolor") {
                    break;
                }
                chain.push("hicolor".into());
                continue;
            };
            i += 1;
            if !valid_name(&theme) {
                continue;
            }
            let roots: Vec<PathBuf> = bases.iter().map(|b| b.join(&theme)).filter(|r| r.is_dir()).collect();
            // The first index wins; a theme can still be spread over several base directories.
            let Some(index) = roots.iter().find_map(|r| read_small(&r.join("index.theme"))) else { continue };
            let index = String::from_utf8_lossy(&index);
            let all = sections(&index);
            let head = all.get("Icon Theme").map_or(&[][..], Vec::as_slice);
            let get = |k: &str| head.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
            let mut subdirs: Vec<((i64, i64), &str)> = list(get("Directories"))
                .chain(list(get("ScaledDirectories")))
                .filter(|d| !d.split('/').any(|part| part == ".." || part.is_empty()))
                .take(MAX_DIRS)
                .map(|d| (distance(all.get(d).map_or(&[][..], Vec::as_slice)), d))
                .collect();
            subdirs.sort_by_key(|(key, _)| *key);
            // Not checked for existence: a theme lists hundreds, and the search stops at a hit.
            let listed = subdirs.iter().flat_map(|(_, sub)| roots.iter().map(move |r| r.join(sub))).collect();
            for parent in list(get("Inherits")) {
                if chain.len() < MAX_THEMES && !chain.iter().any(|t| t == parent) {
                    chain.push(parent.to_string());
                }
            }
            themes.push(Theme { roots, listed, unlisted: std::cell::OnceCell::new() });
        }
        (!themes.is_empty()).then_some(Self { themes })
    }

    /// The first `<icon>.svg` in search order: each theme's listed directories, then its unlisted
    /// ones, before its parent.
    pub fn find_svg(&self, icon: &str) -> Option<PathBuf> {
        if !valid_name(icon) {
            return None;
        }
        let file = format!("{icon}.svg");
        let hit = |dirs: &[PathBuf]| dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file());
        self.themes.iter().find_map(|t| hit(&t.listed).or_else(|| hit(t.unlisted())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway icon tree: `files` are (path under the base, contents).
    fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let base = std::env::temp_dir().join(format!("craft-window-icons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for (path, text) in files {
            let p = base.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        base
    }

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path d="M4 4h8v8H4z" fill="#2e3436"/></svg>"##;

    #[test]
    fn icons_come_from_the_theme_then_its_parents_then_hicolor_best_size_first() {
        let base = tree(
            "chain",
            &[
                (
                    "Child/index.theme",
                    "[Icon Theme]\nInherits=Parent, Child\nDirectories=48,16\n[48]\nSize=48\nType=Fixed\n[16]\nSize=16\nType=Fixed\n",
                ),
                ("Child/48/a-symbolic.svg", "big"),
                // Not in the index: still found, and before the parent's copy.
                ("Child/symbolic/ui/d-symbolic.svg", SQUARE),
                ("Parent/ui/d-symbolic.svg", SQUARE),
                ("Child/16/a-symbolic.svg", SQUARE),
                ("Parent/index.theme", "[Icon Theme]\nDirectories=ui\n[ui]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n"),
                ("Parent/ui/b-symbolic.svg", SQUARE),
                ("hicolor/index.theme", "[Icon Theme]\nDirectories=s\n[s]\nSize=16\nType=Scalable\n"),
                ("hicolor/s/c-symbolic.svg", SQUARE),
            ],
        );
        let theme = IconTheme::load("Child", std::slice::from_ref(&base)).expect("the theme loads");
        assert_eq!(theme.find_svg("a-symbolic"), Some(base.join("Child/16/a-symbolic.svg")), "16 px before 48 px");
        assert_eq!(theme.find_svg("b-symbolic"), Some(base.join("Parent/ui/b-symbolic.svg")));
        assert_eq!(theme.find_svg("c-symbolic"), Some(base.join("hicolor/s/c-symbolic.svg")));
        assert_eq!(theme.find_svg("d-symbolic"), Some(base.join("Child/symbolic/ui/d-symbolic.svg")), "unlisted, but the theme's own");
        assert_eq!(theme.find_svg("missing"), None);
        assert_eq!(theme.find_svg("../Parent/ui/b-symbolic"), None);
        assert!(IconTheme::load("Nowhere", std::slice::from_ref(&base)).is_some(), "hicolor still answers");
        assert!(IconTheme::load("Nowhere", &[base.join("empty")]).is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    /// Hostile theme files: cycles, escapes and junk sizes end the search, never hang or panic.
    #[test]
    fn cyclic_or_hostile_indexes_are_harmless() {
        let base = tree(
            "hostile",
            &[
                (
                    "A/index.theme",
                    "[Icon Theme]\nInherits=B,../etc,\nDirectories=../../x,,/abs,ok\n[ok]\nSize=99999999999999999999\nScale=-3\nType=Threshold\n",
                ),
                ("A/ok/x.svg", SQUARE),
                ("B/index.theme", "[Icon Theme]\nInherits=A\n"),
            ],
        );
        let theme = IconTheme::load("A", std::slice::from_ref(&base)).expect("loads");
        assert_eq!(theme.find_svg("x"), Some(base.join("A/ok/x.svg")));
        assert!(IconTheme::load("..", std::slice::from_ref(&base)).is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn sections_are_read_once_and_the_first_of_a_name_wins() {
        let all = sections("stray=1\n[A]\nSize=16\n[B]\nSize=22\n[A]\nSize=48\n");
        assert_eq!(all["A"], vec![("Size", "16")]);
        assert_eq!(all["B"], vec![("Size", "22")]);
        assert!(!all.contains_key(""));
    }

    #[test]
    fn distance_follows_the_spec() {
        assert_eq!(distance(&[("Size", "16"), ("Type", "Fixed")]), (0, 1));
        assert_eq!(distance(&[("Size", "22"), ("Type", "Fixed")]), (6, 1));
        assert_eq!(distance(&[("Size", "16"), ("Scale", "2"), ("Type", "Fixed")]), (16, 2));
        assert_eq!(distance(&[("Size", "32"), ("MinSize", "8"), ("MaxSize", "512"), ("Type", "Scalable")]), (0, 1));
        assert_eq!(distance(&[("Size", "48"), ("Type", "Scalable")]), (32, 1));
        assert_eq!(distance(&[("Size", "18")]), (0, 1), "Threshold 2 by default");
    }

    #[test]
    fn base_dirs_follow_xdg() {
        let var = |k: &str| match k {
            "HOME" => Some("/home/u".to_string()),
            "XDG_DATA_DIRS" => Some("/a:rel:/b".to_string()),
            _ => None,
        };
        let dirs = base_dirs(&var);
        let want: Vec<PathBuf> = ["/home/u/.icons", "/home/u/.local/share/icons", "/a/icons", "/b/icons"].iter().map(PathBuf::from).collect();
        assert_eq!(dirs, want);
        assert_eq!(base_dirs(&|_| None), vec![PathBuf::from("/usr/local/share/icons"), PathBuf::from("/usr/share/icons")]);
    }
}
