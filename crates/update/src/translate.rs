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

/// A failure whose crate-authored wording can be translated when it is displayed.
/// External text is opaque data; it is never used as a translation key.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reason {
    body: ReasonBody,
    contexts: Vec<Message>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ReasonBody {
    Data(String),
    Message(Message),
    Joined(Box<Reason>, Box<Reason>),
    Trimmed(Box<Reason>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    text: String,
    args: Vec<(String, String)>,
}

impl Message {
    fn new(text: &str, args: &[(&str, &str)]) -> Self {
        Self { text: text.to_owned(), args: args.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect() }
    }

    fn translated(&self, translate: Translate, cause: Option<&str>) -> String {
        let mut args: Vec<(&str, &str)> =
            self.args.iter().filter(|(key, _)| cause.is_none() || key != "reason").map(|(key, value)| (key.as_str(), value.as_str())).collect();
        if let Some(cause) = cause {
            args.push(("reason", cause));
        }
        translate(&self.text, &args)
    }

    fn english_bounded(&self, cause: Option<&str>, limit: usize) -> Option<String> {
        let mut args: Vec<(&str, &str)> =
            self.args.iter().filter(|(key, _)| cause.is_none() || key != "reason").map(|(key, value)| (key.as_str(), value.as_str())).collect();
        if let Some(cause) = cause {
            args.push(("reason", cause));
        }
        fill_bounded(&self.text, &args, limit)
    }
}

impl Reason {
    /// Text supplied by GitHub, an OS/tool, or a caller, kept exactly as supplied.
    pub fn data(text: impl Into<String>) -> Self {
        Self { body: ReasonBody::Data(text.into()), contexts: Vec::new() }
    }

    /// A crate-authored English key and its opaque placeholder values.
    pub fn new(text: &str, args: &[(&str, &str)]) -> Self {
        Self { body: ReasonBody::Message(Message::new(text, args)), contexts: Vec::new() }
    }

    /// Adds an authored context. Its reserved `{reason}` value is the rendered cause.
    pub fn context(mut self, text: &str, args: &[(&str, &str)]) -> Self {
        self.contexts.push(Message::new(text, args));
        self
    }

    /// Renders with the current translator, without interpreting inserted values as templates.
    pub fn translated(&self, translate: Translate) -> String {
        let mut text = match &self.body {
            ReasonBody::Data(text) => text.clone(),
            ReasonBody::Message(message) => message.translated(translate, None),
            ReasonBody::Joined(first, second) => format!("{}; {}", first.translated(translate), second.translated(translate)),
            ReasonBody::Trimmed(reason) => reason.translated(translate).trim().to_owned(),
        };
        for context in &self.contexts {
            text = context.translated(translate, Some(&text));
        }
        text
    }

    // A stored companion is untrusted even when its serialized bytes fit the record limit.
    // Reject oversized intermediate values too, before a later frame can amplify them again.
    pub(crate) fn english_bounded(&self, limit: usize) -> Option<String> {
        let mut text = match &self.body {
            ReasonBody::Data(text) => {
                if text.len() > limit {
                    return None;
                }
                text.clone()
            }
            ReasonBody::Message(message) => message.english_bounded(None, limit)?,
            ReasonBody::Joined(first, second) => {
                let first = first.english_bounded(limit)?;
                let second = second.english_bounded(limit)?;
                let len = first.len().checked_add(2)?.checked_add(second.len())?;
                if len > limit {
                    return None;
                }
                let mut joined = String::with_capacity(len);
                joined.push_str(&first);
                joined.push_str("; ");
                joined.push_str(&second);
                joined
            }
            ReasonBody::Trimmed(reason) => reason.english_bounded(limit)?.trim().to_owned(),
        };
        for context in &self.contexts {
            text = context.english_bounded(Some(&text), limit)?;
        }
        Some(text)
    }

    pub(crate) fn joined(self, other: Self) -> Self {
        Self { body: ReasonBody::Joined(Box::new(self), Box::new(other)), contexts: Vec::new() }
    }

    pub(crate) fn trimmed(self) -> Self {
        Self { body: ReasonBody::Trimmed(Box::new(self)), contexts: Vec::new() }
    }
}

impl From<String> for Reason {
    fn from(text: String) -> Self {
        Self::data(text)
    }
}

impl From<&str> for Reason {
    fn from(text: &str) -> Self {
        Self::data(text)
    }
}

impl From<Note> for Reason {
    fn from(note: Note) -> Self {
        Self {
            body: ReasonBody::Message(Message {
                text: note.text.to_owned(),
                args: note.args.into_iter().map(|(key, value)| (key.to_owned(), value)).collect(),
            }),
            contexts: Vec::new(),
        }
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.translated(english))
    }
}

