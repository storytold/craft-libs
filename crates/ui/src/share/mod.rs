//! One share dialog, the same one in every Craft app.
//!
//! What it offers is what a person asks for when they say "share this": pick
//! several people by typing a few letters of a name, choose whether they may
//! look or change, add a note, decide whether an email goes out, and see who
//! already has access so one of them can be taken away again.
//!
//! Three things shape the implementation.
//!
//! Nothing here opens a socket. Every call goes through [`ShareBackend`], and
//! nothing here reaches the network. A host writes the backend, and this
//! module never learns what it talks to.
//! this file.
//!
//! Nothing here blocks. Answers arrive on a channel and are read once per
//! frame. Autocomplete waits [`DEBOUNCE`] after the last keystroke before it
//! asks anything, and an answer to a query the user has already typed past is
//! dropped rather than painted, so the list cannot flicker backwards.
//!
//! Nothing here is assumed about the provider. [`ShareDialog::available`] is
//! false until the service says sharing exists, and an app hides its Share
//! control rather than offering a button that fails. Contacts and mail are
//! reported separately and are hidden the same way.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use craft_cloud::{Capability, Contact, Permission, Share, ShareTarget};
use egui::{Align, Align2, Color32, Context, CornerRadius, FontId, Id, Layout, Response, RichText, Sense, Ui, UiBuilder, Vec2};

mod backend;
#[cfg(test)]
mod tests;

pub use backend::{Answer, Destination, Failure, Grant, Replier, Request, ShareBackend};

/// How long after the last keystroke the dialog asks for matches.
///
/// A lookup per keystroke would ask five times for "annie" and paint whichever
/// answer happened to arrive last.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

/// How many matches one lookup asks for.
const MATCHES: usize = 8;

/// Shortest query worth asking about. One letter matches most of an address
/// book and tells the user nothing.
const MIN_QUERY: usize = 2;

/// Contact pictures held at once. Reached only by a long session of searching;
/// the whole cache is dropped rather than evicted one entry at a time.
const AVATARS: usize = 256;

/// How long before an unanswered capability question is asked again, so one
/// unreachable moment does not hide Share for the rest of the session.
const RETRY: f64 = 20.0;

const AVATAR: f32 = 20.0;
const AVATAR_RADIUS: u8 = 10;

/// The id of the recipient field, for an app that wants to focus it.
const QUERY: &str = "craft-ui/share/query";

type Inbox = (u64, Result<Answer, String>);

/// Every string the dialog shows, so an application can translate it.
///
/// The defaults are English. Nothing else in `craft-ui` localizes, and this
/// does not change that: the dialog holds the strings an app gives it and
/// translates nothing itself.
#[derive(Clone, Debug)]
pub struct ShareLabels {
    pub title: String,
    pub recipients: String,
    pub recipient_hint: String,
    pub searching: String,
    pub no_matches: String,
    pub add_address: String,
    pub group: String,
    pub permission: String,
    pub can_view: String,
    pub can_edit: String,
    pub note: String,
    pub notify: String,
    pub no_mail: String,
    pub share: String,
    pub sharing: String,
    pub close: String,
    pub remove: String,
    pub shared_with: String,
    pub not_shared_with: String,
    pub access: String,
    pub access_empty: String,
    pub loading: String,
    pub withdraw: String,
    pub withdrawing: String,
    pub copy_link: String,
    pub anyone_with_link: String,
    pub save_first: String,
    /// Shown under "Can edit", the way an office suite explains a permission
    /// rather than leaving a bare radio button to carry the meaning.
    pub can_edit_detail: String,
    pub can_view_detail: String,
    pub message_hint: String,
    pub send: String,
    pub manage_access: String,
    pub upload_first: String,
    pub name_hint: String,
    pub saving: String,
    pub save_and_share: String,
    pub sign_in: String,
    pub attach_instead: String,
    pub create_link: String,
    pub key_hint: String,
    pub key_required_note: String,
    pub link_copied: String,
}

impl Default for ShareLabels {
    fn default() -> ShareLabels {
        ShareLabels {
            title: "Share".into(),
            recipients: "Share with".into(),
            recipient_hint: "Add a name, group, or email".into(),
            searching: "Looking…".into(),
            no_matches: "No matches".into(),
            add_address: "Press Return to share with this address".into(),
            group: "group".into(),
            permission: "They may".into(),
            can_view: "View".into(),
            can_edit: "Edit".into(),
            note: "Note (optional)".into(),
            notify: "Notify by email".into(),
            no_mail: "This server cannot send mail".into(),
            share: "Share".into(),
            sharing: "Sharing…".into(),
            close: "Close".into(),
            remove: "Remove".into(),
            shared_with: "Shared with".into(),
            not_shared_with: "Not shared with".into(),
            access: "Who has access".into(),
            access_empty: "Nobody else yet".into(),
            loading: "Loading…".into(),
            withdraw: "Withdraw".into(),
            withdrawing: "Withdrawing…".into(),
            copy_link: "Copy link".into(),
            anyone_with_link: "Anyone with the link".into(),
            save_first: "Save this document to the cloud before sharing it.".into(),
            can_edit_detail: "Make any changes".into(),
            can_view_detail: "Cannot make changes".into(),
            message_hint: "Add a message".into(),
            send: "Send".into(),
            manage_access: "Who has access".into(),
            upload_first: "Save this document somewhere to share it.".into(),
            name_hint: "Name this document".into(),
            saving: "Saving\u{2026}".into(),
            save_and_share: "Save and share".into(),
            sign_in: "Sign in".into(),
            attach_instead: "Attach a copy instead".into(),
            create_link: "Get a link".into(),
            key_hint: "Key (optional)".into(),
            key_required_note: "This server requires a key on a link.".into(),
            link_copied: "Link copied".into(),
        }
    }
}

/// What the provider behind the service actually does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Provider {
    sharing: bool,
    contacts: bool,
    mail: bool,
    /// Public links at all. An administrator can forbid them.
    links: bool,
    /// A key on a link.
    key: bool,
    /// The server insists on one, so a link cannot be made without it.
    key_required: bool,
}

