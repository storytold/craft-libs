#![cfg(feature = "read")]
#![allow(clippy::unwrap_used)]

use craft_pdf::{CancellableOpenError, OpenError, OpenOptions, open, open_slice_with_options_and_cancel, open_with_options_and_cancel, syntax};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn fixture() -> Vec<u8> {
    b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF".to_vec()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cancellation_takes_precedence_for_an_initial_request() {
    let cancel = syntax::Cancellation::new(|| true);
    let options = OpenOptions::default();
    assert!(matches!(open_with_options_and_cancel(Arc::new(vec![]), None, &options, &cancel), Err(CancellableOpenError::Cancelled)));
    assert!(matches!(open_slice_with_options_and_cancel(b"", None, &options, &cancel), Err(CancellableOpenError::Cancelled)));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cancellation_interrupts_a_reverse_xref_search_and_stays_latched() {
    let mut bytes = fixture();
    bytes.extend(std::iter::repeat_n(b' ', 4 << 20));
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let cancel = syntax::Cancellation::new(move || counter.fetch_add(1, Ordering::Relaxed) + 1 == 10);
    let result = open_with_options_and_cancel(Arc::new(bytes), None, &OpenOptions::default(), &cancel);
    assert!(matches!(result, Err(CancellableOpenError::Cancelled)));
    assert_eq!(polls.load(Ordering::Relaxed), 10);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cancellation_interrupts_borrowed_input_copying() {
    let bytes = vec![b' '; 4 << 20];
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let cancel = syntax::Cancellation::new(move || counter.fetch_add(1, Ordering::Relaxed) + 1 == 10);
    let result = open_slice_with_options_and_cancel(&bytes, None, &OpenOptions::default(), &cancel);
    assert!(matches!(result, Err(CancellableOpenError::Cancelled)));
    assert_eq!(polls.load(Ordering::Relaxed), 10);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_successful_open_releases_callback_captures_and_does_not_poll_later() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let marker = Arc::new(());
    let weak = Arc::downgrade(&marker);
    let cancel = syntax::Cancellation::new(move || {
        let _keep = &marker;
        counter.fetch_add(1, Ordering::Relaxed);
        false
    });
    let bytes = fixture();
    let normal = open(Arc::new(bytes.clone()), None).unwrap();
    let pdf = open_with_options_and_cancel(Arc::new(bytes), None, &OpenOptions::default(), &cancel).unwrap();
    let before = polls.load(Ordering::Relaxed);
    drop(cancel);
    assert!(weak.upgrade().is_none(), "the returned PDF must not retain the callback");
    assert_eq!(pdf.pages().len(), normal.pages().len());
    assert!(pdf.xref().get::<syntax::object::Dict<'_>>(syntax::object::ObjectIdentifier::new(1, 0)).is_some());
    assert_eq!(polls.load(Ordering::Relaxed), before);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn an_uncanceled_open_preserves_bounds_and_loading_errors() {
    let cancel = syntax::Cancellation::new(|| false);
    assert!(matches!(
        open_with_options_and_cancel(Arc::new(b"not a PDF".to_vec()), None, &OpenOptions::default(), &cancel),
        Err(CancellableOpenError::Open(OpenError::Load(syntax::LoadPdfError::Invalid)))
    ));
    let options = OpenOptions { max_input_bytes: 1, ..OpenOptions::default() };
    assert!(matches!(
        open_slice_with_options_and_cancel(&fixture(), None, &options, &cancel),
        Err(CancellableOpenError::Open(OpenError::InputLimit { limit: 1 }))
    ));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[allow(clippy::panic)]
fn callback_panics_become_errors_before_parsing_and_during_copying() {
    let bytes = vec![b' '; 4 << 20];
    for borrowed in [false, true] {
        for at in [1, 2, 10] {
            let polls = Arc::new(AtomicUsize::new(0));
            let counter = polls.clone();
            let cancel = syntax::Cancellation::new(move || {
                if counter.fetch_add(1, Ordering::Relaxed) + 1 == at {
                    panic!("synthetic callback panic");
                }
                false
            });
            let result = if borrowed {
                open_slice_with_options_and_cancel(&bytes, None, &OpenOptions::default(), &cancel)
            } else {
                open_with_options_and_cancel(Arc::new(bytes.clone()), None, &OpenOptions::default(), &cancel)
            };
            assert!(matches!(result, Err(CancellableOpenError::Open(OpenError::BackendPanic))));
        }
    }
}
