#![cfg(all(target_arch = "wasm32", feature = "browser"))]
use craft_platform::browser::{DownloadOptions, PreparedDownload, download_bytes};
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(module = "/tests/browser_harness.js")]
extern "C" {
    fn setup(mode: &str);
    fn restore();
    fn created_count() -> u32;
    fn revoked_count() -> u32;
    fn clicked_count() -> u32;
    fn anchor_count() -> u32;
    fn last_name() -> String;
    fn last_mime() -> String;
    fn timer_before_click() -> bool;
    fn wait_for_cleanup() -> js_sys::Promise;
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

#[wasm_bindgen_test]
fn abandoning_a_prepared_download_releases_its_url() -> Result<(), JsValue> {
    setup("");
    let _restore = Restore;
    let prepared = PreparedDownload::bytes("export.txt", "text/plain", b"content")?;
    assert_eq!(created_count(), 1);
    drop(prepared);
    assert_eq!(revoked_count(), 1);
    assert_eq!(clicked_count(), 0);
    assert_eq!(anchor_count(), 0);
    Ok(())
}

#[wasm_bindgen_test]
fn failed_attachment_and_timer_do_not_click_or_leak() {
    for mode in ["attach-error", "timer-error"] {
        setup(mode);
        let _restore = Restore;
        assert!(download_bytes("export.txt", "text/plain", b"content", Default::default()).is_err());
        assert_eq!(created_count(), 1);
        assert_eq!(revoked_count(), 1);
        assert_eq!(clicked_count(), 0);
        assert_eq!(anchor_count(), 0);
    }
}

#[wasm_bindgen_test]
async fn successful_download_releases_its_url_after_the_click() -> Result<(), JsValue> {
    setup("");
    let _restore = Restore;
    download_bytes("export.txt", "text/plain", b"content", DownloadOptions { revoke_after_ms: 5 })?;
    assert_eq!(clicked_count(), 1);
    assert_eq!(last_name(), "export.txt");
    assert_eq!(last_mime(), "text/plain");
    assert!(timer_before_click());
    assert_eq!(revoked_count(), 0);
    assert_eq!(anchor_count(), 0);
    wasm_bindgen_futures::JsFuture::from(wait_for_cleanup()).await?;
    assert_eq!(revoked_count(), 1);
    Ok(())
}

#[wasm_bindgen_test]
fn invalid_lifetime_cleans_a_prepared_download() {
    setup("");
    let _restore = Restore;
    assert!(download_bytes("export.txt", "text/plain", b"content", DownloadOptions { revoke_after_ms: 0 }).is_err());
    assert_eq!(revoked_count(), 1);
    assert_eq!(clicked_count(), 0);
}

#[wasm_bindgen_test]
fn failed_setup_does_not_allocate_a_url_or_click() {
    for mode in ["element-error", "style-error"] {
        setup(mode);
        let _restore = Restore;
        assert!(download_bytes("export.txt", "text/plain", b"content", Default::default()).is_err());
        assert_eq!(created_count(), 0);
        assert_eq!(revoked_count(), 0);
        assert_eq!(clicked_count(), 0);
        assert_eq!(anchor_count(), 0);
    }
}

#[wasm_bindgen_test]
fn empty_name_is_rejected_before_allocating_resources() {
    setup("");
    let _restore = Restore;
    assert!(download_bytes("  ", "text/plain", b"content", Default::default()).is_err());
    assert_eq!(created_count(), 0);
    assert_eq!(clicked_count(), 0);
}
