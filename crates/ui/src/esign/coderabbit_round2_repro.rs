//! Regression tests for CodeRabbit's second review pass on PR #661,
//! reproduced against 1e84947c (storytold/craft-libs#661).
//!
//! Reuses `review_repro`'s `Hold` backend and helpers, the same as
//! `coderabbit_repro`.

use craft_esign::{Envelope, EnvelopeStatus};

use super::review_repro::{Hold, envelope_for, ready, send, who};
use super::*;

/// A host can drive `Request::Status` through the public `submit_request`
/// path directly, not only through `status_panel`'s own "Refresh status"
/// button. That path must register `status_seq` too, or the reply it
/// produces fails the supersession guard added for the button's path and is
/// dropped.
#[test]
fn submit_request_for_a_public_status_submission_registers_status_seq() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);
    let env_id = dialog.envelope.as_ref().expect("created").id.clone();

    dialog.submit_request(Request::Status { envelope: env_id });
    let (_, status) = hold.take(|r| matches!(r, Request::Status { .. }));
    status.answer(Answer::Status(Envelope { status: EnvelopeStatus::Completed, ..envelope_for("doc-A", "alice") }));
    dialog.pump(2.0);

    assert_eq!(
        dialog.envelope.as_ref().map(|e| e.status),
        Some(EnvelopeStatus::Completed),
        "a Status reply submitted through the public submit_request path was dropped by the status_seq guard"
    );
}

/// `available()` stops asking again once the service has reported
/// `Envelopes`, so a backend whose `linked` value changes out of band (the
/// user signs in after the dialog already probed) is never re-asked unless
/// something clears the cached capabilities. `reset_capabilities` is that
/// something, on the same backend instance.
#[test]
fn reset_capabilities_reprobes_the_same_backend_after_an_out_of_band_sign_in() {
    let hold = Hold::default();
    let mut dialog = EsignDialog::default();
    dialog.set_backend(hold.clone());
    dialog.pump(0.0);
    let (_, caps1) = hold.take(|r| matches!(r, Request::Capabilities));
    caps1.answer(Answer::Capabilities { capabilities: vec![EsignCapability::Envelopes], responsible_party: "Stub Signing".into(), linked: false });
    dialog.pump(0.1);
    assert!(dialog.available());
    assert!(!dialog.linked);

    // Available is already true, so pump's own retry condition
    // (`!available() && ...`) never fires again on its own, no matter how
    // much time passes.
    for t in 0..5 {
        dialog.pump(0.2 + f64::from(t));
    }

    // The user signs in; nothing about the backend instance changes, so
    // only an explicit reset asks again.
    dialog.reset_capabilities();
    dialog.pump(1.0);
    let (_, caps2) = hold.take(|r| matches!(r, Request::Capabilities));
    caps2.answer(Answer::Capabilities { capabilities: vec![EsignCapability::Envelopes], responsible_party: "Stub Signing".into(), linked: true });
    dialog.pump(1.1);

    assert!(dialog.linked, "reset_capabilities did not pick up the backend's new linked state");
}

/// The captured send context belongs only to the create submission that
/// answers a taken [`SendRequest`]. A status refresh submitted for the
/// current document while another document's send is still outstanding must
/// carry the current context, or `pump` drops its reply as belonging to the
/// abandoned send.
#[test]
fn status_for_the_current_document_is_not_consumed_by_an_outstanding_send_from_another() {
    let (mut dialog, hold) = ready();

    // Document A's send is taken by the app but never submitted: its captured
    // context stays pending.
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");

    // The dialog moves on to document B, which already has an envelope the
    // host wants a status for.
    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));
    dialog.envelope = Some(envelope_for("doc-B", "alice"));
    let env_id = dialog.envelope.as_ref().map(|e| e.id.clone()).unwrap_or_default();

    dialog.submit_request(Request::Status { envelope: env_id });
    let (_, status) = hold.take(|r| matches!(r, Request::Status { .. }));
    status.answer(Answer::Status(Envelope { status: EnvelopeStatus::Completed, ..envelope_for("doc-B", "alice") }));
    dialog.pump(1.0);

    assert_eq!(
        dialog.envelope.as_ref().map(|e| e.status),
        Some(EnvelopeStatus::Completed),
        "B's status reply was dropped because the status request consumed A's outstanding send context"
    );
}