/// One contact picture, asked for at most once.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Avatar {
    /// Asked for under this sequence number; nothing to paint yet.
    Pending(u64),
    /// Bytes handed to egui under this URI, for the application's image
    /// loaders to decode.
    Ready(String),
    /// Asked once and refused. Initials are painted instead, and it is never
    /// asked for again.
    Missing,
}

/// Somebody the user has picked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chosen {
    target: ShareTarget,
    label: String,
    detail: Option<String>,
    avatar: Option<String>,
}

/// The dialog's state while it is open on one document.
struct Sheet {
    id: String,
    title: String,
    query: String,
    /// When the query last changed, for the debounce. `None` when settled.
    typed: Option<f64>,
    /// The newest lookup asked for. An answer to anything older is dropped.
    lookup: Option<u64>,
    searching: bool,
    matches: Vec<Contact>,
    chosen: Vec<Chosen>,
    permission: Permission,
    note: String,
    notify: bool,
    /// Who already has access. `None` until the answer arrives.
    access: Option<Vec<Share>>,
    access_seq: Option<u64>,
    /// Withdrawals in flight, as (request, share).
    withdrawing: Vec<(u64, String)>,
    granting: Option<u64>,
    /// Where it could be saved. `None` until the answer arrives.
    destinations: Option<Vec<Destination>>,
    destinations_seq: Option<u64>,
    link_seq: Option<u64>,
    /// Which one is selected, by id.
    destination: Option<String>,
    /// The name typed while the document still has nowhere to live.
    save_as: String,
    /// The save in flight, if one is.
    saving: Option<u64>,
    save_error: Option<String>,
    /// The key to put on a link, while one is being made.
    link_key: String,
    /// The last share's outcome, kept on screen until the next one.
    shared: Vec<Share>,
    failed: Vec<Failure>,
    /// A refusal that stopped a whole call, as opposed to one recipient.
    error: Option<String>,
}

impl Sheet {
    fn new(id: String, title: String) -> Sheet {
        Sheet {
            destinations: None,
            destinations_seq: None,
            link_seq: None,
            destination: None,
            save_as: title.clone(),
            saving: None,
            save_error: None,
            link_key: String::new(),
            id,
            title,
            query: String::new(),
            typed: None,
            lookup: None,
            searching: false,
            matches: Vec::new(),
            chosen: Vec::new(),
            permission: Permission::View,
            note: String::new(),
            notify: true,
            access: None,
            access_seq: None,
            withdrawing: Vec::new(),
            granting: None,
            shared: Vec::new(),
            failed: Vec::new(),
            error: None,
        }
    }
}

/// Something the drawing pass decided and the dialog carries out afterwards.
///
/// Drawing holds the open sheet, so it cannot also reach the request channel.
/// Collecting intentions and acting on them once the frame is painted keeps
/// both honest.
enum Act {
    Lookup(String),
    /// Save a document that has nowhere to live yet, into one destination.
    SaveAs {
        destination: String,
        name: String,
    },
    /// Connect an account for a destination that has none.
    Link(String),
    Grant(Grant),
    Withdraw(String),
    Avatar(String),
    Close,
}

/// A save the dialog is waiting on the application to do.
///
/// Taken once. The application serialises the open document, puts it where
/// `destination` says, and reports back with [`ShareDialog::saved`] or
/// [`ShareDialog::save_failed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRequest {
    /// Which destination, as the dialog was told by [`Destination::id`].
    pub destination: String,
    /// What the user called it.
    pub name: String,
}

/// What the dialog may offer this frame.
#[derive(Clone, Copy)]
struct Allowed {
    contacts: bool,
    mail: bool,
    links: bool,
    key: bool,
    key_required: bool,
}

/// A share dialog.
///
/// Create one per window, call [`ShareDialog::connect`] once a backend exists,
/// call [`ShareDialog::show`] every frame, and open it from a Share control
/// that is drawn only while [`ShareDialog::available`] is true.
pub struct ShareDialog {
    backend: Option<Box<dyn ShareBackend>>,
    tx: Sender<Inbox>,
    rx: Receiver<Inbox>,
    seq: u64,
    provider: Option<Provider>,
    /// When the capability question was last asked.
    asked: Option<f64>,
    open: Option<Sheet>,
    /// Keyed by the URL the contact carried, so reopening the dialog does not
    /// fetch the same faces again.
    avatars: HashMap<String, Avatar>,
    /// A save the application has not picked up yet.
    wants_save: Option<SaveRequest>,
    labels: ShareLabels,
}

impl Default for ShareDialog {
    fn default() -> ShareDialog {
        ShareDialog::new()
    }
}

impl ShareDialog {
    pub fn new() -> ShareDialog {
        let (tx, rx) = std::sync::mpsc::channel();
        ShareDialog {
            backend: None,
            tx,
            rx,
            seq: 0,
            provider: None,
            asked: None,
            open: None,
            avatars: HashMap::new(),
            wants_save: None,
            labels: ShareLabels::default(),
        }
    }

    /// Attach the thing that does the waiting.
    ///
    /// The first frame after this asks what the provider can do, which is what
    /// decides whether sharing is offered at all.
    pub fn connect(&mut self, backend: impl ShareBackend + 'static) {
        self.backend = Some(Box::new(backend));
        self.provider = None;
        self.asked = None;
    }

    pub fn connected(&self) -> bool {
        self.backend.is_some()
    }

    /// Replace the dialog's strings, for a localized application.
    pub fn with_labels(mut self, labels: ShareLabels) -> ShareDialog {
        self.labels = labels;
        self
    }

    pub fn labels_mut(&mut self) -> &mut ShareLabels {
        &mut self.labels
    }

    /// Whether sharing can be offered.
    ///
    /// False until the service says the provider supports it and an account is
    /// linked. An application draws no Share control while this is false: a
    /// hidden feature is better than a button that fails on use.
    /// Ask the provider again what it can do.
    ///
    /// The answer is kept once it arrives, so a host that signs somebody
    /// in after the first answer has to say so. Without this the dialog
    /// keeps the answer it got while nobody was signed in, and Share
    /// stays unavailable for the rest of the run.
    pub fn refresh(&mut self) {
        self.provider = None;
        self.asked = None;
    }

