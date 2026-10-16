//! Request-signatures dialog, the same one in every Craft app that sends PDFs.
//!
//! Nothing here opens a socket. Every call goes through [`EsignBackend`].
//! Nothing here blocks: answers arrive on a channel and are read once per
//! frame. Signing is not sharing: recipients are never [`craft_cloud::ShareTarget`]s.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use craft_cloud::{Contact, Identity};
use craft_esign::{Envelope, EnvelopeDraft, EnvelopeStatus, EsignCapability, FieldPlacement, Recipient, RecipientStatus};
use egui::{Context, RichText, Ui};

mod backend;

pub use backend::{Answer, EsignBackend, Replier, Request};

/// How long after the last keystroke the dialog asks for matches.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

const MATCHES: usize = 8;
const MIN_QUERY: usize = 2;
const RETRY: f64 = 20.0;

type Inbox = (u64, Result<Answer, String>);

/// Every string the dialog shows.
#[derive(Clone, Debug)]
pub struct EsignLabels {
    pub title: String,
    pub recipients: String,
    pub recipient_hint: String,
    pub searching: String,
    pub no_matches: String,
    pub add_address: String,
    pub role: String,
    pub message: String,
    pub send: String,
    pub sending: String,
    pub close: String,
    pub loading: String,
    pub responsible: String,
    pub no_service: String,
    pub sign_in_first: String,
    pub need_recipient: String,
    pub status: String,
    pub void: String,
    pub remind: String,
    pub download: String,
    pub request: String,
}

impl Default for EsignLabels {
    fn default() -> EsignLabels {
        EsignLabels {
            title: "Request signatures".into(),
            recipients: "Signers".into(),
            recipient_hint: "Name or email".into(),
            searching: "Searching…".into(),
            no_matches: "No matches".into(),
            add_address: "Add email address".into(),
            role: "Role".into(),
            message: "Message".into(),
            send: "Send".into(),
            sending: "Sending…".into(),
            close: "Close".into(),
            loading: "Loading…".into(),
            responsible: "Signing is provided by {party}. That party holds legal and compliance responsibility for the ceremony.".into(),
            no_service: "Signatures cannot be requested here.".into(),
            sign_in_first: "Sign in to request signatures.".into(),
            need_recipient: "Add at least one signer.".into(),
            status: "Status".into(),
            void: "Void".into(),
            remind: "Remind".into(),
            download: "Download completed".into(),
            request: "Request signatures".into(),
        }
    }
}

/// What the application must supply when the user presses Send.
///
/// The dialog holds recipients and copy; the application owns the document
/// bytes and encodes them, then submits [`Request::Create`] through the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendRequest {
    pub draft: EnvelopeDraft,
    pub idempotency_key: String,
}

/// One Request signatures dialog.
pub struct EsignDialog {
    open: bool,
    labels: EsignLabels,
    backend: Option<Box<dyn EsignBackend>>,
    capabilities: Option<Vec<EsignCapability>>,
    responsible: Option<String>,
    linked: bool,
    asked_caps_at: f64,
    seq: u64,
    tx: Sender<Inbox>,
    rx: Receiver<Inbox>,
    newest: u64,
    document_id: String,
    document_etag: Option<String>,
    document_title: String,
    sender: Option<Identity>,
    query: String,
    query_changed_at: f64,
    lookup_seq: u64,
    last_lookup_query: String,
    matches: Vec<Contact>,
    looking: bool,
    recipients: Vec<Recipient>,
    message: String,
    placements: Vec<FieldPlacement>,
    busy: bool,
    error: Option<String>,
    envelope: Option<Envelope>,
    send_out: Option<SendRequest>,
    completed_bytes: Option<Vec<u8>>,
    account_fingerprint: Option<String>,
}

impl Default for EsignDialog {
    fn default() -> EsignDialog {
        let (tx, rx) = std::sync::mpsc::channel();
        EsignDialog {
            open: false,
            labels: EsignLabels::default(),
            backend: None,
            capabilities: None,
            responsible: None,
            linked: false,
            asked_caps_at: 0.0,
            seq: 0,
            tx,
            rx,
            newest: 0,
            document_id: String::new(),
            document_etag: None,
            document_title: String::new(),
            sender: None,
            query: String::new(),
            query_changed_at: 0.0,
            lookup_seq: 0,
            last_lookup_query: String::new(),
            matches: Vec::new(),
            looking: false,
            recipients: Vec::new(),
            message: String::new(),
            placements: Vec::new(),
            busy: false,
            error: None,
            envelope: None,
            send_out: None,
            completed_bytes: None,
            account_fingerprint: None,
        }
    }
}

