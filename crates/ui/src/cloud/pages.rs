//! The Backstage pages: File › Open and File › Save As, and the modal a
//! refused save puts up.
//!
//! Every side of a page is drawn only where the service reported the
//! capability behind it. A list under a heading the provider cannot fill reads
//! to a user as a broken window, so the heading goes too.

use craft_cloud::{Capability, Entry, EntryKind};
use egui::{RichText, Ui};

use super::{
    CloudApp, Link, Source, browse, column, converse, dim, document_name, entry_row, fill, link, load_shared, open_entry, opens, parent_of,
    reload_active, save_as, size_text, start, stem_of,
};

/// The character a label ends with when choosing it opens a dialog.
const ELLIPSIS: &str = "\u{2026}";

/// What an overflow menu asked for, deferred until the list is no longer being
/// drawn. Opening a dialog while borrowing the list is not possible.
enum Act {
    Share(Entry),
    Talk(Entry),
}

/// The account line, or the reason there is nothing to show yet.
fn header<A: CloudApp>(app: &mut A, ui: &mut Ui) {
    let style = app.style(ui);
    let mut retry = false;
    match (&app.cloud().account, &app.cloud().account_fault) {
        (_, Some(fault)) => {
            let text = fault.text(app.kind().noun);
            let again = app.tr("Try again");
            ui.horizontal(|ui| {
                dim(ui, &style, &text);
                if ui.button(again).clicked() {
                    retry = true;
                }
            });
        }
        (Some(account), None) => {
            let line = if account.display.is_empty() { app.tr("Signed in") } else { account.display.clone() };
            dim(ui, &style, &line);
        }
        (None, None) => {
            let waiting = app.tr("Connecting to your cloud account…");
            dim(ui, &style, &waiting);
        }
    }
    if retry {
        let cloud = app.cloud_mut();
        cloud.account_fault = None;
        cloud.asked_account = false;
        start(app);
    }
}

/// The offer to link an account, and the wait while the browser is open.
/// Returns true when it drew, because there is nothing else to show until an
/// account exists.
///
/// None of this is an error. The whole state means one thing: there is no
/// account yet, and linking one is a button away.
fn link_panel<A: CloudApp>(app: &mut A, ui: &mut Ui) -> bool {
    let Some(state) = app.cloud().link.clone() else { return false };
    let style = app.style(ui);
    let noun = app.kind().noun;
    let mut begin = false;
    let mut cancel = false;
    let mut browse_to: Option<String> = None;
    let words = LinkWords {
        none_yet: app.tr("No cloud account is linked yet."),
        why: app.tr(&format!("Link one to open and save {noun}s in your cloud storage.")),
        link_one: app.tr("Link a cloud account"),
        starting: app.tr("Starting…"),
        finish: app.tr("Finish linking in your browser."),
        waiting: app.tr("Waiting for your browser…"),
        open_in_browser: app.tr("Open in browser"),
        copy_link: app.tr("Copy link"),
        cancel: app.tr("Cancel"),
        again: app.tr("Try again"),
    };
    egui::Frame::NONE.fill(style.soft).corner_radius(6.0).inner_margin(12).show(ui, |ui| match state {
        Link::Offered => {
            ui.label(RichText::new(&words.none_yet).font(egui::FontId::new(13.0, style.bold.clone())).color(style.text));
            dim(ui, &style, &words.why);
            ui.add_space(8.0);
            begin = ui.button(&words.link_one).clicked();
        }
        Link::Starting => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                dim(ui, &style, &words.starting);
            });
        }
        Link::Waiting { url, .. } => {
            ui.label(RichText::new(&words.finish).font(egui::FontId::new(13.0, style.bold.clone())).color(style.text));
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                dim(ui, &style, &words.waiting);
            });
            ui.add_space(6.0);
            // A link alone is not enough. When the browser cannot be opened,
            // or opens the wrong thing, the only way out is carrying the
            // address across by hand, and a label cannot be selected.
            ui.horizontal(|ui| {
                if ui.button(&words.open_in_browser).clicked() {
                    browse_to = Some(url.clone());
                }
                if ui.button(&words.copy_link).clicked() {
                    ui.ctx().copy_text(url.clone());
                }
            });
            ui.add_space(4.0);
            if ui.link(&url).clicked() {
                browse_to = Some(url.clone());
            }
            ui.add_space(8.0);
            cancel = ui.button(&words.cancel).clicked();
        }
        Link::Failed(fault) => {
            ui.label(RichText::new(fault.text(noun)).font(egui::FontId::new(13.0, style.family.clone())).color(style.text));
            ui.add_space(8.0);
            begin = ui.button(&words.again).clicked();
        }
    });
    if let Some(url) = browse_to {
        app.open_url(&url);
    }
    if begin {
        link(app);
    } else if cancel {
        app.cloud_mut().link = Some(Link::Offered);
    }
    true
}