impl std::error::Error for Reason {}

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
    // This sink always accepts a piece; only the bounded sink below can stop early.
    let _ = fill_into(template, args, |value| {
        out.push_str(value);
        Some(())
    });
    out
}

fn fill_bounded(template: &str, args: &[(&str, &str)], limit: usize) -> Option<String> {
    let mut out = String::with_capacity(limit);
    fill_into(template, args, |value| {
        if out.len().checked_add(value.len())? > limit {
            return None;
        }
        out.push_str(value);
        Some(())
    })?;
    Some(out)
}

fn fill_into(template: &str, args: &[(&str, &str)], mut append: impl FnMut(&str) -> Option<()>) -> Option<()> {
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        append(before)?;
        let Some((name, after_close)) = after_open.split_once('}') else {
            append("{")?;
            rest = after_open;
            break;
        };
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => append(value)?,
            None => {
                append("{")?;
                append(name)?;
                append("}")?;
            }
        }
        rest = after_close;
    }
    append(rest)?;
    Some(())
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    fn first(text: &str, args: &[(&str, &str)]) -> String {
        format!("A<{}>", fill(text, args))
    }

    fn second(text: &str, args: &[(&str, &str)]) -> String {
        format!("B<{}>", fill(text, args))
    }

    #[trial]
    fn reloaded_compound_failures_translate_authored_frames_and_keep_data_opaque() {
        let data = "OS: 雪 {reason} {app}";
        let failure = Reason::data(data)
            .context("copy {path}: {reason}", &[("path", "{app}"), ("reason", "must not replace the cause")])
            .joined(Reason::new("rollback failed", &[]))
            .joined(Reason::new("restart failed", &[]).context("helper: {reason}", &[]))
            .context("update failed: {reason}", &[]);
        let loaded: Reason = serde_json::from_slice(&serde_json::to_vec(&failure).unwrap()).unwrap();
        let a = loaded.translated(first);
        let b = loaded.translated(second);
        assert_ne!(a, b);
        assert_eq!(a.matches("A<").count(), 5);
        assert_eq!(b.matches("B<").count(), 5);
        assert!(a.contains(data) && b.contains(data));
        assert!(a.contains("copy {app}: OS:"));
        assert!(!a.contains("must not replace the cause"));
        assert_eq!(loaded.to_string(), failure.translated(english));
        assert_eq!(Reason::from("rollback failed").translated(first), "rollback failed");
        let english = loaded.to_string();
        assert_eq!(loaded.english_bounded(english.len()), Some(english));
        assert_eq!(loaded.english_bounded(8), None);
        assert_eq!(Reason::new("{value}: {missing}{", &[("value", "é{reason}")]).english_bounded(32), Some("é{reason}: {missing}{".into()));
        assert_eq!(Reason::new("{value}", &[("value", "é")]).english_bounded(1), None);
        let mut expanding = Reason::data("x");
        for _ in 0..32 {
            expanding = expanding.context("{reason}{reason}", &[]);
        }
        assert!(serde_json::to_vec(&expanding).unwrap().len() < 4096);
        assert_eq!(expanding.english_bounded(4096), None);
    }

    #[trial]
    fn outcome_trimming_preserves_later_translation_and_context() {
        let reason = Reason::data("  raw {reason}  ").context("  cause: {reason}  ", &[]).trimmed().context("outer: {reason}", &[]);
        assert_eq!(reason.to_string(), "outer: cause:   raw {reason}");
        let loaded: Reason = serde_json::from_slice(&serde_json::to_vec(&reason).unwrap()).unwrap();
        assert_eq!(loaded.translated(second), reason.translated(second));
        assert_ne!(loaded.translated(first), loaded.translated(second));
        assert_eq!(loaded.english_bounded(4096), Some(reason.to_string()));
    }
}
