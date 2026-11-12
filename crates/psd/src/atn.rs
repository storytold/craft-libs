//! Photoshop Actions files (`.atn`): action sets, their actions and their steps.
//!
//! Implemented clean-room from Adobe's "Photoshop File Formats" specification (the *Actions*
//! and *Descriptor structure* sections) and observation of files Photoshop writes. No code was
//! copied. All numbers are big-endian; strings are the Unicode strings of descriptors (u32
//! code-unit count + UTF-16BE, usually NUL-terminated) unless noted.
//!
//! ```text
//! file   := version u32 (12 or 16), set (version 16 may hold further sets until EOF)
//! set    := name UnicodeString, expanded u8, count u32, count × action
//! action := function key u16 (0 = none, n = Fn), shift u8, command u8, colour u16,
//!           name UnicodeString, expanded u8, count u32, count × item
//! item   := expanded u8, enabled u8, with dialog u8, dialog options u8,
//!           event kind [4]: "TEXT" (u32 length + ASCII event name) | "long" (4-char event code),
//!           dictionary name (u32 length + ASCII: the name the Actions panel shows),
//!           has descriptor i32 (-1 = a descriptor follows, 0 = none), [descriptor]
//! ```
//!
//! The descriptor is a plain ActionDescriptor body ([`Descriptor`], no version prefix): the
//! event's parameters, exactly as Photoshop's own "Make", "Set", "Gaussian Blur"… events
//! carry them. The event name and the panel name are byte strings (ASCII, or the platform's
//! legacy encoding for a localised panel name: Mac Roman "Définir"), kept verbatim;
//! [`AtnItem::label`] gives a readable name. Photoshop 5-era files (version 12) use a
//! legacy descriptor layout: bare four-byte identifiers and no top-level class or name.
//!
//! Which legacy encoding is not recorded: Windows Photoshop uses the system code page (GBK in
//! Chinese, Shift_JIS in Japanese…), Mac Photoshop the Mac one. [`parse`] works it out once per
//! file into [`AtnFile::encoding`], first from steps that also store their name as Unicode (the
//! descriptor's class name), else by guessing from the bytes; [`parse_with_encoding`] takes it
//! from the caller. [`AtnFile::item_label`] reads names with it. The bytes themselves stay as
//! they are.
//!
//! [`parse`] never panics: every count is bounds-checked against the bytes left, descriptor
//! nesting is bounded, and allocations are capped. [`write`] always produces version 16, so
//! `parse(write(f)) == f` for sets [`parse`] read. The existing single-set API retains trailing-byte
//! warnings. [`parse_sets`] and [`write_sets`] preserve complete version-16 multi-set files with
//! one version header; a version-12 file's legacy descriptors keep their meaning, not their layout.

use crate::codepage;
use crate::descriptor::{Descriptor, UnicodeString};
use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt, decode_legacy_name};

mod legacy;
mod sets;

pub use sets::{MAX_SETS, MAX_TOTAL_ACTIONS, MAX_TOTAL_ITEMS, parse_sets, parse_sets_with_encoding, parse_sets_with_spans, write_sets};

/// The version written by this module (Photoshop 6 and every later release).
pub const VERSION: u32 = 16;
/// Most actions read from one set (Photoshop's own sets hold a few dozen).
pub const MAX_ACTIONS: usize = 10_000;
/// Most steps read from one action.
pub const MAX_ITEMS: usize = 100_000;

/// A complete version16 archive and byte provenance into its original input.
/// Ranges address the caller's input, including original names/flags/descriptor encodings.
/// No source bytes are copied or rewritten by this parser.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedAtnArchive {
    /// Parsed sets in physical file order.
    pub sets: Vec<AtnFile>,
    /// Byte spans for the sets, in the same order.
    pub spans: Vec<AtnSetSpan>,
}

/// One physical set, including its original header fields and action occurrences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtnSetSpan {
    /// Half-open input range of the complete set, including its actions.
    pub range: std::ops::Range<usize>,
    /// Set header through the action count, before the first action.
    pub header: std::ops::Range<usize>,
    /// Length-prefixed Unicode set name bytes.
    pub name: std::ops::Range<usize>,
    /// Original expanded-flag byte.
    pub expanded: std::ops::Range<usize>,
    /// Original four-byte action count.
    pub count: std::ops::Range<usize>,
    /// Action spans in physical file order.
    pub actions: Vec<AtnActionSpan>,
}

/// One physical action. Header ranges let a library edit one field without normalizing others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtnActionSpan {
    /// Half-open input range of the complete action, including its steps.
    pub range: std::ops::Range<usize>,
    /// Action header through the step count, before the first step.
    pub header: std::ops::Range<usize>,
    /// Original function-key code bytes.
    pub function_key: std::ops::Range<usize>,
    /// Original Shift-modifier flag byte.
    pub shift: std::ops::Range<usize>,
    /// Original command-modifier flag byte.
    pub command: std::ops::Range<usize>,
    /// Original color-label code bytes.
    pub color: std::ops::Range<usize>,
    /// Length-prefixed Unicode action name bytes.
    pub name: std::ops::Range<usize>,
    /// Original expanded-flag byte.
    pub expanded: std::ops::Range<usize>,
    /// Original four-byte step count.
    pub count: std::ops::Range<usize>,
    /// Step spans in physical file order.
    pub items: Vec<AtnItemSpan>,
}

/// One original event, even when it maps to several native command steps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtnItemSpan {
    /// Half-open input range of the complete original step.
    pub range: std::ops::Range<usize>,
    /// Expanded, enabled, dialog, dialog-options bytes, in that order.
    pub flags: std::ops::Range<usize>,
    /// Descriptor body after its -1 flag; absent for a descriptor-free event.
    pub descriptor: Option<std::ops::Range<usize>>,
}

/// A parsed action set.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AtnFile {
    /// The set's name (the Actions panel folder).
    pub name: UnicodeString,
    /// Whether the set is shown expanded in the Actions panel.
    pub expanded: bool,
    /// The actions, in panel order.
    pub actions: Vec<AtnAction>,
    /// Problems that did not stop the parse (trailing bytes…).
    #[cfg_attr(feature = "serde", serde(default))]
    pub warnings: Vec<String>,
    /// The encoding of the 8-bit names that are not ASCII or UTF-8 ("GBK", "Shift_JIS",
    /// "macintosh"…): the one given to [`parse_with_encoding`], else the one that turns such
    /// names into the Unicode class names stored with them, else a guess from the names shown
    /// from their bytes alone. `None` when there are no such names, or nothing tells and no
    /// name depends on it. [`write`] ignores it: names are written as stored.
    #[cfg_attr(feature = "serde", serde(default))]
    pub encoding: Option<String>,
}

