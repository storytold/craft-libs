//! Request-signatures dialog, the same one in every Craft app that sends PDFs.
//!
//! Nothing here opens a socket. Every call goes through [`EsignBackend`].
//! Nothing here blocks: answers arrive on a channel and are read once per
//! frame. Signing is not sharing: recipients are never [`craft_cloud::ShareTarget`]s.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use craft_cloud::{Contact, Identity};
use craft_esign::{Envelope, EnvelopeDraft, EnvelopeStatus, EsignCapability, FieldPlacement, Recipient, RecipientStatus};
use egui::{Context, RichText, Ui};

mod backend;
#[cfg(test)]
mod coderabbit_repro;
#[cfg(test)]
mod coderabbit_round2_repro;
#[cfg(test)]
mod review_repro;

pub use backend::{Answer, EsignBackend, Replier, Request};

/// How long after the last keystroke the dialog asks for matches.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

const MATCHES: usize = 8;
const MIN_QUERY: usize = 2;
const RETRY: f64 = 20.0;

type Inbox = (u64, Result<Answer, String>);

/// What dialog state a reply belongs to: which document revision, under
/// which signed-in account, opened which time. A reply installs only when
/// this still matches the dialog's current context; see
/// [`EsignDialog::submit_in_context`] and [`EsignDialog::pump`]. Deliberately
/// excludes incidental UI state such as search text, which changes without
/// the dialog meaning something new.
///
/// `opening` is bumped every time [`EsignDialog::open`] actually changes
/// something (document, revision or account). `document_etag` alone would
/// already separate two revisions of the same document id, but it cannot
/// separate "document A, closed, then A opened again" from the first
/// opening: both carry the same id and the same etag. `opening` does.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EsignContext {
    document_id: String,
    document_etag: Option<String>,
    account: Option<String>,
    opening: u64,
}

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
    /// Shown instead of adding the person, when a search match has no email
    /// address the provider can invite.
    pub contact_missing_email: String,
    pub status: String,
    pub void: String,
    pub remind: String,
    pub download: String,
    pub request: String,
    /// Caption of the status panel's manual refresh action.
    pub refresh: String,
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
            contact_missing_email: "This contact has no email address to invite.".into(),
            status: "Status".into(),
            void: "Void".into(),
            remind: "Remind".into(),
            download: "Download completed".into(),
            request: "Request signatures".into(),
            refresh: "Refresh status".into(),
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
    asked_caps_at: Option<f64>,
    caps_seq: Option<u64>,
    draft_idempotency_key: Option<String>,
    seq: u64,
    tx: Sender<Inbox>,
    rx: Receiver<Inbox>,
    newest: u64,
    /// Context (document plus account) each in-flight context-sensitive
    /// request was issued under. Consulted, then removed, in [`Self::pump`].
    contexts: HashMap<u64, EsignContext>,
    /// Sequences whose context has already been consulted once in
    /// [`Self::pump`], bounded to the most recent entries. A backend that
    /// answers the same sequence twice (via [`Replier::dup`]) has its second
    /// answer dropped unconditionally instead of re-entering the match arms
    /// with no context to check.
    consumed: std::collections::HashSet<u64>,
    consumed_order: std::collections::VecDeque<u64>,
    /// The sequence that currently owns `busy`, if any. An answer (success or
    /// failure) for any other sequence must not clear `busy` on its behalf.
    pending_op_seq: Option<u64>,
    /// The most recently issued `Request::Status` sequence. An older status
    /// reply that arrives after a newer one has already installed is
    /// superseded and dropped.
    status_seq: u64,
    /// How many times [`Self::open`] has actually changed something. Part of
    /// [`EsignContext`]; see its docs.
    opening: u64,
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
    /// How many recipients have ever been added for the document currently
    /// open, used to label the next one. Monotonic, not derived from the
    /// current recipient count or roles in use: a count- or scan-based label
    /// is free to repeat once the recipient holding it is removed, and a
    /// freed role can then bind a new recipient to an old recipient's
    /// placements. Reset in [`Self::open`] alongside `recipients`.
    next_role_number: u32,
    message: String,
    placements: Vec<FieldPlacement>,
    busy: bool,
    error: Option<String>,
    envelope: Option<Envelope>,
    send_out: Option<SendRequest>,
    /// The context [`Self::queue_send`] captured for the still-outstanding
    /// [`SendRequest`], consumed by [`Self::submit_request`] or
    /// [`Self::note_create_result`] so a reply is tagged with the context the
    /// send was queued under, not whatever context is current when the
    /// application finally submits it.
    pending_send_context: Option<EsignContext>,
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
            asked_caps_at: None,
            caps_seq: None,
            draft_idempotency_key: None,
            seq: 0,
            tx,
            rx,
            newest: 0,
            contexts: HashMap::new(),
            consumed: std::collections::HashSet::new(),
            consumed_order: std::collections::VecDeque::new(),
            pending_op_seq: None,
            status_seq: 0,
            opening: 0,
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
            next_role_number: 0,
            message: String::new(),
            placements: Vec::new(),
            busy: false,
            error: None,
            envelope: None,
            send_out: None,
            pending_send_context: None,
            completed_bytes: None,
            account_fingerprint: None,
        }
    }
}

