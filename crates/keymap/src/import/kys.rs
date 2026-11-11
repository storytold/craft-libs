//! Photoshop keyboard shortcut sets (`.kys`, Edit › Keyboard Shortcuts › Save Set, and the live
//! `Keyboard Shortcuts.psp`, the same XML): read into a [`ForeignSet`] for [`crate::plan`], and
//! written from a keymap for the dialog's Set row export.
//!
//! A `.kys` is `<command kind="static|dynamic" name="Gaussian Blur...">` rows, each with one or
//! more `<shortcut>Control+Opt+Cmd+G</shortcut>`, then `<tool>` keys. Photoshop names a command by
//! its menu label alone, so [`crate::plan`] matches by label. A set saved on Windows spells the
//! command modifier `Ctrl` ([`crate::Flavor::Windows`]); a Mac set's `Control` is the Control key.
//! Sets the suite writes carry each command's id (`craft-id`, which Photoshop ignores), so they
//! read back exactly.

use crate::chord::foreign::{Flavor, ForeignError};
use crate::chord::{Mods, Trigger};
use crate::import::{ForeignEntry, ForeignKey, ForeignSet};
use crate::keymap::{CommandKind, Keymap};

/// A set is a few tens of kilobytes; a larger file is not one and is not parsed.
// pc: crates/engine/src/kys.rs:26 `MAX_BYTES`.
pub const MAX_BYTES: usize = 4 << 20;

/// The attribute naming the suite's command id on the rows it writes.
// pc: crates/engine/src/kys.rs:260 `PC_ID` ("pc-id"; read too, so PhotoCraft's own sets keep
// matching exactly).
pub const ID_ATTRIBUTE: &str = "craft-id";

/// The attribute PhotoCraft wrote its command ids under; read like [`ID_ATTRIBUTE`].
const PHOTOCRAFT_ID_ATTRIBUTE: &str = "pc-id";

/// Deepest element nesting read. roxmltree recurses once per level, so a few thousand nested tags
/// overflow a thread's stack, an abort nothing can catch.
const MAX_DEPTH: usize = 64;

const ROOT: &str = "photoshop-keyboard-shortcuts";

/// Why a file isn't a readable set.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KysError {
    /// Larger than [`MAX_BYTES`].
    TooLarge,
    /// Not XML, or not a `photoshop-keyboard-shortcuts` root; the parser's message.
    NotASet(String),
}

impl std::fmt::Display for KysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KysError::TooLarge => f.write_str("The file is larger than a keyboard shortcut set can be."),
            KysError::NotASet(message) => write!(f, "The file is not a Photoshop keyboard shortcut set: {message}"),
        }
    }
}

impl std::error::Error for KysError {}

/// The set name in the root `filename` attribute (`$$$/.../{name}={name}`). The name may itself
/// contain `/` or `=`, so split on the key prefix and the doubled name, not on the last separator.
fn filename_to_name(f: &str) -> &str {
    let Some(rest) = f.strip_prefix("$$$/") else { return f.rsplit(['=', '/']).next().unwrap_or(f).trim() };
    let rest = rest.strip_prefix("FileName/Presets/KeyboardCustomization/").unwrap_or(rest);
    let mid = rest.len() / 2;
    if rest.len() % 2 == 1 && rest.is_char_boundary(mid) && rest.as_bytes()[mid] == b'=' && rest[..mid] == rest[mid + 1..] {
        return rest[..mid].trim();
    }
    rest.split_once('=').map_or(rest, |(_, n)| n).trim()
}

/// The index after the `>` that closes the tag opening at `at`, skipping a `>` inside a quoted
/// attribute value; the end of `bytes` when the tag never closes.
fn tag_end(bytes: &[u8], at: usize) -> usize {
    let mut quote = None;
    for (i, &c) in bytes.iter().enumerate().skip(at + 1) {
        match (quote, c) {
            (None, b'"' | b'\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, b'>') => return i + 1,
            _ => {}
        }
    }
    bytes.len()
}

/// Whether `text` nests elements deeper than [`MAX_DEPTH`], counted without building a tree.
fn too_deep(text: &str) -> bool {
    let bytes = text.as_bytes();
    let (mut depth, mut at) = (0usize, 0usize);
    while let Some(open) = bytes.get(at..).and_then(|b| b.iter().position(|&c| c == b'<')) {
        at += open;
        let rest = bytes.get(at + 1..).unwrap_or_default();
        let skip_to = |end: &[u8]| rest.windows(end.len()).position(|w| w == end).map_or(bytes.len(), |p| at + 1 + p + end.len());
        at = match rest.first() {
            Some(b'!') if rest.starts_with(b"!--") => skip_to(b"-->"),
            Some(b'!') if rest.starts_with(b"![CDATA[") => skip_to(b"]]>"),
            Some(b'!' | b'?') => skip_to(b">"),
            Some(b'/') => {
                depth = depth.saturating_sub(1);
                skip_to(b">")
            }
            _ => {
                let end = tag_end(bytes, at);
                let tag = bytes.get(at..end).unwrap_or_default();
                if !tag.ends_with(b"/>") {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return true;
                    }
                }
                end
            }
        };
    }
    false
}

