//! The font folders named by the fontconfig configuration, read with a small tag scanner (no
//! fontconfig library, no XML crate): only `<dir>`, `<remap-dir>` and `<include>` matter here.
//! The work is bounded: include depth, number and size of the files read, the folder entries
//! listed for includes, and every file is read and every folder listed at most once.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::FontEnv;

const MAX_INCLUDE_DEPTH: u32 = 8;
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Folder entries listed for `<include>` folders, all folders together.
const MAX_DIR_ENTRIES: usize = 16_384;

/// The folders the fontconfig configuration for `env` names, in the order it names them.
///
/// The main file is `FONTCONFIG_FILE`, else the first of `/etc/fonts/fonts.conf` and
/// `/usr/local/etc/fonts/fonts.conf` (FreeBSD) that exists. The user's own configuration
/// (`$XDG_CONFIG_HOME/fontconfig/fonts.conf` and `conf.d`) is read too, in case the main file
/// doesn't include it. In a Flatpak sandbox ([`FontEnv::flatpak`]) the host's folders are added:
/// those `/run/host/font-dirs.xml` maps, and those of the host's own configuration when the
/// sandbox can see it (`/run/host/etc/fonts/fonts.conf`), under `/run/host`.
pub fn fontconfig_dirs(env: &FontEnv) -> Vec<PathBuf> {
    let mut reader = Reader { env, out: Vec::new(), seen: HashSet::new(), dirs: HashSet::new(), entries_left: MAX_DIR_ENTRIES, host: None };
    let root = match &env.fontconfig_file {
        Some(f) => Some(f.clone()),
        None => ["/etc/fonts/fonts.conf", "/usr/local/etc/fonts/fonts.conf"].map(|p| env.rooted(p)).into_iter().find(|p| p.is_file()),
    };
    if let Some(root) = root {
        reader.file(&root, 0);
    }
    if let Some(c) = env.config_home() {
        reader.include(&c.join("fontconfig/fonts.conf"), 0);
        reader.include(&c.join("fontconfig/conf.d"), 0);
    }
    if env.flatpak {
        reader.file(&env.rooted("/run/host/font-dirs.xml"), 0);
        reader.host = Some(env.rooted("/run/host"));
        reader.file(&env.rooted("/run/host/etc/fonts/fonts.conf"), 0);
    }
    reader.out
}

struct Reader<'a> {
    env: &'a FontEnv,
    out: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    /// The `<include>` folders already listed (canonical paths).
    dirs: HashSet<PathBuf>,
    /// Folder entries that may still be listed ([`MAX_DIR_ENTRIES`]).
    entries_left: usize,
    /// While reading the host's configuration from a sandbox: where the host's root is.
    host: Option<PathBuf>,
}

impl Reader<'_> {
    /// Read a configuration file or, for a folder, its `NN-name.conf` files in name order.
    fn include(&mut self, path: &Path, depth: u32) {
        if path.is_dir() {
            // Check the budgets before listing: a configuration that names the same folder many
            // times, or many large folders, must not list them over and over.
            if depth > MAX_INCLUDE_DEPTH || self.seen.len() >= MAX_FILES || self.entries_left == 0 {
                return;
            }
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !self.dirs.insert(key) {
                return;
            }
            let Ok(rd) = std::fs::read_dir(path) else { return };
            let mut listed = 0;
            let mut files: Vec<PathBuf> = rd
                .take(self.entries_left)
                .inspect(|_| listed += 1)
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".conf") && n.starts_with(|c: char| c.is_ascii_digit()))
                })
                .collect();
            self.entries_left -= listed;
            files.sort();
            for f in files {
                self.file(&f, depth);
            }
        } else {
            self.file(path, depth);
        }
    }

    fn file(&mut self, path: &Path, depth: u32) {
        if depth > MAX_INCLUDE_DEPTH || self.seen.len() >= MAX_FILES || !path.is_file() {
            return;
        }
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !self.seen.insert(key) {
            return;
        }
        if std::fs::metadata(path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for el in elements(&text) {
            match el.name {
                "dir" | "remap-dir" => {
                    if let Some(p) = self.resolve(&el, &base, false) {
                        self.out.push(p);
                    }
                }
                "include" => {
                    if let Some(p) = self.resolve(&el, &base, true) {
                        self.include(&p, depth.saturating_add(1));
                    }
                }
                _ => {}
            }
        }
    }

    /// The path an element names: `~` is the home folder, `prefix="xdg"` the XDG data (folders)
    /// or config (includes) home, `prefix="relative"` and plain relative paths the including
    /// file's folder, `prefix="default"`/`"cwd"` the working folder. Absolute paths are under the
    /// root, or under the host's root while reading the host's configuration.
    fn resolve(&self, el: &Element, base: &Path, include: bool) -> Option<PathBuf> {
        let v = el.text.trim();
        if v.is_empty() {
            return None;
        }
        if let Some(rest) = v.strip_prefix('~') {
            return Some(self.env.home.as_ref()?.join(rest.trim_start_matches('/')));
        }
        let p = Path::new(v);
        if p.is_absolute() {
            return Some(match &self.host {
                Some(host) => host.join(p.strip_prefix("/").unwrap_or(p)),
                None => self.env.rooted(p),
            });
        }
        match el.attr("prefix") {
            Some("xdg") => Some(if include { self.env.config_home()? } else { self.env.data_home()? }.join(p)),
            Some("default" | "cwd") => std::env::current_dir().ok().map(|c| c.join(p)),
            _ => Some(base.join(p)),
        }
    }
}