/// A send operation id no other send in this process has used, from a random
/// start so one saved by an earlier process is unlikely to repeat either.
/// Mirrors `docking::model::fresh_window_id`: a counter kept on one
/// [`EsignDialog`] repeats across two dialog instances (two windows, or one
/// replaced after a document closes), and two sends sharing an idempotency
/// key have a conforming provider return the first envelope for the second.
fn fresh_send_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| AtomicU64::new(std::collections::hash_map::RandomState::new().build_hasher().finish() >> 1));
    next.fetch_add(1, Ordering::Relaxed)
}

/// Whether the dialog stays open after one [`EsignDialog::show`] frame.
///
/// `window_open` is egui's own title-bar close control; `dialog_open` is
/// whatever [`EsignDialog::ui`] already decided, such as a Close button.
/// Either one closing the dialog must stick: a plain `window_open` would
/// let the window's unchanged binding undo a Close button clicked inside
/// the same frame.
fn still_open(window_open: bool, dialog_open: bool) -> bool {
    window_open && dialog_open
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
        self.reset_capabilities();
    }

    /// Probe the current backend again for capabilities and link state.
    ///
    /// `available()` never asks again once the service has reported
    /// [`EsignCapability::Envelopes`]; the retry in [`Self::pump`] is for a
    /// backend that is still unavailable, not for a `linked` value that is
    /// stale after an out-of-band sign-in. Call this once that sign-in
    /// completes on the backend already set, when replacing it with
    /// [`Self::set_backend`] would be artificial. A reply still in flight
    /// from the probe this replaces is dropped in [`Self::pump`] exactly as
    /// one from a backend [`Self::set_backend`] already replaced is:
    /// `caps_seq` is cleared here the same way.
    pub fn reset_capabilities(&mut self) {
        self.capabilities = None;
        self.asked_caps_at = None;
        self.caps_seq = None;
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
        let account_changed = self.account_fingerprint.as_deref() != Some(fingerprint.as_str());
        let document_changed = self.document_id != document_id;
        let revision_changed = self.document_etag != document_etag;
        if account_changed || document_changed || revision_changed {
            self.recipients.clear();
            self.next_role_number = 0;
            self.message.clear();
            self.envelope = None;
            self.completed_bytes = None;
            self.error = None;
            self.draft_idempotency_key = None;
            // A's fields must never reappear as B's: see the Major finding
            // this guards against.
            self.placements.clear();
            // An untaken send request was built for the context being left;
            // drop it so a later `take_send_request` cannot hand the
            // application A's draft to submit under B's dialog.
            // `pending_send_context`, captured once the request is taken, is
            // deliberately left alone here: see its field doc.
            self.send_out = None;
            self.opening = self.opening.wrapping_add(1);
            // Any job still in flight was issued for the context being left.
            // Its reply will be dropped in `pump` (context no longer
            // matches), so nothing will ever clear `busy` on its behalf.
            self.busy = false;
            self.pending_op_seq = None;
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
        // Same reasoning as `add_contact`/`add_email`/removing a recipient: the
        // fields a retried create would send have changed, so a key from
        // before this call must not be reused for them.
        self.draft_idempotency_key = None;
    }

    /// What Send asked the application to encode and submit, if anything.
    pub fn take_send_request(&mut self) -> Option<SendRequest> {
        self.send_out.take()
    }

    /// Submit a request through the host backend (for example after the app
    /// encoded document bytes for a [`SendRequest`], or a status refresh the
    /// host drives itself rather than through [`Self::status_panel`]'s own
    /// button).
    ///
    /// Tags the reply with the context [`Self::queue_send`] captured when it
    /// built the [`SendRequest`] being submitted, not whatever context is
    /// current now. The application can encode bytes, or the dialog can be
    /// reopened for another document or account, before this runs; a reply
    /// for a [`SendRequest`] built under a now-abandoned context must drop in
    /// [`Self::pump`] exactly as one issued under that context would, not
    /// install under whichever context happens to be current when this is
    /// finally called.
    pub fn submit_request(&mut self, request: Request) {
        // The captured send context belongs to the create submission answering
        // a taken [`SendRequest`], and to nothing else. A status refresh (or
        // any other request) submitted while that send is still outstanding is
        // the current document's business: tagging it with the old send's
        // context would make `pump` drop its reply as stale.
        let context = if matches!(request, Request::Create { .. }) {
            self.pending_send_context.take().unwrap_or_else(|| self.current_context())
        } else {
            self.current_context()
        };
        // A public `Request::Status` submission is request-specific routing
        // state too: without this, `pump`'s `status_seq` guard never sees
        // this sequence and drops the reply that comes back for it.
        let is_status = matches!(request, Request::Status { .. });
        let seq = self.submit(request);
        self.contexts.insert(seq, context);
        self.pending_op_seq = Some(seq);
        if is_status {
            self.status_seq = seq;
        }
    }

    /// Completed document bytes after a successful download request.
    pub fn take_completed_bytes(&mut self) -> Option<Vec<u8>> {
        self.completed_bytes.take()
    }

    /// Tell the dialog create/send finished (or failed) after the app submitted.
    ///
    /// Gated the same way as [`Self::submit_request`]: when a context was
    /// captured for the outstanding send, a result reported after the
    /// dialog moved on to another document or account is dropped instead of
    /// installing under the new context.
    pub fn note_create_result(&mut self, result: Result<Envelope, String>) {
        if let Some(context) = self.pending_send_context.take()
            && context != self.current_context()
        {
            return;
        }
        self.busy = false;
        self.pending_op_seq = None;
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

    /// The context (document revision plus account, plus which opening) a
    /// reply must still match to install. See [`EsignContext`].
    fn current_context(&self) -> EsignContext {
        EsignContext {
            document_id: self.document_id.clone(),
            document_etag: self.document_etag.clone(),
            account: self.account_fingerprint.clone(),
            opening: self.opening,
        }
    }

    /// Like [`Self::submit`], but tags the request with the dialog's current
    /// context. Everything that acts on one envelope (create, status,
    /// completed, void, remind) goes through here. Capabilities does not:
    /// it is answered before a document is ever opened and stays valid
    /// across documents and accounts. Lookup does: a contact search issued
    /// before a reopen must not populate the reopened dialog either.
    fn submit_in_context(&mut self, request: Request) -> u64 {
        let seq = self.submit(request);
        self.contexts.insert(seq, self.current_context());
        seq
    }

    /// Remembers that `seq`'s context has already been consulted once in
    /// [`Self::pump`], bounded to the most recently consulted sequences. A
    /// backend that answers the same sequence twice (only possible through
    /// [`Replier::dup`]) has its second answer dropped unconditionally,
    /// instead of reaching the match arms with no context left to check.
    fn remember_consumed(&mut self, seq: u64) {
        const TRACKED: usize = 64;
        if self.consumed_order.len() >= TRACKED
            && let Some(oldest) = self.consumed_order.pop_front()
        {
            self.consumed.remove(&oldest);
        }
        self.consumed_order.push_back(seq);
        self.consumed.insert(seq);
    }

    fn pump(&mut self, now: f64) {
        while let Ok((seq, result)) = self.rx.try_recv() {
            let is_caps_reply = self.caps_seq == Some(seq);
            if is_caps_reply {
                self.caps_seq = None;
            }
            if self.consumed.contains(&seq) {
                continue;
            }
            // A context-tagged reply installs only for the context that
            // issued it; it drops silently for any other, including a
            // document switch, a revision change or an account switch.
            // Typing in the signer search bumps `newest` and is not a
            // context change, so it no longer gates a create/status/
            // completed/void/remind reply: the rule here is context
            // equality, nothing else.
            if let Some(context) = self.contexts.remove(&seq) {
                self.remember_consumed(seq);
                if context != self.current_context() {
                    continue;
                }
            }
            match result {
                // Guarded by `is_caps_reply`, computed before the swap
                // above cleared it: a reply from a backend `set_backend`
                // already replaced carries a sequence that is not (or no
                // longer) `caps_seq`, so it cannot overwrite capabilities,
                // responsible or linked for the new backend.
                Ok(Answer::Capabilities { capabilities, responsible_party, linked }) if is_caps_reply => {
                    self.capabilities = Some(capabilities);
                    self.responsible = Some(responsible_party);
                    self.linked = linked;
                }
                Ok(Answer::Lookup(contacts)) if seq == self.lookup_seq => {
                    self.matches = contacts;
                    self.looking = false;
                }
                Ok(Answer::Created(env)) => {
                    self.busy = false;
                    self.pending_op_seq = None;
                    self.envelope = Some(env);
                    self.error = None;
                }
                // Guarded by `status_seq`, the most recently issued status
                // request: two "Refresh status" clicks in flight at once can
                // answer out of order, and the older one must not overwrite
                // state a newer one already installed.
                Ok(Answer::Status(env)) if seq == self.status_seq => {
                    self.busy = false;
                    self.pending_op_seq = None;
                    self.envelope = Some(env);
                    self.error = None;
                }
                Ok(Answer::Completed(doc)) => {
                    self.busy = false;
                    self.pending_op_seq = None;
                    self.completed_bytes = Some(doc.bytes);
                    self.error = None;
                }
                Ok(Answer::Voided { .. }) => {
                    self.busy = false;
                    self.pending_op_seq = None;
                    if let Some(env) = self.envelope.as_mut() {
                        env.status = EnvelopeStatus::Voided;
                    }
                }
                Ok(Answer::Reminded { .. }) => {
                    self.busy = false;
                    self.pending_op_seq = None;
                }
                // Clears `busy` only when this failure belongs to the
                // operation that currently owns it: a lookup failing (or a
                // superseded status request failing) must not wrongly
                // report a still-running create as finished.
                Err(reason) => {
                    if self.pending_op_seq == Some(seq) {
                        self.busy = false;
                        self.pending_op_seq = None;
                    }
                    self.looking = false;
                    self.error = Some(reason);
                }
                _ => {}
            }
        }

        if self.backend.is_some() && self.caps_seq.is_none() {
            let first = self.capabilities.is_none() && self.asked_caps_at.is_none();
            let retry = self.asked_caps_at.is_some_and(|asked| !self.available() && now - asked >= RETRY);
            if first || retry {
                self.asked_caps_at = Some(now);
                self.caps_seq = Some(self.submit(Request::Capabilities));
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
        self.lookup_seq = self.submit_in_context(Request::Lookup { query: self.query.clone(), limit: MATCHES });
    }

    fn add_contact(&mut self, contact: Contact) {
        // `Contact::id` is the provider's share identifier, decided by
        // `target`, and is not promised to be an address: `craft_cloud`
        // documents the two as distinct and warns against treating one as
        // the other. Falling back to it here would hand the envelope create
        // request a recipient email that is not an email.
        let Some(email) = contact.email.clone() else {
            self.error = Some(self.labels.contact_missing_email.clone());
            return;
        };
        if self.recipients.iter().any(|r| r.email.eq_ignore_ascii_case(&email)) {
            return;
        }
        let role = self.next_role();
        self.recipients.push(Recipient { id: contact.id, name: contact.name, email, role, required: true, status: RecipientStatus::Pending });
        self.draft_idempotency_key = None;
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
        let role = self.next_role();
        self.recipients.push(Recipient { id: email.clone(), name: email.clone(), email, role, required: true, status: RecipientStatus::Pending });
        self.draft_idempotency_key = None;
        self.query.clear();
        self.matches.clear();
    }

    /// A role label no recipient added for the document currently open has
    /// held, including one since removed: see `next_role_number`'s doc.
    fn next_role(&mut self) -> String {
        self.next_role_number += 1;
        format!("Signer {}", self.next_role_number)
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
        if self.draft_idempotency_key.is_none() {
            self.draft_idempotency_key = Some(format!("esign-{}-{}", self.document_id, fresh_send_id()));
        }
        let idempotency_key = self.draft_idempotency_key.clone().unwrap_or_default();
        self.busy = true;
        self.error = None;
        self.send_out = Some(SendRequest { draft, idempotency_key });
        self.pending_send_context = Some(self.current_context());
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
        // `open` reflects only the window's own title-bar close control;
        // `self.ui` may already have set `self.open` to false from one of
        // its own Close buttons. Either closing the dialog must stick, so
        // reopening it takes a fresh `Self::open` call either way.
        self.open = still_open(open, self.open);
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

        if !self.linked {
            // Send is disabled below without saying why; the compose form is
            // the one place an unlinked user sees before reaching it.
            ui.label(&self.labels.sign_in_first);
            ui.add_space(4.0);
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
            self.draft_idempotency_key = None;
            // A placement keyed to the removed recipient's role would
            // otherwise ride along in the next draft matching no recipient.
            let live_roles: std::collections::HashSet<&str> = self.recipients.iter().map(|r| r.role.as_str()).collect();
            self.placements.retain(|p| live_roles.contains(p.role.as_str()));
        }

        ui.add_space(6.0);
        ui.label(&self.labels.message);
        let message = ui.add(egui::TextEdit::multiline(&mut self.message).desired_rows(3));
        if message.changed() {
            self.draft_idempotency_key = None;
        }

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
                let seq = self.submit_in_context(Request::Remind { envelope: env.id.clone(), recipient: r.id.clone() });
                self.pending_op_seq = Some(seq);
            }
            if caps.contains(&EsignCapability::Void)
                && env.status == EnvelopeStatus::Sent
                && ui.add_enabled(!self.busy, egui::Button::new(&self.labels.void)).clicked()
            {
                self.busy = true;
                let seq = self.submit_in_context(Request::Void { envelope: env.id.clone(), reason: "voided by sender".into() });
                self.pending_op_seq = Some(seq);
            }
            if env.status == EnvelopeStatus::Completed && ui.add_enabled(!self.busy, egui::Button::new(&self.labels.download)).clicked() {
                self.busy = true;
                let seq = self.submit_in_context(Request::Completed { envelope: env.id.clone() });
                self.pending_op_seq = Some(seq);
            }
            if ui.button(&self.labels.close).clicked() {
                self.open = false;
            }
        });
        if ui.button(&self.labels.refresh).clicked() {
            self.busy = true;
            let seq = self.submit_in_context(Request::Status { envelope: env.id.clone() });
            self.pending_op_seq = Some(seq);
            self.status_seq = seq;
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

    /// The texts egui drew, walking into `Shape::Vec` groups the way widgets nest them. Same
    /// pattern as `docking::native_tests::texts`.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
        out
    }

    #[test]
    fn compose_form_shows_sign_in_message_when_not_linked() {
        let mut dialog =
            EsignDialog { capabilities: Some(vec![EsignCapability::Envelopes]), document_title: "A.pdf".into(), linked: false, ..Default::default() };
        let ctx = Context::default();
        let mut full = ctx.run_ui(egui::RawInput::default(), |ui| dialog.ui(ui));
        full.textures_delta.clear();
        let shown = texts(&full.shapes);
        assert!(shown.iter().any(|t| t == &dialog.labels.sign_in_first), "sign-in message not shown for an unlinked account: {shown:?}");
    }

    fn ada() -> Identity {
        Identity { id: "u".into(), display: "Ada".into(), email: Some("ada@example.com".into()), avatar: None }
    }

    fn placement() -> FieldPlacement {
        FieldPlacement { role: "Signer".into(), page: 0, x: 0.1, y: 0.2, w: 0.3, h: 0.05, kind: "signature".into(), required: true }
    }

    #[test]
    fn add_contact_rejects_a_contact_with_no_email() {
        let mut dialog = EsignDialog::default();
        let contact = Contact {
            id: "share-id-123".into(),
            target: craft_cloud::ShareTarget::User("share-id-123".into()),
            name: "Mallory".into(),
            email: None,
            avatar: None,
            is_group: false,
        };
        dialog.add_contact(contact);
        assert!(dialog.recipients.is_empty(), "a contact with no email became a recipient: {:?}", dialog.recipients);
        assert_eq!(dialog.error.as_deref(), Some(dialog.labels.contact_missing_email.as_str()));
    }

    #[test]
    fn set_placements_invalidates_the_idempotency_key() {
        let mut dialog = EsignDialog { linked: true, sender: Some(ada()), ..Default::default() };
        dialog.add_email("bob@example.com".into());
        dialog.queue_send();
        let first = dialog.take_send_request().expect("first send").idempotency_key;

        dialog.set_placements(vec![placement()]);
        dialog.queue_send();
        let second = dialog.take_send_request().expect("second send").idempotency_key;

        assert_ne!(first, second, "changing placements kept the old idempotency key: {first}");
    }

    #[test]
    fn placements_do_not_survive_a_document_change() {
        let mut dialog = EsignDialog::default();
        dialog.open("doc-A".into(), "A.pdf".into(), None, ada());
        dialog.set_placements(vec![placement()]);

        dialog.open("doc-B".into(), "B.pdf".into(), None, ada());

        assert!(dialog.placements.is_empty(), "doc-B inherited doc-A's placements: {:?}", dialog.placements);
    }

    #[test]
    fn queue_send_keys_are_unique_across_dialog_instances() {
        fn send_key(doc: &str) -> String {
            let mut dialog = EsignDialog { linked: true, sender: Some(ada()), document_id: doc.into(), ..Default::default() };
            dialog.add_email("bob@example.com".into());
            dialog.queue_send();
            dialog.take_send_request().expect("send").idempotency_key
        }
        let a = send_key("doc-A");
        let b = send_key("doc-A");
        assert_ne!(a, b, "two fresh dialog instances for the same document produced the same idempotency key: {a}");
    }

    #[test]
    fn recipient_roles_never_repeat_after_a_removal() {
        let mut dialog = EsignDialog::default();
        dialog.add_email("a@example.com".into());
        dialog.add_email("b@example.com".into());
        dialog.recipients.remove(0);
        dialog.add_email("c@example.com".into());

        let roles: Vec<&str> = dialog.recipients.iter().map(|r| r.role.as_str()).collect();
        assert_eq!(roles.len(), 2, "{roles:?}");
        assert_ne!(roles[0], roles[1], "B and C were both assigned the same role after A was removed: {roles:?}");
    }

    #[test]
    fn still_open_requires_both_the_window_and_the_dialog_to_agree() {
        // A Close button inside `ui` already set `self.open` to false; the
        // window's own close control was not used.
        assert!(!still_open(true, false), "an internal Close click was undone by the window's unchanged open binding");
        // The title-bar close control was used; nothing inside set it false.
        assert!(!still_open(false, true));
        // Neither closed it.
        assert!(still_open(true, true));
    }

    fn envelope_stub() -> Envelope {
        Envelope {
            id: "env-1".into(),
            title: "Contract".into(),
            message: None,
            sender: ada(),
            document_id: None,
            document_etag: None,
            content_sha256: String::new(),
            recipients: Vec::new(),
            placements: Vec::new(),
            status: EnvelopeStatus::Sent,
            expires: None,
            created: None,
            completed_at: None,
        }
    }

    #[test]
    fn refresh_status_caption_is_localized() {
        let labels = EsignLabels { refresh: "Actualizar estado".into(), ..EsignLabels::default() };
        let mut dialog =
            EsignDialog { capabilities: Some(vec![EsignCapability::Envelopes]), labels, envelope: Some(envelope_stub()), ..Default::default() };
        let ctx = Context::default();
        let mut full = ctx.run_ui(egui::RawInput::default(), |ui| dialog.ui(ui));
        full.textures_delta.clear();
        let shown = texts(&full.shapes);
        assert!(shown.iter().any(|t| t == "Actualizar estado"), "refresh caption is not localized: {shown:?}");
        assert!(!shown.iter().any(|t| t == "Refresh status"), "hardcoded English caption still shown: {shown:?}");
    }
}
