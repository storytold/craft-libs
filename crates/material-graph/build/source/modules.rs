//! Which files a part of the program can reach at all, by the module paths its code spells (`crate::tex`, `super::x`,
//! `use crate::paint::{par, other}`, `mod child;`). Names are followed by name within those files only: without this, a
//! `.parse()` in the graph engine would reach every function called `parse` in the whole program, and every kind would
//! depend on everything. Sound because code can only name what its module paths lead to, and a value of a type from
//! elsewhere arrives through a function whose file names that type's module.

use super::SourceFile;
use super::items::past_item;
use super::tokens::{Token, tokenize};
use std::collections::{BTreeMap, BTreeSet};

/// Tokens of `#[cfg(test)]`.
const TEST_ATTRIBUTE: usize = 7;

fn is_test_attribute(tokens: &[Token], at: usize) -> bool {
    let is_punct = |k: usize, c: char| tokens.get(at + k).is_some_and(|t| t.is_punct(c));
    is_punct(0, '#')
        && is_punct(1, '[')
        && tokens.get(at + 2).is_some_and(|t| t.is("cfg"))
        && is_punct(3, '(')
        && tokens.get(at + 4).is_some_and(|t| t.is("test"))
        && is_punct(5, ')')
        && is_punct(6, ']')
}

/// A folder of modules and how a file sits in it.
fn children_folder(file: &str) -> String {
    let path = file.trim_end_matches(".rs");
    match path.rsplit_once('/') {
        Some((folder, "mod" | "lib" | "main")) => folder.to_string(),
        _ => path.to_string(),
    }
}

fn parent_folder(folder: &str) -> String {
    folder.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_string())
}

struct Lookup<'a> {
    files: &'a BTreeMap<&'a str, &'a SourceFile>,
    root: String,
}

impl Lookup<'_> {
    /// The file of module `name` in `folder`.
    fn module(&self, folder: &str, name: &str) -> Option<String> {
        let candidates = [format!("{folder}/{name}.rs"), format!("{folder}/{name}/mod.rs")];
        candidates.into_iter().find(|c| self.files.contains_key(c.as_str()))
    }

    /// The file of the module whose children are in `folder`.
    fn file_of_folder(&self, folder: &str) -> Option<String> {
        let candidates = [format!("{folder}/mod.rs"), format!("{folder}.rs"), format!("{folder}/lib.rs")];
        candidates.into_iter().find(|c| self.files.contains_key(c.as_str()))
    }

    /// Follows `segments` from `folder` as far as they are modules, into `out`: the deepest one reached, which is where
    /// the path's item is.
    fn follow(&self, folder: &str, segments: &[String], out: &mut BTreeSet<String>) {
        let mut folder = folder.to_string();
        let mut last = None;
        for s in segments {
            let Some(file) = self.module(&folder, s) else { break };
            folder = children_folder(&file);
            last = Some(file);
        }
        out.extend(last);
    }

    /// The paths in `tokens` from a `::` chain's first word, with `{a, b::c}` groups spread out.
    fn paths(&self, tokens: &[Token], at: usize, prefix: Vec<String>, out: &mut Vec<Vec<String>>) -> usize {
        let mut path = prefix;
        let mut i = at;
        loop {
            match tokens.get(i) {
                Some(Token::Ident(name)) => {
                    path.push(name.clone());
                    i += 1;
                }
                Some(t) if t.is_punct('{') => {
                    i += 1;
                    while tokens.get(i).is_some_and(|t| !t.is_punct('}')) {
                        i = self.paths(tokens, i, path.clone(), out);
                        if tokens.get(i).is_some_and(|t| t.is_punct(',')) {
                            i += 1;
                        }
                    }
                    return i + 1;
                }
                _ => break,
            }
            let more = tokens.get(i).is_some_and(|t| t.is_punct(':')) && tokens.get(i + 1).is_some_and(|t| t.is_punct(':'));
            if !more {
                break;
            }
            i += 2;
        }
        out.push(path);
        i.max(at + 1)
    }

    fn references(&self, file: &SourceFile) -> BTreeSet<String> {
        let tokens = tokenize(&file.text);
        let own = children_folder(&file.name);
        let parent = parent_folder(&own);
        let mut out = BTreeSet::new();
        let mut i = 0;
        while let Some(t) = tokens.get(i) {
            // test code doesn't run in a result, and a test module's `super` would reach the whole crate
            if is_test_attribute(&tokens, i) {
                i = past_item(&tokens, i + TEST_ATTRIBUTE, true);
                continue;
            }
            let Some(word) = t.ident() else {
                i += 1;
                continue;
            };
            if word == "mod" {
                if let Some(name) = tokens.get(i + 1).and_then(Token::ident).filter(|_| tokens.get(i + 2).is_some_and(|n| n.is_punct(';'))) {
                    out.extend(self.module(&own, name));
                }
                i += 2;
                continue;
            }
            let chained = tokens.get(i + 1).is_some_and(|n| n.is_punct(':')) && tokens.get(i + 2).is_some_and(|n| n.is_punct(':'));
            if !chained {
                i += 1;
                continue;
            }
            let mut paths = Vec::new();
            let next = self.paths(&tokens, i, Vec::new(), &mut paths);
            for path in &paths {
                let Some((first, rest)) = path.split_first() else { continue };
                match first.as_str() {
                    "crate" => self.follow(&self.root, rest, &mut out),
                    "self" => self.follow(&own, rest, &mut out),
                    "super" => {
                        let mut folder = parent.clone();
                        let mut rest = rest;
                        while let Some(("super", more)) = rest.split_first().map(|(a, b)| (a.as_str(), b)) {
                            folder = parent_folder(&folder);
                            rest = more;
                        }
                        // `super::helper()` and `use super::*` name what is in the parent's own file
                        out.extend(self.file_of_folder(&folder));
                        self.follow(&folder, rest, &mut out);
                    }
                    // a child of this module, or a sibling a `use` brought in under that name
                    _ => {
                        self.follow(&own, path, &mut out);
                        self.follow(&parent, path, &mut out);
                    }
                }
            }
            i = next.max(i + 1);
        }
        out
    }
}

