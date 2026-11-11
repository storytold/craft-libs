//! `Env::folder` and `Env::collator`: the text folding and the sort order of the scope's language.

use super::{Env, EnvView, Scoped};
use craft_i18n::{Collator, Folder};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

impl Scoped {
    /// The language code of the host's interface language, English without a host or a catalog.
    fn language_code(&self) -> String {
        let code = self.host.as_ref().and_then(|host| host.i18n_shared().map(|i18n| i18n.code(host.lang())));
        code.unwrap_or("en").to_owned()
    }

    /// The folder of the host's interface language.
    fn language_folder(&self) -> Folder {
        Folder::new(&self.language_code())
    }

    /// The collator of the host's interface language, built once per language.
    fn language_collator(&self) -> Arc<Collator> {
        static BUILT: OnceLock<Mutex<HashMap<String, Arc<Collator>>>> = OnceLock::new();
        let code = self.language_code();
        let mut built = BUILT.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(built.entry(code).or_insert_with_key(|code| Arc::new(Collator::new(code))))
    }
}

impl Env {
    /// Folds text for search in the scope's language: case and accents ignored, dotted and dotless i by locale.
    pub fn folder(&self) -> Folder {
        self.language_folder()
    }

    /// Orders text in the scope's language, with runs of digits by value (`Layer 2` before `Layer 10`).
    pub fn collator(&self) -> Arc<Collator> {
        self.language_collator()
    }
}

impl EnvView<'_> {
    /// [`Env::folder`] for the env a component's context reads.
    pub fn folder(&self) -> Folder {
        self.scoped.language_folder()
    }

    /// [`Env::collator`] for the env a component's context reads.
    pub fn collator(&self) -> Arc<Collator> {
        self.scoped.language_collator()
    }
}
