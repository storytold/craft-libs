//! For a build script: the generated lists of a family's changes. Each change is one `.rs` file in
//! the family's folder, in `v<N>/` once a release fixed its number, else in `pending/`; the lists
//! are generated so that a change adds a file and never edits a line another change also edits.
//!
//! The generated code, for the app to `include!`, is per family a `FOLDED` list (each folded
//! format and its parts) and a `PENDING` list, both prefixed by the family's `list_prefix`, then a
//! module per change file. It names the app's `Part` type, which the including module has in scope
//! (`type Part = craft_format::Part<AppError>;`), and a change file's step as `step` (a
//! `pub fn step(v: &mut Value) -> Result<(), AppError>`, when the file has one).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A family's change files: their folder (relative to the crate's manifest), the last format
/// before them, and the prefix of the family's lists.
pub struct Family {
    pub folder: &'static str,
    pub base: u32,
    pub list_prefix: &'static str,
}

/// What [`lists`] made: the code to write, and the folders the build script reruns on.
pub struct Lists {
    pub code: String,
    pub watch: Vec<PathBuf>,
}

/// The `FOLDED` and `PENDING` lists of every family, and a module for each change file. Refuses
/// versions that are not contiguous from a family's base, a change file not named with lowercase
/// letters, digits and dashes, and one that doesn't start with a `//!` note.
pub fn lists(manifest: &Path, families: &[Family]) -> Result<Lists, String> {
    let mut code = String::new();
    let mut mods = String::new();
    let mut watch = Vec::new();
    for family in families {
        let root = manifest.join(family.folder);
        family_lists(family, &root, &mut code, &mut mods)?;
        watch.push(root);
    }
    code.push_str(&mods);
    Ok(Lists { code, watch })
}

/// A family's `FOLDED` and `PENDING` lists; its folded formats follow on from its base.
fn family_lists(family: &Family, root: &Path, out: &mut String, mods: &mut String) -> Result<(), String> {
    let Family { folder, base, list_prefix } = family;
    let mut folded: Vec<(u32, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
        // an entry that can't be read could be a version folder: fail rather than drop it
        let path = entry.map_err(|e| format!("{}: {e}", root.display()))?.path();
        let version = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix('v')).and_then(|n| n.parse().ok());
        if let Some(version) = version
            && path.is_dir()
        {
            folded.push((version, path));
        }
    }
    folded.sort();
    for (i, (version, _)) in folded.iter().enumerate() {
        let expected = base + 1 + i as u32;
        if *version != expected {
            return Err(format!("{folder}/: expected v{expected}/ next, found v{version}/ (versions are contiguous)"));
        }
    }
    // Module names start with the family's (`drawing_pending_…`), so two families' changes of one
    // name never clash.
    let module = list_prefix.to_ascii_lowercase();
    let _ = writeln!(
        out,
        "/// The changes of each format after the base, oldest first (`{folder}/v<N>/`).\nconst {list_prefix}FOLDED: &[(u32, &[Part])] = &["
    );
    for (version, dir) in &folded {
        let _ = writeln!(out, "    ({version}, &[");
        parts(dir, &format!("{module}v{version}"), false, out, mods)?;
        out.push_str("    ]),\n");
    }
    let _ = writeln!(out, "];\n\n/// The changes no release has a number for yet (`{folder}/pending/`).\nconst {list_prefix}PENDING: &[Part] = &[");
    parts(&root.join("pending"), &format!("{module}pending"), true, out, mods)?;
    out.push_str("];\n\n");
    Ok(())
}

/// Whether a change file declares its `step` function (`fn step(`, `pub fn step (` …) on a line of
/// code: a mention in a comment or a note is not a declaration.
fn declares_step(text: &str) -> bool {
    let code = without_comments(text);
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    // `fn`, whitespace, `step`, then `(` (or generics), across lines
    code.match_indices("fn").any(|(at, _)| {
        let before_ok = code[..at].chars().next_back().is_none_or(|c| !ident(c));
        let rest = &code[at + 2..];
        before_ok
            && rest.starts_with(char::is_whitespace)
            && rest
                .trim_start()
                .strip_prefix("step")
                .is_some_and(|rest| matches!(rest.trim_start().chars().next(), Some('(' | '<')) && !rest.starts_with(ident))
    })
}

/// `text` as code only: its `//` and (nested) `/* */` comments become a space, and the contents
/// of its string literals (plain, byte and raw) are dropped, line breaks kept throughout. A comment
/// marker inside a string is not a comment, and `fn/* note */step` still reads as `fn step`.
fn without_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut depth) = (0usize, 0usize);
    while let Some(c) = at(i) {
        let next = at(i + 1);
        if depth > 0 {
            match (c, next) {
                ('/', Some('*')) => (depth, i) = (depth + 1, i + 2),
                ('*', Some('/')) => (depth, i) = (depth - 1, i + 2),
                _ => {
                    if c == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            if depth == 0 {
                out.push(' ');
            }
            continue;
        }
        let ident_before = i > 0 && at(i - 1).is_some_and(|p| p.is_alphanumeric() || p == '_');
        match (c, next) {
            ('/', Some('/')) => {
                while at(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
                out.push(' ');
            }
            ('/', Some('*')) => (depth, i) = (1, i + 2),
            ('"', _) => {
                out.push('"');
                i += 1;
                while let Some(c) = at(i) {
                    i += 1;
                    match c {
                        '\\' => i += 1,
                        '"' => break,
                        '\n' => out.push('\n'),
                        _ => {}
                    }
                }
                out.push('"');
            }
            // a raw string, `r"…"` or `r#"…"#`, after a byte prefix or on its own
            ('r', Some('"' | '#'))
                if !ident_before || (i > 0 && at(i - 1) == Some('b') && !(i > 1 && at(i - 2).is_some_and(|p| p.is_alphanumeric() || p == '_'))) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while at(j) == Some('#') {
                    hashes += 1;
                    j += 1;
                }
                if at(j) != Some('"') {
                    out.push(c);
                    i += 1;
                    continue;
                }
                out.push('"');
                i = j + 1;
                while let Some(c) = at(i) {
                    if c == '"' && (1..=hashes).all(|k| at(i + k) == Some('#')) {
                        i += 1 + hashes;
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                out.push('"');
            }
            // a character literal (`'"'`, `'\''`), not a lifetime
            ('\'', Some('\\')) => {
                i += 2;
                while at(i).is_some_and(|c| c != '\'') {
                    i += 1;
                }
                i += 1;
                out.push_str("' '");
            }
            ('\'', Some(_)) if at(i + 2) == Some('\'') => {
                i += 3;
                out.push_str("' '");
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The `Part`s of the change files in `dir`, in file-name order, and a module for each. A missing
/// folder is an empty list only where it may be absent (`optional`: `pending/`); any other error
/// reading it fails the build rather than leave a change out of the lists.
fn parts(dir: &Path, prefix: &str, optional: bool, out: &mut String, mods: &mut String) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if optional && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().is_some_and(|x| x == "rs") {
            files.push(path);
        }
    }
    files.sort();
    for path in files {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(format!("{}: name a format change file with lowercase letters, digits and dashes", path.display()));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !text.trim_start().starts_with("//!") {
            return Err(format!("{}: start a format change with a `//!` note", path.display()));
        }
        let module = format!("{prefix}_{}", name.replace('-', "_"));
        let step = if declares_step(&text) { format!("Some({module}::step)") } else { "None".into() };
        let _ = writeln!(out, "        Part {{ name: {name:?}, step: {step} }},");
        let _ = writeln!(mods, "#[path = {:?}]\nmod {module};", path.display().to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
