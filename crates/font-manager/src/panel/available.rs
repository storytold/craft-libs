//! Making a store family available to all apps: the detail view's switch, what the last change
//! did, the styles not copied yet, and the confirmation before removing an available family.
//!
//! The store copies the family's files into the OS user font folder. Only the user's own action
//! does that, so the panel never offers it to an agent; it runs for the user alone. The interface
//! names the folder after the OS ("macOS Fonts"): "all apps" read as all Craft apps.

use std::path::PathBuf;
use std::sync::Arc;

use craft_ui::direction;
use egui::{Context, Frame, Margin, RichText, Ui};

use crate::panel::app::{FontsPanel, store_files};
use crate::panel::i18n::{fmt, tl, tn};
use crate::panel::jobs::{Finished, Kind, Request, Running};
use crate::panel::widgets::{self, Skin};
use crate::store::{Conflict, MadeAvailable, StoreError};
use crate::{FaceInfo, Source};

/// What the last change to a family's availability did, shown under its switch.
#[derive(Clone, Debug)]
pub(crate) enum Shared {
    Done,
    Refused(Vec<Conflict>),
    /// Withdrawn, leaving these copies in place because they changed since.
    Kept(Vec<PathBuf>),
    Failed(Kind, String),
}

/// A family's availability as last read from the store, for the catalog generation it was read
/// at: reading it can mean reading the store's manifest, too slow for every frame.
pub(crate) struct Availability {
    generation: u64,
    available: bool,
    /// The store names of its files not available to all apps yet.
    pending: Vec<String>,
    /// Its copies in the OS folder this build can't change.
    out_of_reach: Vec<PathBuf>,
}

/// The OS user font folder as the interface names it, for `{place}`.
pub(crate) fn os_fonts() -> &'static str {
    if cfg!(target_os = "macos") {
        tl!("macOS Fonts")
    } else if cfg!(target_os = "windows") {
        tl!("Windows Fonts")
    } else if cfg!(target_os = "linux") {
        tl!("the Linux fonts folder")
    } else {
        tl!("your fonts folder")
    }
}

/// `template` translated, with `{place}` the OS user font folder.
pub(crate) fn in_os_fonts(template: &str) -> String {
    fmt(template, &[("place", os_fonts())])
}

/// The line under the switch: who gains from it.
fn help() -> &'static str {
    if cfg!(target_os = "macos") {
        tl!("Other apps on this computer, like Word or Pages, can then use it. Craft apps have it either way.")
    } else if cfg!(target_os = "windows") {
        tl!("Other apps on this computer, like Word or LibreOffice, can then use it. Craft apps have it either way.")
    } else {
        tl!("Other apps on this computer, like LibreOffice, can then use it. Craft apps have it either way.")
    }
}

impl FontsPanel {
    /// The store family `family` holding `files`: whether it is available to all apps, its
    /// files not available yet, and its copies this build can't change.
    fn availability(&mut self, family: &str, files: &[PathBuf]) -> &Availability {
        let generation = self.catalog.generation();
        let key = family.to_lowercase();
        if self.availability.get(&key).is_none_or(|a| a.generation != generation) {
            let available = self.installer.is_available_to_all(family, files);
            let pending = if available { self.installer.not_yet_available(family, files) } else { Vec::new() };
            let out_of_reach = if available { self.installer.copies_out_of_reach(family, files) } else { Vec::new() };
            self.availability.insert(key.clone(), Availability { generation, available, pending, out_of_reach });
        }
        &self.availability[&key]
    }