    pub fn available(&self) -> bool {
        self.provider.is_some_and(|p| p.sharing)
    }

    /// Open on one document.
    ///
    /// `id` is the handle the application was given for it, passed back
    /// untouched. An empty one means the document has never been saved to the
    /// cloud, which the dialog says rather than failing on Share. Does nothing
    /// while sharing is unavailable.
    pub fn open(&mut self, id: impl Into<String>, title: impl Into<String>) {
        if !self.available() {
            return;
        }
        let id = id.into();
        self.open = Some(Sheet::new(id.clone(), title.into()));
        if id.is_empty() {
            // Nothing to share yet, so the dialog opens on the places it could
            // go. Asked for here rather than on the first frame, so the list is
            // usually there by the time anybody looks.
            let seq = self.issue(Request::Destinations);
            if let Some(sheet) = &mut self.open {
                sheet.destinations_seq = Some(seq);
            }
        }
        if !id.is_empty() {
            let seq = self.issue(Request::Access { id });
            if let Some(sheet) = &mut self.open {
                sheet.access_seq = Some(seq);
            }
        }
    }

    /// A save the application should perform, if one was asked for.
    ///
    /// Taken, so it is handled once. Call [`ShareDialog::saved`] or
    /// [`ShareDialog::save_failed`] when it is done, or the dialog waits.
    pub fn take_save_request(&mut self) -> Option<SaveRequest> {
        self.wants_save.take()
    }

    /// The document was saved, and this is the handle the provider issued.
    pub fn saved(&mut self, id: impl Into<String>, name: impl Into<String>) {
        let id = id.into();
        if let Some(sheet) = &mut self.open {
            sheet.saving = None;
            sheet.save_error = None;
            sheet.id = id.clone();
            sheet.title = name.into();
        }
        // The document exists now, so ask who already has access, exactly as
        // opening on a saved document would.
        let seq = self.issue(Request::Access { id });
        if let Some(sheet) = &mut self.open {
            sheet.access_seq = Some(seq);
        }
    }

