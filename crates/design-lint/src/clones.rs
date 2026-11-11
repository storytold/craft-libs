//! The clone detector: token-level copies across the source, counted per family against a baseline.
//!
//! Files are reduced to a normalised token stream (identifiers and literals blur into one token each,
//! doc comments and `#[cfg(test)]` code are dropped). A window of [`WINDOW`] tokens that occurs again, in
//! another file or without overlap in the same file, marks its tokens as duplicated. A family's count is
//! the duplicated tokens in its files. `design/clones/<crate>/<family>.toml` records the count:
//!
//! ```toml
//! paths = ["crates/ui/src/buttons"]   # files and directories of the family, relative to the root
//! duplicated-tokens = 0               # the baseline; 0 when omitted
//! ```
//!
//! A count above the baseline is `clone/baseline`; below it, `clone/slack` (lower the baseline, so a fall
//! is recorded and a later rise fails).

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens as _;
use serde::Deserialize;
use syn::visit_mut::{self, VisitMut};

use crate::Error;
use crate::layers::{finding, read_toml, relative, sorted_entries};
use crate::report::Finding;
use crate::rules::is_test_attr;

/// A family has more duplicated tokens than its baseline.
pub const BASELINE: &str = "clone/baseline";
/// A family has fewer duplicated tokens than its baseline.
pub const SLACK: &str = "clone/slack";

/// Tokens a repeated run must span to count as a clone; about three statements.
pub const WINDOW: usize = 40;

const KEYWORDS: [&str; 34] = [
    "as", "async", "await", "break", "const", "continue", "else", "enum", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "Self", "self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "dyn",
];

/// A set of files whose duplication is counted together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Family {
    /// The crate directory name under `design/clones`.
    pub krate: String,
    /// The family name (the file stem).
    pub name: String,
    /// Files and directories, relative to the root.
    pub paths: Vec<PathBuf>,
    /// The recorded count of duplicated tokens.
    pub baseline: usize,
    /// The baseline file, relative to the root.
    pub file: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFamily {
    paths: Vec<PathBuf>,
    #[serde(default, rename = "duplicated-tokens")]
    duplicated_tokens: usize,
}

/// Reads `dir` (`design/clones`); a missing directory holds no families.
pub fn load_families(root: &Path, dir: &Path) -> Result<Vec<Family>, Error> {
    let mut families = Vec::new();
    for crate_dir in sorted_entries(dir)?.into_iter().filter(|p| p.is_dir()) {
        let krate = crate_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        for file in sorted_entries(&crate_dir)?.into_iter().filter(|p| p.extension().is_some_and(|e| e == "toml")) {
            let raw: RawFamily = read_toml(&file)?;
            let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
            families.push(Family { krate: krate.clone(), name, paths: raw.paths, baseline: raw.duplicated_tokens, file: relative(root, &file) });
        }
    }
    Ok(families)
}

/// Compares each family's duplicated tokens with its baseline.
pub fn check_clones(root: &Path, families: &[Family]) -> Result<Vec<Finding>, Error> {
    let mut files: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (i, family) in families.iter().enumerate() {
        for path in &family.paths {
            let mut found = Vec::new();
            crate::collect_rust_files(&root.join(path), &mut found)?;
            for file in found.into_iter().filter(|f| !crate::is_test_path(f.strip_prefix(root).unwrap_or(f))) {
                files.entry(file).or_default().push(i);
            }
        }
    }
    let mut streams = Vec::new();
    for file in files.keys() {
        streams.push(tokens_of(file)?);
    }
    let duplicated = duplicated_tokens(&streams);
    let mut counts = vec![0_usize; families.len()];
    for ((_, owners), n) in files.iter().zip(duplicated) {
        for &i in owners {
            counts[i] += n;
        }
    }
    Ok(families.iter().zip(counts).filter_map(|(family, now)| ratchet_finding(family, now)).collect())
}