impl AtnFile {
    /// [`AtnItem::label`], with 8-bit names read in this file's [`encoding`](Self::encoding).
    /// A name that is valid UTF-8 is read in that encoding only if it reads entirely as common
    /// text there (GBK 去色 is also valid UTF-8, "ȥɫ").
    pub fn item_label(&self, item: &AtnItem) -> String {
        let Some(enc) = self.encoding.as_deref().and_then(|label| codepage::resolve_label(label).ok()) else { return item.label() };
        let read = |bytes: &[u8]| match std::str::from_utf8(bytes) {
            Ok(s) if s.is_ascii() || !codepage::reads_as_common(bytes, enc) => s.to_owned(),
            _ => codepage::decode(bytes, enc),
        };
        if let Some(class) = class_name(item) {
            return class;
        }
        let dict = read(&item.dictionary);
        if !dict.trim().is_empty() {
            return dict;
        }
        match &item.event {
            AtnEvent::Name(n) => read(n),
            AtnEvent::Code(_) => item.event.id(),
        }
    }
}

/// One action of a set.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AtnAction {
    /// Function key that plays the action: 0 = none, `n` = F`n`.
    pub function_key: u16,
    /// Shift modifier of the function key.
    pub shift: bool,
    /// Command (Control on Windows) modifier of the function key.
    pub command: bool,
    /// Button Mode colour index (0 = none, then red, orange, yellow, green, blue, violet, gray).
    pub color: u16,
    /// The action's name.
    pub name: UnicodeString,
    /// Whether the action is shown expanded in the Actions panel.
    pub expanded: bool,
    /// The steps, in playback order.
    pub items: Vec<AtnItem>,
}

/// One step of an action: a Photoshop event with its parameters.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AtnItem {
    /// Whether the step's parameters are shown expanded in the Actions panel.
    pub expanded: bool,
    /// The panel's check mark: a switched-off step is skipped by playback.
    pub enabled: bool,
    /// The panel's dialog toggle: playback shows the step's dialog.
    pub with_dialog: bool,
    /// Dialog options (0 = none, 2 = show the dialog, 3 = don't show), kept verbatim.
    pub dialog_options: u8,
    /// The event.
    pub event: AtnEvent,
    /// The name the Actions panel shows for the step ("Gaussian Blur", "Make", …), as stored:
    /// ASCII, or the authoring platform's legacy encoding when localised. See [`Self::label`].
    pub dictionary: Vec<u8>,
    /// The event's parameters; `None` for events without any.
    pub descriptor: Option<Descriptor>,
}

impl Default for AtnItem {
    fn default() -> Self {
        AtnItem {
            expanded: false,
            enabled: true,
            with_dialog: false,
            dialog_options: 0,
            event: AtnEvent::Code(*b"    "),
            dictionary: Vec::new(),
            descriptor: None,
        }
    }
}

impl AtnItem {
    /// The panel name as text: UTF-8 when it is, otherwise decoded byte by byte (Latin-1).
    pub fn dictionary(&self) -> String {
        decode_legacy_name(&self.dictionary)
    }

    /// A readable name for the step: the descriptor's class name when Photoshop stored one
    /// (Unicode, so a localised name survives), else [`Self::dictionary`], else the event id.
    pub fn label(&self) -> String {
        if let Some(class) = class_name(self) {
            return class;
        }
        let dict = self.dictionary();
        if dict.trim().is_empty() { self.event.id() } else { dict }
    }
}

/// The descriptor's class name, when Photoshop stored one: the step's name as Unicode.
fn class_name(item: &AtnItem) -> Option<String> {
    let class = item.descriptor.as_ref().map(|d| d.name.to_string_lossy())?;
    (!class.trim().is_empty()).then_some(class)
}

/// A Photoshop event id: a four-character code (`GsnB`) or a string name (`trim`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AtnEvent {
    /// Stored as `long` + 4 bytes.
    Code([u8; 4]),
    /// Stored as `TEXT` + length-prefixed bytes (ASCII in every file seen).
    Name(Vec<u8>),
}

impl AtnEvent {
    /// A string-named event.
    pub fn name(s: &str) -> Self {
        AtnEvent::Name(s.as_bytes().to_vec())
    }
    /// The event id as text: the code's four characters, or the name.
    pub fn id(&self) -> String {
        match self {
            AtnEvent::Code(c) => String::from_utf8_lossy(c).into_owned(),
            AtnEvent::Name(n) => String::from_utf8_lossy(n).into_owned(),
        }
    }
    /// `true` if the event is the code or name `s`.
    pub fn is(&self, s: &str) -> bool {
        match self {
            AtnEvent::Code(c) => c == s.as_bytes(),
            AtnEvent::Name(n) => n == s.as_bytes(),
        }
    }
}

/// Reads a u32 length + bytes (event and dictionary names), verbatim.
fn read_ascii(r: &mut Reader<'_>) -> Result<Vec<u8>> {
    let n = r.u32()?;
    Ok(r.bytes_u64(u64::from(n))?.to_vec())
}

fn write_ascii(out: &mut Vec<u8>, s: &[u8]) -> Result<()> {
    let n = u32::try_from(s.len()).map_err(|_| PsdError::LimitExceeded("action name too long"))?;
    out.put_u32(n);
    out.put(s);
    Ok(())
}