/// Reads a `.kys` or `Keyboard Shortcuts.psp`: the set's name (from the root's `filename`,
/// `$$$/FileName/Presets/KeyboardCustomization/{name}={name}`), one [`crate::ForeignEntry`] per
/// command row with a name (its keys in the file's flavour, an id from [`ID_ATTRIBUTE`] or
/// `pc-id` as the entry's [`ForeignEntry::id`] when present), and the `<tool>` rows as tool
/// entries (a tool key with modifiers is read as an unsupported-modifier error, so [`crate::plan`] reports it unreadable and binds nothing). Rows without a name are skipped; rows without a shortcut stay, with no keys.
// pc: crates/engine/src/kys.rs:66 `parse`, :54 `filename_to_name`, :128 `to_ours`.
pub fn read(text: &str) -> Result<ForeignSet, KysError> {
    if text.len() > MAX_BYTES {
        return Err(KysError::TooLarge);
    }
    let text = text.trim_start_matches('\u{feff}');
    if too_deep(text) {
        return Err(KysError::NotASet(format!("elements are nested more than {MAX_DEPTH} levels deep")));
    }
    let doc = roxmltree::Document::parse(text).map_err(|e| KysError::NotASet(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != ROOT {
        return Err(KysError::NotASet(format!("no <{ROOT}> root")));
    }
    let rows: Vec<roxmltree::Node> = root.children().filter(roxmltree::Node::is_element).collect();
    let shortcuts = |node: roxmltree::Node| -> Vec<String> {
        match node.tag_name().name() {
            "command" => node
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "shortcut")
                .filter_map(|n| n.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
                .collect(),
            _ => node.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).into_iter().collect(),
        }
    };
    let flavor = Flavor::detect(rows.iter().filter(|n| n.tag_name().name() == "command").flat_map(|n| shortcuts(*n)));
    let mut commands = Vec::new();
    let mut tools = Vec::new();
    for node in rows {
        let tool = match node.tag_name().name() {
            "command" => false,
            "tool" => true,
            _ => continue,
        };
        let Some(name) = node.attribute("name").map(str::trim).filter(|n| !n.is_empty()) else { continue };
        let mut entry = ForeignEntry::new(None, [name]).at_line(doc.text_pos_at(node.range().start).row as usize);
        if let Some(id) = [ID_ATTRIBUTE, PHOTOCRAFT_ID_ATTRIBUTE].iter().find_map(|a| node.attribute(*a).map(str::trim).filter(|i| !i.is_empty())) {
            entry = entry.with_id(id);
        }
        for key in shortcuts(node) {
            let mut key = ForeignKey::read(None, &key, flavor);
            // Photoshop's tool shortcuts are single keys: one with modifiers is reported as
            // unreadable and binds nothing, and the tool keeps the key it has.
            if tool && key.trigger.as_ref().is_ok_and(|t| !t.chord.mods.is_none() || t.double) {
                key.trigger = Err(ForeignError::UnsupportedModifier(key.text.clone()));
            }
            entry = entry.with_key(key);
        }
        if tool {
            tools.push(entry.tool());
        } else {
            commands.push(entry);
        }
    }
    commands.extend(tools);
    let set = ForeignSet::new(commands);
    Ok(match root.attribute("filename").map(filename_to_name).filter(|n| !n.is_empty()) {
        Some(name) => set.with_name(name),
        None => set,
    })
}

/// A key in the spelling Photoshop's macOS sets use: `Control+Opt+Shift+Cmd+G`.
fn photoshop_key(trigger: &Trigger) -> String {
    let Mods { cmd, ctrl, alt, shift } = trigger.chord.mods;
    let mut parts: Vec<&str> = Vec::new();
    for (on, word) in [(ctrl, "Control"), (alt, "Opt"), (shift, "Shift"), (cmd, "Cmd")] {
        if on {
            parts.push(word);
        }
    }
    parts.push(trigger.chord.key.name());
    parts.join("+")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A `.kys` file named `name` with every command and tool of `keymap` that has a binding:
/// commands then tools, labels with Photoshop's `...` for `…`, keys in the spelling Photoshop's
/// macOS sets use (`Control+Opt+Shift+Cmd+G`), and [`ID_ATTRIBUTE`] on each row. Double presses
/// and scoped bindings (which Photoshop can't express) are left out.
// pc: crates/engine/src/kys.rs:296 `write`, :273 `photoshop_key`.
pub fn write(name: &str, keymap: &Keymap) -> String {
    let name = escape(name.trim());
    let mut out = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{ROOT} version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/{name}={name}\">\n"
    );
    for tools in [false, true] {
        for b in keymap.catalog().iter().filter(|b| matches!(b.kind, CommandKind::Tool { .. }) == tools) {
            let keys: Vec<String> =
                keymap.bindings(&b.command).iter().filter(|k| k.scope.is_none() && !k.trigger.double).map(|k| photoshop_key(&k.trigger)).collect();
            let Some(first) = keys.first() else { continue };
            let label = escape(&b.label.replace('…', "..."));
            let id = escape(&b.command.key());
            if tools {
                out.push_str(&format!("\t<tool name=\"{label}\" {ID_ATTRIBUTE}=\"{id}\">{}</tool>\n", escape(first)));
            } else {
                let shortcuts: String = keys.iter().map(|k| format!("<shortcut>{}</shortcut>", escape(k))).collect();
                out.push_str(&format!("\t<command kind=\"static\" name=\"{label}\" {ID_ATTRIBUTE}=\"{id}\">{shortcuts}</command>\n"));
            }
        }
    }
    out.push_str(&format!("</{ROOT}>\n"));
    out
}
