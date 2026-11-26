//! Regression tests for the three gate items skylartaylor raised on PR #661,
//! reproduced against 0a937ef (storytold/craft-libs#661):
//! <https://gist.github.com/skylartaylor/c1108ecf20d63c5e3183947408ddab7a>.
//!
//! `Hold` is a backend that keeps every request and its `Replier` so a test
//! chooses when, and in what order, the service answers.

use std::sync::{Arc, Mutex};

use craft_cloud::Identity;
use craft_esign::{Envelope, EnvelopeStatus, EsignCapability};

use super::*;

/// Reused by `coderabbit_repro`, the regression tests for the CodeRabbit
/// findings reviewed on the same pull request.
#[derive(Clone, Default)]
pub(super) struct Hold(Arc<Mutex<Vec<(Request, Replier)>>>);

impl EsignBackend for Hold {
    fn submit(&self, request: Request, reply: Replier) {
        self.0.lock().unwrap().push((request, reply));
    }
}

impl Hold {
    /// Takes the oldest held request matching `pick`.
    pub(super) fn take(&self, pick: impl Fn(&Request) -> bool) -> (Request, Replier) {
        let mut held = self.0.lock().unwrap();
        let i = held.iter().position(|(r, _)| pick(r)).expect("request was submitted");
        held.remove(i)
    }
}

pub(super) fn who(id: &str) -> Identity {
    Identity { id: id.into(), display: id.into(), email: Some(format!("{id}@example.com")), avatar: None }
}

pub(super) fn envelope_for(doc: &str, sender: &str) -> Envelope {
    Envelope {
        id: format!("env-{doc}"),
        title: format!("Contract {doc}"),
        message: None,
        sender: who(sender),
        document_id: Some(doc.into()),
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

/// A dialog with a held backend whose capabilities probe has been answered.
pub(super) fn ready() -> (EsignDialog, Hold) {
    let hold = Hold::default();
    let mut dialog = EsignDialog::default();
    dialog.set_backend(hold.clone());
    dialog.pump(0.0); // submits Capabilities
    let (_, caps) = hold.take(|r| matches!(r, Request::Capabilities));
    caps.answer(Answer::Capabilities {
        capabilities: vec![EsignCapability::Envelopes, EsignCapability::Void],
        responsible_party: "Stub Signing".into(),
        linked: true,
    });
    dialog.pump(0.1);
    assert!(dialog.available());
    (dialog, hold)
}

/// The app's Send path: queue_send -> take_send_request -> submit Create.
pub(super) fn send(dialog: &mut EsignDialog, doc_bytes: &[u8]) {
    dialog.add_email("bob@example.com".into());
    dialog.queue_send();
    let out = dialog.take_send_request().expect("Send produced a request");
    dialog.submit_request(Request::Create {
        draft: Box::new(out.draft),
        document: doc_bytes.to_vec(),
        idempotency_key: out.idempotency_key,
        send: true,
    });
}

/// Gate item 1 (document variant): the dialog is reopened for document B
/// while document A's create is in flight. A's late reply carries document
/// A's context and must not be installed as document B's envelope.
#[test]
fn review_h5_reply_for_previous_document_is_installed_after_reopen() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));

    let (_, create_a) = hold.take(|r| matches!(r, Request::Create { .. }));
    create_a.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);

    let shown = dialog.envelope.as_ref().and_then(|e| e.document_id.clone());
    assert!(
        shown.is_none() || shown.as_deref() == Some("doc-B"),
        "dialog is open for doc-B but shows the envelope for {shown:?} (status panel: Void/Remind act on it); busy={}",
        dialog.busy
    );
}

/// Gate item 1 (account variant): reopened for the same document under
/// another signed-in account. Alice's envelope must never render for
/// mallory.
#[test]
fn review_h5_reply_for_previous_account_is_installed_after_account_switch() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("mallory"));

    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);

    let shown = dialog.envelope.as_ref().map(|e| e.sender.id.clone());
    assert!(shown.is_none() || shown.as_deref() == Some("mallory"), "dialog now belongs to mallory but shows alice's envelope: sender={shown:?}");
}

/// Gate item 2: Send is in flight; the user types in the signer search box
/// (a lookup is submitted, which used to bump the old staleness counter).
/// The create reply then arrives for the same context and must still
/// install: typing in a search box is not a context change.
#[test]
fn review_n1_create_reply_dropped_after_a_lookup_leaves_dialog_busy() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    assert!(dialog.busy);

    dialog.query = "ca".into(); // typed after pressing Send
    dialog.query_changed_at = 1.0;
    dialog.maybe_lookup(2.0);

    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    for t in 0..10 {
        dialog.pump(3.0 + f64::from(t));
    }
    assert!(
        !dialog.busy && dialog.envelope.is_some(),
        "provider created env-doc-A but dialog busy={} envelope={:?} (reply dropped as stale)",
        dialog.busy,
        dialog.envelope.as_ref().map(|e| e.id.clone())
    );
}
