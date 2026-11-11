//! The font sources, for a host's settings page: the built-in ones, the index URLs the user
//! added (never curated, and flagged when their file changed since it was added), adding one,
//! and downloading them all again.

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_host::Host;
use craft_theme::variant::{Emphasis, Tone};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Badge, Shade, Spinner, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::runtime::env::Env;
use craft_ui::sections::Section;
use craft_ui::style::{TypeRole, Variant};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use crate::panel::app::FontsPanel;

/// The address shown in the empty add-source field.
const ADDRESS_EXAMPLE: &str = "https://example.org/fonts/index.json";
/// What the store reports for a user source whose index has not been fetched yet: shown quietly, not as a failure.
const NOT_DOWNLOADED: &str = "not downloaded yet";

craft_ui::words! {
    /// The font sources' words, English by default.
    #[non_exhaustive]
    pub struct FontSourcesWords {
        title: "fontSources.title" = "Font sources";
        /// `{n}` families.
        families: count "fontSources.families" = "{n} family" | "{n} families";
        remove: "fontSources.remove" = "Remove";
        /// `{source}`.
        remove_source: template "fontSources.removeSource" = "Remove {source}";
        not_curated: "fontSources.notCurated" = "Not curated";
        changed: "fontSources.changed" = "Changed since added";
        changed_note: "fontSources.changedNote" = "The index at this address isn't the one first downloaded from it. Check the source before installing from it.";
        add_a_source: "fontSources.addASource" = "Add a source";
        add: "fontSources.add" = "Add";
        index_address: "fontSources.indexAddress" = "Index address";
        add_warning: "fontSources.addWarning" = "A source you add isn't reviewed: add only indexes you trust. Every font is still checked against its index before it is used.";
        downloading: "fontSources.downloading" = "Downloading the sources…";
        update: "fontSources.update" = "Update sources";
        allow_to_update: "fontSources.allowToUpdate" = "Allow downloads to update";
        added: "fontSources.added" = "Added. It downloads once downloads are allowed.";
        /// `{error}`.
        couldnt_add: template "fontSources.couldntAdd" = "The source couldn't be added: {error}";
        /// `{error}`.
        couldnt_remove: template "fontSources.couldntRemove" = "The source couldn't be removed: {error}";
    }
}

/// What the font sources report.
#[derive(Debug)]
#[non_exhaustive]
pub struct FontSourcesOutput {
    /// The section's response.
    pub response: Response,
}

impl Outcome for FontSourcesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The font sources of `panel`: the built-in ones and the user's, a field to add one, and Update. A host shows it in
/// its settings.
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct FontSources<'a> {
    panel: &'a mut FontsPanel,
}

impl<'a> FontSources<'a> {
    /// The sources of `panel`.
    pub fn new(panel: &'a mut FontsPanel) -> Self {
        FontSources { panel }
    }

    /// Shows the sources; adding, removing and updating act on the panel in this pass.
    pub fn show(self, ui: &mut Ui) -> FontSourcesOutput {
        let FontSources { panel } = self;
        let env = Env::of(ui);
        let words = env.words::<FontSourcesWords>();
        let host: Option<Arc<Host>> = env.view().host().cloned();
        panel.poll_sources();
        let index = panel.index();
        let user = panel.installer.user_sources();
        let allowed = panel.allowed();
        let fetching = panel.fetching_sources();
        let message = panel.source_message.clone();
        let mut url = std::mem::take(&mut panel.source_url);
        let (mut add, mut update, mut remove) = (false, false, None);
        let families = |n: usize| words.families.text(n as u64, &env.format().integer(n as u64));
        let sources = || index.iter().flat_map(|i| i.sources().iter());

        let shown = Section::new(words.title.clone()).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for src in sources().filter(|src| src.user_url.is_none()) {
                    Row::new().show(ui, |ui, _| {
                        Text::new(src.name.clone()).show(ui);
                        Text::new(families(src.families)).shade(Shade::Faint).show(ui);
                    });
                }
                for added in &user {
                    let source = sources().find(|src| src.user_url.as_deref() == Some(added.url.as_str()));
                    let problem = index.iter().flat_map(|i| i.problems().iter()).find(|p| p.source == added.url);
                    Stack::new().show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            Text::new(source.map_or(added.url.clone(), |src| src.name.clone())).show(ui);
                            let button = Button::new(words.remove.clone())
                                .description(words.remove_source.fill(&[("source", &added.url)]))
                                .variant(Variant::DANGER)
                                .show(ui);
                            if clicked(&button) {
                                remove = Some(added.url.clone());
                            }
                        });
                        Row::new().show(ui, |ui, _| {
                            Badge::new(words.not_curated.clone()).variant(Variant::look(Emphasis::Soft, Tone::Warn)).show(ui);
                            if source.is_some_and(|src| src.changed) {
                                Badge::new(words.changed.clone())
                                    .variant(Variant::look(Emphasis::Soft, Tone::Bad))
                                    .tooltip(words.changed_note.clone())
                                    .show(ui);
                            }
                        });
                        Text::new(added.url.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                        match (source, problem) {
                            (_, Some(p)) if p.error == NOT_DOWNLOADED => {
                                Text::new(p.error.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
                            }
                            (_, Some(p)) => {
                                Text::new(p.error.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
                            }
                            (Some(src), None) => {
                                Text::new(families(src.families)).shade(Shade::Faint).show(ui);
                            }
                            (None, None) => {}
                        }
                    });
                }

                Text::new(words.add_a_source.clone()).shade(Shade::Faint).show(ui);
                Row::new().show(ui, |ui, _| {
                    let field = TextField::new(words.index_address.clone(), Bind::bound(&mut url)).placeholder(ADDRESS_EXAMPLE).submits().show(ui);
                    let ready = !url.trim().is_empty();
                    let button = Button::new(words.add.clone()).enabled(ready).show(ui);
                    add = ready && (field.submitted || clicked(&button));
                });
                Text::new(words.add_warning.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);

                Row::new().show(ui, |ui, _| {
                    if fetching {
                        Spinner::new().show(ui);
                        Text::new(words.downloading.clone()).shade(Shade::Muted).show(ui);
                    } else {
                        update = clicked(&Button::new(words.update.clone()).enabled(allowed).show(ui));
                        if !allowed {
                            Text::new(words.allow_to_update.clone()).shade(Shade::Faint).show(ui);
                        }
                    }
                });
                if let Some((error, text)) = &message {
                    Notice::new(if *error { Tone::Bad } else { Tone::Info }, text.clone()).show(ui);
                }
            });
        });

        panel.source_url = url;
        if update {
            panel.fetch_sources(host.as_ref());
        }
        if add {
            let url = panel.source_url.trim().to_owned();
            match panel.installer.add_user_source(&url) {
                Ok(()) => {
                    panel.source_url.clear();
                    panel.source_message = (!allowed).then(|| (false, env.view().resolve(&words.added).as_str().to_owned()));
                    panel.reload_index();
                    panel.fetch_sources(host.as_ref());
                }
                Err(e) => panel.source_message = Some((true, words.couldnt_add.fill(&[("error", &e.0)]))),
            }
        }
        if let Some(url) = remove {
            match panel.installer.remove_user_source(&url) {
                Ok(()) => {
                    panel.source_message = None;
                    panel.reload_index();
                }
                Err(e) => panel.source_message = Some((true, words.couldnt_remove.fill(&[("error", &e.0)]))),
            }
        }
        FontSourcesOutput { response: shown.output.response }
    }
}
