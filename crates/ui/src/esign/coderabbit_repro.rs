//! Regression tests for the CodeRabbit findings reviewed on PR #661,
//! reproduced against ef496161 (storytold/craft-libs#661).
//!
//! Reuses `review_repro`'s `Hold` backend and helpers: a test chooses when,
//! and in what order, a held request is answered.

use craft_esign::{Envelope, EnvelopeStatus};

use super::review_repro::{envelope_for, ready, send, who};
use super::*;

/// A document's revision changing is itself a context change: a reply for
/// the bytes under the old `document_etag` must not install under the new
/// one, even though the document id never changed.
#[test]
fn stale_create_reply_for_old_revision_is_dropped_after_reopen_with_new_etag() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), Some("etag-1".into()), who("alice"));
    send(&mut dialog, b"A bytes etag1");
    dialog.open("doc-A".into(), "A.pdf".into(), Some("etag-2".into()), who("alice"));

    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);

    assert!(
        dialog.envelope.is_none(),
        "stale reply for etag-1 installed into the etag-2 opening: {:?}",
        dialog.envelope.as_ref().map(|e| e.id.clone())
    );
}

/// `take_send_request` already handed the application doc-A's draft; the
/// dialog then reopens for doc-B before the application calls
/// `submit_request`. The reply this produces must still be tagged with
/// doc-A's context, not doc-B's.
#[test]
fn create_reply_for_a_request_taken_before_reopening_does_not_install_in_the_new_document() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    dialog.add_email("bob@example.com".into());
    dialog.queue_send();
    let out = dialog.take_send_request().expect("send produced a request");

    // The dialog moves on before the application gets around to submitting
    // what it already took.
    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));
    dialog.submit_request(Request::Create { draft: Box::new(out.draft), document: b"A bytes".to_vec(), idempotency_key: out.idempotency_key, send: true });

    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);

    assert!(
        dialog.envelope.is_none(),
        "doc-A's envelope, submitted after reopening doc-B, installed into doc-B's dialog: {:?}",
        dialog.envelope.as_ref().map(|e| e.id.clone())
    );
}

/// An untaken `SendRequest` is specific to the context that queued it; it
/// must not survive into a reopened dialog for another document.
#[test]
fn an_untaken_send_request_does_not_survive_reopening_for_another_document() {
    let (mut dialog, _hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    dialog.add_email("bob@example.com".into());
    dialog.queue_send();

    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));

    assert!(dialog.take_send_request().is_none(), "doc-B's dialog still held doc-A's untaken send request");
}

/// `note_create_result` is the synchronous twin of `submit_request`: an app
/// that handles create itself, rather than through the backend channel,
/// reports the outcome here. It must drop a result for a context the dialog
/// has already left, exactly as a channel reply would.
#[test]
fn note_create_result_for_a_previous_document_does_not_install_in_the_reopened_dialog() {
    let (mut dialog, _hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    dialog.add_email("bob@example.com".into());
    dialog.queue_send();
    let _out = dialog.take_send_request().expect("send produced a request");

    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));
    dialog.note_create_result(Ok(envelope_for("doc-A", "alice")));

    assert!(
        dialog.envelope.is_none(),
        "doc-A's result, reported after reopening doc-B, installed into doc-B's dialog: {:?}",
        dialog.envelope.as_ref().map(|e| e.id.clone())
    );
}

/// `Replier::dup` is the only way a backend can answer one sequence twice.
/// The dialog answers the first reply normally, but a second reply for the
/// same (already resolved) sequence must drop unconditionally, not reach
/// the match arms with no context left to check against.
#[test]
fn a_duplicated_answer_for_an_already_resolved_sequence_is_dropped() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");

    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    let dup = create.dup();
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);
    assert_eq!(dialog.envelope.as_ref().map(|e| e.id.clone()), Some("env-doc-A".to_owned()), "the first answer should have installed normally");

    // The dialog moves on; a backend still holding `dup` from the first
    // request answers it again.
    dialog.open("doc-B".into(), "B.pdf".into(), None, who("alice"));
    dup.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(2.0);

    let shown = dialog.envelope.as_ref().and_then(|e| e.document_id.clone());
    assert!(
        shown.is_none() || shown.as_deref() == Some("doc-B"),
        "a duplicate answer for the first request reinstalled doc-A's envelope into doc-B's dialog: {shown:?}"
    );
}

/// `busy` belongs to whichever operation is still pending. A lookup that
/// fails while a create is genuinely in flight must not report the create
/// as finished.
#[test]
fn a_failed_lookup_does_not_clear_busy_for_a_pending_create() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    assert!(dialog.busy, "queue_send should have left the dialog busy");

    dialog.query = "ca".into();
    dialog.query_changed_at = 0.0;
    dialog.maybe_lookup(1.0);
    let (_, lookup) = hold.take(|r| matches!(r, Request::Lookup { .. }));
    lookup.refuse("lookup failed");
    dialog.pump(2.0);

    assert!(dialog.busy, "a failed lookup cleared busy while the create for doc-A was still pending");
}

/// Two "Refresh status" clicks before either reply arrives race two
/// `Request::Status` calls. If the older one answers after the newer one
/// already installed, it must not overwrite the newer status.
#[test]
fn an_older_status_reply_does_not_overwrite_a_newer_one() {
    let (mut dialog, hold) = ready();
    dialog.open("doc-A".into(), "A.pdf".into(), None, who("alice"));
    send(&mut dialog, b"A bytes");
    let (_, create) = hold.take(|r| matches!(r, Request::Create { .. }));
    create.answer(Answer::Created(envelope_for("doc-A", "alice")));
    dialog.pump(1.0);
    let env_id = dialog.envelope.as_ref().expect("created").id.clone();

    dialog.busy = true;
    let seq1 = dialog.submit_in_context(Request::Status { envelope: env_id.clone() });
    dialog.pending_op_seq = Some(seq1);
    dialog.status_seq = seq1;
    dialog.busy = true;
    let seq2 = dialog.submit_in_context(Request::Status { envelope: env_id.clone() });
    dialog.pending_op_seq = Some(seq2);
    dialog.status_seq = seq2;

    let (_, status1) = hold.take(|r| matches!(r, Request::Status { .. }));
    let (_, status2) = hold.take(|r| matches!(r, Request::Status { .. }));

    // The newer request (seq2) answers first.
    status2.answer(Answer::Status(Envelope { status: EnvelopeStatus::Completed, ..envelope_for("doc-A", "alice") }));
    dialog.pump(2.0);
    assert_eq!(dialog.envelope.as_ref().map(|e| e.status), Some(EnvelopeStatus::Completed));

    // The older request (seq1), issued first, answers last.
    status1.answer(Answer::Status(Envelope { status: EnvelopeStatus::Sent, ..envelope_for("doc-A", "alice") }));
    dialog.pump(3.0);
    assert_eq!(dialog.envelope.as_ref().map(|e| e.status), Some(EnvelopeStatus::Completed), "an older status reply overwrote a newer one");
}
