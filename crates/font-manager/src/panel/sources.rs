//! The font sources, for a host's settings page: the built-in ones, the index URLs the user
//! added (never curated, and flagged when their file changed since it was added), adding one,
//! and downloading them all again.

use craft_ui::direction;
use egui::{RichText, Ui, vec2};

use crate::panel::app::FontsPanel;
use crate::panel::i18n::{fmt, tl, tn};
use crate::panel::widgets::{self, Skin};

impl FontsPanel {
    /// Draws the font sources into `ui`, in the panel's look: a host shows it in its settings.
    pub fn sources_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let s = Skin { p: self.look.palette(&ctx), look: self.look.clone() };
        self.poll_sources();
        let index = self.index();
        let user = self.installer.user_sources();
        let now = ui.input(|i| i.time);
        let allowed = self.allowed(now);
        let mut remove = None;

        ui.spacing_mut().item_spacing.y = 6.0;
        let width = ui.available_width();
        egui::Frame::new().stroke(s.hairline()).corner_radius(s.look.corner).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let builtin = index.iter().flat_map(|i| i.sources().iter()).filter(|src| src.user_url.is_none());
            let mut first = true;
            for src in builtin {
                row(ui, &s, &mut first, |ui| {
                    ui.label(RichText::new(&src.name).color(s.p.ink_strong));
                    widgets::label(ui, &s, &tn(src.families as u64, "{n} family", "{n} families"), s.p.faint);
                });
            }
            for added in &user {
                let source = index.iter().flat_map(|i| i.sources().iter()).find(|src| src.user_url.as_deref() == Some(added.url.as_str()));
                let problem = index.iter().flat_map(|i| i.problems().iter()).find(|p| p.source == added.url);
                row(ui, &s, &mut first, |ui| {
                    ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h() - 6.0), direction::row(ui), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(source.map_or(added.url.as_str(), |src| src.name.as_str())).color(s.p.ink_strong));
                        ui.with_layout(direction::trailing(ui), |ui| {
                            let button = widgets::button(ui, &s, tl!("Remove"), widgets::Kind::Danger, 28.0, 0.0);
                            widgets::name(ui, &button, &fmt("Remove {source}", &[("source", &added.url)]));
                            if button.clicked() {
                                remove = Some(added.url.clone());
                            }
                        });
                    });
                    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), direction::row(ui), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        widgets::badge(ui, &s, tl!("Not curated"), s.p.warn, false);
                        if source.is_some_and(|src| src.changed) {
                            widgets::badge(ui, &s, tl!("Changed since added"), s.p.bad, true).on_hover_text(tl!(
                                "The index at this address isn't the one first downloaded from it. Check the source before installing from it."
                            ));
                        }
                    });
                    ui.label(RichText::new(&added.url).size(12.0).color(s.p.muted));
                    match (source, problem) {
                        (_, Some(p)) => {
                            ui.label(RichText::new(&p.error).size(12.0).color(if p.error == "not downloaded yet" { s.p.faint } else { s.p.bad }));
                        }
                        (Some(src), None) => {
                            widgets::label(ui, &s, &tn(src.families as u64, "{n} family", "{n} families"), s.p.faint);
                        }
                        (None, None) => {}
                    }
                });
            }
        });

        ui.add_space(8.0);
        widgets::label(ui, &s, tl!("Add a source"), s.p.faint);
        let mut add = false;
        ui.allocate_ui_with_layout(vec2(width, s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let button_w = s.button_galley(ui, tl!("Add"), s.p.ink).size().x + 28.0;
            let field = widgets::field(
                ui,
                &s,
                &mut self.source_url,
                "https://example.org/fonts/index.json",
                tl!("Index address"),
                ui.available_width() - button_w - 8.0,
                false,
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let enabled = !self.source_url.trim().is_empty();
            let clicked = ui.add_enabled_ui(enabled, |ui| widgets::button(ui, &s, tl!("Add"), widgets::Kind::Secondary, s.h(), 0.0)).inner.clicked();
            add = enabled && (clicked || enter);
        });
        let warning =
            tl!("A source you add isn't reviewed: add only indexes you trust. Every font is still checked against its index before it is used.");
        ui.add(egui::Label::new(RichText::new(warning).size(12.0).color(s.p.muted)).wrap());

        ui.add_space(4.0);
        ui.allocate_ui_with_layout(vec2(width, s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if self.fetching_sources() {
                ui.add(egui::Spinner::new().size(16.0).color(s.p.muted));
                ui.label(RichText::new(tl!("Downloading the sources…")).color(s.p.muted));
            } else {
                let update =
                    ui.add_enabled_ui(allowed, |ui| widgets::button(ui, &s, tl!("Update sources"), widgets::Kind::Secondary, s.h(), 0.0)).inner;
                if !allowed {
                    widgets::label(ui, &s, tl!("Allow downloads to update"), s.p.faint);
                }
                if update.clicked() {
                    self.fetch_sources(&ctx);
                }
            }
        });
        if let Some((error, text)) = &self.source_message {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *error { s.p.bad } else { s.p.muted })).wrap());
        }

        if add {
            let url = self.source_url.trim().to_owned();
            match self.installer.add_user_source(&url) {
                Ok(()) => {
                    self.source_url.clear();
                    self.source_message = (!allowed).then(|| (false, tl!("Added. It downloads once downloads are allowed.").to_owned()));
                    self.reload_index();
                    self.fetch_sources(&ctx);
                }
                Err(e) => self.source_message = Some((true, fmt("The source couldn't be added: {error}", &[("error", &e.0)]))),
            }
        }
        if let Some(url) = remove {
            match self.installer.remove_user_source(&url) {
                Ok(()) => {
                    self.source_message = None;
                    self.reload_index();
                }
                Err(e) => self.source_message = Some((true, fmt("The source couldn't be removed: {error}", &[("error", &e.0)]))),
            }
        }
    }
}

/// One source in the list: a hairline above every one but the first.
fn row(ui: &mut Ui, s: &Skin, first: &mut bool, add: impl FnOnce(&mut Ui)) {
    if !std::mem::take(first) {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, s.hairline());
    }
    egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.with_layout(direction::column(ui), add);
    });
}
