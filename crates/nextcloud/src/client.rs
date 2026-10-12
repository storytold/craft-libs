//! The Nextcloud client: listing and metadata over WebDAV.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use percent_encoding::percent_decode_str;
use rustls::crypto::CryptoProvider;

use crate::dav::{PROPFIND_BODY, parse_multistatus};
use crate::http::{Http, Kind, read_capped};
use crate::path::{dav_suffix, encode_segment};
use crate::{Account, Error, RemotePath};

/// No bytes from the server for this long and a download is `Unreachable`.
const DEFAULT_STALL: Duration = Duration::from_secs(30);
const MAX_LISTING: u64 = 64 << 20;

/// A file or directory on the server.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Last path segment; empty for the root in [`Client::stat`].
    pub name: String,
    pub is_dir: bool,
    /// Bytes; for a directory the total of its contents, 0 if the server didn't say.
    pub size: u64,
    /// Without the surrounding quotes.
    pub etag: String,
    pub fileid: Option<u64>,
    /// Unix seconds.
    pub mtime: Option<i64>,
    pub content_type: Option<String>,
    pub has_preview: bool,
}

/// A signed-in Nextcloud account. Clones share connections.
///
/// This crate never logs the app password, but `ureq` and `ureq_proto` log raw HTTP bytes at TRACE, including the
/// `Authorization` header: cap those log targets below TRACE.
#[derive(Clone)]
pub struct Client {
    account: Account,
    http: Http,
    // Kept to rebuild `http` in `with_stall_limit`.
    password: String,
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("account", &self.account).field("password", &"<redacted>").finish_non_exhaustive()
    }
}

impl Client {
    /// A client for `account`, authenticating with `app_password`; `provider` is the TLS crypto provider.
    pub fn new(account: Account, app_password: String, provider: Arc<CryptoProvider>) -> Result<Client, Error> {
        let http = Http::new(&account, Some(&app_password), provider.clone(), DEFAULT_STALL)?;
        Ok(Client { account, http, password: app_password, provider })
    }

    /// How long the server may send nothing before a request fails as `Unreachable` (default 30 s).
    pub fn with_stall_limit(mut self, limit: Duration) -> Result<Client, Error> {
        self.http = Http::new(&self.account, Some(&self.password), self.provider.clone(), limit)?;
        Ok(self)
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    /// The direct children of `dir`, in server order. `Malformed` when the reply has no entry for `dir` itself (a proxy
    /// that rewrites paths), rather than an empty listing.
    pub fn list(&self, dir: &RemotePath) -> Result<Vec<Entry>, Error> {
        let mut url = self.dav_url(dir)?;
        if !url.ends_with('/') {
            url.push('/');
        }
        let entries = self.propfind(&url, "1")?;
        let base = base_path(&url);
        // A Depth 1 reply always has the folder's own entry; without it (a proxy rewriting paths) every child was
        // dropped too, and an empty listing would be a lie.
        if !entries.iter().any(|(path, _)| *path == base) {
            return Err(Error::Malformed("the server's paths don't match the requested folder".into()));
        }
        Ok(entries.into_iter().filter(|(path, _)| *path != base).map(|(_, e)| e).collect())
    }

    /// Metadata of `path` itself.
    pub fn stat(&self, path: &RemotePath) -> Result<Entry, Error> {
        let url = self.dav_url(path)?;
        let base = base_path(&url);
        let found = self.propfind(&url, "0")?.into_iter().find(|(p, _)| *p == base);
        let mut entry = found.map(|(_, e)| e).ok_or_else(|| Error::Malformed("no entry for the path".into()))?;
        entry.name = path.name().unwrap_or_default().to_string();
        Ok(entry)
    }

    fn dav_url(&self, path: &RemotePath) -> Result<String, Error> {
        if path.account_key() != self.account.key() {
            return Err(Error::Malformed("path belongs to another account".into()));
        }
        Ok(format!("{}/remote.php/dav/files/{}{}", self.http.base(), encode_segment(&self.account.user), dav_suffix(path)))
    }

    fn propfind(&self, url: &str, depth: &str) -> Result<Vec<(String, Entry)>, Error> {
        let headers = [("Depth", depth), ("Content-Type", "application/xml")];
        let res = self.http.request("PROPFIND", url, &headers, Some(PROPFIND_BODY.as_bytes()), Kind::Quick)?;
        let status = res.status().as_u16();
        if status != 207 {
            return Err(Error::Malformed(format!("unexpected status {status}")));
        }
        let xml = read_capped(res.into_body(), MAX_LISTING)?;
        parse_multistatus(&xml, &base_path(url))
    }
}

/// The decoded path part of `url`, without a trailing `/`.
fn base_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("", |i| rest.get(i..).unwrap_or_default());
    let path = percent_decode_str(path).decode_utf8_lossy().into_owned();
    path.strip_suffix('/').map_or(path.clone(), str::to_string)
}