/// The panel's strings, read before it is drawn. The application is borrowed
/// mutably inside the frame, so nothing can be asked of it there.
struct LinkWords {
    none_yet: String,
    why: String,
    link_one: String,
    starting: String,
    finish: String,
    waiting: String,
    open_in_browser: String,
    copy_link: String,
    cancel: String,
    again: String,
}

/// The progress line for a read or a write in flight.
pub fn progress<A: CloudApp>(app: &A, ui: &mut Ui) {
    let Some(busy) = app.cloud().busy.clone() else { return };
    let style = app.style(ui);
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(15.0));
        ui.label(RichText::new(busy.text()).font(egui::FontId::new(12.5, style.family.clone())).color(style.text));
    });
}

/// The `…` beside a document, and what a person chose from it.
///
/// Only the actions the provider actually offers appear. A greyed-out item says
/// the action exists and is not possible right now. An absent one says this
/// provider does not do that at all, and those are different facts.
fn row_actions<A: CloudApp>(app: &A, ui: &mut Ui, entry: &Entry) -> Option<String> {
    let can = |c: Capability| app.cloud().account.as_ref().is_some_and(|a| a.can(c));
    let mut items = vec![crate::menu::MenuItem::new("open", app.tr("Open"))];
    if can(Capability::Sharing) {
        items.push(
            crate::menu::MenuItem::new("share", format!("{}{}", app.tr("Share"), ELLIPSIS))
                .detail(app.tr("See who this is shared with, and invite somebody"))
                .separated(),
        );
    }
    if can(Capability::Conversation) {
        items.push(crate::menu::MenuItem::new("talk", app.tr("Work together")).detail(app.tr("Open a conversation about this document")));
    }
    let labels = crate::menu::MenuLabels { more: app.tr("More options"), ..Default::default() };
    // Salted by the handle. Two rows drawing the same control need two menus,
    // and the handle is the one thing that differs between them.
    ui.push_id(entry.id.clone(), |ui| crate::menu::overflow(ui, &items, &labels)).inner
}

/// File › Open, the cloud part. Your own files sit on one side and what other
/// people shared with you sits on the other.
pub fn open_page<A: CloudApp>(app: &mut A, ui: &mut Ui) {
    start(app);
    header(app, ui);
    ui.add_space(8.0);
    if link_panel(app, ui) {
        return;
    }
    progress(app, ui);
    // Hidden where the provider has no incoming shares at all. An empty list
    // under that heading would read as a broken feature.
    let shared_offered = app.cloud().account.as_ref().is_some_and(|a| a.can(Capability::SharedWithMe));
    if shared_offered && !app.cloud().shared.loaded && !app.cloud().shared.loading {
        load_shared(app);
    }
    let mut pick: Option<(Entry, Source)> = None;
    let mut go: Option<String> = None;
    let mut act: Option<Act> = None;
    if shared_offered {
        ui.columns(2, |columns| {
            if let Some(ui) = columns.get_mut(0) {
                my_files(app, ui, &mut pick, &mut go, &mut act);
            }
            if let Some(ui) = columns.get_mut(1) {
                shared_files(app, ui, &mut pick, &mut act);
            }
        });
    } else {
        my_files(app, ui, &mut pick, &mut go, &mut act);
    }
    match act {
        // Sharing a document in the list, which is not the one that happens to
        // be open. The dialog is told which.
        Some(Act::Share(entry)) => app.sharing().open(entry.id.clone(), entry.name.clone()),
        Some(Act::Talk(entry)) => converse(app, entry.id.clone()),
        None => {}
    }
    if let Some(path) = go {
        browse(app, &path);
    } else if let Some((entry, source)) = pick {
        app.cloud_mut().source = source;
        open_entry(app, entry, None);
    }
    if let Some(status) = app.cloud().status.clone() {
        let style = app.style(ui);
        ui.add_space(10.0);
        dim(ui, &style, &status);
    }
}

