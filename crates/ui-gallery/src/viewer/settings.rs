//! What the viewer keeps between runs, in the gallery's own `config.toml`.

use super::appearance::{CONTRASTS, DENSITIES, DIRECTIONS, MODES, MOTIONS, SCALES, TEXT_SIZES, theme_named};
use super::catalog::Catalog;
use super::layout;
use super::state::{Preview, Viewer, Workspace};
use craft_settings::Settings;
use craft_ui::runtime::env::RootEnv;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The viewer's saved state. Choices are saved by name; a name this build does not know falls back to the default
/// of its field, and a layout it cannot read to the default layout, without costing the other fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GallerySettings {
    /// The id of the theme the stories are shown in.
    pub theme: String,
    /// Light or dark.
    pub mode: String,
    /// Standard or high contrast.
    pub contrast: String,
    /// The text-size step.
    pub text_size: String,
    /// The density preset.
    pub density: String,
    /// The motion mode.
    pub motion: String,
    /// The interface scale in percent.
    pub scale: String,
    /// The layout direction.
    pub direction: String,
    /// The workspace the dock shows: `components` or `theme`.
    pub workspace: String,
    /// What the theme workspace's preview shows: `app` or `components`.
    pub preview: String,
    /// The slug of the level the Overview shows; empty when none is chosen.
    pub level: String,
    /// The name of the story each component's card shows, by the component's registered type name.
    pub stories: BTreeMap<String, String>,
    /// The text that narrows the component tree.
    pub filter: String,
    /// The registered type name of the selected component; empty when none is.
    pub selected: String,
    /// The slugs of the levels that are open in the tree.
    pub open_levels: Vec<String>,
    /// The components workspace's dock layout, as `layout::save` writes it. Its own field and its own text, so a layout
    /// this build cannot read costs only the layout.
    pub dock: String,
    /// The theme workspace's dock layout, as `layout::save` writes it.
    pub theme_dock: String,
}

impl Settings for GallerySettings {
    const APP_ID: &'static str = "craft-ui-gallery";
    const SCHEMA_VERSION: u32 = 2;
}

impl GallerySettings {
    /// The saved state of `viewer`.
    pub fn of(viewer: &Viewer) -> GallerySettings {
        let env = &viewer.env;
        let browser = &viewer.browser;
        GallerySettings {
            theme: env.theme.as_str().to_owned(),
            mode: MODES.name(env.mode).to_owned(),
            contrast: CONTRASTS.name(env.contrast).to_owned(),
            text_size: TEXT_SIZES.name(env.text_size).to_owned(),
            density: DENSITIES.name(env.density).to_owned(),
            motion: MOTIONS.name(env.motion).to_owned(),
            scale: SCALES.name(env.scale).to_owned(),
            direction: DIRECTIONS.name(env.direction).to_owned(),
            workspace: workspace_name(viewer.workspace).to_owned(),
            preview: preview_name(viewer.preview).to_owned(),
            level: browser.level.clone(),
            stories: browser.story_of.iter().map(|(component, story)| ((*component).to_owned(), (*story).to_owned())).collect(),
            filter: browser.filter.clone(),
            selected: viewer.selected_type().unwrap_or_default().to_owned(),
            open_levels: viewer.open_levels(),
            dock: layout::save(&browser.dock),
            theme_dock: layout::save(&viewer.theme_dock),
        }
    }

    /// Sets the root values these settings name on `env`; a field with an unknown name takes its default.
    pub fn restore_env(&self, env: &mut RootEnv) {
        let default = RootEnv::default();
        env.theme = theme_named(&self.theme).unwrap_or(default.theme);
        env.mode = MODES.parse(&self.mode).unwrap_or(default.mode);
        env.contrast = CONTRASTS.parse(&self.contrast).unwrap_or(default.contrast);
        env.text_size = TEXT_SIZES.parse(&self.text_size).unwrap_or(default.text_size);
        env.density = DENSITIES.parse(&self.density).unwrap_or(default.density);
        env.motion = MOTIONS.parse(&self.motion).unwrap_or(default.motion);
        env.scale = SCALES.parse(&self.scale).unwrap_or(default.scale);
        env.direction = DIRECTIONS.parse(&self.direction).unwrap_or(default.direction);
    }

    /// The workspace these settings show; the components workspace when the name is unknown.
    pub fn workspace(&self) -> Workspace {
        if self.workspace == workspace_name(Workspace::Theme) { Workspace::Theme } else { Workspace::Components }
    }

    /// The preview these settings show; the app when the name is unknown.
    pub fn preview(&self) -> Preview {
        if self.preview == preview_name(Preview::Components) { Preview::Components } else { Preview::App }
    }

    /// The story each component shows, for the components and stories `catalog` has.
    pub fn story_of(&self, catalog: &Catalog) -> HashMap<&'static str, &'static str> {
        let mut shown = HashMap::new();
        for (component, story) in &self.stories {
            let Some(info) = catalog.component(component) else { continue };
            if let Some(name) = info.stories.iter().find(|name| **name == story.as_str()) {
                shown.insert(info.type_name, *name);
            }
        }
        shown
    }
}

fn workspace_name(workspace: Workspace) -> &'static str {
    match workspace {
        Workspace::Components => "components",
        Workspace::Theme => "theme",
    }
}

fn preview_name(preview: Preview) -> &'static str {
    match preview {
        Preview::App => "app",
        Preview::Components => "components",
    }
}