    /// The save did not happen, and this is why.
    pub fn save_failed(&mut self, reason: impl Into<String>) {
        if let Some(sheet) = &mut self.open {
            sheet.saving = None;
            sheet.save_error = Some(reason.into());
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// Who the last share reached, and who it did not.
    pub fn outcome(&self) -> (&[Share], &[Failure]) {
        match &self.open {
            Some(sheet) => (&sheet.shared, &sheet.failed),
            None => (&[], &[]),
        }
    }

    /// The recipient field's id, for an application that wants to focus it.
    pub fn query_field_id() -> Id {
        Id::new(QUERY)
    }

    /// One frame, as a modal over the window. Call this unconditionally:
    /// answers are collected here whether the dialog is open or not.
    pub fn show(&mut self, ctx: &Context) {
        self.collect(ctx);
        if self.open.is_none() {
            return;
        }
        let modal = egui::Modal::new(Id::new("craft-ui/share")).show(ctx, |ui| {
            ui.set_min_width(440.0);
            ui.set_max_width(560.0);
            self.contents(ui);
        });
        if modal.should_close() {
            self.close();
        }
    }

    /// One frame's contents without the modal chrome, for an application that
    /// hosts the dialog in its own panel.
    pub fn show_in(&mut self, ui: &mut Ui) {
        self.collect(ui.ctx());
        if self.open.is_some() {
            self.contents(ui);
        }
    }

    fn contents(&mut self, ui: &mut Ui) {
        let allowed = match self.provider {
            Some(p) => Allowed { contacts: p.contacts, mail: p.mail, links: p.links, key: p.key, key_required: p.key_required },
            None => Allowed { contacts: false, mail: false, links: false, key: false, key_required: false },
        };
        let mut acts = Vec::new();
        {
            let Some(sheet) = self.open.as_mut() else { return };
            draw(ui, sheet, &self.avatars, &self.labels, allowed, &mut acts);
        }
        self.apply(ui.ctx(), acts);
    }

    // ---- the channel ------------------------------------------------------

    fn issue(&mut self, request: Request) -> u64 {
        self.seq += 1;
        let seq = self.seq;
        if let Some(backend) = &self.backend {
            backend.submit(request, Replier::new(seq, self.tx.clone()));
        }
        seq
    }

    fn collect(&mut self, ctx: &Context) {
        while let Ok((seq, result)) = self.rx.try_recv() {
            self.receive(ctx, seq, result);
        }
        let now = ctx.input(|i| i.time);
        if self.backend.is_some() && self.provider.is_none() && self.asked.is_none_or(|at| now - at >= RETRY) {
            self.asked = Some(now);
            self.issue(Request::Capabilities);
        }
    }

    fn receive(&mut self, ctx: &Context, seq: u64, result: Result<Answer, String>) {
        let answer = match result {
            Ok(answer) => answer,
            Err(reason) => return self.refused(seq, reason),
        };
        // Set while an answer leaves something else worth asking for; issued
        // once the open sheet is no longer borrowed.
        let mut reload = None;
        match answer {
            Answer::Capabilities { capabilities, linked } => {
                self.provider = Some(Provider {
                    sharing: linked && capabilities.contains(&Capability::Sharing),
                    contacts: capabilities.contains(&Capability::Contacts),
                    // ShareNotify, not Mail. Mail means "can send arbitrary
                    // mail"; this box only asks the provider to tell the
                    // person being shared with, which is a far smaller claim
                    // and one most providers can honour.
                    mail: capabilities.contains(&Capability::ShareNotify),
                    links: capabilities.contains(&Capability::LinkShares),
                    key: capabilities.contains(&Capability::LinkPassword),
                    key_required: capabilities.contains(&Capability::LinkPasswordRequired),
                });
                // A provider that cannot share must not leave a dialog up that
                // was opened while the answer was still outstanding.
                if !self.available() {
                    self.open = None;
                }
            }
            Answer::Lookup(found) => {
                if let Some(sheet) = &mut self.open
                    && sheet.lookup == Some(seq)
                {
                    sheet.matches = found;
                    sheet.searching = false;
                }
            }
            Answer::Access(list) => {
                if let Some(sheet) = &mut self.open
                    && sheet.access_seq == Some(seq)
                {
                    sheet.access = Some(list);
                    sheet.access_seq = None;
                }
            }
            Answer::Granted { shared, failed } => {
                // A link was asked for in order to be handed out, so it goes
                // to the clipboard without a second click.
                if let Some(url) = shared.iter().find_map(|s| s.url.clone()) {
                    ctx.copy_text(url);
                }
                if let Some(sheet) = &mut self.open
                    && sheet.granting == Some(seq)
                {
                    sheet.granting = None;
                    sheet.error = None;
                    // Recipients that went through leave the picker. The ones
                    // that did not stay in it, so a corrected retry is one
                    // press rather than a retyped list.
                    sheet.chosen.retain(|c| failed.iter().any(|f| f.target == c.target));
                    sheet.shared = shared;
                    sheet.failed = failed;
                    if !sheet.shared.is_empty() {
                        reload = Some(sheet.id.clone());
                    }
                }
            }
            Answer::Destinations(list) => {
                if let Some(sheet) = &mut self.open
                    && sheet.destinations_seq == Some(seq)
                {
                    // One destination and nothing to choose between: pick it,
                    // so the only linked account is not a click somebody has
                    // to make for no reason.
                    if sheet.destination.is_none()
                        && let [only] = list.as_slice()
                        && only.linked
                    {
                        sheet.destination = Some(only.id.clone());
                    }
                    sheet.destinations = Some(list);
                    sheet.destinations_seq = None;
                }
            }
            Answer::Linked => {
                // An account arrived, so the list is stale.
                if let Some(sheet) = &mut self.open {
                    sheet.destinations = None;
                }
                let seq = self.issue(Request::Destinations);
                if let Some(sheet) = &mut self.open {
                    sheet.destinations_seq = Some(seq);
                }
            }
            Answer::Withdrawn { share_id } => {
                if let Some(sheet) = &mut self.open {
                    sheet.withdrawing.retain(|(_, id)| *id != share_id);
                    if let Some(access) = &mut sheet.access {
                        access.retain(|s| s.id != share_id);
                    }
                    sheet.shared.retain(|s| s.id != share_id);
                }
            }
            Answer::Avatar { url, bytes } => {
                let uri = avatar_uri(&url);
                ctx.include_bytes(uri.clone(), bytes);
                self.avatars.insert(url, Avatar::Ready(uri));
            }
        }
        if let Some(id) = reload {
            let seq = self.issue(Request::Access { id });
            if let Some(sheet) = &mut self.open {
                sheet.access_seq = Some(seq);
            }
        }
    }

    /// One request came back with a reason instead of an answer.
    fn refused(&mut self, seq: u64, reason: String) {
        // A picture nobody can fetch is not worth telling the user about, and
        // is never asked for again.
        if let Some(url) = self.avatars.iter().find(|(_, a)| **a == Avatar::Pending(seq)).map(|(url, _)| url.clone()) {
            self.avatars.insert(url, Avatar::Missing);
            return;
        }
        let Some(sheet) = &mut self.open else { return };
        if sheet.lookup == Some(seq) {
            sheet.searching = false;
            sheet.matches.clear();
            sheet.error = Some(reason);
        } else if sheet.access_seq == Some(seq) {
            sheet.access_seq = None;
            sheet.access = Some(Vec::new());
            sheet.error = Some(reason);
        } else if sheet.granting == Some(seq) {
            sheet.granting = None;
            sheet.error = Some(reason);
        } else if sheet.withdrawing.iter().any(|(s, _)| *s == seq) {
            sheet.withdrawing.retain(|(s, _)| *s != seq);
            sheet.error = Some(reason);
        } else if sheet.destinations_seq == Some(seq) {
            sheet.destinations_seq = None;
            sheet.error = Some(reason);
        } else if sheet.link_seq == Some(seq) {
            sheet.link_seq = None;
            sheet.error = Some(reason);
        }
    }

    fn apply(&mut self, ctx: &Context, acts: Vec<Act>) {
        for act in acts {
            match act {
                Act::Close => self.close(),
                Act::Lookup(query) => {
                    let seq = self.issue(Request::Lookup { query, limit: MATCHES });
                    if let Some(sheet) = &mut self.open {
                        sheet.lookup = Some(seq);
                        sheet.searching = true;
                    }
                }
                Act::Grant(grant) => {
                    let seq = self.issue(Request::Grant(grant));
                    if let Some(sheet) = &mut self.open {
                        sheet.granting = Some(seq);
                        sheet.shared.clear();
                        sheet.failed.clear();
                        sheet.error = None;
                    }
                }
                Act::SaveAs { destination, name } => {
                    // Handed to the application rather than the backend. Only
                    // it can turn the open document into bytes, and a share
                    // transport is the wrong place to pass a workbook through.
                    self.wants_save = Some(SaveRequest { destination, name });
                    if let Some(sheet) = &mut self.open {
                        sheet.saving = Some(0);
                        sheet.save_error = None;
                    }
                }
                Act::Link(destination) => {
                    // The answer to this is usually a refusal carrying the URL
                    // to open, because launching a browser is the host
                    // application's business and not this crate's.
                    let seq = self.issue(Request::Link { destination });
                    if let Some(sheet) = &mut self.open {
                        sheet.link_seq = Some(seq);
                        sheet.error = None;
                    }
                }
                Act::Withdraw(share_id) => {
                    let seq = self.issue(Request::Withdraw { share_id: share_id.clone() });
                    if let Some(sheet) = &mut self.open {
                        sheet.withdrawing.push((seq, share_id));
                    }
                }
                Act::Avatar(url) => {
                    if self.avatars.contains_key(&url) {
                        continue;
                    }
                    if self.avatars.len() >= AVATARS {
                        for (_, avatar) in self.avatars.drain() {
                            if let Avatar::Ready(uri) = avatar {
                                ctx.forget_image(&uri);
                            }
                        }
                    }
                    let seq = self.issue(Request::Avatar { url: url.clone() });
                    self.avatars.insert(url, Avatar::Pending(seq));
                }
            }
        }
    }
}

// ---- drawing --------------------------------------------------------------

fn draw(ui: &mut Ui, sheet: &mut Sheet, avatars: &HashMap<String, Avatar>, labels: &ShareLabels, allowed: Allowed, acts: &mut Vec<Act>) {
    let weak = ui.visuals().weak_text_color();
    let danger = ui.visuals().error_fg_color;

    // The document is named in the title, in quotes. This dialog gives other
    // people access to one specific thing, and sharing the wrong one is the
    // mistake it exists to prevent.
    ui.horizontal(|ui| {
        let heading = if sheet.title.is_empty() { labels.title.clone() } else { format!("{} \u{201c}{}\u{201d}", labels.title, sheet.title) };
        ui.label(RichText::new(heading).strong().size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let close = egui::Button::new(RichText::new("\u{00d7}").size(18.0)).frame(false);
            if ui.add(close).on_hover_text(labels.close.as_str()).clicked() {
                acts.push(Act::Close);
            }
        });
    });
    ui.add_space(8.0);

    // Nothing to share yet. The old build stopped here with a sentence and a
    // Close button, naming the obstacle and leaving somebody to deal with it
    // elsewhere. Saving is the first step of sharing now.
    if sheet.id.is_empty() {
        save_first(ui, sheet, labels, acts);
        return;
    }

    // Who, and what they may do, on one row. The permission applies to these
    // recipients, so it belongs beside them. A separate pair of radio buttons
    // further down reads as an unrelated setting.
    ui.horizontal_top(|ui| {
        let width = (ui.available_width() - 104.0).max(160.0);
        ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
            recipients(ui, sheet, avatars, labels, allowed, acts);
        });
        permission_menu(ui, sheet, labels);
    });
    // Only once something has actually been asked. "No matches" under an
    // untouched field reads as a failure before any search ran.
    if sheet.searching || !sheet.matches.is_empty() || sheet.query.trim().chars().count() >= MIN_QUERY {
        matches(ui, sheet, avatars, labels, allowed, acts);
    }

    ui.add_space(6.0);

    // The message, as a bordered area with its own placeholder. People write a
    // sentence here, not a value, and a label above a one-line field invites
    // neither.
    // TextEdit draws its own border, so no group around it. Wrapping one in a
    // Frame::group gives two nested borders.
    ui.add(
        egui::TextEdit::multiline(&mut sheet.note)
            .hint_text(labels.message_hint.as_str())
            .desired_width(f32::INFINITY)
            .desired_rows(3)
            .char_limit(500),
    );

    ui.add_space(8.0);

    // One primary action, on the right where the eye finishes. Notify sits
    // beside it rather than loose in the middle of the form.
    // A link, for anybody who has no account. Drawn only where the server
    // allows public links, because an administrator can forbid them and a
    // button that always fails is worse than no button.
    if allowed.links {
        ui.horizontal(|ui| {
            if allowed.key {
                ui.add(egui::TextEdit::singleline(&mut sheet.link_key).hint_text(labels.key_hint.as_str()).password(true).desired_width(150.0));
            }
            // A key the server insists on cannot be skipped, so the control
            // stays disabled until one is typed.
            let keyed = !sheet.link_key.trim().is_empty();
            let ready = keyed || !allowed.key_required;
            if ui.add_enabled(ready, egui::Button::new(labels.create_link.as_str())).clicked() {
                acts.push(Act::Grant(Grant {
                    id: sheet.id.clone(),
                    targets: vec![ShareTarget::Link],
                    permission: sheet.permission,
                    note: Some(sheet.note.trim().to_owned()).filter(|n| !n.is_empty()),
                    notify: false,
                    password: Some(sheet.link_key.trim().to_owned()).filter(|k| !k.is_empty()),
                }));
            }
            if allowed.key_required && !keyed {
                ui.label(RichText::new(labels.key_required_note.as_str()).small().color(weak));
            }
        });
        ui.add_space(6.0);
    }

    ui.horizontal(|ui| {
        if allowed.mail {
            ui.checkbox(&mut sheet.notify, labels.notify.as_str());
        } else {
            // Reported, never assumed. A server with no mail is said to have
            // none, instead of being given a box that quietly does nothing.
            sheet.notify = false;
            ui.add_enabled(false, egui::Checkbox::new(&mut sheet.notify, labels.notify.as_str())).on_disabled_hover_text(labels.no_mail.as_str());
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let busy = sheet.granting.is_some();
            let ready = !sheet.chosen.is_empty() && !busy;
            let label = if busy { labels.sharing.clone() } else { labels.send.clone() };
            let send = egui::Button::new(RichText::new(label).strong()).fill(ui.visuals().selection.bg_fill).min_size(egui::vec2(88.0, 26.0));
            if ui.add_enabled(ready, send).clicked() {
                acts.push(Act::Grant(Grant {
                    id: sheet.id.clone(),
                    password: None,
                    targets: sheet.chosen.iter().map(|c| c.target.clone()).collect(),
                    permission: sheet.permission,
                    note: Some(sheet.note.trim().to_owned()).filter(|n| !n.is_empty()),
                    notify: sheet.notify,
                }));
            }
            if ui.button(labels.close.as_str()).clicked() {
                acts.push(Act::Close);
            }
        });
    });

    outcome(ui, sheet, labels, danger);

    // Who already holds the document, open. Folding it hid the one fact
    // a person needs before adding anybody else. It still folds on request.
    ui.add_space(4.0);
    egui::CollapsingHeader::new(RichText::new(labels.manage_access.as_str()).color(weak))
        .id_salt("craft-ui/share/manage-access")
        .default_open(true)
        .show(ui, |ui| {
            access(ui, sheet, labels, weak, acts);
        });
}

