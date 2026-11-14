//! The shortcut summary behind Summarize…: every command that has a shortcut, grouped as in the
//! dialog's All Commands view, as an HTML page and as plain text. Apps can also call these for a
//! cheat sheet outside the dialog.

use std::fmt::Write as _;

use crate::keymap::{Binding, Keymap};
use crate::scope::Scope;
use crate::ui::rows::{AreaId, group};
use crate::ui::text::{Text, trigger_text};

struct Group {
    rank: u8,
    name: String,
    /// (label, shortcuts as shown)
    rows: Vec<(String, Vec<String>)>,
}

fn binding_text(km: &Keymap, text: &dyn Text, b: &Binding, home: &Scope) -> String {
    let chord = trigger_text(text, &b.trigger, km.platform());
    match &b.scope {
        Some(s) if s != home => {
            let scope = if s.is_app() { text.ui("Application").into_owned() } else { text.segment(s.as_str()).into_owned() };
            craft_i18n::fmt(&text.ui("{chord} ({scope})"), &[("chord", &chord), ("scope", &scope)])
        }
        _ => chord,
    }
}

/// Groups in the table's order; commands in catalog order within a group.
fn groups(km: &Keymap, text: &dyn Text) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for b in km.catalog() {
        let bindings = km.bindings(&b.command);
        if bindings.is_empty() {
            continue;
        }
        let keys = bindings.iter().map(|x| binding_text(km, text, x, &b.scope)).collect();
        let (rank, name, _) = group(&AreaId::All, b, false, text);
        let name = name.unwrap_or_default();
        let row = (text.command(b).into_owned(), keys);
        match out.iter_mut().find(|g| g.rank == rank && g.name == name) {
            Some(g) => g.rows.push(row),
            None => out.push(Group { rank, name, rows: vec![row] }),
        }
    }
    out.sort_by_key(|g| g.rank);
    out
}

/// Text for HTML element content and attribute values.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// One line of plain text: control characters (line breaks, tabs) become spaces, so a label
/// cannot break the layout.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

const CSS: &str = "body{font-family:system-ui,sans-serif;margin:2em;color:#111;background:#fff}\
h1{font-size:1.4em}h2{font-size:1.1em;margin-top:1.5em;border-bottom:1px solid #ccc}\
table{border-collapse:collapse;width:100%}th,td{padding:.2em .6em;vertical-align:top}\
th{font-weight:normal;text-align:start;width:60%}tr:nth-child(even){background:#f4f4f4}\
kbd{font-family:inherit;border:1px solid #bbb;border-radius:3px;padding:0 .3em;margin-inline-end:.3em;white-space:nowrap}\
@media print{body{margin:0}tr{break-inside:avoid}}";

/// A standalone HTML page titled `title`. Chords are marked left to right so a right-to-left page
/// keeps them as written.
pub fn html(km: &Keymap, text: &dyn Text, title: &str) -> String {
    let dir = if text.rtl() { "rtl" } else { "ltr" };
    let title = escape_html(title);
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html>\n<html dir=\"{dir}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n<style>{CSS}</style>\n</head>\n<body>\n<h1>{title}</h1>\n"
    );
    for g in groups(km, text) {
        let _ = writeln!(s, "<h2>{}</h2>\n<table>", escape_html(&g.name));
        for (label, keys) in &g.rows {
            let keys: String = keys.iter().map(|k| format!("<kbd dir=\"ltr\">{}</kbd>", escape_html(k))).collect();
            let _ = writeln!(s, "<tr><th scope=\"row\">{}</th><td>{keys}</td></tr>", escape_html(label));
        }
        s.push_str("</table>\n");
    }
    s.push_str("</body>\n</html>\n");
    s
}

/// Plain text titled `title`: a line per group, then `  label<TAB>shortcuts` per command.
pub fn text(km: &Keymap, text: &dyn Text, title: &str) -> String {
    let mut s = one_line(title);
    s.push('\n');
    for g in groups(km, text) {
        let _ = write!(s, "\n{}\n", one_line(&g.name));
        for (label, keys) in &g.rows {
            let _ = writeln!(s, "  {}\t{}", one_line(label), one_line(&keys.join(", ")));
        }
    }
    s
}