fn ratchet_finding(family: &Family, now: usize) -> Option<Finding> {
    let name = format!("{}/{}", family.krate, family.name);
    let (rule, message) = match now.cmp(&family.baseline) {
        std::cmp::Ordering::Equal => return None,
        std::cmp::Ordering::Greater => {
            (BASELINE, format!("family `{name}` has {now} duplicated tokens, above its baseline {}: remove the copies", family.baseline))
        }
        std::cmp::Ordering::Less => {
            (SLACK, format!("family `{name}` has {now} duplicated tokens, below its baseline {}: lower the baseline to {now}", family.baseline))
        }
    };
    Some(finding(rule, family.file.clone(), message))
}

/// Per stream, how many tokens lie in a window that occurs elsewhere.
pub(crate) fn duplicated_tokens(streams: &[Vec<u64>]) -> Vec<usize> {
    let mut seen: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (f, stream) in streams.iter().enumerate() {
        for (pos, window) in stream.windows(WINDOW).enumerate() {
            let mut hasher = DefaultHasher::new();
            window.hash(&mut hasher);
            seen.entry(hasher.finish()).or_default().push((f, pos));
        }
    }
    let mut covered: Vec<Vec<bool>> = streams.iter().map(|s| vec![false; s.len()]).collect();
    for places in seen.values().filter(|p| p.len() > 1) {
        for &(f, pos) in places {
            let repeated = places.iter().any(|&(g, other)| g != f || other.abs_diff(pos) >= WINDOW);
            if repeated {
                covered[f][pos..pos + WINDOW].fill(true);
            }
        }
    }
    covered.iter().map(|c| c.iter().filter(|&&b| b).count()).collect()
}

fn tokens_of(file: &Path) -> Result<Vec<u64>, Error> {
    let src = std::fs::read_to_string(file).map_err(|source| Error::Io { path: file.to_owned(), source })?;
    let mut parsed = syn::parse_file(&src).map_err(|e| Error::Io {
        path: file.to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, format!("does not parse: {e}")),
    })?;
    Strip.visit_file_mut(&mut parsed);
    let mut out = Vec::new();
    normalise(parsed.into_token_stream(), &mut out);
    Ok(out)
}

/// Drops doc attributes and test code.
struct Strip;

impl VisitMut for Strip {
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        file.attrs.retain(|a| !a.path().is_ident("doc"));
        file.items.retain(|i| !is_test_item(i));
        visit_mut::visit_file_mut(self, file);
    }

    fn visit_item_mod_mut(&mut self, item: &mut syn::ItemMod) {
        if let Some((_, items)) = &mut item.content {
            items.retain(|i| !is_test_item(i));
        }
        visit_mut::visit_item_mod_mut(self, item);
    }

    fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
        item.items.retain(|i| !matches!(i, syn::ImplItem::Fn(f) if f.attrs.iter().any(is_test_attr)));
        visit_mut::visit_item_impl_mut(self, item);
    }
}

fn is_test_item(item: &syn::Item) -> bool {
    let attrs = match item {
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        _ => return false,
    };
    attrs.iter().any(is_test_attr)
}

fn code(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn normalise(stream: TokenStream, out: &mut Vec<u64>) {
    let mut tokens = stream.into_iter().peekable();
    while let Some(tt) = tokens.next() {
        match tt {
            TokenTree::Ident(i) => {
                let text = i.to_string();
                out.push(code(if KEYWORDS.contains(&text.as_str()) { &text } else { "<ident>" }));
            }
            TokenTree::Literal(_) => out.push(code("<literal>")),
            TokenTree::Punct(p) => {
                if p.as_char() == '#' && is_doc_attribute(tokens.peek()) {
                    tokens.next();
                    continue;
                }
                out.push(code(&p.as_char().to_string()));
            }
            TokenTree::Group(g) => {
                let (open, close) = match g.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                out.push(code(open));
                normalise(g.stream(), out);
                out.push(code(close));
            }
        }
    }
}

/// Whether the token after a `#` is the bracket group of a `doc` attribute.
fn is_doc_attribute(next: Option<&TokenTree>) -> bool {
    matches!(next, Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket
        && matches!(g.stream().into_iter().next(), Some(TokenTree::Ident(i)) if i == "doc"))
}
