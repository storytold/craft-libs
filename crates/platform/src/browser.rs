//! Browser downloads. Caller owns MIME/name decisions and pacing; this module owns resource cleanup.
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url, Window};

/// Object-URL lifetime for a requested download; the default is 60 seconds.
#[derive(Clone, Copy, Debug)]
pub struct DownloadOptions {
    /// Positive delay in milliseconds before cleanup; nonpositive values reject the click.
    pub revoke_after_ms: i32,
}
impl Default for DownloadOptions {
    fn default() -> Self {
        Self { revoke_after_ms: 60_000 }
    }
}

/// Prepared DOM link and object URL. Dropping without clicking cleans both up.
/// Preparation can occur before an app's download pacer yields.
pub struct PreparedDownload {
    window: Window,
    anchor: HtmlAnchorElement,
    url: Option<String>,
}
impl PreparedDownload {
    /// Prepare an owned Blob and hidden link using the caller's bytes, MIME and filename.
    /// No download is requested until [`Self::click`]; browser setup failures return an error.
    pub fn bytes(name: &str, mime: &str, bytes: &[u8]) -> Result<Self, JsValue> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let options = BlobPropertyBag::new();
        options.set_type(mime);
        Self::blob(name, &Blob::new_with_u8_array_sequence_and_options(&parts, &options)?)
    }
    /// Prepare a hidden link and owned object URL for an existing Blob.
    /// Blank filenames, missing browser DOM and setup failures return an error.
    pub fn blob(name: &str, blob: &Blob) -> Result<Self, JsValue> {
        if name.trim().is_empty() {
            return Err(JsValue::from_str("a file name is required"));
        }
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no browser window"))?;
        let document = window.document().ok_or_else(|| JsValue::from_str("no browser document"))?;
        document.body().ok_or_else(|| JsValue::from_str("the page has no body"))?;
        let anchor = document.create_element("a")?.dyn_into::<HtmlAnchorElement>()?;
        anchor.set_download(name);
        anchor.set_attribute("style", "display:none")?;
        let download = Self { window, anchor, url: Some(Url::create_object_url_with_blob(blob)?) };
        if let Some(url) = &download.url {
            download.anchor.set_href(url);
        }
        Ok(download)
    }
    /// Register cleanup before clicking: a timer failure returns an error without triggering
    /// a partial download. Success only means a click was issued; browser acceptance is unobservable.
    pub fn click(mut self, options: DownloadOptions) -> Result<(), JsValue> {
        if options.revoke_after_ms <= 0 {
            return Err(JsValue::from_str("download lifetime must be positive"));
        }
        let Some(url) = self.url.as_ref().cloned() else {
            return Err(JsValue::from_str("download has no object URL"));
        };
        let body = self.window.document().and_then(|d| d.body()).ok_or_else(|| JsValue::from_str("the page has no body"))?;
        body.append_child(&self.anchor)?;
        let revoke = Closure::once(move || {
            let _ = Url::revoke_object_url(&url);
        });
        self.window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.as_ref().unchecked_ref(), options.revoke_after_ms)?;
        revoke.forget(); // Ownership transfers only after successful timer registration.
        self.url = None;
        self.anchor.click();
        Ok(())
    }
}
impl Drop for PreparedDownload {
    fn drop(&mut self) {
        self.anchor.remove();
        if let Some(url) = self.url.take() {
            let _ = Url::revoke_object_url(&url);
        }
    }
}
/// Prepare bytes and request a download with the specified URL lifetime.
/// Success means the click was issued, not that the browser accepted or saved the file.
pub fn download_bytes(name: &str, mime: &str, bytes: &[u8], options: DownloadOptions) -> Result<(), JsValue> {
    PreparedDownload::bytes(name, mime, bytes)?.click(options)
}
/// Prepare a Blob and request a download with the specified URL lifetime.
/// Setup or scheduling failures clean up owned resources and return an error.
pub fn download_blob(name: &str, blob: &Blob, options: DownloadOptions) -> Result<(), JsValue> {
    PreparedDownload::blob(name, blob)?.click(options)
}