/// Your own files: where you are, a way up, then the folders and the documents
/// this application can open.
fn my_files<A: CloudApp>(app: &mut A, ui: &mut Ui, pick: &mut Option<(Entry, Source)>, go: &mut Option<String>, act: &mut Option<Act>) {
    let style = app.style(ui);
    let kind = app.kind();
    let folder = app.cloud().mine.clone();
    let heading = app.tr("My files");
    let (home, up, loading) = (app.tr("Home"), app.tr("Up"), app.tr("Loading…"));
    let nothing = app.tr(&format!("Nothing here that {} can open.", app.app_name()));
    column(ui, &style, &heading, |ui| {
        let here = if folder.path.is_empty() { home } else { folder.path.clone() };
        ui.horizontal(|ui| {
            if let Some(parent) = parent_of(&folder.path)
                && ui.button(&up).clicked()
            {
                *go = Some(parent);
            }
            dim(ui, &style, &here);
        });
        ui.add_space(4.0);
        if folder.loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                dim(ui, &style, &loading);
            });
            return;
        }
        if let Some(fault) = &folder.error {
            dim(ui, &style, &fault.text(kind.noun));
            return;
        }
        let mut shown = 0;
        for entry in &folder.entries {
            let is_folder = entry.kind == EntryKind::Folder;
            if !is_folder && !opens(entry, kind) {
                continue;
            }
            shown += 1;
            let detail = if is_folder { String::new() } else { size_text(entry.size) };
            // A folder has nothing behind an ellipsis. Opening it is the only
            // thing to do with one here.
            let reserve = if is_folder { 0.0 } else { 26.0 };
            let (row, chose) = ui
                .horizontal(|ui| {
                    let row = entry_row(app, ui, entry, &detail, reserve).clicked();
                    let chose = if is_folder { None } else { row_actions(app, ui, entry) };
                    (row, chose)
                })
                .inner;
            match chose.as_deref() {
                Some("share") => *act = Some(Act::Share(entry.clone())),
                Some("talk") => *act = Some(Act::Talk(entry.clone())),
                Some("open") => *pick = Some((entry.clone(), Source::Mine)),
                _ if row => {
                    if is_folder {
                        *go = Some(entry.path.clone());
                    } else {
                        *pick = Some((entry.clone(), Source::Mine));
                    }
                }
                _ => {}
            }
        }
        if shown == 0 && folder.loaded {
            dim(ui, &style, &nothing);
        }
    });
}

/// What other people shared with you. One flat list, because an incoming share
/// has no folder of yours to sit in. Each row says who shared it.
fn shared_files<A: CloudApp>(app: &mut A, ui: &mut Ui, pick: &mut Option<(Entry, Source)>, act: &mut Option<Act>) {
    let style = app.style(ui);
    let kind = app.kind();
    let shared = app.cloud().shared.clone();
    let heading = app.tr("Shared with me");
    let (loading, with_you, read_only) = (app.tr("Loading…"), app.tr("Shared with you"), app.tr("read-only"));
    let nobody = app.tr(&format!("Nobody has shared a {} with you.", kind.noun));
    column(ui, &style, &heading, |ui| {
        if shared.loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                dim(ui, &style, &loading);
            });
            return;
        }
        if let Some(fault) = &shared.error {
            dim(ui, &style, &fault.text(kind.noun));
            return;
        }
        let mut shown = 0;
        for entry in shared.entries.iter().filter(|e| opens(e, kind)) {
            shown += 1;
            let who = entry.shared_by.clone().unwrap_or_else(|| with_you.clone());
            let detail = if entry.writable { who } else { format!("{who} ({read_only})") };
            let (row, chose) = ui
                .horizontal(|ui| {
                    let row = entry_row(app, ui, entry, &detail, 26.0).clicked();
                    (row, row_actions(app, ui, entry))
                })
                .inner;
            match chose.as_deref() {
                Some("share") => *act = Some(Act::Share(entry.clone())),
                Some("talk") => *act = Some(Act::Talk(entry.clone())),
                _ if row || chose.is_some() => *pick = Some((entry.clone(), Source::Shared)),
                _ => {}
            }
        }
        if shown == 0 && shared.loaded {
            dim(ui, &style, &nobody);
        }
    });
}

