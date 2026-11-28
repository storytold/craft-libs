//! Toolbar presets (`.tbr`): one action descriptor with no file header.
//!
//! Layout from saved presets measured for storytold/photocraft#14624; no external implementation
//! copied. The descriptor's name is a lone NUL and its class `null`, and it holds these 11 keys in
//! this order: `tver` (long, 1), `tpad` (long, 0), the five Show toggles `fgbg`, `qkmm`, `sImI`,
//! `scmd` and `shov` (bool), `nana` (bool, false in saved files), `ddkb` (bool) and the two lists
//! `tlst` (toolbar) and `oflt` (extra tools). Each list holds groups, each group a list of tool
//! objects `{toolKey: long, keyboardShortcut: TEXT}`, where `toolKey` is a four-char id and
//! `keyboardShortcut` is left out for a tool without one.
//!
//! This module reads and writes that layout only. Which tool a key names, and what a shortcut
//! means, is the caller's.

use crate::descriptor::{Descriptor, Id, ParseLimits, UnicodeString, Value};
use crate::error::{PsdError, Result};

/// The largest preset read. A preset with every tool is about 3 KiB.
pub const MAX_BYTES: usize = 64 * 1024;

/// Per-container limits for a preset's descriptors and lists (hostile input).
const LIMITS: ParseLimits = ParseLimits { max_descriptor_items: 64, max_list_items: 256 };

/// A tool's shortcut in a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortcut {
    /// No `keyboardShortcut` key.
    Absent,
    /// A `keyboardShortcut` text, without its trailing NUL. It can be empty.
    Text(String),
    /// A `keyboardShortcut` that isn't text. It is not written back.
    NotText,
}

/// One tool in a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// The four-char tool id (`toolKey`).
    pub key: [u8; 4],
    /// Its shortcut.
    pub shortcut: Shortcut,
}

/// A toolbar preset. [`Default`] is the values a preset holds when nothing is set: version 1,
/// every Show toggle on, the other flags off, both lists empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarPreset {
    /// `tver`: the format version.
    pub version: i32,
    /// `tpad`: 0 in saved files; meaning unknown.
    pub pad: i32,
    /// `fgbg`: show the foreground and background colours.
    pub show_colors: bool,
    /// `qkmm`: show the quick mask mode button.
    pub show_quick_mask: bool,
    /// `sImI`: show the add-or-generate-images button.
    pub show_add_images: bool,
    /// `scmd`: show the screen mode button.
    pub show_screen_mode: bool,
    /// `shov`: show the slot that opens the extra tools.
    pub show_extras_slot: bool,
    /// `nana`: false in saved files; meaning unknown.
    pub nana: bool,
    /// `ddkb`: shortcuts of tools that sit only in the extra tools are off.
    pub disable_extras_shortcuts: bool,
    /// `tlst`: the toolbar's groups, in order.
    pub toolbar: Vec<Vec<Tool>>,
    /// `oflt`: the extra tools' groups, in order.
    pub extras: Vec<Vec<Tool>>,
}

impl Default for ToolbarPreset {
    fn default() -> Self {
        Self {
            version: 1,
            pad: 0,
            show_colors: true,
            show_quick_mask: true,
            show_add_images: true,
            show_screen_mode: true,
            show_extras_slot: true,
            nana: false,
            disable_extras_shortcuts: false,
            toolbar: Vec::new(),
            extras: Vec::new(),
        }
    }
}

/// A descriptor as presets hold them: the name is a lone NUL, the class `null`.
fn object() -> Descriptor {
    Descriptor { name: UnicodeString::new_nul(""), class_id: Id::new("null"), items: Vec::new() }
}