/// Where to put it, then what to call it.
///
/// A document with no handle cannot be shared. Rather than saying so and
/// stopping, this offers the places it could go, each naming the account it
/// would land in, with a sign-in for any that is configured without one.
///
/// A list, because there is rarely only one answer. A second provider is an
/// implementation of the contracts, and this is where it shows up.
fn save_first(ui: &mut Ui, sheet: &mut Sheet, labels: &ShareLabels, acts: &mut Vec<Act>) {
    let weak = ui.visuals().weak_text_color();
    ui.label(labels.upload_first.as_str());
    ui.add_space(8.0);

    let Some(destinations) = sheet.destinations.clone() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(labels.loading.as_str()).color(weak));
        });
        return;
    };

    for destination in &destinations {
        let chosen = sheet.destination.as_deref() == Some(destination.id.as_str());
        let stroke = if chosen { ui.visuals().selection.stroke } else { ui.visuals().widgets.noninteractive.bg_stroke };
        let row = egui::Frame::group(ui.style())
            .stroke(stroke)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(destination.name.as_str()).strong());
                        // The account, because two entries both reading
                        // "Cloud" tell nobody which one a document is about to
                        // land in.
                        let under = if destination.linked { destination.account.clone().unwrap_or_default() } else { labels.sign_in.clone() };
                        ui.label(RichText::new(under).small().color(weak));
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if row.clicked() {
            if destination.linked {
                sheet.destination = Some(destination.id.clone());
            } else {
                acts.push(Act::Link(destination.id.clone()));
            }
        }
        ui.add_space(4.0);
    }

    let Some(into) = sheet.destination.clone() else { return };

    ui.add_space(8.0);
    let saving = sheet.saving.is_some();
    ui.add_enabled(!saving, egui::TextEdit::singleline(&mut sheet.save_as).hint_text(labels.name_hint.as_str()).desired_width(f32::INFINITY));

    if let Some(problem) = &sheet.save_error {
        ui.add_space(4.0);
        ui.label(RichText::new(problem).color(ui.visuals().error_fg_color));
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let named = !sheet.save_as.trim().is_empty();
        let label = if saving { labels.saving.clone() } else { labels.save_and_share.clone() };
        let button = egui::Button::new(RichText::new(label).strong()).fill(ui.visuals().selection.bg_fill).min_size(egui::vec2(124.0, 26.0));
        if ui.add_enabled(named && !saving, button).clicked() {
            acts.push(Act::SaveAs { destination: into.clone(), name: sheet.save_as.trim().to_owned() });
        }
        if ui.button(labels.close.as_str()).clicked() {
            acts.push(Act::Close);
        }
    });
}

