//! The dialog's words: its own strings, command labels, menu path parts, key names and text
//! direction, all supplied by the app through [`Text`].

use std::borrow::Cow;

use crate::chord::{Key, Platform, Trigger};
use crate::keymap::Bindable;

/// Where the dialog gets its words. Apps on craft-i18n pass [`I18nText`]; others implement the two
/// required methods with their own lookup.
pub trait Text {
    /// The dialog's own strings, by English source text (listed in [`STRINGS`]). Placeholders
    /// (`{chord}`, `{label}`, `{set}`, `{n}`) are filled after the lookup.
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str>;
    /// A count's wording with `{n}` filled in. `one` and `other` are the English forms.
    fn plural(&self, n: u64, one: &str, other: &str) -> String;
    /// A command's label.
    fn command<'a>(&'a self, b: &'a Bindable) -> Cow<'a, str> {
        b.label.as_str().into()
    }
    /// A menu path part or a scope name.
    fn segment<'a>(&'a self, s: &'a str) -> Cow<'a, str> {
        s.into()
    }
    /// Extra lines for a command's tooltip.
    fn hint(&self, _b: &Bindable) -> Option<String> {
        None
    }
    /// A key cap's name on `platform` (`Strg`, `Umschalt`); `None` keeps the crate's name.
    fn key(&self, _k: Key, _p: Platform) -> Option<String> {
        None
    }
    /// Whether the interface language is written right to left.
    fn rtl(&self) -> bool {
        false
    }
    /// A label or message put into display order (the app's bidi reordering). Chord text is never
    /// passed here.
    fn visual(&self, s: &str) -> String {
        s.to_string()
    }
}

/// [`Text`] over a craft-i18n catalog.
pub struct I18nText<'a> {
    pub i18n: &'a craft_i18n::I18n,
    pub lang: craft_i18n::Lang,
}

impl Text for I18nText<'_> {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        self.i18n.t(self.lang, source).into()
    }

    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        self.i18n.trn(self.lang, n, one, other)
    }

    fn command<'a>(&'a self, b: &'a Bindable) -> Cow<'a, str> {
        self.i18n.tid(self.lang, &b.command.id, &b.label).into()
    }

    fn segment<'a>(&'a self, s: &'a str) -> Cow<'a, str> {
        self.i18n.t(self.lang, s).into()
    }

    fn key(&self, k: Key, p: Platform) -> Option<String> {
        // Glyphs (⌫, ↑) need no translation.
        if p.is_mac() && k.mac_symbol() != k.text_name() {
            return None;
        }
        self.i18n.get(self.lang, "key", k.text_name()).map(str::to_string)
    }

    fn rtl(&self) -> bool {
        self.i18n.rtl(self.lang)
    }
}

/// English [`Text`]: every string as written.
pub struct English;

impl Text for English {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        source.into()
    }

    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        craft_i18n::fmt(if n == 1 { one } else { other }, &[("n", &n.to_string())])
    }
}

/// A trigger as `platform` shows it, with key names from [`Text::key`]. Modifiers come in the
/// order of [`Chord::display`](crate::Chord::display).
pub fn trigger_text(text: &dyn Text, trigger: &Trigger, platform: Platform) -> String {
    let key = trigger.chord.key;
    let Some(name) = text.key(key, platform) else { return trigger.display(platform) };
    let base = if platform.is_mac() { key.mac_symbol() } else { key.text_name() };
    let one = trigger.chord.display(platform);
    let one = match one.strip_suffix(base) {
        Some(mods) => format!("{mods}{name}"),
        None => one,
    };
    if trigger.double { format!("{one} {one}") } else { one }
}

/// `s` in display order with `chord`, text inside it, left as it is: chord text never goes
/// through [`Text::visual`]. Right to left, the parts before and after the chord change sides.
pub(crate) fn visual_around(text: &dyn Text, s: &str, chord: &str) -> String {
    let Some(i) = s.find(chord).filter(|_| !chord.is_empty()) else { return text.visual(s) };
    let (before, rest) = s.split_at(i);
    let (before, after) = (text.visual(before), text.visual(&rest[chord.len()..]));
    if text.rtl() { format!("{after}{chord}{before}") } else { format!("{before}{chord}{after}") }
}