/// Write a preset: the 11 keys in their order, each tool as `{toolKey, keyboardShortcut}`, and
/// `keyboardShortcut` only for a [`Shortcut::Text`].
///
/// A preset [`read`] would refuse is an error instead: a list with more groups, or a group with
/// more tools, than [`read`] takes, or bytes over [`MAX_BYTES`].
pub fn write(preset: &ToolbarPreset) -> Result<Vec<u8>> {
    let max_entries = usize::try_from(LIMITS.max_list_items).unwrap_or(usize::MAX);
    for groups in [&preset.toolbar, &preset.extras] {
        if groups.len() > max_entries {
            return Err(PsdError::LimitExceeded("toolbar preset groups"));
        }
        if groups.iter().any(|group| group.len() > max_entries) {
            return Err(PsdError::LimitExceeded("toolbar preset tools in a group"));
        }
    }
    let groups = |groups: &[Vec<Tool>]| -> Value {
        Value::List(
            groups
                .iter()
                .map(|group| {
                    Value::List(
                        group
                            .iter()
                            .map(|tool| {
                                let d = object().with("toolKey", Value::Integer(i32::from_be_bytes(tool.key)));
                                Value::Descriptor(match &tool.shortcut {
                                    Shortcut::Text(s) => d.with("keyboardShortcut", Value::Text(UnicodeString::new_nul(s))),
                                    Shortcut::Absent | Shortcut::NotText => d,
                                })
                            })
                            .collect(),
                    )
                })
                .collect(),
        )
    };
    let bytes = object()
        .with("tver", Value::Integer(preset.version))
        .with("tpad", Value::Integer(preset.pad))
        .with("fgbg", Value::Boolean(preset.show_colors))
        .with("qkmm", Value::Boolean(preset.show_quick_mask))
        .with("sImI", Value::Boolean(preset.show_add_images))
        .with("scmd", Value::Boolean(preset.show_screen_mode))
        .with("shov", Value::Boolean(preset.show_extras_slot))
        .with("nana", Value::Boolean(preset.nana))
        .with("ddkb", Value::Boolean(preset.disable_extras_shortcuts))
        .with("tlst", groups(&preset.toolbar))
        .with("oflt", groups(&preset.extras))
        .to_bytes();
    if bytes.len() > MAX_BYTES {
        return Err(PsdError::LimitExceeded("toolbar preset bytes"));
    }
    Ok(bytes)
}

/// Read a preset. The file must be one whole descriptor of at most [`MAX_BYTES`] whose `tlst` and
/// `oflt` are lists of groups; anything else is an error. A group entry that isn't an object with
/// an integer `toolKey` is skipped. A flag or number that is missing or of another type reads as
/// its [`Default`].
pub fn read(bytes: &[u8]) -> Result<ToolbarPreset> {
    if bytes.len() > MAX_BYTES {
        return Err(PsdError::LimitExceeded("toolbar preset bytes"));
    }
    let (d, used) = Descriptor::parse_prefix_with_limits(bytes, LIMITS)?;
    if used != bytes.len() {
        return Err(PsdError::invalid("trailing bytes after the toolbar preset"));
    }
    let lists = |key: &str| -> Result<Vec<Vec<Tool>>> {
        let Some(Value::List(groups)) = d.get(key) else { return Err(PsdError::invalid(format!("toolbar preset: `{key}` is missing"))) };
        groups
            .iter()
            .map(|group| {
                let Value::List(tools) = group else {
                    return Err(PsdError::invalid(format!("toolbar preset: `{key}` holds something other than a tool group")));
                };
                Ok(tools.iter().filter_map(tool).collect())
            })
            .collect()
    };
    let (toolbar, extras) = (lists("tlst")?, lists("oflt")?);
    let def = ToolbarPreset::default();
    let int = |key: &str, or: i32| match d.get(key) {
        Some(Value::Integer(v)) => *v,
        _ => or,
    };
    let flag = |key: &str, or: bool| match d.get(key) {
        Some(Value::Boolean(v)) => *v,
        _ => or,
    };
    Ok(ToolbarPreset {
        version: int("tver", def.version),
        pad: int("tpad", def.pad),
        show_colors: flag("fgbg", def.show_colors),
        show_quick_mask: flag("qkmm", def.show_quick_mask),
        show_add_images: flag("sImI", def.show_add_images),
        show_screen_mode: flag("scmd", def.show_screen_mode),
        show_extras_slot: flag("shov", def.show_extras_slot),
        nana: flag("nana", def.nana),
        disable_extras_shortcuts: flag("ddkb", def.disable_extras_shortcuts),
        toolbar,
        extras,
    })
}

/// A group entry as a tool, or `None` when it isn't an object with an integer `toolKey`.
fn tool(entry: &Value) -> Option<Tool> {
    let Value::Descriptor(t) = entry else { return None };
    let Some(Value::Integer(key)) = t.get("toolKey") else { return None };
    let shortcut = match t.get("keyboardShortcut") {
        None => Shortcut::Absent,
        Some(Value::Text(s)) => Shortcut::Text(s.to_string_lossy()),
        Some(_) => Shortcut::NotText,
    };
    Some(Tool { key: key.to_be_bytes(), shortcut })
}