/// Read one step with the descriptor dialect selected by the file version.
fn read_item(r: &mut Reader<'_>, legacy: bool, mut span: Option<&mut AtnItemSpan>) -> Result<AtnItem> {
    let start = r.pos();
    let expanded = r.u8()? != 0;
    let enabled = r.u8()? != 0;
    let with_dialog = r.u8()? != 0;
    let dialog_options = r.u8()?;
    let event = match &r.array::<4>()? {
        b"TEXT" => AtnEvent::Name(read_ascii(r)?),
        b"long" => AtnEvent::Code(r.array()?),
        other => return Err(PsdError::invalid(format!("unknown event kind {:?}", String::from_utf8_lossy(other)))),
    };
    let dictionary = read_ascii(r)?;
    let descriptor = match r.i32()? {
        0 => None,
        -1 => {
            let start = r.pos();
            let descriptor = if legacy { legacy::read(r)? } else { Descriptor::read(r)? };
            if let Some(span) = span.as_deref_mut() {
                span.descriptor = Some(start..r.pos());
            }
            Some(descriptor)
        }
        n => return Err(PsdError::invalid(format!("bad descriptor flag {n}"))),
    };
    if let Some(span) = span {
        span.range = start..r.pos();
        span.flags = start..start.saturating_add(4);
    }
    Ok(AtnItem { expanded, enabled, with_dialog, dialog_options, event, dictionary, descriptor })
}

/// Read an action and its steps using the file's descriptor dialect.
fn read_action(r: &mut Reader<'_>, legacy: bool, budget: Option<&mut sets::Budget>, mut span: Option<&mut AtnActionSpan>) -> Result<AtnAction> {
    let start = r.pos();
    let function_key = r.u16()?;
    let shift_start = r.pos();
    let shift = r.u8()? != 0;
    let command_start = r.pos();
    let command = r.u8()? != 0;
    let color_start = r.pos();
    let color = r.u16()?;
    let name_start = r.pos();
    let name = UnicodeString::read(r)?;
    let expanded_start = r.pos();
    let expanded = r.u8()? != 0;
    let count_start = r.pos();
    let n = r.u32()?;
    if let Some(span) = span.as_deref_mut() {
        span.header = start..r.pos();
        span.function_key = start..shift_start;
        span.shift = shift_start..command_start;
        span.command = command_start..color_start;
        span.color = color_start..name_start;
        span.name = name_start..expanded_start;
        span.expanded = expanded_start..count_start;
        span.count = count_start..r.pos();
    }
    // Each item needs at least its 4 flags, the event kind, two empty names and the descriptor flag.
    r.check_count(u64::from(n), 20)?;
    if n as usize > MAX_ITEMS {
        return Err(PsdError::LimitExceeded("too many steps in one action"));
    }
    if let Some(budget) = budget {
        budget.items(n as usize)?;
    }
    let mut items = Vec::with_capacity((n as usize).min(1024));
    for _ in 0..n {
        let mut item_span = span.as_ref().map(|_| AtnItemSpan::default());
        items.push(read_item(r, legacy, item_span.as_mut())?);
        if let (Some(span), Some(item)) = (span.as_deref_mut(), item_span) {
            span.items.push(item);
        }
    }
    if let Some(span) = span {
        span.range = start..r.pos();
    }
    Ok(AtnAction { function_key, shift, command, color, name, expanded, items })
}

/// Parses a version 12 or 16 `.atn` file, working out the encoding of its 8-bit names (see the
/// module docs).
pub fn parse(data: &[u8]) -> Result<AtnFile> {
    parse_with_encoding(data, None)
}

/// [`parse`], reading 8-bit names in `encoding` (a label such as `shift_jis`, `gbk`, `big5`,
/// `euc-kr`, `windows-1252`, `macintosh`) instead of working it out. An unknown label, or one
/// for an encoding that cannot hold an 8-bit name (UTF-16, `replacement`), is an error.
pub fn parse_with_encoding(data: &[u8], encoding: Option<&str>) -> Result<AtnFile> {
    let forced = encoding.map(codepage::resolve_label).transpose()?;
    let mut f = parse_layout(data)?;
    let mut warnings = Vec::new();
    let enc = forced.or_else(|| name_encoding(&f, &mut warnings));
    f.warnings.extend(warnings);
    f.encoding = enc.map(|e| e.name().to_owned());
    Ok(f)
}

fn parse_layout(data: &[u8]) -> Result<AtnFile> {
    let mut r = Reader::new(data);
    let version = read_version(&mut r)?;
    let mut file = read_set(&mut r, version == 12, None)?;
    if !r.is_empty() {
        file.warnings.push(format!("{} trailing bytes ignored", r.remaining()));
    }
    Ok(file)
}

fn read_version(r: &mut Reader<'_>) -> Result<u32> {
    let version = r.u32()?;
    if !matches!(version, 12 | VERSION) {
        return Err(PsdError::Unsupported(format!("actions file version {version} (supported versions: 12 and 16)")));
    }
    Ok(version)
}

fn read_set(r: &mut Reader<'_>, legacy: bool, budget: Option<&mut sets::Budget>) -> Result<AtnFile> {
    read_set_with_spans(r, legacy, budget, None)
}

fn read_set_with_spans(
    r: &mut Reader<'_>,
    legacy: bool,
    mut budget: Option<&mut sets::Budget>,
    mut span: Option<&mut AtnSetSpan>,
) -> Result<AtnFile> {
    let start = r.pos();
    let name = UnicodeString::read(r)?;
    let expanded_start = r.pos();
    let expanded = r.u8()? != 0;
    let count_start = r.pos();
    let n = r.u32()?;
    if let Some(span) = span.as_deref_mut() {
        span.header = start..r.pos();
        span.name = start..expanded_start;
        span.expanded = expanded_start..count_start;
        span.count = count_start..r.pos();
    }
    // Each action needs at least its key, modifiers, colour, an empty name, the flag and a count.
    r.check_count(u64::from(n), 15)?;
    if n as usize > MAX_ACTIONS {
        return Err(PsdError::LimitExceeded("too many actions in one set"));
    }
    if let Some(budget) = budget.as_deref_mut() {
        budget.actions(n as usize)?;
    }
    let mut actions = Vec::with_capacity((n as usize).min(256));
    for _ in 0..n {
        let mut action_span = span.as_ref().map(|_| AtnActionSpan::default());
        actions.push(read_action(r, legacy, budget.as_deref_mut(), action_span.as_mut())?);
        if let (Some(span), Some(action)) = (span.as_deref_mut(), action_span) {
            span.actions.push(action);
        }
    }
    if let Some(span) = span {
        span.range = start..r.pos();
    }
    Ok(AtnFile { name, expanded, actions, warnings: Vec::new(), encoding: None })
}

/// The encoding of `f`'s 8-bit names that are not ASCII or UTF-8, or `None` if no name shown
/// from them needs one; adds a warning when the guess is uncertain.
fn name_encoding(f: &AtnFile, warnings: &mut Vec<String>) -> Option<&'static encoding_rs::Encoding> {
    name_encoding_sets(std::slice::from_ref(f), warnings)
}

