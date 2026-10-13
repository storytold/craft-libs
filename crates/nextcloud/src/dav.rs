//! Files over WebDAV.
//!
//! Nextcloud serves each user's files at `/remote.php/dav/files/<user>/`. Listing is a `PROPFIND`
//! with `Depth: 1`, which answers `207 Multi-Status`; reading and writing are plain `GET` and
//! `PUT`. Written from [RFC 4918] and Nextcloud's published WebDAV documentation.
//!
//! [RFC 4918]: https://www.rfc-editor.org/rfc/rfc4918

use crate::error::{Error, Result};
use crate::http::{Http, Method};
use crate::{Account, encode_path, send, xml};

/// The properties worth asking for. Keeping the set small keeps the answer small, which matters on
/// a folder with thousands of entries.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
  <d:prop>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getcontenttype/>
    <d:getlastmodified/>
    <d:getetag/>
    <oc:fileid/>
    <oc:permissions/>
  </d:prop>
</d:propfind>"#;

/// One file or folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Last path segment, percent-decoded.
    pub name: String,
    /// Path relative to the user's files root, with a leading slash and no trailing slash.
    pub path: String,
    pub is_dir: bool,
    /// Zero for a folder, which WebDAV does not give a content length.
    pub size: u64,
    pub content_type: String,
    /// As the server wrote it, which is an HTTP date. Left as text because this crate has no date
    /// dependency and every caller formats it differently.
    pub modified: String,
    /// Quoted by the protocol. The quotes are stripped here.
    pub etag: String,
    /// Nextcloud's own stable file id, which survives a rename. Empty when the server omits it.
    pub file_id: String,
}

/// A held WebDAV lock, as returned by [`Files::lock`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// The token to send back as `If` or to `UNLOCK` with.
    pub token: String,
    pub path: String,
}

/// File operations against one account.
pub struct Files<'a> {
    account: &'a Account,
}