    /// Whether `face` of `family` is a store style the family's copies don't include yet.
    pub(crate) fn pending_style(&self, family: &str, face: &FaceInfo) -> bool {
        let Some(a) = self.availability.get(&family.to_lowercase()) else { return false };
        let name = face.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy());
        face.source == Source::Store && name.is_some_and(|n| a.pending.iter().any(|p| *p == n))
    }

    /// The "Available to all apps" switch of a store family, under its install and remove
    /// buttons, with what the last change did. `faces` are the family's installed faces.
    pub(crate) fn available_to_all(&mut self, ui: &mut Ui, s: &Skin, family: &str, faces: &[FaceInfo]) {
        let files = store_files(faces);
        if files.is_empty() {
            return;
        }
        if !self.installer.available_to_all_supported() {
            let out_of_reach = self.availability(family, &files).out_of_reach.clone();
            if !out_of_reach.is_empty() {
                ui.add_space(4.0);
                self.out_of_reach(ui, s, &out_of_reach);
            }
            return;
        }
        ui.add_space(4.0);
        let ctx = ui.ctx().clone();
        let a = self.availability(family, &files);
        let (available, pending) = (a.available, a.pending.clone());
        let job = self.jobs.of("", family).map(|j| j.kind);
        let busy = job.is_some();
        // While a change runs, the state it asks for.
        let mut on = match job {
            Some(Kind::MakeAvailable) => true,
            Some(Kind::Withdraw) => false,
            _ => available,
        };
        let label = in_os_fonts("Install in {place}");
        let changed = ui.add_enabled_ui(!busy, |ui| ui.checkbox(&mut on, label)).inner.changed();
        ui.add(egui::Label::new(RichText::new(help()).size(12.5).color(s.p.muted)).wrap());
        if changed {
            let request =
                if on { Request::MakeAvailable(family.to_owned(), files.clone()) } else { Request::Withdraw(family.to_owned(), files.clone()) };
            self.start_sharing(&ctx, family, request);
        }
        if available && !pending.is_empty() && !busy {
            let mut again = false;
            let action = in_os_fonts("Install these in {place} too");
            widgets::note_with_actions(
                ui,
                s,
                |ui| {
                    let text = tn(pending.len() as u64, "{n} style isn't in {place} yet.", "{n} styles aren't in {place} yet.");
                    let text = craft_i18n::fmt(&text, &[("place", os_fonts())]);
                    ui.label(RichText::new(text).color(s.p.warn));
                },
                &[&action],
                |ui, _| again = widgets::button(ui, s, &action, widgets::Kind::Secondary, 28.0, 0.0).clicked(),
                false,
            );
            if again {
                self.start_sharing(&ctx, family, Request::MakeAvailable(family.to_owned(), files));
            }
        }
        let Some(shared) = self.sharing.get(&family.to_lowercase()).cloned() else { return };
        let small = |text: &str, color| egui::Label::new(RichText::new(text).size(12.5).color(color)).wrap();
        match shared {
            Shared::Done => {
                ui.add(small(&in_os_fonts("Some apps that are already running see the fonts in {place} only after they restart."), s.p.muted));
            }
            Shared::Refused(conflicts) => {
                widgets::note(ui, s, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(egui::Label::new(RichText::new(in_os_fonts("Not installed in {place}.")).color(s.p.bad)).wrap());
                        let named: Vec<(&String, &PathBuf)> = conflicts.iter().filter_map(|c| Some((c.postscript.as_ref()?, &c.path))).collect();
                        if !named.is_empty() {
                            ui.label(RichText::new(tl!("Fonts on this system already have these names:")).color(s.p.ink));
                            for (postscript, path) in named {
                                ui.label(RichText::new(postscript).color(s.p.ink));
                                ui.add(small(&path.display().to_string(), s.p.muted));
                            }
                        }
                        for c in conflicts.iter().filter(|c| c.postscript.is_none()) {
                            let text = fmt("A file the store didn't add is in the way: {path}", &[("path", &c.path.display().to_string())]);
                            ui.add(small(&text, s.p.ink));
                        }
                    });
                });
            }
            Shared::Kept(paths) if !paths.is_empty() => {
                widgets::note(ui, s, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let text = in_os_fonts("These copies in {place} changed after they were added, so they stay there:");
                        ui.add(egui::Label::new(RichText::new(text).color(s.p.warn)).wrap());
                        for path in &paths {
                            ui.add(small(&path.display().to_string(), s.p.muted));
                        }
                    });
                });
            }
            Shared::Kept(_) => {}
            Shared::Failed(kind, error) => {
                let what = match kind {
                    Kind::Withdraw => in_os_fonts("Couldn't remove from {place}"),
                    _ => in_os_fonts("Couldn't install in {place}"),
                };
                widgets::note(ui, s, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(what).color(s.p.bad));
                        ui.add(small(&error, s.p.muted));
                    });
                });
            }
        }
    }

    /// The copies of a family in the OS folder this build can't change, and what would let it.
    fn out_of_reach(&self, ui: &mut Ui, s: &Skin, copies: &[PathBuf]) {
        widgets::note(ui, s, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let text = in_os_fonts("This family has copies in {place}, and this version of the app can't remove them.");
                ui.add(egui::Label::new(RichText::new(text).color(s.p.warn)).wrap());
                if self.catalog.env().flatpak {
                    let how = tl!("To remove them, give the app access to xdg-data/fonts again (in Flatseal, or with flatpak override --user --filesystem=xdg-data/fonts:create).");
                    ui.add(egui::Label::new(RichText::new(how).size(12.5).color(s.p.muted)).wrap());
                }
                let rest = tl!("Removing the family here removes it from Craft apps only. Its copies stay:");
                ui.add(egui::Label::new(RichText::new(rest).size(12.5).color(s.p.muted)).wrap());
                for path in copies {
                    ui.add(egui::Label::new(RichText::new(path.display().to_string()).size(12.5).color(s.p.muted)).wrap());
                }
            });
        });
    }

    fn start_sharing(&mut self, ctx: &Context, family: &str, request: Request) {
        self.sharing.remove(&family.to_lowercase());
        self.jobs.start(ctx, Arc::clone(&self.installer), Arc::clone(&self.catalog), request);
    }

    /// Takes the end of a job that made a family available or withdrew it.
    pub(crate) fn sharing_done(&mut self, job: &Running, result: Result<Finished, StoreError>) {
        let key = job.family.to_lowercase();
        self.availability.remove(&key);
        let shared = match result {
            Ok(Finished::MadeAvailable(MadeAvailable::Done)) => Shared::Done,
            Ok(Finished::MadeAvailable(MadeAvailable::Refused(conflicts))) => Shared::Refused(conflicts),
            Ok(Finished::Withdrawn(kept)) => Shared::Kept(kept),
            Ok(_) => return,
            Err(e) => {
                log::warn!("{} ({:?}): {e}", job.family, job.kind);
                Shared::Failed(job.kind, e.0)
            }
        };
        self.sharing.insert(key, shared);
    }

    /// The confirmation before removing fonts of a family available to all apps, whose copies in
    /// the OS font folder go with them.
    pub(crate) fn confirm_removal(&mut self, ctx: &Context) {
        let Some(request) = self.confirming.clone() else { return };
        let family = request.family.as_str();
        let args = [("family", family), ("place", os_fonts())];
        let text = if request.whole {
            fmt("{family} is installed in {place}. Removing it also removes it from {place}, and other apps can't use it any more.", &args)
        } else {
            fmt("{family} is installed in {place}. Removing this style also removes it from {place}.", &args)
        };
        let title = fmt("Remove {family}?", &[("family", family)]);
        let s = self.skin.clone();
        let mut remove = false;
        let mut cancel = false;
        let frame =
            Frame::new().fill(s.p.bg).stroke(egui::Stroke::new(1.0, s.p.line_strong)).corner_radius(s.look.corner).inner_margin(Margin::same(24));
        let response = egui::Modal::new(egui::Id::unique("craft-font-manager.remove-available")).frame(frame).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            let title = RichText::new(&title).size(22.0).family(s.look.display.clone()).color(s.p.ink_strong);
            ui.add(egui::Label::new(title).wrap());
            ui.add(egui::Label::new(RichText::new(text).color(s.p.ink)).wrap());
            let kept = in_os_fonts("Copies in {place} that changed after they were added stay there.");
            ui.add(egui::Label::new(RichText::new(kept).color(s.p.muted)).wrap());
            ui.add_space(8.0);
            direction::horizontal(ui, |ui| {
                remove = widgets::button(ui, &s, tl!("Remove"), widgets::Kind::Danger, s.h(), 0.0).clicked();
                cancel = widgets::button(ui, &s, tl!("Cancel"), widgets::Kind::Secondary, s.h(), 0.0).clicked();
            });
        });
        if remove {
            self.confirming = None;
            self.sharing.remove(&request.family.to_lowercase());
            self.start_removal(ctx, request);
        } else if cancel || response.should_close() {
            self.confirming = None;
        }
    }
}
