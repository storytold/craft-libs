//! The browser save picker's refusal rule (#1246).

use craft_host::Picked;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

use crate::browser_files::rejected;

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