/// The permission picker, as a menu with a line of explanation under each
/// choice.
///
/// Two radio buttons reading View and Edit leave the difference to be guessed.
/// Only the permissions the contracts can actually keep are offered: a choice
/// the server cannot honour is worse than one fewer choice.
fn permission_menu(ui: &mut Ui, sheet: &mut Sheet, labels: &ShareLabels) {
    let current = match sheet.permission {
        Permission::Edit => labels.can_edit.as_str(),
        Permission::View => labels.can_view.as_str(),
    };
    // A triangle, so it reads as something that opens. Painted rather than
    // typed: the character is missing from the default font set, where it
    // renders as a box.
    let button = ui.button(format!("{current}    "));
    crate::buttons::chevron(ui, button.rect, ui.visuals().text_color());
    // The padding makes room for the triangle. It is layout, so it must not
    // reach anybody reading the screen aloud.
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, current));
    egui::Popup::menu(&button).show(|ui| {
        ui.set_min_width(190.0);
        for (value, name, detail) in [
            (Permission::Edit, labels.can_edit.as_str(), labels.can_edit_detail.as_str()),
            (Permission::View, labels.can_view.as_str(), labels.can_view_detail.as_str()),
        ] {
            let chosen = sheet.permission == value;
            if ui.selectable_label(chosen, RichText::new(name).strong()).clicked() {
                sheet.permission = value;
                ui.close();
            }
            ui.label(RichText::new(detail).small().color(ui.visuals().weak_text_color()));
            ui.add_space(2.0);
        }
    });
}

/// The chips, and the field that adds another.
fn recipients(ui: &mut Ui, sheet: &mut Sheet, avatars: &HashMap<String, Avatar>, labels: &ShareLabels, allowed: Allowed, acts: &mut Vec<Act>) {
    let now = ui.input(|i| i.time);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let mut remove = None;
            for (i, chosen) in sheet.chosen.iter().enumerate() {
                if chip(ui, avatars, chosen, labels, acts).clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove.filter(|i| *i < sheet.chosen.len()) {
                sheet.chosen.remove(i);
            }

            let field = ui.add(
                egui::TextEdit::singleline(&mut sheet.query)
                    .id(ShareDialog::query_field_id())
                    .hint_text(labels.recipient_hint.as_str())
                    // Fills whatever the chips left. A fixed width leaves a
                    // short field floating in a frame twice as wide.
                    .desired_width(ui.available_width().max(120.0)),
            );
            if field.changed() {
                sheet.typed = Some(now);
            }
            // Backspace on an empty field takes the last chip back, which is
            // what every other recipient field does.
            if field.has_focus() && sheet.query.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                sheet.chosen.pop();
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit(sheet);
            }
        });
    });

    // The debounce. Asking per keystroke asks five times for "annie" and
    // paints whichever answer happens to arrive last.
    if let Some(at) = sheet.typed {
        let wait = DEBOUNCE.as_secs_f64();
        if now - at >= wait {
            sheet.typed = None;
            let query = sheet.query.trim().to_owned();
            if allowed.contacts && query.chars().count() >= MIN_QUERY {
                // Set here rather than when the request is issued, so the
                // frame that fires it does not read as "no matches".
                sheet.searching = true;
                acts.push(Act::Lookup(query));
            } else {
                sheet.matches.clear();
                sheet.searching = false;
            }
        } else {
            // Without this the frame that fires the lookup only arrives when
            // something else happens to ask for a repaint.
            ui.ctx().request_repaint_after(Duration::from_secs_f64((wait - (now - at)).clamp(0.0, wait)));
        }
    }
}