/// The files of `files` reachable from `start` (file names), all of them in one crate with its modules under `root`.
pub fn reachable<'a>(files: &'a [SourceFile], root: &str, start: &[&str]) -> Vec<&'a SourceFile> {
    let by_name: BTreeMap<&str, &SourceFile> = files.iter().map(|f| (f.name.as_str(), f)).collect();
    let lookup = Lookup { files: &by_name, root: root.to_string() };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = start.iter().map(|s| (*s).to_string()).collect();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(file) = by_name.get(name.as_str()) {
            queue.extend(lookup.references(file));
        }
    }
    files.iter().filter(|f| seen.contains(&f.name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> SourceFile {
        SourceFile { name: name.into(), text: text.into(), included: Vec::new() }
    }

    fn names(files: &[SourceFile], start: &[&str]) -> Vec<String> {
        reachable(files, "src", start).iter().map(|f| f.name.clone()).collect()
    }

    #[test]
    fn follows_paths_and_groups_and_stops_at_what_is_not_named() {
        let files = [
            file("src/a.rs", "use crate::b::f; fn x() { crate::c::g(); }"),
            file("src/b.rs", "use crate::{d, e::{h, i}};"),
            file("src/c.rs", ""),
            file("src/d.rs", ""),
            file("src/e/mod.rs", "mod h; mod i;"),
            file("src/e/h.rs", ""),
            file("src/e/i.rs", ""),
            file("src/e/j.rs", ""),
            file("src/far.rs", ""),
        ];
        assert_eq!(names(&files, &["src/a.rs"]), ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs", "src/e/h.rs", "src/e/i.rs"]);
    }

    #[test]
    fn a_deep_path_does_not_bring_the_whole_module() {
        let files = [
            file("src/a.rs", "fn x() { crate::big::small::go(); }"),
            file("src/big/mod.rs", "pub mod small; pub mod huge;"),
            file("src/big/small.rs", ""),
            file("src/big/huge.rs", ""),
        ];
        assert_eq!(names(&files, &["src/a.rs"]), ["src/a.rs", "src/big/small.rs"]);
    }

    #[test]
    fn a_reexport_brings_the_module_with_its_children() {
        let files =
            [file("src/a.rs", "use crate::big::Thing;"), file("src/big/mod.rs", "mod small; pub use small::Thing;"), file("src/big/small.rs", "")];
        assert_eq!(names(&files, &["src/a.rs"]), ["src/a.rs", "src/big/mod.rs", "src/big/small.rs"]);
    }

    #[test]
    fn a_child_reaches_what_it_uses_of_its_parent() {
        let files = [
            file("src/a.rs", "fn x() { crate::big::small::go(); crate::other::child::go(); }"),
            file("src/big/mod.rs", "pub fn helper() {}"),
            file("src/big/small.rs", "fn go() { super::helper(); }"),
            file("src/other.rs", "mod child; pub fn helper() {}"),
            file("src/other/child.rs", "use super::*; fn go() { helper(); }"),
        ];
        assert_eq!(names(&files, &["src/a.rs"]), ["src/a.rs", "src/big/mod.rs", "src/big/small.rs", "src/other.rs", "src/other/child.rs"]);
    }

    #[test]
    fn test_code_reaches_nothing() {
        let files = [
            file("src/lib.rs", "mod a; mod far;"),
            file("src/a.rs", "fn x() {} #[cfg(test)] mod tests { use super::*; use crate::far; }"),
            file("src/far.rs", ""),
        ];
        assert_eq!(names(&files, &["src/a.rs"]), ["src/a.rs"]);
    }

    #[test]
    fn relative_paths() {
        let files = [
            file("src/m/mod.rs", "mod child; use self::child::f;"),
            file("src/m/child.rs", "use super::sister::g; use super::super::top;"),
            file("src/m/sister.rs", ""),
            file("src/top.rs", ""),
        ];
        assert_eq!(names(&files, &["src/m/mod.rs"]).len(), 4);
    }
}
