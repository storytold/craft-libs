//! The browser save picker's refusal rule (#1246).

use craft_host::Picked;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

use crate::browser_files::{page_or_download, rejected};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn a_rejected_picker_downloads_once_unless_cancelled() {
    let mut calls = 0;
    let picked = rejected("AbortError", "Report.docx", || {
        calls += 1;
        Ok(())
    });
    assert_eq!(picked, Picked::Cancelled);
    assert_eq!(calls, 0, "a cancelled picker never downloads");

    for error in ["SecurityError", "NotSupportedError", "TypeError", ""] {
        let mut calls = 0;
        let picked = rejected(error, "Report.docx", || {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 1, "{error:?} downloads exactly once");
        assert_eq!(picked, Picked::Downloaded("Report.docx".into()), "{error:?}");
    }

    let mut calls = 0;
    let picked = rejected("SecurityError", "Report.docx", || {
        calls += 1;
        Err("the download was blocked".into())
    });
    assert_eq!(calls, 1);
    assert_eq!(picked, Picked::Failed("the download was blocked".into()), "a failed fallback stays a failed save");
}

/// A write goes to the embedding page whenever it answers, Save As passed on, and downloads only when no page hosts
/// the app: an app embedded in a file manager saves back into it, not into the downloads folder.
#[wasm_bindgen_test]
fn a_write_goes_to_the_embedding_page_and_downloads_only_without_one() {
    let page = |name: &str, bytes: &[u8], save_as: bool| Some(if save_as { Err(format!("{name} {}", bytes.len())) } else { Ok(()) });
    let mut downloads = 0;
    assert_eq!(
        page_or_download(Some(&page), "a.pdf", b"pdf", false, || {
            downloads += 1;
            Ok(())
        }),
        Ok(true)
    );
    assert_eq!(
        page_or_download(Some(&page), "a.pdf", b"pdf", true, || {
            downloads += 1;
            Ok(())
        }),
        Err(craft_host::FileError::Other("a.pdf 3".into())),
        "the page's refusal is the write's"
    );
    assert_eq!(downloads, 0, "a page that answers is the only save");
    let unhosted = |_: &str, _: &[u8], _: bool| None;
    assert_eq!(
        page_or_download(Some(&unhosted), "a.pdf", b"pdf", false, || {
            downloads += 1;
            Ok(())
        }),
        Ok(false)
    );
    assert_eq!(
        page_or_download(None, "a.pdf", b"pdf", false, || {
            downloads += 1;
            Ok(())
        }),
        Ok(false)
    );
    assert_eq!(downloads, 2, "without a page each write downloads once");
}
