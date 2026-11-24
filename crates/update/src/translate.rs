//! The words the update dialogs and the [`Updater`](crate::updater::Updater) show, in the app's
//! interface language.
//!
//! The crate keeps its text in English, and that English text is the lookup key, as in the
//! craft-i18n catalogs. An app with translations passes a [`Translate`] function to
//! `StartupPrompt::with_translator`, `CheckResultDialog::with_translator` and
//! `Updater::with_translator`. The dialogs call it each time they draw a string, so a change of
//! interface language shows on the next frame. Without one the text stays in English.
//!
//! A template names its values in braces (`{app} {version} is available…`) and the function
//! gets the values beside it, so a translation can put them in another order. An app built on
//! craft-i18n passes `|text, args| craft_i18n::fmt(i18n.t(lang, text), args)`. That also keeps a
//! Latin app name in its place in a right-to-left sentence. Button labels are looked up in title
//! case (`Later`, `Check Again`) and drawn in capitals, so the rows the app already has for its
//! own buttons serve here too.
//!
//! The card otherwise draws only ASCII, so a font that lacks a glyph can't trip it. An app that
//! translates it must have fonts for its translations, which it needs for its own interface anyway.

/// The English `text` (a key) in the interface language, its `{name}` placeholders filled from
/// `args`. A `fn`, not a closure, so the dialogs stay `Copy` and `Debug`. An app's lookup reads
/// the current language from its own state.
pub type Translate = fn(text: &str, args: &[(&str, &str)]) -> String;

/// The [`Translate`] used when the app gives none: the English text, its placeholders filled.
pub fn english(text: &str, args: &[(&str, &str)]) -> String {
    fill(text, args)
}

/// `template` with each `{name}` in `args` replaced by its value, by the rules of
/// `craft_i18n::fmt` (without its bidi isolates): a placeholder runs from a `{` to the next `}`,
/// and one that `args` lacks stays as written, braces and all. An inserted value is never read as
/// a template, so a value containing `{app}` stays intact.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, after_close)) = after_open.split_once('}') else {
            out.push('{');
            rest = after_open;
            break;
        };
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after_close;
    }
    out.push_str(rest);
    out
}

/// One of the crate's own messages in the startup prompt ("The update is installed…"), kept as
/// its English text and values so it is translated each time the prompt draws: a change of
/// interface language while it shows changes it too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The English text, the lookup key.
    pub text: &'static str,
    /// The values of its `{name}` placeholders.
    pub args: Vec<(&'static str, String)>,
}

impl Note {
    pub fn new(text: &'static str, args: &[(&'static str, &str)]) -> Self {
        Self { text, args: args.iter().map(|(k, v)| (*k, (*v).to_owned())).collect() }
    }

    /// The note in the interface language.
    pub fn translated(&self, translate: Translate) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        translate(self.text, &args)
    }
}

/// A button label: `label` (title case, the lookup key) translated, then in capitals, as the card
/// draws its buttons.
#[cfg(any(feature = "egui", test))]
pub(crate) fn button(translate: Translate, label: &str) -> String {
    translate(label, &[]).to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_and_values_are_not_templates() {
        assert_eq!(fill("{app} {version} is out", &[("app", "PhotoCraft"), ("version", "0.2.0")]), "PhotoCraft 0.2.0 is out");
        // Reordered, as a translation may do.
        assert_eq!(fill("{version}: {app}", &[("app", "A"), ("version", "1")]), "1: A");
        // Unknown and unclosed placeholders stay; a value holding a placeholder stays as it is.
        assert_eq!(fill("{nope} and {app", &[("app", "X")]), "{nope} and {app");
        // An unknown candidate stays whole through its closing brace, as craft_i18n::fmt keeps it.
        assert_eq!(fill("{bad {app}", &[("app", "X")]), "{bad {app}");
        assert_eq!(fill("{bad} {app}", &[("app", "X")]), "{bad} X");
        assert_eq!(fill("by {developer}", &[("developer", "{developer}")]), "by {developer}");
        assert_eq!(fill("é {app} ü", &[("app", "日本")]), "é 日本 ü");
        assert_eq!(fill("", &[]), "");
        assert_eq!(fill("{}", &[]), "{}");
    }

    #[test]
    fn english_is_the_default_and_buttons_are_capitals() {
        assert_eq!(english("{app} is ready", &[("app", "PdfCraft")]), "PdfCraft is ready");
        assert_eq!(button(english, "Check Again"), "CHECK AGAIN");
        fn french(text: &str, args: &[(&str, &str)]) -> String {
            fill(if text == "Later" { "Plus tard" } else { text }, args)
        }
        assert_eq!(button(french, "Later"), "PLUS TARD");
        let note = Note::new("Close {app} to finish", &[("app", "PdfCraft")]);
        assert_eq!(note.translated(english), "Close PdfCraft to finish");
    }
}
