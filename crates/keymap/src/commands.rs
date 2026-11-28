//! Command words for the Host: labels from a craft-i18n catalog.

use std::sync::Arc;

use craft_host::Commands;
use craft_i18n::{I18n, Lang};

type HintSource = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// [`Commands`] over a craft-i18n catalog: a command's label is the catalog's `@id` row for its id
/// in `lang`, the English label where there is none. Build one with the language the Host is
/// built for. It does not hold the [`Keymap`](crate::Keymap).
#[derive(Clone)]
pub struct KeymapCommands {
    i18n: Arc<I18n>,
    lang: Lang,
    hints: Option<HintSource>,
}

impl KeymapCommands {
    /// Commands for `lang`, with texts from `i18n`.
    pub fn new(i18n: Arc<I18n>, lang: Lang) -> Self {
        KeymapCommands { i18n, lang, hints: None }
    }

    /// Tooltip lines for a command id, from the app's own source; without one there are none.
    #[must_use]
    pub fn with_hints(mut self, hints: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        self.hints = Some(Arc::new(hints));
        self
    }
}

impl Commands for KeymapCommands {
    fn label(&self, id: &str, english: &str) -> String {
        self.i18n.tid(self.lang, id, english).to_owned()
    }

    fn hint(&self, id: &str) -> Option<String> {
        self.hints.as_ref()?(id)
    }
}