impl EsignDialog {
    pub fn new() -> EsignDialog {
        EsignDialog::default()
    }

    pub fn labels(mut self, labels: EsignLabels) -> EsignDialog {
        self.labels = labels;
        self
    }

    pub fn set_backend(&mut self, backend: impl EsignBackend + 'static) {
        self.backend = Some(Box::new(backend));
        self.capabilities = None;
        self.asked_caps_at = 0.0;
    }

    /// False until the service reports [`EsignCapability::Envelopes`].
    pub fn available(&self) -> bool {
        self.capabilities.as_ref().is_some_and(|c| c.contains(&EsignCapability::Envelopes))
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open for a document. `sender` is the active account identity.
    pub fn open(&mut self, document_id: String, document_title: String, document_etag: Option<String>, sender: Identity) {
        let fingerprint = format!("{}@{}", sender.id, sender.email.as_deref().unwrap_or(""));
        if self.account_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            self.recipients.clear();
            self.message.clear();
            self.envelope = None;
            self.completed_bytes = None;
            self.error = None;
        }
        self.account_fingerprint = Some(fingerprint);
        self.document_id = document_id;
        self.document_title = document_title;
        self.document_etag = document_etag;
        self.sender = Some(sender);
        self.open = true;
        self.query.clear();
        self.matches.clear();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Field placements the app prepared (page-normalised). Replaces any prior set.
    pub fn set_placements(&mut self, placements: Vec<FieldPlacement>) {
        self.placements = placements;
    }

    /// What Send asked the application to encode and submit, if anything.
    pub fn take_send_request(&mut self) -> Option<SendRequest> {
        self.send_out.take()
    }

    /// Submit a request through the host backend (for example after the app
    /// encoded document bytes for a [`SendRequest`]).
    pub fn submit_request(&mut self, request: Request) {
        let _ = self.submit(request);
    }

    /// Completed document bytes after a successful download request.
    pub fn take_completed_bytes(&mut self) -> Option<Vec<u8>> {
        self.completed_bytes.take()
    }

    /// Tell the dialog create/send finished (or failed) after the app submitted.
    pub fn note_create_result(&mut self, result: Result<Envelope, String>) {
        self.busy = false;
        match result {
            Ok(env) => {
                self.envelope = Some(env);
                self.error = None;
            }
            Err(reason) => self.error = Some(reason),
        }
    }

    fn submit(&mut self, request: Request) -> u64 {
        self.seq = self.seq.saturating_add(1);
        let seq = self.seq;
        self.newest = seq;
        if let Some(backend) = self.backend.as_ref() {
            backend.submit(request, Replier::new(seq, self.tx.clone()));
        } else {
            let _ = self.tx.send((seq, Err("no e-sign backend".into())));
        }
        seq
    }

    fn pump(&mut self, now: f64) {
        while let Ok((seq, result)) = self.rx.try_recv() {
            if seq < self.newest && !matches!(result, Ok(Answer::Capabilities { .. })) {
                // Stale lookup or status; keep capabilities even if older.
                if !matches!(&result, Ok(Answer::Lookup(_))) {
                    continue;
                }
                if seq != self.lookup_seq {
                    continue;
                }
            }
            match result {
                Ok(Answer::Capabilities { capabilities, responsible_party, linked }) => {
                    self.capabilities = Some(capabilities);
                    self.responsible = Some(responsible_party);
                    self.linked = linked;
                }
                Ok(Answer::Lookup(contacts)) if seq == self.lookup_seq => {
                    self.matches = contacts;
                    self.looking = false;
                }
                Ok(Answer::Created(env)) | Ok(Answer::Status(env)) => {
                    self.busy = false;
                    self.envelope = Some(env);
                    self.error = None;
                }
                Ok(Answer::Completed(doc)) => {
                    self.busy = false;
                    self.completed_bytes = Some(doc.bytes);
                    self.error = None;
                }
                Ok(Answer::Voided { .. }) => {
                    self.busy = false;
                    if let Some(env) = self.envelope.as_mut() {
                        env.status = EnvelopeStatus::Voided;
                    }
                }
                Ok(Answer::Reminded { .. }) => {
                    self.busy = false;
                }
                Err(reason) => {
                    self.busy = false;
                    self.looking = false;
                    self.error = Some(reason);
                }
                _ => {}
            }
        }

        if self.backend.is_some() && (self.capabilities.is_none() || (now - self.asked_caps_at) > RETRY && !self.available()) {
            self.asked_caps_at = now;
            let _ = self.submit(Request::Capabilities);
            // A synchronous test backend answers before submit returns; drain once more.
            while let Ok((seq, result)) = self.rx.try_recv() {
                if let Ok(Answer::Capabilities { capabilities, responsible_party, linked }) = result {
                    let _ = seq;
                    self.capabilities = Some(capabilities);
                    self.responsible = Some(responsible_party);
                    self.linked = linked;
                }
            }
        }
    }

    fn maybe_lookup(&mut self, now: f64) {
        if !self.open || self.query.len() < MIN_QUERY {
            return;
        }
        if (now - self.query_changed_at) < DEBOUNCE.as_secs_f64() {
            return;
        }
        if self.looking || self.last_lookup_query == self.query {
            return;
        }
        self.looking = true;
        self.last_lookup_query = self.query.clone();
        self.lookup_seq = self.submit(Request::Lookup { query: self.query.clone(), limit: MATCHES });
    }

    fn add_contact(&mut self, contact: Contact) {
        let email = contact.email.clone().unwrap_or_else(|| contact.id.clone());
        if self.recipients.iter().any(|r| r.email.eq_ignore_ascii_case(&email)) {
            return;
        }
        let role = format!("Signer {}", self.recipients.len() + 1);
        self.recipients.push(Recipient { id: contact.id, name: contact.name, email, role, required: true, status: RecipientStatus::Pending });
        self.query.clear();
        self.matches.clear();
    }

    fn add_email(&mut self, email: String) {
        let email = email.trim().to_string();
        if email.is_empty() || !email.contains('@') {
            return;
        }
        if self.recipients.iter().any(|r| r.email.eq_ignore_ascii_case(&email)) {
            return;
        }
        let role = format!("Signer {}", self.recipients.len() + 1);
        self.recipients.push(Recipient { id: email.clone(), name: email.clone(), email, role, required: true, status: RecipientStatus::Pending });
        self.query.clear();
        self.matches.clear();
    }

    fn queue_send(&mut self) {
        let Some(sender) = self.sender.clone() else {
            self.error = Some(self.labels.sign_in_first.clone());
            return;
        };
        if self.recipients.is_empty() {
            self.error = Some(self.labels.need_recipient.clone());
            return;
        }
        if !self.linked {
            self.error = Some(self.labels.sign_in_first.clone());
            return;
        }
        let draft = EnvelopeDraft {
            title: if self.document_title.is_empty() { "Document".into() } else { self.document_title.clone() },
            message: if self.message.trim().is_empty() { None } else { Some(self.message.clone()) },
            sender,
            document_id: if self.document_id.is_empty() { None } else { Some(self.document_id.clone()) },
            document_etag: self.document_etag.clone(),
            recipients: self.recipients.clone(),
            placements: self.placements.clone(),
            expires: None,
        };
        let idempotency_key = format!("esign-{}-{}", self.document_id, self.seq.saturating_add(1));
        self.busy = true;
        self.error = None;
        self.send_out = Some(SendRequest { draft, idempotency_key });
    }

    /// Draw the dialog when open. Call every frame (even when closed) so
    /// capability probes and inbox pumping continue.
    pub fn show(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        self.pump(now);
        if self.open {
            self.maybe_lookup(now);
        }
        if !self.open {
            return;
        }

        let mut open = self.open;
        let labels = self.labels.clone();
        egui::Window::new(&labels.title).open(&mut open).resizable(true).default_width(420.0).show(ctx, |ui| {
            self.ui(ui);
        });
        self.open = open;
    }

    fn ui(&mut self, ui: &mut Ui) {
        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
            ui.add_space(4.0);
        }

        if !self.available() {
            ui.label(if self.capabilities.is_none() { self.labels.loading.as_str() } else { self.labels.no_service.as_str() });
            return;
        }

        if let Some(party) = &self.responsible {
            ui.label(RichText::new(self.labels.responsible.replace("{party}", party)).small());
            ui.add_space(6.0);
        }

        if let Some(env) = self.envelope.clone() {
            self.status_panel(ui, &env);
            return;
        }

        ui.label(RichText::new(&self.document_title).strong());
        ui.add_space(4.0);

        ui.label(&self.labels.recipients);
        let response = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(&self.labels.recipient_hint));
        if response.changed() {
            self.query_changed_at = ui.input(|i| i.time);
            self.matches.clear();
            self.looking = false;
            self.lookup_seq = 0;
            self.last_lookup_query.clear();
        }
        if self.looking {
            ui.spinner();
            ui.label(&self.labels.searching);
        } else if self.query.len() >= MIN_QUERY && self.matches.is_empty() {
            ui.horizontal(|ui| {
                ui.label(&self.labels.no_matches);
                if ui.button(&self.labels.add_address).clicked() {
                    self.add_email(self.query.clone());
                }
            });
        }
        for contact in self.matches.clone() {
            let label = match &contact.email {
                Some(email) => format!("{} <{}>", contact.name, email),
                None => contact.name.clone(),
            };
            if ui.selectable_label(false, label).clicked() {
                self.add_contact(contact);
            }
        }