/// File › Save As, the cloud destination. Pick a folder, name the file, save.
/// Returns true when a save started, so a Backstage view can return to the
/// document.
pub fn save_page<A: CloudApp>(app: &mut A, ui: &mut Ui) -> bool {
    let style = app.style(ui);
    let kind = app.kind();
    start(app);
    // The destination above already names the account. This line is here to
    // say when something is wrong with it.
    if app.cloud().account.is_none() || app.cloud().account_fault.is_some() {
        header(app, ui);
        ui.add_space(8.0);
    }
    if link_panel(app, ui) {
        return false;
    }
    progress(app, ui);
    if app.cloud().save_name.is_empty() {
        let suggested = app.active().map(|uid| app.title(uid)).unwrap_or_default();
        app.cloud_mut().save_name = document_name(&suggested, kind);
    }
    let folder = app.cloud().mine.clone();
    let heading = app.tr("Folder");
    let (home, up, loading) = (app.tr("Home"), app.tr("Up"), app.tr("Loading…"));
    let only_folder = app.tr(&format!("No folders here. The {} saves in this one.", kind.noun));
    let mut go: Option<String> = None;
    column(ui, &style, &heading, |ui| {
        let here = if folder.path.is_empty() { home } else { folder.path.clone() };
        ui.horizontal(|ui| {
            if let Some(parent) = parent_of(&folder.path)
                && ui.button(&up).clicked()
            {
                go = Some(parent);
            }
            dim(ui, &style, &here);
        });
        ui.add_space(4.0);
        if folder.loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                dim(ui, &style, &loading);
            });
            return;
        }
        if let Some(fault) = &folder.error {
            dim(ui, &style, &fault.text(kind.noun));
            return;
        }
        let mut folders = 0;
        for entry in folder.entries.iter().filter(|e| e.kind == EntryKind::Folder) {
            folders += 1;
            if entry_row(app, ui, entry, "", 0.0).clicked() {
                go = Some(entry.path.clone());
            }
        }
        if folders == 0 && folder.loaded {
            dim(ui, &style, &only_folder);
        }
    });
    if let Some(path) = go {
        browse(app, &path);
    }
    ui.add_space(12.0);
    let (file_name, save_label) = (app.tr("File name"), app.tr("Save to the cloud"));
    let mut asked = false;
    ui.horizontal(|ui| {
        ui.label(file_name);
        let typing = ui.add(egui::TextEdit::singleline(&mut app.cloud_mut().save_name).desired_width(280.0));
        asked = typing.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    });
    ui.add_space(10.0);
    let ready = app.cloud().busy.is_none() && !document_name(&app.cloud().save_name, kind).is_empty();
    if ui.add_enabled(ready, egui::Button::new(save_label)).clicked() {
        asked = true;
    }
    if let Some(note) = app.save_note() {
        ui.add_space(8.0);
        dim(ui, &style, &note);
    }
    // A provider with no conditional writes cannot refuse one, so a save here
    // can land on top of somebody else's. Said plainly, because the user is the
    // only one who can decide whether that matters.
    if app.cloud().account.as_ref().is_some_and(|a| !a.can(Capability::Etags)) {
        let warning = app.tr("This cloud account cannot detect a save that would overwrite somebody else's changes.");
        ui.add_space(4.0);
        dim(ui, &style, &warning);
    }
    if asked && ready && app.ready_to_save() {
        let (folder, name) = (app.cloud().mine.path.clone(), app.cloud().save_name.clone());
        match save_as(app, &folder, &name) {
            Ok(_) => return true,
            Err(error) => app.say(error),
        }
    }
    false
}

/// A save refused because the stored copy had moved on.
///
/// Three ways out, and none of them throws anything away without saying so.
/// Reloading is the one a user usually wants, so it comes first. The text says
/// plainly that it discards their edits.
pub fn conflict_modal<A: CloudApp>(app: &mut A, ctx: &egui::Context) {
    let Some(conflict) = app.cloud().conflict.clone() else { return };
    let noun = app.kind().noun;
    let words = ConflictWords {
        title: app.tr(&format!("This {noun} was not saved")),
        what: fill(
            app,
            "Somebody else saved {} while you were editing it. Nothing was written: saving now would overwrite their work.",
            &conflict.entry.name,
        ),
        yours: app.tr("Your own changes are still open here. Reloading replaces them with the stored copy."),
        reload: app.tr("Reload from the cloud"),
        copy: app.tr(&format!("Save as a copy{ELLIPSIS}")),
        keep: app.tr("Keep editing"),
    };
    let mut reload = false;
    let mut copy = false;
    let mut dismiss = false;
    egui::Modal::new(egui::Id::new("craft-ui/cloud/conflict")).show(ctx, |ui| {
        let style = app.style(ui);
        ui.set_width(430.0);
        ui.label(RichText::new(&words.title).font(egui::FontId::new(15.0, style.bold.clone())));
        ui.add_space(8.0);
        ui.add(egui::Label::new(&words.what).wrap());
        ui.add_space(6.0);
        ui.add(egui::Label::new(&words.yours).wrap());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            reload = ui.button(&words.reload).clicked();
            copy = ui.button(&words.copy).clicked();
            dismiss = ui.button(&words.keep).clicked();
        });
    });
    if reload {
        app.cloud_mut().conflict = None;
        if let Err(error) = reload_active(app) {
            app.say(error);
        }
    } else if copy {
        let name = document_name(&format!("{} (copy)", stem_of(&conflict.entry.name)), app.kind());
        let cloud = app.cloud_mut();
        cloud.conflict = None;
        cloud.save_name = name;
        app.go_to_save_as();
    } else if dismiss {
        app.cloud_mut().conflict = None;
    }
}

/// The modal's strings, read before it is drawn.
struct ConflictWords {
    title: String,
    what: String,
    yours: String,
    reload: String,
    copy: String,
    keep: String,
}
