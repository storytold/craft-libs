//! The right-hand end of a title bar, plus the Save As destination.
//!
//! Who is signed in, who else is here, and Share. Every office suite puts
//! these three in the same corner, in that order. A person coming from one of
//! them looks there first.

use egui::{RichText, Ui};

use super::{CloudApp, CopyFormat, Guest, available, fill, link, origin, sign_in, sign_out, work_together};

/// Who this window is for. Who else is in the document, beside them.
///
/// Three cases, each a different person. Somebody signed in gets the account
/// badge and its menu. Somebody who followed a shared link gets their own
/// address and no menu, having no account to manage. A build with no service
/// configured gets neither. A badge that cannot sign anybody in would be a
/// control that fails on use.
///
/// Nothing is asked for here. Drawing a badge must not begin a sign-in: a
/// worker signs in before any job it is given, and it then blocks polling the
/// browser for minutes. Starting that from a paint call pops a browser open at
/// launch and leaves every later job queued behind it. Signing in is a gesture,
/// so it waits for a press.
pub fn who<A: CloudApp>(app: &mut A, ui: &mut Ui) {
    match app.cloud().guest().cloned() {
        Some(guest) => shared_with(app, ui, &guest),
        None => account(app, ui),
    }
    // Who else is here, beside whoever this window belongs to. Nothing is
    // drawn while a person is working alone, which keeps a solitary document
    // looking like one.
    if app.presence().shared() {
        ui.add_space(6.0);
        app.presence().show(ui);
    }
}

/// Who a shared link was sent to, for somebody with no account anywhere.
///
/// A label and not a button. Every item an account menu holds is a sign-out, a
/// switch or a storage connection, and a recipient has none of those. A
/// control that opens a menu of failures is worse than no control.
fn shared_with<A: CloudApp>(app: &mut A, ui: &mut Ui, guest: &Guest) {
    let explained = fill(app, "This document was shared with {}", &guest.recipient);
    let reading = app.tr("Reading only");
    let dim = ui.visuals().weak_text_color();
    if guest.read_only {
        // Said where the account would be. Both answer what this window may
        // do with the document in it. A recipient who finds out at the moment
        // of saving has already done the work twice.
        ui.label(RichText::new(reading).small().color(dim));
        ui.add_space(6.0);
    }
    ui.label(RichText::new(guest.recipient.as_str()).small().color(dim)).on_hover_text(explained);
}

/// The account badge and its menu, for somebody signed in.
fn account<A: CloudApp>(app: &mut A, ui: &mut Ui) {
    if !available(app) {
        return;
    }
    let service = app.service();
    let known = app.cloud().account.as_ref().map(|a| crate::account::Account {
        // One account for now. The control already holds a list, so a second
        // deployment is a second entry here and nothing else changes.
        id: "default".to_owned(),
        display: a.display.clone(),
        email: None,
        service,
        linked: a.provider_linked,
    });
    let new_accounts: Vec<_> = known.into_iter().collect();
    let cloud = app.cloud_mut();
    if cloud.accounts.accounts() != new_accounts.as_slice() {
        cloud.accounts.set(new_accounts);
    }
    cloud.accounts.show(ui);

    while let Some(action) = app.cloud_mut().accounts.take_action() {
        use crate::account::AccountAction as Act;
        match action {
            // Signing in and connecting storage are two different things.
            // Signing in is the device flow against the service, which every
            // job already triggers when there is no token. Asking who the
            // account is therefore starts it. Linking storage comes after, and
            // needs that session to exist first.
            Act::SignIn | Act::AddAccount => {
                app.cloud_mut().sign_in_from_menu = true;
                sign_in(app);
            }
            Act::Link(_) => link(app),
            Act::SignOut(_) => sign_out(app),
            // One account, so there is nothing to switch to yet.
            Act::Use(_) => {}
            Act::Manage(_) => {
                let host = app.service();
                if !host.is_empty() {
                    app.open_url(&format!("https://{host}/account"));
                }
            }
        }
    }
}

/// Whether the Share split button can be drawn.
///
/// False until the service says the provider supports sharing. An application
/// draws whatever it offered before. A button that fails on use is worse than
/// no button at all.
pub fn shares<A: CloudApp>(app: &mut A) -> bool {
    app.sharing().available()
}

/// Share, as a split button.
///
/// Pressing the label shares, which is what somebody wants most of the time.
/// The chevron carries the variations, and working together is one of them.
/// `copies` are the application's own Save a Copy formats. Those have nothing
/// in common between one application and the next.
///
/// Answers with the identifier of a copy format, when that is what was chosen.
/// Everything else is carried out here.
pub fn share_button<A: CloudApp>(app: &mut A, ui: &mut Ui, copies: &[CopyFormat]) -> Option<String> {
    let noun = app.kind().noun;
    let mut items = Vec::new();
    if app.cloud().can_converse() {
        let detail = app.tr(&format!("Open a chat about this {noun}"));
        items.push(crate::menu::MenuItem::new("talk", app.tr("Open chat")).detail(detail));
    }
    for (at, copy) in copies.iter().enumerate() {
        let item = crate::menu::MenuItem::new(copy.id.clone(), copy.label.clone());
        // A rule above the first copy, so sharing and exporting read as two
        // groups rather than one long list.
        items.push(if at == 0 && !items.is_empty() { item.separated() } else { item });
    }
    let labels = crate::menu::MenuLabels::default();
    let primary = app.tr("Share");
    match crate::menu::split(ui, &primary, &items, &labels)? {
        crate::menu::SplitHit::Primary => {
            // The handle the provider issued, which is what a share is about.
            // A document that has never been in the cloud has none, and the
            // dialog says so.
            let (id, title) = match origin(app) {
                Some(origin) => (origin.entry.id.clone(), origin.entry.name.clone()),
                None => (String::new(), app.active().map(|uid| app.title(uid)).unwrap_or_default()),
            };
            app.sharing().open(id, title);
            None
        }
        crate::menu::SplitHit::Item(what) if what == "talk" => {
            work_together(app);
            None
        }
        crate::menu::SplitHit::Item(format) => Some(format),
    }
}

/// What to call the cloud destination on a Save As page.
///
/// The account's own name. The destination then reads as the user's own
/// storage. Somebody with an account on two deployments needs to know which
/// one a document is about to land in.
pub fn destination_name<A: CloudApp>(app: &A) -> String {
    app.cloud().account.as_ref().map(|a| a.display.clone()).filter(|display| !display.is_empty()).unwrap_or_else(|| app.tr("Cloud storage"))
}

/// Whether the cloud destination is the one to offer first.
///
/// A document that came from the cloud is saved back there unless the user says
/// otherwise. One that came from this computer is not.
pub fn prefers_cloud<A: CloudApp>(app: &A) -> bool {
    available(app) && origin(app).is_some()
}