/// Take whatever has been typed as a recipient: the first match if there is
/// one, otherwise an address if it looks like one.
fn commit(sheet: &mut Sheet) {
    let typed = sheet.query.trim().to_lowercase();

    // An exact match wins. Somebody who typed a whole address that belongs to
    // an account should get the account, because a share to an account appears
    // in their files rather than only in their inbox.
    if let Some(exact) = sheet
        .matches
        .iter()
        .find(|c| c.id.to_lowercase() == typed || c.email.as_deref().is_some_and(|e| e.to_lowercase() == typed) || c.name.to_lowercase() == typed)
        .cloned()
    {
        choose(sheet, &exact);
        return;
    }

    // Then whatever was typed, if it is an address. The list is a convenience
    // and not the set of permitted answers: taking its first row here would
    // share with whoever happened to match the opening characters of an
    // address somebody typed out in full.
    if let Some(address) = address(&sheet.query) {
        add(sheet, ShareTarget::Email(address.clone()), address, None, None);
        return;
    }

    // Otherwise the best the list has, which is what a partial name means.
    if let Some(contact) = sheet.matches.first().cloned() {
        choose(sheet, &contact);
    }
}

fn matches(ui: &mut Ui, sheet: &mut Sheet, avatars: &HashMap<String, Avatar>, labels: &ShareLabels, allowed: Allowed, acts: &mut Vec<Act>) {
    let weak = ui.visuals().weak_text_color();
    // Somebody with no account on this server is reached by address or not at
    // all, so the one way to do that is said out loud. A dialog that hides it
    // leaves the user guessing which keys work.
    if sheet.matches.is_empty()
        && address(&sheet.query).is_some()
        && !sheet.chosen.iter().any(|c| c.target == ShareTarget::Email(sheet.query.trim().to_owned()))
    {
        ui.label(RichText::new(labels.add_address.as_str()).color(weak));
    }
    if !allowed.contacts {
        return;
    }
    if sheet.searching && sheet.matches.is_empty() {
        ui.label(RichText::new(labels.searching.as_str()).color(weak));
        return;
    }
    if sheet.matches.is_empty() {
        if sheet.typed.is_none() && sheet.query.trim().chars().count() >= MIN_QUERY && address(&sheet.query).is_none() {
            ui.label(RichText::new(labels.no_matches.as_str()).color(weak));
        }
        return;
    }
    let mut pick = None;
    egui::ScrollArea::vertical().max_height(150.0).id_salt("craft-ui/share/matches").show(ui, |ui| {
        for (i, contact) in sheet.matches.iter().enumerate() {
            if row(ui, avatars, contact, labels, acts).clicked() {
                pick = Some(i);
            }
        }
    });
    if let Some(contact) = pick.and_then(|i| sheet.matches.get(i).cloned()) {
        choose(sheet, &contact);
    }
}

fn outcome(ui: &mut Ui, sheet: &Sheet, labels: &ShareLabels, danger: Color32) {
    if let Some(error) = &sheet.error {
        ui.colored_label(danger, error.as_str());
    }
    if !sheet.shared.is_empty() {
        let who: Vec<String> = sheet.shared.iter().map(|s| share_label(s, labels)).collect();
        ui.label(format!("{}: {}", labels.shared_with, who.join(", ")));
    }
    // One line each. Five recipients with one bad address shares with four and
    // names the one that failed; collapsing that into a single error would
    // hide both halves.
    for failure in &sheet.failed {
        ui.colored_label(danger, format!("{} {}: {}", labels.not_shared_with, target_label(&failure.target, labels), describe(failure)));
    }
}

fn describe(failure: &Failure) -> String {
    if failure.detail.trim().is_empty() { failure.code.clone() } else { failure.detail.clone() }
}

fn access(ui: &mut Ui, sheet: &Sheet, labels: &ShareLabels, weak: Color32, acts: &mut Vec<Act>) {
    // The header above already names this list. Naming it twice reads as
    // two headings for one thing.
    let Some(list) = &sheet.access else {
        ui.label(RichText::new(labels.loading.as_str()).color(weak));
        return;
    };
    if list.is_empty() {
        ui.label(RichText::new(labels.access_empty.as_str()).color(weak));
        return;
    }
    egui::ScrollArea::vertical().max_height(160.0).id_salt("craft-ui/share/access").show(ui, |ui| {
        for share in list {
            ui.horizontal(|ui| {
                ui.label(share_label(share, labels));
                let permission = match share.permission {
                    Permission::View => labels.can_view.as_str(),
                    Permission::Edit => labels.can_edit.as_str(),
                };
                ui.label(RichText::new(permission).color(weak));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let busy = sheet.withdrawing.iter().any(|(_, id)| *id == share.id);
                    // A bin, with the word on hover. The row is already a
                    // line of text; a second word on it reads as more content
                    // rather than as the control it is.
                    let glyph = if busy { "\u{22ef}" } else { "\u{1f5d1}" };
                    let bin = egui::Button::new(RichText::new(glyph).size(15.0)).frame(false);
                    let hint = if busy { labels.withdrawing.as_str() } else { labels.withdraw.as_str() };
                    let pressed = ui.add_enabled(!busy, bin);
                    // A glyph is not a name. Left alone this button announces itself
                    // to a screen reader as a wastebasket character, so the word goes
                    // on explicitly.
                    pressed.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, hint));
                    if pressed.on_hover_text(hint).clicked() {
                        acts.push(Act::Withdraw(share.id.clone()));
                    }
                    if let Some(url) = &share.url
                        && ui.small_button(labels.copy_link.as_str()).clicked()
                    {
                        ui.ctx().copy_text(url.clone());
                    }
                });
            });
            if let Some(note) = &share.note
                && !note.trim().is_empty()
            {
                ui.label(RichText::new(note.as_str()).color(weak).small());
            }
        }
    });
}

// ---- pieces ---------------------------------------------------------------

