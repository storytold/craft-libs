//! The words the update dialogs and the `Updater` (feature `ui`) show, in the app's
//! interface language.
//!
//! The crate keeps its text in English, and that English text is the lookup key, as in the
//! craft-i18n catalogs. An app with translations passes a [`Translate`] function to
//! `Updater::with_translator`. The updater calls it each time it hands the prompt a message, so a change of
//! interface language shows on the next frame. Without one the text stays in English. The dialogs' own words
//! are the app's `UpdateWords` set.
//!
//! A template names its values in braces (`{app} {version} is available…`) and the function
//! gets the values beside it, so a translation can put them in another order. An app built on
//! craft-i18n passes `|text, args| craft_i18n::fmt(i18n.t(lang, text), args)`. That also keeps a
//! Latin app name in its place in a right-to-left sentence.

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

/// One of the crate's own messages under the startup prompt ("The update is installed…"), kept as
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
    /// A note for the English `text`, with the values of its `{name}` placeholders in `args`.
    pub fn new(text: &'static str, args: &[(&'static str, &str)]) -> Self {
        Self { text, args: args.iter().map(|(k, v)| (*k, (*v).to_owned())).collect() }
    }

    /// The note in the interface language.
    pub fn translated(&self, translate: Translate) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        translate(self.text, &args)
    }
}
