//! The Nextcloud client: listing and metadata over WebDAV.

use std::fmt;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
const MAX_PREVIEW: u64 = 32 << 20;
const MAX_RANGE: u64 = 64 << 20;
const CHUNK: usize = 64 << 10;

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

    /// The server's thumbnail of `path`, at most `max_edge` pixels on a side; `None` if it has none.
    pub fn preview(&self, path: &RemotePath, max_edge: u32) -> Result<Option<Vec<u8>>, Error> {
        let file = dav_suffix(path);
        self.dav_url(path)?; // account check
        let url = format!("{}/index.php/core/preview.png?file={file}&x={max_edge}&y={max_edge}&a=1", self.http.base());
        let res = match self.http.request("GET", &url, &[], None, Kind::Quick) {
            Err(Error::NotFound) => return Ok(None),
            res => res?,
        };
        match res.status().as_u16() {
            200 => read_capped(res.into_body(), MAX_PREVIEW).map(Some),
            status => Err(Error::Malformed(format!("unexpected status {status}"))),
        }
    }

    /// The bytes of `path` in `range`, at most 64 MiB; shorter (or empty) when the file ends first.
    pub fn read_range(&self, path: &RemotePath, range: Range<u64>) -> Result<Vec<u8>, Error> {
        let Range { start, end } = range;
        let Some(len) = end.checked_sub(start).filter(|len| *len > 0) else { return Ok(Vec::new()) };
        if len > MAX_RANGE {
            return Err(Error::TooLarge);
        }
        let url = self.dav_url(path)?;
        let header = format!("bytes={start}-{}", end - 1);
        let res = self.http.request("GET", &url, &[("Range", &header)], None, Kind::Range)?;
        match res.status().as_u16() {
            206 => {
                let declared = res.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(parse_content_range);
                let (a, b, total) = declared.ok_or_else(|| Error::Malformed("bad Content-Range".into()))?;
                let want = b.checked_sub(a).and_then(|n| n.checked_add(1)).filter(|_| a == start && total.min(end).checked_sub(1) == Some(b));
                let Some(want) = want else { return Err(Error::Malformed("Content-Range doesn't match the request".into())) };
                let mut out = Vec::new();
                // Never read past what the header declared, whatever the body holds.
                io::copy(&mut res.into_body().into_reader().take(want), &mut out).map_err(body_error)?;
                if out.len() as u64 != want {
                    return Err(Error::Unreachable("body ended early".into()));
                }
                Ok(out)
            }
            200 => {
                // The server ignored Range. Skipping a start beyond 64 MiB would mean downloading it all: give up.
                if start > MAX_RANGE {
                    return Err(Error::Malformed("server ignored Range".into()));
                }
                let mut body = res.into_body().into_reader().take(end);
                io::copy(&mut (&mut body).take(start), &mut io::sink()).map_err(body_error)?;
                let mut out = Vec::new();
                body.read_to_end(&mut out).map_err(body_error)?;
                Ok(out)
            }
            416 => Ok(Vec::new()),
            status => Err(Error::Malformed(format!("unexpected status {status}"))),
        }
    }

    /// Streams `path` into `writer` and returns the bytes written. `progress` gets (done, total if known) after
    /// every chunk; `cancel` is checked before each.
    pub fn download_to(
        &self,
        path: &RemotePath,
        writer: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, Error> {
        let url = self.dav_url(path)?;
        let res = self.http.request("GET", &url, &[], None, Kind::Download)?;
        if res.status().as_u16() != 200 {
            return Err(Error::Malformed(format!("unexpected status {}", res.status().as_u16())));
        }
        let mut body = res.into_body();
        let total = body.content_length();
        let mut reader = body.as_reader();
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(body_error)?;
            if n == 0 {
                break;
            }
            writer.write_all(buf.get(..n).unwrap_or_default()).map_err(Error::Io)?;
            done = done.saturating_add(n as u64);
            progress(done, total);
        }
        if total.is_some_and(|total| total != done) {
            return Err(Error::Unreachable("download ended early".into()));
        }
        Ok(done)
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

/// `bytes a-b/total` as (a, b, total).
fn parse_content_range(v: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = v.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?, total.parse().ok()?))
}

/// A failure reading a response body is the network's.
fn body_error(e: io::Error) -> Error {
    Error::Unreachable(e.to_string())
}

/// The decoded path part of `url`, without a trailing `/`.
fn base_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("", |i| rest.get(i..).unwrap_or_default());
    let path = percent_decode_str(path).decode_utf8_lossy().into_owned();
    path.strip_suffix('/').map_or(path.clone(), str::to_string)
}