impl<'a> Files<'a> {
    pub fn new(account: &'a Account) -> Files<'a> {
        Files { account }
    }

    /// Absolute URL for a path under the user's files root.
    fn url(&self, path: &str) -> Result<String> {
        Ok(format!("{}{}", self.account.files_root(), encode_path(path)?))
    }

    /// List the direct children of a folder.
    ///
    /// The folder itself is returned by the protocol first and is dropped here, so the result is
    /// children only. A folder with no children gives an empty list.
    pub fn list<H: Http>(&self, http: &H, path: &str) -> Result<Vec<Entry>> {
        let url = self.url(path)?;
        let req = self
            .account
            .request(Method::Propfind, &url)
            .header("depth", "1")
            .header("content-type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = send(http, req)?;

        let root_href = href_path(&url);
        let mut out = Vec::new();
        for group in xml::groups(&resp.body, "response")? {
            let Some(href) = group.get("href") else { continue };
            // The collection itself. Compare on the decoded path so a differently encoded but
            // equal href still matches.
            if same_path(href, &root_href) {
                continue;
            }
            out.push(entry_from(&group, href, &self.account.files_root()));
        }
        Ok(out)
    }

    /// Metadata for one file or folder, without its children.
    ///
    /// Depth 0, which is what distinguishes this from [`Files::list`]. That
    /// one asks for children and drops the entry it was pointed at, so asking
    /// it about a file returns nothing at all.
    pub fn stat<H: Http>(&self, http: &H, path: &str) -> Result<Entry> {
        let url = self.url(path)?;
        let req = self
            .account
            .request(Method::Propfind, &url)
            .header("depth", "0")
            .header("content-type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = send(http, req)?;

        let group = xml::groups(&resp.body, "response")?
            .into_iter()
            .next()
            .ok_or_else(|| Error::malformed("propfind", "no response element"))?;
        let href = group.get("href").ok_or_else(|| Error::malformed("propfind", "no href"))?.to_owned();
        Ok(entry_from(&group, &href, &self.account.files_root()))
    }

    /// Read a file.
    pub fn get<H: Http>(&self, http: &H, path: &str) -> Result<Vec<u8>> {
        let req = self.account.request(Method::Get, self.url(path)?);
        Ok(send(http, req)?.body)
    }

    /// Write a file, creating or replacing it.
    ///
    /// `if_match` carries an etag from a previous read. Passing one turns the write into a
    /// compare-and-set, which is how a save avoids clobbering someone else's change. Passing
    /// `None` overwrites unconditionally.
    pub fn put<H: Http>(&self, http: &H, path: &str, bytes: Vec<u8>, if_match: Option<&str>) -> Result<()> {
        let mut req = self.account.request(Method::Put, self.url(path)?).header("content-type", "application/octet-stream").body(bytes);
        if let Some(etag) = if_match {
            req = req.header("if-match", quoted(etag));
        }
        send(http, req)?;
        Ok(())
    }

    /// Create a folder. Fails when the parent is missing, which WebDAV reports as 409.
    pub fn mkdir<H: Http>(&self, http: &H, path: &str) -> Result<()> {
        let req = self.account.request(Method::Mkcol, self.url(path)?);
        send(http, req)?;
        Ok(())
    }

    /// Delete a file or folder.
    pub fn delete<H: Http>(&self, http: &H, path: &str) -> Result<()> {
        let req = self.account.request(Method::Delete, self.url(path)?);
        send(http, req)?;
        Ok(())
    }

    /// Whether a path exists, by asking for its own properties only.
    ///
    /// A 404 is an answer here, not a failure, so the status is read directly instead of going
    /// through [`send`].
    pub fn exists<H: Http>(&self, http: &H, path: &str) -> Result<bool> {
        let req = self
            .account
            .request(Method::Propfind, self.url(path)?)
            .header("depth", "0")
            .header("content-type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes().to_vec());
        let resp = http.send(req).map_err(|e| Error::Transport(e.to_string()))?;
        match resp.status {
            404 => Ok(false),
            s if (200..300).contains(&s) => Ok(true),
            _ => Err(Error::status(Method::Propfind, path, &resp)),
        }
    }

    /// Take an exclusive write lock, which is the check-out half of check-out and check-in.
    ///
    /// `timeout_secs` is a request, and the server may shorten it. The returned token must be sent
    /// to [`Files::unlock`] to release the lock early.
    pub fn lock<H: Http>(&self, http: &H, path: &str, timeout_secs: u32) -> Result<Lock> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<d:lockinfo xmlns:d="DAV:">
  <d:lockscope><d:exclusive/></d:lockscope>
  <d:locktype><d:write/></d:locktype>
  <d:owner><d:href>{}</d:href></d:owner>
</d:lockinfo>"#,
            xml::escape(self.account.user())
        );
        let req = self
            .account
            .request(Method::Lock, self.url(path)?)
            .header("timeout", format!("Second-{timeout_secs}"))
            .header("content-type", "application/xml; charset=utf-8")
            .body(body.into_bytes());
        let resp = send(http, req)?;

        // The token arrives in the Lock-Token header, and also in the body as locktoken/href.
        let token = resp
            .get_header("lock-token")
            .map(|t| t.trim_matches(|c| c == '<' || c == '>').to_owned())
            .or_else(|| xml::one_group(&resp.body, "activelock").ok().and_then(|g| g.get("href").map(str::to_owned)))
            .ok_or_else(|| Error::malformed("lock response", "no lock token in header or body"))?;
        Ok(Lock { token, path: path.to_owned() })
    }

    /// Release a lock taken by [`Files::lock`].
    pub fn unlock<H: Http>(&self, http: &H, lock: &Lock) -> Result<()> {
        let req = self.account.request(Method::Unlock, self.url(&lock.path)?).header("lock-token", format!("<{}>", lock.token));
        send(http, req)?;
        Ok(())
    }
}

/// Build an [`Entry`] from one `response` group.
///
/// Every property is optional. A server that omits one gives a default, which is what keeps this
/// lenient. `files_root` is used to turn the absolute href into a path relative to the user's root.
fn entry_from(group: &xml::Group, href: &str, files_root: &str) -> Entry {
    let decoded = decode(href);
    let root_path = href_path(files_root);
    let relative = decoded.strip_prefix(&decode(&root_path)).unwrap_or(&decoded).trim_end_matches('/').to_owned();
    let path = if relative.starts_with('/') || relative.is_empty() { relative.clone() } else { format!("/{relative}") };
    let name = path.rsplit('/').next().unwrap_or_default().to_owned();

    Entry {
        name,
        path,
        // WebDAV marks a collection with an empty <d:collection/> inside <d:resourcetype/>.
        is_dir: group.has("collection"),
        size: group.get("getcontentlength").and_then(|s| s.parse().ok()).unwrap_or(0),
        content_type: group.get("getcontenttype").unwrap_or_default().to_owned(),
        modified: group.get("getlastmodified").unwrap_or_default().to_owned(),
        etag: group.get("getetag").map(unquote).unwrap_or_default(),
        file_id: group.get("fileid").unwrap_or_default().to_owned(),
    }
}

/// The path part of an absolute URL, or the input when it has no scheme.
fn href_path(url: &str) -> String {
    match url.find("://") {
        Some(i) => match url[i + 3..].find('/') {
            Some(j) => url[i + 3 + j..].to_owned(),
            None => "/".to_owned(),
        },
        None => url.to_owned(),
    }
}

/// Compare two hrefs by their decoded path, ignoring a trailing slash.
fn same_path(a: &str, b: &str) -> bool {
    decode(a).trim_end_matches('/') == decode(b).trim_end_matches('/')
}

/// Percent-decode. Invalid escapes are kept as written, which is the lenient choice: a server that
/// emits a bare `%` should not cost us the whole listing.
pub(crate) fn decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // A let chain, so an invalid escape falls through to the byte-copy below.
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Strip the quotes and any weak marker from an etag.
fn unquote(etag: &str) -> String {
    etag.trim().trim_start_matches("W/").trim_matches('"').to_owned()
}

/// Put quotes back on for an `If-Match`, unless the caller already did.
fn quoted(etag: &str) -> String {
    if etag.starts_with('"') || etag.starts_with("W/") { etag.to_owned() } else { format!("\"{etag}\"") }
}