        ui.add_space(4.0);
        let mut remove: Option<usize> = None;
        for (i, recipient) in self.recipients.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{} — {}", recipient.name, recipient.email));
                ui.label(RichText::new(&recipient.role).small());
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.recipients.remove(i);
        }

        ui.add_space(6.0);
        ui.label(&self.labels.message);
        ui.add(egui::TextEdit::multiline(&mut self.message).desired_rows(3));

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_send = !self.busy && self.linked && !self.recipients.is_empty();
            if ui.add_enabled(can_send, egui::Button::new(if self.busy { &self.labels.sending } else { &self.labels.send })).clicked() {
                self.queue_send();
            }
            if ui.button(&self.labels.close).clicked() {
                self.open = false;
            }
        });
    }

    fn status_panel(&mut self, ui: &mut Ui, env: &Envelope) {
        ui.label(RichText::new(&env.title).strong());
        ui.label(format!("{}: {:?}", self.labels.status, env.status));
        for r in &env.recipients {
            ui.label(format!("{} — {:?} ({})", r.name, r.status, r.role));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let caps = self.capabilities.clone().unwrap_or_default();
            if caps.contains(&EsignCapability::Reminders)
                && ui.add_enabled(!self.busy, egui::Button::new(&self.labels.remind)).clicked()
                && let Some(r) = env.recipients.iter().find(|r| r.status == RecipientStatus::Pending)
            {
                self.busy = true;
                let _ = self.submit(Request::Remind { envelope: env.id.clone(), recipient: r.id.clone() });
            }
            if caps.contains(&EsignCapability::Void)
                && env.status == EnvelopeStatus::Sent
                && ui.add_enabled(!self.busy, egui::Button::new(&self.labels.void)).clicked()
            {
                self.busy = true;
                let _ = self.submit(Request::Void { envelope: env.id.clone(), reason: "voided by sender".into() });
            }
            if env.status == EnvelopeStatus::Completed && ui.add_enabled(!self.busy, egui::Button::new(&self.labels.download)).clicked() {
                self.busy = true;
                let _ = self.submit(Request::Completed { envelope: env.id.clone() });
            }
            if ui.button(&self.labels.close).clicked() {
                self.open = false;
            }
        });
        if ui.button("Refresh status").clicked() {
            self.busy = true;
            let _ = self.submit(Request::Status { envelope: env.id.clone() });
        }
    }
}