/// Plural forms the dialog looks up, as `one|other`, the key of craft-i18n's `@plural` rows.
pub const PLURALS: &[(&str, &str)] = &[
    ("{n} shortcut is assigned more than once", "{n} shortcuts are assigned more than once"),
    ("{n} saved shortcut could not be used", "{n} saved shortcuts could not be used"),
    ("{n} command will change", "{n} commands will change"),
    ("{n} item without a shortcut", "{n} items without a shortcut"),
    ("{n} item not matched", "{n} items not matched"),
    ("{n} item could not be read", "{n} items could not be read"),
    ("{n} key read approximately", "{n} keys read approximately"),
    ("{n} entry cut at a limit", "{n} entries cut at a limit"),
    ("{n} command changed", "{n} commands changed"),
    ("{n} shortcut also works in another scope", "{n} shortcuts also work in another scope"),
    ("{n} entry was ignored: too many commands", "{n} entries were ignored: too many commands"),
];

/// Every source string the dialog looks up with [`Text::ui`], for the shared catalog.
pub const STRINGS: &[&str] = &[
    // Window and footer.
    "Keyboard Shortcuts",
    "Custom",
    "OK",
    "Cancel",
    "Close",
    "Undo",
    "Redo",
    "Reset All…",
    "Restore every default shortcut in this set?",
    "Reset All",
    // Header.
    "Show:",
    "Filter:",
    "Search:",
    "Search by key",
    "All Commands",
    "{menu} Menu",
    "Tools",
    "{panel} Panel",
    "Other",
    "Fixed Keys",
    "All",
    "With Shortcut",
    "Changed",
    "Conflicts",
    "No matching commands",
    "(cycles)",
    "{label} (cycles)",
    "{label} (Changed)",
    // Table and detail pane.
    "Command",
    "Commands",
    "Detail pane size",
    "Select a command to see and change its shortcuts",
    "Shortcut",
    "Default",
    "{chord} (Default)",
    "{chord} ({scope})",
    "{label} ({scope})",
    "Application",
    "Current shortcuts:",
    "New shortcut:",
    "New shortcut: {chord}",
    "press keys",
    "{chord} (press again for a double press)",
    "Type…",
    "Context:",
    "Assign",
    "Use Default",
    "Clear",
    "Default: {chord}",
    "None",
    "Currently assigned to:",
    "Not assigned",
    "{label} ({scope}, both work)",
    "Add a shortcut to {label}",
    "Remove {chord} from {label}",
    "Restore the default for {label}",
    "{label}: {chord}",
    "{label}: press keys",
    "Now used by {label}",
    "{label}: {chord} (Now used by {holder})",
    // Typed shortcuts and saved texts that do not parse.
    "the shortcut has no key",
    "unknown modifier {key}",
    "modifier {key} given twice",
    "key sequences are not supported",
    "the scope name is not valid",
    // On-screen keyboard.
    "Keyboard",
    "Keyboard: US",
    "Keyboard: US (macOS)",
    "Control",
    "Option",
    "Shift",
    "Ctrl",
    "Alt",
    "Caps Lock",
    "Win",
    "Super",
    "Panel or context",
    "Both",
    "Modifier on",
    "{chord}: {label}",
    "Click a key to see its shortcuts",
    "Key: {key}",
    "Assign to Selected",
    "Assign {chord} to {label}",
    "Select a command first",
    "Nothing uses this key",
    // Messages and conflicts.
    "⚠ {chord} is used by {label}",
    "{chord} was removed from {label}",
    "Also used by {label}",
    "{chord} is reserved by the system",
    "A command can have at most {n} shortcuts",
    "The scope {scope} is unknown",
    "Too many shortcuts are assigned",
    "This command is unknown",
    "Take It",
    "Take It and Go to Conflict",
    "Keep Both",
    "Go to Conflict",
    "Cancel the new shortcut",
    "Dismiss",
    // Saved shortcuts that could not be used.
    "{key}: no command has this name",
    "{key}: “{text}” is not a shortcut ({error})",
    "{key}: {text} is reserved by the system",
    "{key}: the scope in “{text}” is unknown",
    "{key}: listed more than once",
    "{key}: part of the entry was dropped",
    // Sets.
    "Set:",
    "(modified)",
    "New Set…",
    "Rename…",
    "Delete",
    "Save",
    "Revert Set",
    "Import…",
    "Export",
    "Export…",
    "Summarize…",
    "{title}: {set}",
    "Create",
    "Rename",
    "More",
    "Name:",
    "Based on:",
    "Current shortcuts",
    "Save changes to “{set}”?",
    "Don't Save",
    "Delete the set “{set}”?",
    "Keyboard Shortcuts (JSON)",
    "The name is empty, too long or has control characters",
    "A set with this name exists",
    "The set was not found",
    "Built-in sets cannot be changed",
    "There are too many sets",
    "This is not a shortcut set file",
    "This is not a craft-keymap file",
    "This file is from a newer version (format {n})",
    "The file is too large",
    // Import review.
    "Import “{set}”",
    "Set name:",
    "Import",
    "Cancel Import",
    "line {n}",
    "Will change",
    "Not matched",
    "Unreadable",
    "Approximated",
    "{title} ({n})",
    "Suggestions ({n})",
    "numeric keypad key read as main key",
    "Matched without change",
    "Command for {item}",
    "Choose a command…",
    "no command has this name",
    "more than one command fits",
    "in a context that is not imported",
    "only a command of the other kind fits (tool or menu command)",
    "empty shortcut",
    "the shortcut is too long",
    "unknown key {key}",
    "unsupported modifier {key}",
    "{label} keeps its shortcuts (matched by a panel menu name)",
    "Import as New Set",
    "Imported",
    "Imported as “{set}”",
    "Skipped",
    "Taken from other commands",
    "Not changed",
    "taken by {label}",
    "taken by a later entry",
    "reserved by the system",
    "the scope is unknown",
    "the set's shortcuts could not be added",
    "the shortcuts could not be removed",
    "Copy Report",
    "Done",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::Chord;

    struct German;
    impl Text for German {
        fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
            source.into()
        }
        fn plural(&self, n: u64, _one: &str, other: &str) -> String {
            other.replace("{n}", &n.to_string())
        }
        fn key(&self, k: Key, _p: Platform) -> Option<String> {
            (k == Key::Delete).then(|| "Entf".to_string())
        }
    }

    #[test]
    fn trigger_text_uses_localized_key_names() -> Result<(), crate::ParseError> {
        let t = Trigger::parse("Cmd+Shift+Delete")?;
        assert_eq!(trigger_text(&German, &t, Platform::Windows), "Ctrl+Shift+Entf");
        assert_eq!(trigger_text(&German, &Trigger::parse("F7 F7")?, Platform::Windows), "F7 F7");
        assert_eq!(trigger_text(&English, &Trigger::single(Chord::parse("⇧⌘Z")?), Platform::Mac), "⇧⌘Z");
        Ok(())
    }

    /// Every `ui("…")` and `plural(n, "…", "…")` literal in the dialog's sources is listed.
    #[test]
    fn every_looked_up_string_is_listed() {
        let sources = [
            include_str!("mod.rs"),
            include_str!("state.rs"),
            include_str!("rows.rs"),
            include_str!("capture.rs"),
            include_str!("view.rs"),
            include_str!("keyboard.rs"),
            include_str!("style.rs"),
            include_str!("sets.rs"),
            include_str!("review.rs"),
            include_str!("summary.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            let src = src.split("#[cfg(test)]").next().unwrap_or(src);
            for (i, open) in src.match_indices(".ui(\"") {
                let rest = &src[i + open.len()..];
                if let Some(end) = rest.find('"') {
                    let s = &rest[..end];
                    if !STRINGS.contains(&s) {
                        missing.push(s.to_string());
                    }
                }
            }
            for (i, _) in src.match_indices(".plural(") {
                let quoted: Vec<&str> = src[i..].split('"').skip(1).step_by(2).take(2).collect();
                if let [one, other] = quoted[..]
                    && !PLURALS.contains(&(one, other))
                {
                    missing.push(format!("{one}|{other}"));
                }
            }
        }
        assert!(missing.is_empty(), "not in STRINGS or PLURALS: {missing:?}");
        let mut sorted = STRINGS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), STRINGS.len(), "STRINGS has duplicates");
    }
}