fn name_encoding_sets(files: &[AtnFile], warnings: &mut Vec<String>) -> Option<&'static encoding_rs::Encoding> {
    let items = || files.iter().flat_map(|file| &file.actions).flat_map(|action| &action.items);
    let legacy = |b: &[u8]| !b.is_ascii();
    // Steps that store their name as Unicode too: the encoding that turns one into the other.
    let pairs: Vec<(&[u8], String)> = items()
        .filter(|i| legacy(&i.dictionary))
        .filter_map(|i| class_name(i).filter(|c| std::str::from_utf8(&i.dictionary) != Ok(c.as_str())).map(|c| (i.dictionary.as_slice(), c)))
        .take(codepage::KNOWN_PAIRS)
        .collect();
    let pairs: Vec<(&[u8], &str)> = pairs.iter().map(|(b, c)| (*b, c.as_str())).collect();
    if let Some(enc) = codepage::match_known(&pairs) {
        return Some(enc);
    }
    // Otherwise guess from the names shown from bytes alone.
    let invalid = |b: &[u8]| std::str::from_utf8(b).is_err();
    let mut orphans: Vec<&[u8]> = Vec::new();
    for i in items() {
        if class_name(i).is_none() && invalid(&i.dictionary) {
            orphans.push(&i.dictionary);
        }
        if let AtnEvent::Name(n) = &i.event
            && invalid(n)
        {
            orphans.push(n);
        }
    }
    if orphans.is_empty() {
        return None;
    }
    // The set, action and class names are UTF-16: their script points at the writer's locale.
    let mut hint = String::new();
    for (index, file) in files.iter().enumerate() {
        if index == 0 {
            hint = file.name.to_string_lossy();
        } else if hint.len() <= codepage::GUESS_SAMPLE {
            hint.push('\n');
            hint.push_str(&file.name.to_string_lossy());
        }
        for name in file
            .actions
            .iter()
            .map(|action| action.name.to_string_lossy())
            .chain(file.actions.iter().flat_map(|action| &action.items).filter_map(class_name))
        {
            if hint.len() > codepage::GUESS_SAMPLE {
                break;
            }
            hint.push('\n');
            hint.push_str(&name);
        }
    }
    let guess = codepage::guess(&orphans, &hint);
    if guess.uncertain {
        warnings.push(format!(
            "step names aren't UTF-8 and their encoding couldn't be determined; read as {}; load with an explicit encoding to choose another",
            guess.encoding.name()
        ));
    }
    Some(guess.encoding)
}

fn write_item(out: &mut Vec<u8>, item: &AtnItem) -> Result<()> {
    out.put_u8(u8::from(item.expanded));
    out.put_u8(u8::from(item.enabled));
    out.put_u8(u8::from(item.with_dialog));
    out.put_u8(item.dialog_options);
    match &item.event {
        AtnEvent::Name(n) => {
            out.put(b"TEXT");
            write_ascii(out, n)?;
        }
        AtnEvent::Code(c) => {
            out.put(b"long");
            out.put(c);
        }
    }
    write_ascii(out, &item.dictionary)?;
    match &item.descriptor {
        None => out.put_i32(0),
        Some(d) => {
            out.put_i32(-1);
            d.write(out);
        }
    }
    Ok(())
}

fn count(n: usize, what: &'static str) -> Result<u32> {
    u32::try_from(n).map_err(|_| PsdError::LimitExceeded(what))
}

/// Writes an `.atn` file (version 16).
pub fn write(f: &AtnFile) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.put_u32(VERSION);
    write_set(&mut out, f)?;
    Ok(out)
}