/// One picked recipient. The whole chip is the remove control.
fn chip(ui: &mut Ui, avatars: &HashMap<String, Avatar>, chosen: &Chosen, labels: &ShareLabels, acts: &mut Vec<Act>) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let inner = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        // A selectable label senses click and drag of its own, and one drawn
        // inside a clickable row wins the hit test over the row itself. For
        // narrow rows like a chip the pointer lands on the text nearly always,
        // so the press would be swallowed and the chip never removed.
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.add_space(3.0);
            picture(ui, avatars, chosen.avatar.as_deref(), &chosen.label, acts);
            ui.label(chosen.label.as_str());
            // The same cross the dialog closes with. U+2715 is absent from
            // the default font set, where it renders as a box.
            ui.label(RichText::new("×").color(ui.visuals().weak_text_color()));
            ui.add_space(1.0);
        });
    });
    let response = inner.response;
    let fill = if response.hovered() { ui.visuals().widgets.hovered.bg_fill } else { ui.visuals().widgets.inactive.bg_fill };
    ui.painter().set(background, egui::Shape::rect_filled(response.rect, CornerRadius::same(AVATAR_RADIUS), fill));
    let label = format!("{} {}", labels.remove, chosen.label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.clone()));
    // The address, where there is one. Two colleagues share a display name
    // often enough that a chip reading "Ann Reed" is not an identity.
    let hint = match &chosen.detail {
        Some(detail) if !detail.is_empty() => format!("{} · {detail}", labels.remove),
        _ => labels.remove.clone(),
    };
    response.on_hover_text(hint)
}

/// One match in the autocomplete list.
fn row(ui: &mut Ui, avatars: &HashMap<String, Avatar>, contact: &Contact, labels: &ShareLabels, acts: &mut Vec<Act>) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let width = ui.available_width();
    let inner = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        // A selectable label senses click and drag of its own, and one drawn
        // inside a clickable row wins the hit test over the row itself. For
        // narrow rows like a chip the pointer lands on the text nearly always,
        // so the press would be swallowed and the chip never removed.
        ui.style_mut().interaction.selectable_labels = false;
        ui.set_min_width(width);
        ui.horizontal(|ui| {
            ui.add_space(3.0);
            picture(ui, avatars, contact.avatar.as_deref(), &contact.name, acts);
            ui.label(contact.name.as_str());
            let detail = if contact.is_group { format!("({})", labels.group) } else { contact.email.clone().unwrap_or_default() };
            if !detail.is_empty() {
                ui.label(RichText::new(detail).color(ui.visuals().weak_text_color()));
            }
        });
    });
    let response = inner.response;
    if response.hovered() {
        ui.painter().set(background, egui::Shape::rect_filled(response.rect, CornerRadius::same(4), ui.visuals().widgets.hovered.bg_fill));
    }
    let label = contact.name.clone();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.clone()));
    response
}

/// A contact picture, or initials while there is none.
///
/// A picture is asked for here, as the row that needs it is painted, and only
/// when that row is actually on screen. A search answering with ten people
/// therefore costs no pictures at all until they are looked at, and the ones
/// scrolled past below the fold cost nothing either. `avatars` holds every URL
/// ever asked about, the refusals included, so nothing is asked for twice.
fn picture(ui: &mut Ui, avatars: &HashMap<String, Avatar>, url: Option<&str>, label: &str, acts: &mut Vec<Act>) {
    let size = Vec2::splat(AVATAR);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    match url.and_then(|u| avatars.get(u)) {
        Some(Avatar::Ready(uri)) => {
            let image = egui::Image::from_uri(uri.clone()).fit_to_exact_size(size).corner_radius(CornerRadius::same(AVATAR_RADIUS));
            if image.load_for_size(ui.ctx(), size).is_ok() {
                image.paint_at(ui, rect);
                return;
            }
        }
        Some(_) => {}
        None => {
            if let Some(url) = url.filter(|_| ui.is_rect_visible(rect)) {
                acts.push(Act::Avatar(url.to_owned()));
            }
        }
    }
    ui.painter().circle_filled(rect.center(), AVATAR / 2.0, ui.visuals().widgets.inactive.bg_fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initials(label), FontId::proportional(AVATAR * 0.42), ui.visuals().text_color());
}

fn initials(label: &str) -> String {
    label.split_whitespace().filter_map(|word| word.chars().next()).take(2).flat_map(|c| c.to_uppercase()).collect()
}

/// A stable URI for one avatar's bytes. egui caches by URI, so two contacts
/// must never share one.
fn avatar_uri(url: &str) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("bytes://craft-ui/share/avatar/{:016x}", hasher.finish())
}

fn choose(sheet: &mut Sheet, contact: &Contact) {
    // What the provider said, never a guess from the id. A contact with no
    // account is reachable only by address, and sharing that address as a user
    // id fails with "not found", because no such user exists.
    let target = contact.target.clone();
    add(sheet, target, contact.name.clone(), contact.email.clone(), contact.avatar.clone());
}

/// Add one recipient, unless it is already there.
fn add(sheet: &mut Sheet, target: ShareTarget, label: String, detail: Option<String>, avatar: Option<String>) {
    if !sheet.chosen.iter().any(|c| c.target == target) {
        sheet.chosen.push(Chosen { target, label, detail, avatar });
    }
    sheet.query.clear();
    sheet.matches.clear();
    sheet.typed = None;
    sheet.searching = false;
    sheet.lookup = None;
}

/// What the user typed, when it is an address and not a name.
///
/// Deliberately shallow. The server decides whether an address exists, and a
/// strict local check would refuse valid addresses it had not heard of.
fn address(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.chars().any(char::is_whitespace) {
        return None;
    }
    let (user, host) = typed.split_once('@')?;
    if user.is_empty() || host.len() < 3 || !host.contains('.') || host.starts_with('.') || host.ends_with('.') || host.contains('@') {
        return None;
    }
    Some(typed.to_owned())
}

/// What to call the person or group a share is with.
///
/// The handle is the fallback. Showing an account id to somebody who knows
/// that person by name tells them nothing, and two colleagues with similar
/// ids are indistinguishable.
fn share_label(share: &Share, labels: &ShareLabels) -> String {
    match share.display.as_deref().filter(|name| !name.trim().is_empty()) {
        Some(name) => match share.target {
            ShareTarget::Group(_) => format!("{name} ({})", labels.group),
            _ => name.to_owned(),
        },
        None => target_label(&share.target, labels),
    }
}

fn target_label(target: &ShareTarget, labels: &ShareLabels) -> String {
    match target {
        ShareTarget::User(id) => id.clone(),
        ShareTarget::Group(id) => format!("{id} ({})", labels.group),
        ShareTarget::Email(address) => address.clone(),
        ShareTarget::Link => labels.anyone_with_link.clone(),
    }
}
