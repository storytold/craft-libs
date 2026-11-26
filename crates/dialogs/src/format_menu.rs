//! The Format menu of a macOS Save panel with two or more file types: which types it lists, which one is chosen
//! when the panel opens, the name a choice gives the file and the extensions the panel then allows. Plain rules on
//! names and filters, so they are tested on every platform; `macos.rs` shows the menu.

use std::path::Path;

use crate::Filter;
use crate::service::normal;

/// The indices into `filters` of the file types the menu lists, in order: types with an extension other than `*`.
/// Empty when fewer than two remain: the panel then has no menu.
pub(crate) fn entries(filters: &[Filter]) -> Vec<usize> {
    let listed: Vec<usize> = filters
        .iter()
        .enumerate()
        .filter(|(_, f)| f.extensions.iter().any(|e| !normal(e).is_empty()) && !f.extensions.iter().any(|e| normal(e) == "*"))
        .map(|(at, _)| at)
        .collect();
    if listed.len() < 2 { Vec::new() } else { listed }
}

/// The menu's item titles for `entries`, one per entry and all different: a pop-up menu keeps one item per title, so
/// a name that repeats gets its extensions after it (`Document (docx)`), and a title that still matches an earlier
/// title or another type's name gets a number after it.
pub(crate) fn titles(filters: &[Filter], entries: &[usize]) -> Vec<String> {
    let names: Vec<&str> = entries.iter().map(|&at| filters.get(at).map_or("", |f| f.name.as_str())).collect();
    let mut out: Vec<String> = Vec::new();
    for (&at, name) in entries.iter().zip(&names) {
        let mut base = (*name).to_owned();
        if names.iter().filter(|n| *n == name).count() > 1 {
            let extensions: Vec<String> = filters.get(at).map(|f| f.extensions.iter().map(|e| normal(e)).collect()).unwrap_or_default();
            base = format!("{name} ({})", extensions.join(", "));
        }
        let taken = |title: &str| out.iter().any(|t| t == title) || (title != *name && names.contains(&title));
        let mut title = base.clone();
        let mut number = 2;
        while taken(&title) {
            title = format!("{base} {number}");
            number += 1;
        }
        out.push(title);
    }
    out
}

/// The file type a Save answer reports, as an index into `filters`: the one chosen in the menu (menu position
/// `chosen`), unless the name the panel returned ends in an extension that only another listed type has: the user
/// typed it, and the file is saved in that type. `kept` is the name the field still showed when the last choice could
/// not rename it. A returned name equal to it that still has the extension of the name the panel opened with
/// (`opened_with`) keeps the chosen type: the save flow gives a name with the opened extension the chosen type's
/// extension (`with_chosen_type`). Any other extension stays on the name, so the type reported is the one the name has.
pub(crate) fn answered_type(
    returned: &str,
    kept: Option<&str>,
    opened_with: Option<&str>,
    filters: &[Filter],
    entries: &[usize],
    chosen: usize,
) -> Option<usize> {
    let chosen = entries.get(chosen).copied()?;
    let Some(ext) = extension(returned) else { return Some(chosen) };
    let kept_opened = kept == Some(returned) && opened_with.and_then(extension).as_ref() == Some(&ext);
    if filters.get(chosen).is_some_and(|f| lists(f, &ext)) || kept_opened {
        return Some(chosen);
    }
    Some(entries.iter().copied().find(|&at| filters.get(at).is_some_and(|f| lists(f, &ext))).unwrap_or(chosen))
}

/// The menu position chosen when the panel opens: the type whose extensions include `name`'s, else the first.
pub(crate) fn initial(name: &str, filters: &[Filter], entries: &[usize]) -> usize {
    let Some(ext) = extension(name) else { return 0 };
    entries.iter().position(|&at| filters.get(at).is_some_and(|f| lists(f, &ext))).unwrap_or(0)
}

/// The name the panel shows once `chosen` is picked: an extension `chosen` lists stays; one that another of
/// `filters` lists becomes `chosen`'s first; any other is part of the name (`art.v2`), and `chosen`'s first extension
/// is added after it. An empty name stays empty.
pub(crate) fn renamed(name: &str, chosen: &Filter, filters: &[Filter]) -> String {
    let Some(first) = chosen.extensions.iter().map(|e| normal(e)).find(|e| !e.is_empty()) else { return name.to_owned() };
    if name.is_empty() {
        return String::new();
    }
    match extension(name) {
        Some(ext) if lists(chosen, &ext) => name.to_owned(),
        Some(ext) if filters.iter().any(|f| lists(f, &ext)) => Path::new(name).with_extension(first).to_string_lossy().into_owned(),
        _ => format!("{name}.{first}"),
    }
}

/// The extensions the panel allows with menu position `selected` chosen: that type's first, so that AppKit adds it
/// to a name typed without one, then the other listed types', so that a name typed with one of theirs is kept.
pub(crate) fn allowed(filters: &[Filter], entries: &[usize], selected: usize) -> Vec<String> {
    let chosen = entries.get(selected).copied();
    let order = chosen.into_iter().chain(entries.iter().copied().filter(|&at| Some(at) != chosen));
    let mut out: Vec<String> = Vec::new();
    for ext in order.filter_map(|at| filters.get(at)).flat_map(|f| f.extensions.iter().map(|e| normal(e))) {
        if !ext.is_empty() && !out.contains(&ext) {
            out.push(ext);
        }
    }
    out
}

/// `name`'s extension as compared, if it has one.
fn extension(name: &str) -> Option<String> {
    Path::new(name).extension().and_then(|e| e.to_str()).map(normal)
}

/// Whether `filter` lists `ext` (already [`normal`]).
fn lists(filter: &Filter, ext: &str) -> bool {
    filter.extensions.iter().any(|e| normal(e) == ext)
}

#[cfg(test)]
pub(crate) mod tests;