struct Element<'a> {
    name: &'a str,
    attrs: &'a str,
    text: String,
}

impl Element<'_> {
    fn attr(&self, key: &str) -> Option<&str> {
        let mut rest = self.attrs;
        while let Some(eq) = rest.find('=') {
            let k = rest.get(..eq)?.trim();
            let after = rest.get(eq + 1..)?.trim_start();
            let q = after.chars().next()?;
            if q != '"' && q != '\'' {
                return None;
            }
            let body = after.get(1..)?;
            let end = body.find(q)?;
            if k == key {
                return body.get(..end);
            }
            rest = body.get(end + 1..)?;
        }
        None
    }
}

/// The `<dir>`, `<remap-dir>` and `<include>` elements of a fontconfig file, comments skipped.
fn elements(src: &str) -> Vec<Element<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = src.get(i..).and_then(|s| s.find('<')) {
        let start = i + off;
        let Some(tail) = src.get(start..) else { break };
        if tail.starts_with("<!--") {
            match tail.find("-->") {
                Some(e) => {
                    i = start + e + 3;
                    continue;
                }
                None => break,
            }
        }
        let Some(close) = tail.find('>') else { break };
        let inner = tail.get(1..close).unwrap_or_default();
        i = start + close + 1;
        let name_len = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
        let name = inner.get(..name_len).unwrap_or_default();
        if !matches!(name, "dir" | "remap-dir" | "include") || inner.ends_with('/') {
            continue;
        }
        let attrs = inner.get(name_len..).unwrap_or_default();
        let end_tag = format!("</{name}>");
        let Some(body_len) = src.get(i..).and_then(|s| s.find(&end_tag)) else { break };
        let body = src.get(i..i + body_len).unwrap_or_default();
        i += body_len + end_tag.len();
        out.push(Element { name, attrs, text: decode_entities(body) });
    }
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// Uses Unix paths, which Windows doesn't treat as absolute (#619).
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_folder_included_many_times_is_listed_once() {
        let root = std::env::temp_dir().join(format!("craft-font-catalog-fc-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let conf_d = root.join("conf.d");
        std::fs::create_dir_all(&conf_d).unwrap();
        for (i, dir) in ["/a", "/b", "/c"].iter().enumerate() {
            std::fs::write(conf_d.join(format!("{i}0-x.conf")), format!("<fontconfig><dir>{dir}</dir></fontconfig>")).unwrap();
        }
        let main = root.join("fonts.conf");
        std::fs::write(&main, format!("<fontconfig>{}</fontconfig>", "<include>conf.d</include>".repeat(5000))).unwrap();
        let env = FontEnv { root: root.clone(), ..Default::default() };
        let mut reader = Reader { env: &env, out: Vec::new(), seen: HashSet::new(), dirs: HashSet::new(), entries_left: MAX_DIR_ENTRIES, host: None };
        reader.file(&main, 0);
        assert_eq!(reader.out, vec![root.join("a"), root.join("b"), root.join("c")]);
        assert_eq!(reader.dirs.len(), 1);
        assert_eq!(reader.entries_left, MAX_DIR_ENTRIES - 3);
        let _ = std::fs::remove_dir_all(&root);
    }
}
