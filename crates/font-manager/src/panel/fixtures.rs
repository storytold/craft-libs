//! What the panel's stories and trials show: four families of the Google Fonts index, an in-memory catalog of two
//! bundled faces, and a store that refuses downloads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft_font_catalog::{Limits, Os};

use crate::index::Index;
use crate::store::FakeStore;
use crate::{Catalog, FontEnv, FontsPanel, Options, Source};

/// Four families of the Google Fonts index, as the story's own fixed JSON.
pub(crate) fn story_index() -> Result<Index, String> {
    let file = |path: &str| {
        format!(r#"{{"path":"{path}","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}"#, "0a".repeat(32))
    };
    let family = |name: &str, category: &str, languages: &str, scripts: &str, path: &str| {
        format!(
            r#"{{"family":"{name}","designer":"","categories":["{category}"],"languages":[{languages}],"scripts":[{scripts}],"license":"OFL","license_path":null,"files":[{}]}}"#,
            file(path)
        )
    };
    let json = format!(
        r#"{{"schema":1,"version":"1.0","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/","fallback_base_url":"https://raw.githubusercontent.com/google/fonts/45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[
          {{"id":"en_Latn","name":"English","autonym":"English","script":"Latn","population":1,"historical":false}},
          {{"id":"ja_Jpan","name":"Japanese","autonym":"日本語","script":"Jpan","population":1,"historical":false}}],
        "scripts":[{{"id":"Jpan","name":"Japanese"}},{{"id":"Latn","name":"Latin"}}],
        "families":[{},{},{},{}]}}"#,
        family("Amiri", "SERIF", "1", r#""Latn""#, "ofl/amiri/Amiri-Regular.ttf"),
        family("Hack", "MONOSPACE", "0", r#""Latn""#, "ofl/hack/Hack-Regular.ttf"),
        family("Noto Sans JP", "SANS_SERIF", "0,1", r#""Jpan","Latn""#, "ofl/notosansjp/NotoSansJP[wght].ttf"),
        family("Ubuntu", "SANS_SERIF", "0", r#""Latn""#, "ufl/ubuntu/Ubuntu-Light.ttf"),
    );
    Index::from_json(json.as_bytes()).map_err(|e| format!("the story index: {e}"))
}

/// An in-memory catalog of two bundled faces (Ubuntu Light and Hack), no folders scanned.
pub(crate) fn story_catalog() -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    for bytes in [epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::HACK_REGULAR] {
        let _ = catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default());
    }
    catalog
}

/// The panel over the story catalog and index; its store is a fake that refuses downloads.
pub(crate) fn story_panel() -> FontsPanel {
    let store = FakeStore::new(std::env::temp_dir().join("craft-fonts-panel-story"), HashMap::new());
    FontsPanel::new(Options { catalog: story_catalog(), installer: Arc::new(store), index: story_index() })
}

/// A panel a story keeps between passes: cloning shares it, and two handles are equal when they are the same panel.
#[derive(Clone)]
pub(crate) struct SharedPanel(Arc<Mutex<FontsPanel>>);

impl PartialEq for SharedPanel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl SharedPanel {
    /// Shares `panel`.
    pub(crate) fn new(panel: FontsPanel) -> Self {
        SharedPanel(Arc::new(Mutex::new(panel)))
    }

    /// The panel, for the pass.
    pub(crate) fn lock(&self) -> MutexGuard<'_, FontsPanel> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