/// Split button that opens the request-signatures dialog.
///
/// `open` is called when the primary action is chosen. The chevron offers
/// status refresh when an envelope id is known.
pub fn request_button(ui: &mut Ui, labels: &EsignLabels, available: bool, menu: &[crate::menu::MenuItem]) -> Option<crate::menu::SplitHit> {
    if !available {
        return None;
    }
    let menu_labels = crate::menu::MenuLabels::default();
    crate::menu::split(ui, &labels.request, menu, &menu_labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Capture {
        last: Mutex<Option<Request>>,
    }

    impl EsignBackend for Arc<Capture> {
        fn submit(&self, request: Request, reply: Replier) {
            *self.last.lock().expect("lock") = Some(request);
            reply.answer(Answer::Capabilities {
                capabilities: vec![EsignCapability::Envelopes, EsignCapability::Void],
                responsible_party: "Stub Signing".into(),
                linked: true,
            });
        }
    }

    #[test]
    fn available_after_capabilities() {
        let capture = Arc::new(Capture { last: Mutex::new(None) });
        let mut dialog = EsignDialog::default();
        dialog.set_backend(capture);
        let ctx = Context::default();
        dialog.show(&ctx);
        dialog.show(&ctx);
        assert!(dialog.available());
        assert_eq!(dialog.responsible.as_deref(), Some("Stub Signing"));
    }

    #[test]
    fn send_request_needs_recipient_and_sender() {
        let mut dialog = EsignDialog {
            linked: true,
            sender: Some(Identity { id: "u".into(), display: "Ada".into(), email: Some("ada@example.com".into()), avatar: None }),
            ..Default::default()
        };
        dialog.queue_send();
        assert!(dialog.take_send_request().is_none());
        dialog.add_email("bob@example.com".into());
        dialog.queue_send();
        let req = dialog.take_send_request().expect("send");
        assert_eq!(req.draft.recipients.len(), 1);
        assert_eq!(req.draft.recipients[0].email, "bob@example.com");
    }
}