fn write_set(out: &mut Vec<u8>, f: &AtnFile) -> Result<()> {
    f.name.write(out);
    out.put_u8(u8::from(f.expanded));
    out.put_u32(count(f.actions.len(), "too many actions")?);
    for a in &f.actions {
        out.put_u16(a.function_key);
        out.put_u8(u8::from(a.shift));
        out.put_u8(u8::from(a.command));
        out.put_u16(a.color);
        a.name.write(out);
        out.put_u8(u8::from(a.expanded));
        out.put_u32(count(a.items.len(), "too many steps")?);
        for item in &a.items {
            write_item(out, item)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Value;

    fn px(v: f64) -> Value {
        Value::UnitFloat { unit: *b"#Pxl", value: v }
    }

    fn set() -> AtnFile {
        let blur = AtnItem {
            expanded: true,
            enabled: true,
            with_dialog: true,
            dialog_options: 2,
            event: AtnEvent::Code(*b"GsnB"),
            dictionary: b"Gaussian Blur".to_vec(),
            descriptor: Some(Descriptor::new("null").with("Rds ", px(2.5))),
        };
        let trim = AtnItem {
            event: AtnEvent::name("trim"),
            dictionary: b"Trim".to_vec(),
            descriptor: Some(
                Descriptor { name: UnicodeString::new_nul("Trim"), class_id: crate::descriptor::Id::Str(b"trim".to_vec()), items: Vec::new() }
                    .with(
                        "trimBasedOn",
                        Value::Enumerated {
                            type_id: crate::descriptor::Id::new("trimBasedOn"),
                            value: crate::descriptor::Id::new("topLeftPixelColor"),
                        },
                    )
                    .with("Top ", Value::Boolean(true)),
            ),
            ..Default::default()
        };
        let flatten = AtnItem { enabled: false, event: AtnEvent::Code(*b"Fltn"), dictionary: b"Flatten Image".to_vec(), ..Default::default() };
        AtnFile {
            name: UnicodeString::new_nul("Set ✓"),
            expanded: true,
            actions: vec![
                AtnAction {
                    function_key: 2,
                    shift: true,
                    command: false,
                    color: 3,
                    name: UnicodeString::new_nul("Soften"),
                    expanded: true,
                    items: vec![blur, flatten],
                },
                AtnAction { name: UnicodeString::new_nul("Tidy"), items: vec![trim], ..Default::default() },
                AtnAction { name: UnicodeString::new_nul("Empty"), ..Default::default() },
            ],
            warnings: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn round_trip() {
        let f = set();
        let bytes = write(&f).unwrap();
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed, f);
        assert_eq!(write(&parsed).unwrap(), bytes, "writing is stable");
        assert_eq!(parsed.actions[0].items[0].event.id(), "GsnB");
        assert!(parsed.actions[1].items[0].event.is("trim"));
        assert_eq!(parsed.actions[1].items[0].event.id(), "trim");
        assert_eq!(parsed.actions[0].items[0].label(), "Gaussian Blur");
        assert_eq!(parsed.actions[1].items[0].label(), "Trim");
        // A localised panel name in the platform's legacy encoding survives a round trip and
        // the Unicode class name is the readable label.
        let localised = AtnItem {
            event: AtnEvent::name("set"),
            dictionary: b"D\x8efinir".to_vec(),
            descriptor: Some(Descriptor {
                name: UnicodeString::new_nul("Définir"), class_id: crate::descriptor::Id::new("setd"), items: Vec::new()
            }),
            ..Default::default()
        };
        assert_eq!(localised.label(), "Définir");
        let f = AtnFile { actions: vec![AtnAction { items: vec![localised.clone()], ..Default::default() }], ..Default::default() };
        let bytes = write(&f).unwrap();
        assert_eq!(parse(&bytes).unwrap().actions[0].items[0], localised);
        assert_eq!(AtnItem { event: AtnEvent::name("x"), ..Default::default() }.label(), "x");
    }

    /// The layout Photoshop writes for a one-step Trim action, assembled by hand from the
    /// format description: field by field, so a mistake names the field.
    #[test]
    fn layout_matches_the_specification() {
        let f = AtnFile {
            name: UnicodeString::new_nul("Trim"),
            expanded: false,
            actions: vec![AtnAction {
                name: UnicodeString::new_nul("Action 1"),
                expanded: true,
                items: vec![AtnItem {
                    expanded: true,
                    event: AtnEvent::name("trim"),
                    dictionary: b"Trim".to_vec(),
                    descriptor: Some(Descriptor {
                        name: UnicodeString::new_nul("Trim"),
                        class_id: crate::descriptor::Id::Str(b"trim".to_vec()),
                        items: Vec::new(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            warnings: Vec::new(),
            ..Default::default()
        };
        let mut expect = Vec::new();
        expect.extend_from_slice(&[0, 0, 0, 16]); // version
        expect.extend_from_slice(&[0, 0, 0, 5, 0, b'T', 0, b'r', 0, b'i', 0, b'm', 0, 0]); // set name
        expect.push(0); // set collapsed
        expect.extend_from_slice(&[0, 0, 0, 1]); // one action
        expect.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no function key, no modifiers, no colour
        expect.extend_from_slice(&[0, 0, 0, 9]);
        for c in "Action 1\0".encode_utf16() {
            expect.extend_from_slice(&c.to_be_bytes());
        }
        expect.push(1); // action expanded
        expect.extend_from_slice(&[0, 0, 0, 1]); // one step
        expect.extend_from_slice(&[1, 1, 0, 0]); // expanded, enabled, no dialog, options 0
        expect.extend_from_slice(b"TEXT\0\0\0\x04trim");
        expect.extend_from_slice(b"\0\0\0\x04Trim");
        expect.extend_from_slice(&[0xff; 4]); // a descriptor follows
        expect.extend_from_slice(&[0, 0, 0, 5, 0, b'T', 0, b'r', 0, b'i', 0, b'm', 0, 0]); // class name
        expect.extend_from_slice(b"\0\0\0\x04trim"); // class id, length-4 string form
        expect.extend_from_slice(&[0, 0, 0, 0]); // no items
        assert_eq!(write(&f).unwrap(), expect);
        assert_eq!(parse(&expect).unwrap(), f);
    }

    #[test]
    fn truncation_never_parses_and_never_panics() {
        let bytes = write(&set()).unwrap();
        for cut in 0..bytes.len() {
            assert!(parse(&bytes[..cut]).is_err(), "cut at {cut}/{}", bytes.len());
        }
        let mut padded = bytes.clone();
        padded.extend_from_slice(b"junk");
        let f = parse(&padded).unwrap();
        assert_eq!(f.warnings, ["4 trailing bytes ignored"]);
        assert_eq!(f.actions, set().actions);
    }

    #[test]
    fn rejects_other_versions_and_bad_structure() {
        let mut bytes = write(&set()).unwrap();
        bytes[3] = 7;
        assert!(matches!(parse(&bytes), Err(PsdError::Unsupported(_))));
        // A count far beyond the data must fail before allocating.
        let mut huge = vec![0, 0, 0, 16, 0, 0, 0, 0, 0];
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(parse(&huge), Err(PsdError::UnexpectedEof { .. })));
        // Unknown event kinds and descriptor flags are reported, not guessed.
        let mut bad_kind = write(&set()).unwrap();
        let at = bad_kind.windows(4).position(|w| w == b"long").unwrap();
        bad_kind[at..at + 4].copy_from_slice(b"what");
        assert!(matches!(parse(&bad_kind), Err(PsdError::Invalid(_))));
        let mut bad_flag = write(&set()).unwrap();
        let at = bad_flag.windows(4).position(|w| w == [0xff; 4]).unwrap();
        bad_flag[at..at + 4].copy_from_slice(&[0, 0, 0, 7]);
        assert!(matches!(parse(&bad_flag), Err(PsdError::Invalid(_))));
    }

    const GBK_SELECTIVE_COLOR: &[u8] = b"\xbf\xc9\xd1\xa1\xd1\xd5\xc9\xab"; // 可选颜色 (from photocraft#6743)
    const GBK_DESATURATE: &[u8] = b"\xc8\xa5\xc9\xab"; // 去色: valid UTF-8 by accident ("ȥɫ")
    const LABELS: [Option<&str>; 5] = [None, Some("shift_jis"), Some("gbk"), Some("utf-8"), Some("macintosh")];

    /// A step stored with no descriptor, so nothing but its 8-bit names tells what it is called.
    fn bare(event: AtnEvent, dictionary: &[u8]) -> AtnItem {
        AtnItem { event, dictionary: dictionary.to_vec(), ..Default::default() }
    }

    /// A step whose descriptor also stores its panel name as Unicode, as Photoshop does for most.
    fn known(dictionary: &[u8], class: &str) -> AtnItem {
        let descriptor = Descriptor { name: UnicodeString::new_nul(class), class_id: crate::descriptor::Id::new("setd"), items: Vec::new() };
        AtnItem { event: AtnEvent::name("set"), dictionary: dictionary.to_vec(), descriptor: Some(descriptor), ..Default::default() }
    }

    fn one_action(set: &str, action: &str, items: Vec<AtnItem>) -> Vec<u8> {
        let f = AtnFile {
            name: UnicodeString::new_nul(set),
            actions: vec![AtnAction { name: UnicodeString::new_nul(action), items, ..Default::default() }],
            ..Default::default()
        };
        write(&f).unwrap()
    }

    /// The issue's set: Chinese Photoshop's Selective Color step, named in GBK.
    fn gbk_set() -> Vec<u8> {
        one_action("相馆", "调白", vec![bare(AtnEvent::name("syntheticFill"), GBK_SELECTIVE_COLOR)])
    }

    fn labels(f: &AtnFile) -> Vec<String> {
        f.actions.iter().flat_map(|a| a.items.iter().map(|i| f.item_label(i))).collect()
    }

    #[test]
    fn gbk_step_names_read_as_gbk() {
        let f = parse(&gbk_set()).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("GBK"));
        assert_eq!(labels(&f), ["可选颜色"]);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        // The item itself, without the file's code page, reads as before.
        assert_ne!(f.actions[0].items[0].label(), "可选颜色");
    }

    #[test]
    fn legacy_names_are_kept_byte_for_byte() {
        let bytes = gbk_set();
        let f = parse(&bytes).unwrap();
        assert_eq!(f.actions[0].items[0].dictionary, GBK_SELECTIVE_COLOR);
        assert_eq!(write(&f).unwrap(), bytes);
        assert_eq!(parse(&write(&f).unwrap()).unwrap(), f);
    }

    #[test]
    fn text_event_names_decode_too() {
        let f = parse(&one_action("相馆", "调白", vec![bare(AtnEvent::Name(GBK_SELECTIVE_COLOR.to_vec()), b"")])).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("GBK"));
        assert_eq!(labels(&f), ["可选颜色"]);
    }

    #[test]
    fn known_names_decide_the_code_page_of_the_rest() {
        // French Photoshop on a Mac: "Définir" is stored as Mac Roman and as Unicode; the bare
        // step's "Répéter" is only Mac Roman, which also reads as windows-1252 ("RŽpŽter").
        let items = vec![known(b"D\x8efinir", "Définir"), bare(AtnEvent::name("x"), b"R\x8ep\x8eter")];
        let f = parse(&one_action("Set 1", "Action 1", items)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("macintosh"));
        assert_eq!(labels(&f), ["Définir", "Répéter"]);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    }

    #[test]
    fn class_names_hint_the_script() {
        // GBK 保存 reads as common text in GBK, Big5 and EUC-KR alike; only the Chinese class
        // name of another step (whose panel name is ASCII, so it settles nothing itself) says GBK.
        let items = vec![known(b"Adjust", "调整"), bare(AtnEvent::name("x"), b"\xb1\xa3\xb4\xe6")];
        let f = parse(&one_action("Set 1", "Action 1", items)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("GBK"));
        assert_eq!(labels(&f), ["调整", "保存"]);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    }

    #[test]
    fn latin1_symbol_names_read_as_windows_1252() {
        // Windows Photoshop "Größe »" and "50 °" with no Unicode copy: Mac Roman reads these
        // bytes as letters and symbols ("Grˆﬂe ª", "50 ∞"), but they are windows-1252, as before
        // photocraft#6743.
        let f = parse(&one_action("Größe ändern", "Aktion", vec![bare(AtnEvent::name("x"), b"Gr\xf6\xdfe \xbb")])).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("windows-1252"));
        assert_eq!(labels(&f), ["Größe »"]);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        let f = parse(&one_action("Größe ändern", "Aktion", vec![bare(AtnEvent::name("x"), b"50 \xb0")])).unwrap();
        assert_eq!(labels(&f), ["50 °"]);
        // Known limitation: "Logo ®" alone reads as Mac Roman "Logo Æ", as Mac "Æble" needs that
        // byte (codepage's symbols_on_mac_roman_letters_stay_ambiguous).
        let f = parse(&one_action("Set 1", "Action 1", vec![bare(AtnEvent::name("x"), b"Logo \xae")])).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("macintosh"));
        assert_eq!(labels(&f), ["Logo Æ"]);
        // Mac Roman bytes that windows-1252 reads as controls or rare letters stay Mac Roman.
        let f =
            parse(&one_action("Définir", "Action 1", vec![bare(AtnEvent::name("x"), b"Gr\x9a\xa7e"), bare(AtnEvent::name("y"), b"Arri\x8fre-plan")]))
                .unwrap();
        assert_eq!(f.encoding.as_deref(), Some("macintosh"));
        assert_eq!(labels(&f), ["Größe", "Arrière-plan"]);
    }

    #[test]
    fn mac_roman_sharp_s_and_slashed_o_stay_mac_roman() {
        // Mac Roman ß ø æ Æ are the bytes of windows-1252 § ¿ ¾ ®: German and Danish Mac names
        // with no Unicode copy still read as Mac Roman, each alone and as a set.
        let words: [&[u8]; 5] = [b"Wei\xa7", b"Gau\xa7scher Weichzeichner", b"St\xbfrrelse", b"L\xberredst\xbfrrelse", b"\xaeble"];
        let text = ["Weiß", "Gaußscher Weichzeichner", "Størrelse", "Lærredstørrelse", "Æble"];
        for (bytes, text) in words.iter().zip(text) {
            let f = parse(&one_action("Größe ändern", "Aktion 1", vec![bare(AtnEvent::name("x"), bytes)])).unwrap();
            assert_eq!((f.encoding.as_deref(), labels(&f)), (Some("macintosh"), vec![text.to_string()]));
            assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        }
        let items = words.iter().map(|w| bare(AtnEvent::name("x"), w)).collect();
        let f = parse(&one_action("Größe ändern", "Aktion 1", items)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("macintosh"));
        assert_eq!(labels(&f), text);
    }

    #[test]
    fn mac_roman_bullets_stay_mac_roman() {
        // Mac Roman • (0xA5) is windows-1252 ¥, which must not count for windows-1252.
        for (set, bytes, text) in [
            ("Größe ändern", &b"Ebene \xa5 Kopie"[..], "Ebene • Kopie"),
            ("Définir", b"Niveaux \xa5 Courbes", "Niveaux • Courbes"),
            ("Tamaño de imagen", b"Tama\x96o \xa5 50%", "Tamaño • 50%"),
        ] {
            let f = parse(&one_action(set, "Aktion 1", vec![bare(AtnEvent::name("x"), bytes)])).unwrap();
            assert_eq!((f.encoding.as_deref(), labels(&f)), (Some("macintosh"), vec![text.to_string()]));
        }
    }

    #[test]
    fn russian_sets_with_latin_compounds_stay_windows_1251() {
        // Ordinary windows-1251 names plus one glued to a Latin word ("CMYKканал"): the whole
        // set reads as Cyrillic, with no warning.
        let words: [&[u8]; 5] = [
            b"\xcd\xee\xe2\xfb\xe9 \xf1\xeb\xee\xe9",
            b"\xd0\xe0\xe7\xec\xe5\xf0 \xe8\xe7\xee\xe1\xf0\xe0\xe6\xe5\xed\xe8\xff",
            b"\xca\xee\xef\xe8\xff \xf1\xeb\xee\xff 2",
            b"\xca\xe0\xed\xe0\xeb RGB",
            b"CMYK\xea\xe0\xed\xe0\xeb",
        ];
        let text = ["Новый слой", "Размер изображения", "Копия слоя 2", "Канал RGB", "CMYKканал"];
        let items = words.iter().map(|w| bare(AtnEvent::name("x"), w)).collect();
        let f = parse(&one_action("Набор 1", "Операция 1", items)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("windows-1251"));
        assert_eq!(labels(&f), text);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    }

    #[test]
    fn known_names_alone_never_warn() {
        // Nothing decodes to the Unicode name, but nothing is shown from the bytes either.
        let f = parse(&one_action("Set 1", "Action 1", vec![known(b"\x80\xff\x80", "Something")])).unwrap();
        assert_eq!(f.encoding, None);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(labels(&f), ["Something"]);
    }

    #[test]
    fn uncertain_guess_warns() {
        let f = parse(&one_action("Set 1", "Action 1", vec![bare(AtnEvent::name("x"), b"\x80\xff\x80")])).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("windows-1251"));
        assert_eq!(f.warnings.len(), 1, "{:?}", f.warnings);
        assert!(f.warnings[0].contains("windows-1251"), "{:?}", f.warnings);
        assert_eq!(labels(&f), ["ЂяЂ"]);
    }

    #[test]
    fn unresolved_codepage_tie_keeps_a_top_candidate_and_the_original_bytes() {
        // GBK 保存 ties with other CJK pages. No Unicode name settles it, but windows-1252
        // scored lower, so its "±£´æ" must not replace the tied candidates' readable text.
        let bytes = one_action("Set 1", "Action 1", vec![bare(AtnEvent::name("x"), b"\xb1\xa3\xb4\xe6")]);
        let f = parse(&bytes).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("GBK"));
        assert_eq!(labels(&f), ["保存"]);
        assert_eq!(f.warnings.len(), 1, "{:?}", f.warnings);
        assert!(f.warnings[0].contains("read as GBK"), "{:?}", f.warnings);
        assert_eq!(write(&f).unwrap(), bytes);
        // Callers can still resolve an ambiguous guess explicitly.
        let forced = parse_with_encoding(&bytes, Some("windows-1252")).unwrap();
        assert_eq!(labels(&forced), ["±£´æ"]);
        assert!(forced.warnings.is_empty(), "{:?}", forced.warnings);
        assert_eq!(write(&forced).unwrap(), bytes);
    }

    #[test]
    fn utf8_files_have_no_encoding() {
        let f = parse(&write(&set()).unwrap()).unwrap();
        assert_eq!(f.encoding, None);
        let items: Vec<&AtnItem> = f.actions.iter().flat_map(|a| &a.items).collect();
        assert_eq!(labels(&f), items.iter().map(|i| i.label()).collect::<Vec<_>>());
        let f = parse(&one_action("Set", "A", vec![bare(AtnEvent::name("x"), "高斯模糊 ✓".as_bytes())])).unwrap();
        assert_eq!((f.encoding.clone(), labels(&f)), (None, vec!["高斯模糊 ✓".to_string()]));
    }

    #[test]
    fn utf8_lookalike_names_decode_with_the_file() {
        let items = vec![
            bare(AtnEvent::name("x"), GBK_SELECTIVE_COLOR),
            bare(AtnEvent::name("y"), GBK_DESATURATE),
            bare(AtnEvent::name("z"), "Gaussian Blur ✓".as_bytes()),
        ];
        let f = parse(&one_action("相馆", "调白", items)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("GBK"));
        assert_eq!(labels(&f), ["可选颜色", "去色", "Gaussian Blur ✓"]);
    }

    #[test]
    fn utf8_names_stay_in_single_byte_files() {
        // windows-1252 "Größe" beside a UTF-8 "Weiß"; windows-1251 "Уровни" beside a UTF-8 "Привет".
        let latin = vec![bare(AtnEvent::name("x"), b"Gr\xf6\xdfe"), bare(AtnEvent::name("y"), "Weiß".as_bytes())];
        let f = parse(&one_action("Größe ändern", "Aktion", latin)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("windows-1252"));
        assert_eq!(labels(&f), ["Größe", "Weiß"]);
        let cyrillic = vec![bare(AtnEvent::name("x"), b"\xd3\xf0\xee\xe2\xed\xe8"), bare(AtnEvent::name("y"), "Привет".as_bytes())];
        let f = parse(&one_action("Набор", "Операция", cyrillic)).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("windows-1251"));
        assert_eq!(labels(&f), ["Уровни", "Привет"]);
        // A forced single-byte encoding leaves UTF-8 names alone too.
        let f = parse_with_encoding(&one_action("S", "A", vec![bare(AtnEvent::name("y"), "Weiß".as_bytes())]), Some("windows-1252")).unwrap();
        assert_eq!(labels(&f), ["Weiß"]);
    }

    #[test]
    fn forced_encoding_wins() {
        let f = parse_with_encoding(&gbk_set(), Some("big5")).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("Big5"));
        assert_ne!(labels(&f), ["可选颜色"]);
        // A wrong override still loads, and saves the bytes it read.
        let f = parse_with_encoding(&gbk_set(), Some("shift_jis")).unwrap();
        assert_eq!(f.encoding.as_deref(), Some("Shift_JIS"));
        assert_eq!(write(&f).unwrap(), gbk_set());
        // Forcing applies to all-UTF-8 files too.
        assert_eq!(parse_with_encoding(&write(&set()).unwrap(), Some("gbk")).unwrap().encoding.as_deref(), Some("GBK"));
    }

    #[test]
    fn bad_labels_are_errors() {
        for bad in ["nope", "replacement", "utf-16be", ""] {
            assert!(matches!(parse_with_encoding(&gbk_set(), Some(bad)), Err(PsdError::Unsupported(_))), "{bad}");
        }
    }

    #[test]
    fn legacy_files_never_panic() {
        let items = vec![known(b"D\x8efinir", "Définir"), bare(AtnEvent::Name(GBK_SELECTIVE_COLOR.to_vec()), GBK_DESATURATE)];
        let seed = one_action("相馆", "调白", items);
        let check = |bytes: &[u8]| {
            for label in LABELS {
                if let Ok(f) = parse_with_encoding(bytes, label) {
                    let _ = labels(&f);
                }
            }
        };
        for cut in 0..seed.len() {
            check(&seed[..cut]);
        }
        let mut x: u32 = 0x85eb_ca6b;
        for i in 0..500 {
            let mut bytes = seed.clone();
            for _ in 0..(i % 5 + 1) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = (x as usize) % bytes.len();
                bytes[at] = (x >> 8) as u8;
            }
            check(&bytes);
        }
    }

    #[test]
    fn garbage_never_panics() {
        let seed = write(&set()).unwrap();
        let mut x: u32 = 0x9e37_79b9;
        for i in 0..2000 {
            let mut bytes = seed.clone();
            for _ in 0..(i % 7 + 1) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = (x as usize) % bytes.len();
                bytes[at] = (x >> 8) as u8;
            }
            let _ = parse(&bytes);
        }
    }

    /// Independent wire input: duplicate raw UTF16 names, noncanonical true bytes, and a
    /// descriptor boolean whose semantic value cannot preserve its original byte by itself.
    fn spanned_wire_archive() -> Vec<u8> {
        fn ascii(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            out.extend_from_slice(bytes);
        }
        let mut out = 16u32.to_be_bytes().to_vec();
        let raw_name = [0, 0, 0, 2, 0xd8, 0, 0, 0]; // unpaired high surrogate, then NUL
        for _ in 0..2 {
            out.extend_from_slice(&raw_name);
            out.push(2); // expanded=true, retaining its original byte matters
            out.extend_from_slice(&1u32.to_be_bytes());
            out.extend_from_slice(&[0, 3, 2, 0, 0, 12]); // F3, Shift byte2, no Command, colour12
            out.extend_from_slice(&raw_name);
            out.push(2);
            out.extend_from_slice(&2u32.to_be_bytes());
            out.extend_from_slice(&[2, 2, 0, 3]);
            out.extend_from_slice(b"longFltn");
            ascii(&mut out, b"Flatten");
            out.extend_from_slice(&0i32.to_be_bytes());
            out.extend_from_slice(&[0, 1, 2, 2]);
            out.extend_from_slice(b"TEXT");
            ascii(&mut out, b"futureAction");
            ascii(&mut out, b"Future");
            out.extend_from_slice(&(-1i32).to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes()); // empty descriptor Unicode name
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(b"null");
            out.extend_from_slice(&1u32.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(b"flagbool");
            out.push(2);
        }
        out
    }

    #[test]
    fn archive_spans_retain_physical_occurrences_and_original_wire_fields() {
        let bytes = spanned_wire_archive();
        let archive = parse_sets_with_spans(&bytes).unwrap();
        assert_eq!(archive.sets.len(), 2);
        assert_eq!(archive.spans.len(), 2);
        assert_eq!(archive.spans[0].range.start, 4);
        assert_eq!(archive.spans[0].range.end, archive.spans[1].range.start);
        assert_eq!(archive.spans[1].range.end, bytes.len());
        let mut untouched = bytes[..4].to_vec();
        for (file, set) in archive.sets.iter().zip(&archive.spans) {
            assert_eq!(file.name.0, [0xd800, 0]);
            assert_eq!(bytes[set.expanded.clone()], [2]);
            assert_eq!(bytes[set.count.clone()], 1u32.to_be_bytes());
            assert_eq!(set.header.end, set.actions[0].range.start);
            let action = &set.actions[0];
            assert_eq!(bytes[action.function_key.clone()], [0, 3]);
            assert_eq!(bytes[action.shift.clone()], [2]);
            assert_eq!(bytes[action.command.clone()], [0]);
            assert_eq!(bytes[action.color.clone()], [0, 12]);
            assert_eq!(bytes[action.expanded.clone()], [2]);
            assert_eq!(bytes[action.count.clone()], 2u32.to_be_bytes());
            assert_eq!(action.header.end, action.items[0].range.start);
            assert_eq!(bytes[action.items[0].flags.clone()], [2, 2, 0, 3]);
            assert_eq!(bytes[action.items[1].flags.clone()], [0, 1, 2, 2]);
            assert_eq!(action.items[0].range.end, action.items[1].range.start);
            assert_eq!(action.items[1].range.end, action.range.end);
            assert_eq!(action.range.end, set.range.end);
            assert_eq!(bytes[action.items[1].range.clone()].last(), Some(&2));
            untouched.extend_from_slice(&bytes[set.range.clone()]);
        }
        assert_eq!(untouched, bytes, "no semantic writer generates the expected raw bytes");
        assert_eq!(parse_sets(&bytes).unwrap(), archive.sets, "ordinary parsing keeps its semantic API");
        assert_ne!(write_sets(&archive.sets).unwrap(), bytes, "typed boolean rewriting is not a raw library round trip");
    }

    #[test]
    fn archive_spans_reject_partial_later_records_and_legacy_layout() {
        let mut bytes = spanned_wire_archive();
        bytes.pop(); // the second set's descriptor value is incomplete
        assert!(parse_sets_with_spans(&bytes).is_err(), "no partial archive is admitted");
        assert!(parse_sets_with_spans(&16u32.to_be_bytes()).is_err(), "a version prefix is not an invented empty ATN");
        assert!(matches!(parse_sets_with_spans(&12u32.to_be_bytes()), Err(PsdError::Unsupported(_))));
    }
}
