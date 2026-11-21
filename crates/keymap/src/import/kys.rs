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
use craft_xml::{Doctype, Event, Limits, Options};

/// A set is a few tens of kilobytes; a larger file is not one and is not parsed.
// pc: crates/engine/src/kys.rs:26 `MAX_BYTES`.
pub const MAX_BYTES: usize = 4 << 20;

/// The attribute naming the suite's command id on the rows it writes.
// pc: crates/engine/src/kys.rs:260 `PC_ID` ("pc-id"; read too, so PhotoCraft's own sets keep
// matching exactly).
pub const ID_ATTRIBUTE: &str = "craft-id";

/// The attribute PhotoCraft wrote its command ids under; read like [`ID_ATTRIBUTE`].
const PHOTOCRAFT_ID_ATTRIBUTE: &str = "pc-id";

/// Deepest element nesting read. The reader keeps no stack per level, so this only bounds the
/// work a crafted file can ask for.
// pc: crates/engine/src/kys.rs (craft-xml `max_depth: 512`).
const MAX_DEPTH: usize = 512;

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

/// A `command` or `tool` row as read, before the file's flavour is known.
struct Row {
    tool: bool,
    name: Option<String>,
    id: Option<String>,
    /// The 1-based line of the row's start tag.
    line: usize,
    /// A command's `<shortcut>` texts; a tool's own text.
    keys: Vec<String>,
}

/// The trimmed value of the attribute `name`, when it has one that is not blank.
fn attribute(attrs: &[(String, String)], name: &str) -> Option<String> {
    attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.trim()).filter(|value| !value.is_empty()).map(str::to_string)
}

/// Reads a `.kys` or `Keyboard Shortcuts.psp`: the set's name (from the root's `filename`,
/// `$$$/FileName/Presets/KeyboardCustomization/{name}={name}`), one [`crate::ForeignEntry`] per
/// command row with a name (its keys in the file's flavour, an id from [`ID_ATTRIBUTE`] or
/// `pc-id` as the entry's [`ForeignEntry::id`] when present), and the `<tool>` rows as tool
/// entries (a tool key with modifiers is read as an unsupported-modifier error, so [`crate::plan`] reports it unreadable and binds nothing). Rows without a name are skipped; rows without a shortcut stay, with no keys.
/// A file that is not well-formed, declares a document type or nests deeper than 512 elements
/// is not a set; each entry carries the line its row starts on, for the import report.
// pc: crates/engine/src/kys.rs:66 `parse`, :54 `filename_to_name`, :128 `to_ours`.
pub fn read(text: &str) -> Result<ForeignSet, KysError> {
    if text.len() > MAX_BYTES {
        return Err(KysError::TooLarge);
    }
    let text = text.trim_start_matches('\u{feff}');
    let options =
        Options { limits: Limits { max_depth: MAX_DEPTH, ..Limits::DEFAULT }, doctype: Doctype::Reject, strict: true, ..Options::default() };
    let mut events = options.events(text.as_bytes());
    let not_a_set = |e: craft_xml::Error| KysError::NotASet(e.to_string());
    let (mut depth, mut rooted, mut set_name) = (0_usize, false, None);
    let mut rows: Vec<Row> = Vec::new();
    // The row being read (at depth 2), and the text of the row's own content or of its `<shortcut>` (depth 3).
    let (mut row, mut shortcut, mut own_text): (Option<Row>, Option<String>, String) = (None, None, String::new());
    // Lines are counted once over the text, up to each row's start.
    let (mut counted, mut line) = (0_usize, 1_usize);
    while let Some(event) = events.next().map_err(not_a_set)? {
        match event {
            Event::Start { name, attrs } => {
                depth += 1;
                match depth {
                    1 => {
                        if name != ROOT {
                            return Err(KysError::NotASet(format!("no <{ROOT}> root")));
                        }
                        rooted = true;
                        set_name = attrs.iter().find(|(key, _)| key == "filename").map(|(_, value)| filename_to_name(value).to_string());
                    }
                    2 if name == "command" || name == "tool" => {
                        let at = events.span().start;
                        line += text.as_bytes().get(counted..at).unwrap_or_default().iter().filter(|&&b| b == b'\n').count();
                        counted = counted.max(at);
                        let id = [ID_ATTRIBUTE, PHOTOCRAFT_ID_ATTRIBUTE].iter().find_map(|key| attribute(&attrs, key));
                        row = Some(Row { tool: name == "tool", name: attribute(&attrs, "name"), id, line, keys: Vec::new() });
                        own_text.clear();
                    }
                    3 if name == "shortcut" && row.as_ref().is_some_and(|row| !row.tool) => shortcut = Some(String::new()),
                    _ => {}
                }
            }
            Event::Text(part) | Event::CData(part) => match (depth, shortcut.as_mut()) {
                (3, Some(key)) => key.push_str(&part),
                (2, _) if row.is_some() => own_text.push_str(&part),
                _ => {}
            },
            Event::End { .. } => {
                match depth {
                    3 => {
                        if let (Some(key), Some(row)) = (shortcut.take(), row.as_mut()) {
                            let key = key.trim();
                            if !key.is_empty() {
                                row.keys.push(key.to_string());
                            }
                        }
                    }
                    2 => {
                        if let Some(mut done) = row.take() {
                            let key = own_text.trim();
                            if done.tool && !key.is_empty() {
                                done.keys.push(key.to_string());
                            }
                            rows.push(done);
                        }
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    if !rooted {
        return Err(KysError::NotASet(format!("no <{ROOT}> root")));
    }
    let flavor = Flavor::detect(rows.iter().filter(|row| !row.tool).flat_map(|row| row.keys.iter().cloned()));
    let mut commands = Vec::new();
    let mut tools = Vec::new();
    for row in rows {
        let Some(name) = row.name else { continue };
        let mut entry = ForeignEntry::new(None, [name.as_str()]).at_line(row.line);
        if let Some(id) = &row.id {
            entry = entry.with_id(id);
        }
        for key in &row.keys {
            let mut key = ForeignKey::read(None, key, flavor);
            // Photoshop's tool shortcuts are single keys: one with modifiers is reported as
            // unreadable and binds nothing, and the tool keeps the key it has.
            if row.tool && key.trigger.as_ref().is_ok_and(|t| !t.chord.mods.is_none() || t.press != crate::chord::PressKind::Single) {
                key.trigger = Err(ForeignError::UnsupportedModifier(key.text.clone()));
            }
            entry = entry.with_key(key);
        }
        if row.tool {
            tools.push(entry.tool());
        } else {
            commands.push(entry);
        }
    }
    commands.extend(tools);
    let set = ForeignSet::new(commands);
    Ok(match set_name.filter(|name| !name.is_empty()) {
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
            let keys: Vec<String> = keymap
                .bindings(&b.command)
                .iter()
                .filter(|k| k.scope.is_none() && k.trigger.press == crate::chord::PressKind::Single)
                .map(|k| photoshop_key(&k.trigger))
                .collect();
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
