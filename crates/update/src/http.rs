//! HTTPS requests: small JSON/text answers, redirect probes, and verified downloads.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::protocol::{CANCELLED, PAUSED};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Largest JSON or text answer we read.
const MAX_SMALL: u64 = 4 << 20;
/// Largest GitHub API answer we read: 100 releases of an app with 49 assets are about 10 MB.
const MAX_API: u64 = 16 << 20;

/// What a GET of a small resource produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    #[allow(missing_docs)]
    Body {
        /// The response body.
        text: String,
        /// The response's `ETag`, to send back as `If-None-Match` on the next request.
        etag: Option<String>,
    },
    /// `If-None-Match` matched: the saved copy is current.
    NotModified,
    #[allow(missing_docs)]
    NotFound,
    /// GitHub's API refused: out of quota until `reset` (Unix seconds), if it said.
    RateLimited {
        /// Unix seconds at which the quota resets, when the response said.
        reset: Option<i64>,
    },
}

/// Blocking HTTP client for release lookups and downloads.
///
/// Small requests, the release feed, redirect probing and file downloads use separate agents with
/// their own timeouts. Cloning is cheap and shares nothing mutable.
#[derive(Clone)]
pub struct Http {
    small: ureq::Agent,
    /// ArtCraft's release feed: a short timeout, so a slow server never holds up a check.
    feed: ureq::Agent,
    no_redirect: ureq::Agent,
    download: ureq::Agent,
    direct: ureq::Agent,
    direct_no_redirect: ureq::Agent,
    direct_download: ureq::Agent,
    token: Option<String>,
}

/// Why ArtCraft's release feed couldn't be read.
#[derive(Debug)]
pub struct FeedError {
    /// Message for the log, naming the host and the cause.
    pub why: String,
    /// The server answered HTTP 429 Too Many Requests.
    pub throttled: bool,
    /// The server's `Retry-After`, in seconds.
    pub retry_after: Option<i64>,
}

/// `http://<loopback host>:<port>` and an optional path: the host must be exactly 127.0.0.1,
/// localhost or `::1` (no `user@` in front of another host), with a numeric port.
pub fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let Some((host, port)) = authority.rsplit_once(':') else { return false };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && !port.is_empty() && port.parse::<u16>().is_ok()
}

impl Http {
    /// `user_agent` names the product and its version (`PhotoCraft/0.9.0`) and nothing about the
    /// person using it.
    pub fn new(user_agent: &str) -> Result<Self, String> {
        Self::build(user_agent)
    }

    /// The agents for remote hosts, and every redirect they follow, stay on HTTPS in every build:
    /// a release download that GitHub's file host redirected to `http://` is refused, not read in
    /// plain text. The `direct` agents only ever fetch `is_loopback` links (a local mirror), which
    /// are plain HTTP.
    fn build(user_agent: &str) -> Result<Self, String> {
        // The OS checks each certificate chain itself: Windows fetches roots it hasn't stored yet
        // and trusts what antivirus HTTPS scanning installs. Reading the store ourselves missed
        // both ("invalid peer certificate: UnknownIssuer").
        let tls = || ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        // Chosen once: the proxy environment variables, else the OS proxy setting.
        let proxy = crate::proxy::pick();
        let small = ureq::Agent::config_builder()
            .tls_config(tls())
            .proxy(proxy.clone())
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let feed = ureq::Agent::config_builder()
            .tls_config(tls())
            .proxy(proxy.clone())
            .https_only(true)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let no_redirect = ureq::Agent::config_builder()
            .tls_config(tls())
            .proxy(proxy.clone())
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let download = ureq::Agent::config_builder()
            .tls_config(tls())
            .proxy(proxy.clone())
            .https_only(true)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            // ureq has no idle timeout: cap each attempt instead, and resume where it stopped.
            .timeout_recv_body(Some(ATTEMPT_BODY_TIME))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let direct = ureq::Agent::config_builder()
            .proxy(None)
            .tls_config(tls())
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let direct_no_redirect = ureq::Agent::config_builder()
            .proxy(None)
            .tls_config(tls())
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        let direct_download = ureq::Agent::config_builder()
            .proxy(None)
            .tls_config(tls())
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(ATTEMPT_BODY_TIME))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        Ok(Self { small, feed, no_redirect, download, direct, direct_no_redirect, direct_download, token: None })
    }

    /// An optional token raises GitHub's limit from 60 to 5000 requests an hour. Only ever sent
    /// to api.github.com. Blank means none.
    ///
    /// ureq's Debug logs list only an allow-list of safe headers and leave `Authorization` out,
    /// but its Trace logs are the unredacted wire, this token's `Authorization: Bearer` among it:
    /// an app that sets one should redact Bearer tokens in its logger, or keep `ureq` and
    /// `ureq_proto` at Debug or quieter.
    #[must_use]
    pub fn with_github_token(mut self, token: Option<String>) -> Self {
        self.token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        self
    }

    /// GET a GitHub API resource, revalidating with `etag` when given.
    pub fn get_api(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String> {
        let agent = if is_loopback(url) { &self.direct } else { &self.small };
        let mut req = agent.get(url).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(tag) = etag {
            req = req.header("If-None-Match", tag);
        }
        if let Some(token) = &self.token
            && url.starts_with("https://api.github.com/")
        {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        let mut resp = req.call().map_err(|e| format!("couldn't reach GitHub ({})", call_failed(&e)))?;
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        let status = resp.status().as_u16();
        match status {
            200 => {
                let etag = header("etag");
                let text = read_text_capped(&mut resp, MAX_API).map_err(|e| format!("unreadable answer from GitHub ({e})"))?;
                Ok(Fetched::Body { text, etag })
            }
            304 => Ok(Fetched::NotModified),
            404 => Ok(Fetched::NotFound),
            403 | 429 => {
                let remaining = header("x-ratelimit-remaining");
                let reset = header("x-ratelimit-reset").and_then(|r| r.trim().parse::<i64>().ok());
                let retry_after = header("retry-after").and_then(|r| r.trim().parse::<i64>().ok());
                match refused_until(status, remaining.as_deref(), reset, retry_after, crate::now()) {
                    Some(reset) => Ok(Fetched::RateLimited { reset }),
                    None => Err(format!("GitHub refused the request (HTTP {status})")),
                }
            }
            s => Err(format!("GitHub answered HTTP {s}")),
        }
    }

    /// ArtCraft's release feed, or why it couldn't be read. Never takes more than ~10 seconds.
    pub fn get_feed(&self, url: &str) -> Result<String, FeedError> {
        let failed = |why: String| FeedError { why, throttled: false, retry_after: None };
        let agent = if is_loopback(url) { &self.direct } else { &self.feed };
        let mut resp = agent
            .get(url)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| failed(format!("couldn't reach {} ({})", host(url), call_failed(&e))))?;
        // Seconds only; an HTTP date falls back to our own backoff.
        let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|r| r.trim().parse::<i64>().ok());
        match resp.status().as_u16() {
            200 => read_text_capped(&mut resp, MAX_SMALL).map_err(|e| failed(format!("unreadable answer ({e})"))),
            s => Err(FeedError { why: format!("{} answered HTTP {s}", host(url)), throttled: s == 429, retry_after }),
        }
    }

    /// GET a small text file (following redirects).
    pub fn get_text(&self, url: &str) -> Result<Option<String>, String> {
        let agent = if is_loopback(url) { &self.direct } else { &self.small };
        let mut resp = agent.get(url).call().map_err(|e| format!("couldn't reach {} ({})", host(url), call_failed(&e)))?;
        match resp.status().as_u16() {
            200 => read_text_capped(&mut resp, MAX_SMALL).map(Some).map_err(|e| format!("unreadable answer ({e})")),
            404 => Ok(None),
            s => Err(format!("{} answered HTTP {s}", host(url))),
        }
    }

    /// GET a small binary file (following redirects). `Ok(None)` for a 404. A file of more than
    /// `max` bytes is refused: by its announced length before a byte is read, and by a hard limit
    /// on the body when the length wasn't announced or was wrong.
    pub fn get_bytes(&self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        let agent = if is_loopback(url) { &self.direct } else { &self.small };
        let mut resp = agent.get(url).call().map_err(|e| format!("couldn't reach {} ({})", host(url), call_failed(&e)))?;
        match resp.status().as_u16() {
            200 => {
                let announced = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                let too_big = || format!("{} sent more than {max} bytes", host(url));
                if announced.is_some_and(|n| n > max as u64) {
                    return Err(too_big());
                }
                let body = read_bytes_capped(&mut resp, max)
                    .map_err(|e| if e.contains("larger than") { too_big() } else { format!("unreadable answer ({e})") })?;
                Ok(Some(body))
            }
            404 => Ok(None),
            s => Err(format!("{} answered HTTP {s}", host(url))),
        }
    }

    /// Where `url` redirects to, without following it.
    pub fn redirect_target(&self, url: &str) -> Result<Option<String>, String> {
        let agent = if is_loopback(url) { &self.direct_no_redirect } else { &self.no_redirect };
        let resp = agent.head(url).call().map_err(|e| format!("couldn't reach {} ({})", host(url), call_failed(&e)))?;
        match resp.status().as_u16() {
            300..=399 => Ok(resp.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string)),
            404 => Ok(None),
            s => Err(format!("{} answered HTTP {s}", host(url))),
        }
    }

    /// Downloads `url` to `dest`, checking it against the supplied SHA-256 checksum (and always
    /// against `size`, when known). The caller must provide a published checksum.
    ///
    /// Tolerant by design: an interrupted or stalled transfer resumes where it stopped (HTTP range
    /// requests) with backoff; a paused download, or one left by an earlier run, continues from
    /// its partial file; a file that fails its checksum is fetched once more from scratch before
    /// giving up. When the connection stays down, the download ends paused (`PAUSED`), never
    /// losing what it has. `stop` pauses or cancels it between reads.
    pub fn download(&self, url: &str, dest: &Path, size: u64, sha256: &str, stop: &Stop, progress: &mut dyn FnMut(Transfer)) -> Result<(), String> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("the release publishes no valid checksum for this file, so it wasn't downloaded".into());
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
        }
        // A finished copy from an earlier attempt that never got installed. Marking it as just
        // used, before checking it, keeps `clean_downloads` away from it while it's installed.
        if dest.is_file() && touch(dest) && matches_published(dest, sha256) {
            progress(Transfer::Bytes { done: size, total: size });
            return Ok(());
        }
        // The partial file is named after the release file and its size; if it still turns out
        // to belong to something else, the checksum catches it and the download starts over.
        let part = part_path(dest, size);
        let mut fresh_start_used = false;
        loop {
            if let Err(e) = self.fetch_resuming(url, &part, size, stop, progress) {
                if e == CANCELLED {
                    let _ = std::fs::remove_file(&part);
                } else if e != PAUSED {
                    log::warn!("download of {url} failed: {e}");
                }
                return Err(e);
            }
            if matches_published(&part, sha256) {
                if let Err(e) = rename_retrying(&part, dest, stop) {
                    if e == CANCELLED {
                        let _ = std::fs::remove_file(&part);
                    }
                    return Err(e);
                }
                return Ok(());
            }
            let _ = std::fs::remove_file(&part);
            if fresh_start_used {
                return Err("the download doesn't match its published checksum, so it wasn't installed".into());
            }
            // A resumed file can be spliced wrongly by a misbehaving proxy: try once from zero.
            log::warn!("checksum mismatch for {url}; downloading again from the start");
            fresh_start_used = true;
        }
    }

    /// Fetches `url` into `part`, resuming from whatever `part` already holds. Retries while the
    /// transfer keeps making progress; after several attempts in a row get nowhere, it stops and
    /// reports `PAUSED` so the user can resume later.
    fn fetch_resuming(&self, url: &str, part: &Path, size: u64, stop: &Stop, progress: &mut dyn FnMut(Transfer)) -> Result<(), String> {
        let mut fruitless = 0u32;
        loop {
            stop.check()?;
            let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
            if fruitless >= MAX_FRUITLESS_ATTEMPTS {
                log::warn!("{url}: no progress after {fruitless} attempts; pausing");
                return Err(PAUSED.into());
            }
            if fruitless > 0 {
                progress(Transfer::Reconnecting { done: have, total: size });
                // 1 s, 2 s, 4 s … 30 s, staying responsive to Pause and Cancel.
                let wait = Duration::from_secs((1u64 << (fruitless - 1).min(5)).min(30));
                let until = std::time::Instant::now() + wait;
                while std::time::Instant::now() < until {
                    stop.check()?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            if size > 0 && have == size {
                return Ok(());
            }
            if size > 0 && have > size {
                std::fs::remove_file(part).map_err(|e| format!("couldn't save the download ({e})"))?;
                continue;
            }
            match self.attempt(url, part, have, size, stop, progress) {
                Ok(()) => return Ok(()),
                Err(AttemptError::Stopped(e)) => return Err(e),
                Err(AttemptError::Fatal(e)) => return Err(e),
                Err(AttemptError::Retry { error, progressed }) => {
                    log::info!("{url}: {error}; retrying");
                    fruitless = if progressed { 1 } else { fruitless + 1 };
                }
            }
        }
    }

    /// One request: appends to `part` from byte `have`.
    fn attempt(&self, url: &str, part: &Path, have: u64, size: u64, stop: &Stop, progress: &mut dyn FnMut(Transfer)) -> Result<(), AttemptError> {
        let retry = |error: String, progressed: bool| AttemptError::Retry { error, progressed };
        // Binaries are already compressed; ask for the bytes exactly as published.
        let agent = if is_loopback(url) { &self.direct_download } else { &self.download };
        let mut req = agent.get(url).header("Accept-Encoding", "identity");
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let mut resp = match req.call() {
            Ok(r) => r,
            // Asking again gets the same answer.
            Err(ureq::Error::RequireHttpsOnly(to)) => {
                return Err(AttemptError::Fatal(format!("the download led to plain HTTP at {}, so it wasn't fetched", host(&to))));
            }
            Err(e) => return Err(retry(format!("couldn't connect: {e}"), false)),
        };
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        let (start, total) = match status {
            200 => (0, header("content-length").and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or(size)),
            206 => match header("content-range").as_deref().and_then(parse_content_range) {
                Some((start, Some(total))) => (start, total),
                Some((start, None)) => (start, size),
                None => return Err(retry("the server sent an unreadable partial response".into(), false)),
            },
            // The range is past the end: the part file is already complete (or wrong; the
            // checksum decides).
            416 if have > 0 => return Ok(()),
            404 | 410 => return Err(AttemptError::Fatal("the file is no longer on the release page".into())),
            401 | 403 => return Err(retry(format!("the server refused the download (HTTP {status})"), false)),
            s => return Err(retry(format!("HTTP {s}"), false)),
        };
        if size > 0 && total != size {
            return Err(AttemptError::Fatal(format!("the server offered {total} bytes, but the release lists {size}")));
        }
        if let Some(enc) = header("content-encoding")
            && !enc.eq_ignore_ascii_case("identity")
        {
            return Err(AttemptError::Fatal(format!("unexpected Content-Encoding '{enc}' on binary artifact download")));
        }
        let limit = if total > 0 { total } else { MAX_DOWNLOAD };
        let appends = status == 206 && start == have;
        if appends && have > limit {
            return Err(AttemptError::Fatal(format!("the download exceeds the permitted size of {limit} bytes")));
        }
        // Open the part file at the right place: a 200 restarts it, a 206 continues it.
        let mut file = match (status, start) {
            (206, s) if s == have => std::fs::OpenOptions::new().append(true).open(part),
            _ => std::fs::File::create(part),
        }
        .map_err(|e| AttemptError::Fatal(format!("couldn't save the download ({e})")))?;
        let mut done = if status == 206 && start == have { have } else { 0 };
        // We check the length and checksum ourselves; the reader's limit is only a backstop
        // against an endless body. (ureq errors once the limit is *reached*, so leave headroom.)
        let remaining = limit.saturating_sub(done);
        let mut reader = resp.body_mut().with_config().limit(remaining.saturating_add(1 << 20)).reader();
        let mut buf = vec![0u8; 256 * 1024];
        let at_start = done;
        progress(Transfer::Bytes { done, total });
        loop {
            stop.check().map_err(AttemptError::Stopped)?;
            let n = match reader.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let _ = file.sync_all();
                    return Err(retry(format!("the connection dropped: {e}"), done > at_start));
                }
            };
            if n == 0 {
                break;
            }
            let next_done = done.saturating_add(n as u64);
            if next_done > limit {
                return Err(AttemptError::Fatal(format!("the server sent more than the permitted size of {limit} bytes")));
            }
            let chunk = buf.get(..n).ok_or_else(|| AttemptError::Fatal("read past the buffer".into()))?;
            file.write_all(chunk).map_err(|e| AttemptError::Fatal(format!("couldn't save the download ({e}). Is the disk full?")))?;
            done = next_done;
            progress(Transfer::Bytes { done, total });
        }
        file.sync_all().map_err(|e| AttemptError::Fatal(format!("couldn't save the download ({e})")))?;
        if total > 0 && done < total {
            return Err(retry(format!("the connection closed early ({done} of {total} bytes)"), done > at_start));
        }
        Ok(())
    }
}

/// Attempts in a row without any new bytes before pausing (about two minutes of backoff).
const MAX_FRUITLESS_ATTEMPTS: u32 = 8;
/// The longest one request may spend receiving; the next request resumes.
const ATTEMPT_BODY_TIME: Duration = Duration::from_secs(5 * 60);
/// Backstop when the server doesn't say how big the file is.
const MAX_DOWNLOAD: u64 = 16 << 30;

/// What a download reports as it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// Bytes are arriving.
    Bytes {
        /// Bytes of the file on disk so far.
        done: u64,
        /// Size of the whole file in bytes.
        total: u64,
    },
    /// The connection dropped; waiting before trying again.
    Reconnecting {
        /// Bytes of the file on disk so far.
        done: u64,
        /// Size of the whole file in bytes.
        total: u64,
    },
}

/// Asks a running download to pause (keeping its partial file) or cancel (deleting it).
#[derive(Debug, Default)]
pub struct Stop(AtomicU8);

impl Stop {
    const RUN: u8 = 0;
    const PAUSE: u8 = 1;
    const CANCEL: u8 = 2;

    /// Asks the download to stop and keep its partial file. Has no effect once cancel was asked.
    pub fn pause(&self) {
        let _ = self.0.compare_exchange(Self::RUN, Self::PAUSE, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Asks the download to stop and delete its partial file. Overrides a pause.
    pub fn cancel(&self) {
        self.0.store(Self::CANCEL, Ordering::SeqCst);
    }

    /// `Err(PAUSED | CANCELLED)` once asked to stop.
    pub fn check(&self) -> Result<(), String> {
        match self.0.load(Ordering::SeqCst) {
            Self::PAUSE => Err(PAUSED.into()),
            Self::CANCEL => Err(CANCELLED.into()),
            _ => Ok(()),
        }
    }
}

/// Whether a file on disk is the published one: its SHA-256 matches.
fn matches_published(path: &Path, sha256: &str) -> bool {
    sha256_file(path).is_ok_and(|h| h.eq_ignore_ascii_case(sha256))
}

/// How much of a download is already on disk (a paused or interrupted one).
pub fn partial_bytes(dest: &Path, size: u64) -> Option<u64> {
    std::fs::metadata(part_path(dest, size)).ok().map(|m| m.len()).filter(|n| *n > 0)
}

/// Renames `from` to `to`, retrying for a few seconds while Windows reports the file busy: an
/// antivirus scanner often holds a just-written installer open, and the rename then fails with
/// "Access denied" (os error 5) or a sharing violation (os error 32). `stop` pauses or cancels
/// it between tries.
fn rename_retrying(from: &Path, to: &Path, stop: &Stop) -> Result<(), String> {
    let mut wait = Duration::from_millis(100);
    loop {
        stop.check()?;
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32)) && wait <= Duration::from_secs(4) => {
                log::info!("{} is busy ({e}); retrying the rename", to.display());
                std::thread::sleep(wait);
                wait = wait.saturating_mul(2);
            }
            result => return result.map_err(|e| format!("couldn't save the download ({e})")),
        }
    }
}

/// Sets a file's modification time to now.
fn touch(path: &Path) -> bool {
    std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(std::time::SystemTime::now())).is_ok()
}

/// Whether a finished download is waiting at `dest`. Judged by size alone: `download` checks it
/// against the published checksum before anything uses it.
pub fn finished(dest: &Path, size: u64) -> bool {
    std::fs::metadata(dest).is_ok_and(|m| m.is_file() && m.len() == size)
}

/// Removes leftovers from the download folder: partial files in an old naming scheme, paused
/// downloads untouched for `partial_max_age`, and finished ones (kept after a failed install)
/// untouched for `finished_max_age` (they're for releases long since replaced, or given up on).
/// Files written within the last minute are always kept, so a running download is never touched.
pub fn clean_downloads(dir: &Path, partial_max_age: Duration, finished_max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
        if age < Duration::from_secs(60) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Current partial files are `<file>.<size>.part`.
        let current_part = name
            .strip_suffix(".part")
            .and_then(|stem| stem.rsplit_once('.'))
            .is_some_and(|(_, size)| !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()));
        let legacy_part = name.ends_with(".part") && !current_part;
        let max_age = if name.ends_with(".part") { partial_max_age } else { finished_max_age };
        if legacy_part || age > max_age {
            log::info!("removing stale download {}", path.display());
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Throws away a paused download, and a finished one kept for another try at installing.
pub fn discard(dest: &Path, size: u64) {
    let _ = std::fs::remove_file(part_path(dest, size));
    let _ = std::fs::remove_file(dest);
}

enum AttemptError {
    /// Paused or cancelled.
    Stopped(String),
    /// Won't get better by trying again.
    Fatal(String),
    Retry {
        error: String,
        progressed: bool,
    },
}

/// `<dest>.<size>.part`: where an unfinished download of `dest` is kept.
fn part_path(dest: &Path, size: u64) -> std::path::PathBuf {
    let tag = size.to_string();
    let mut name = dest.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(format!(".{tag}.part"));
    dest.with_file_name(name)
}

/// `bytes 100-199/1000` → (100, Some(1000)); `bytes 100-199/*` → (100, None).
fn parse_content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    let total = if total.trim() == "*" { None } else { Some(total.trim().parse().ok()?) };
    Some((start, total))
}

/// Lowercase hex of `bytes`, two digits per byte.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// SHA-256 of a file on disk.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("couldn't read {} ({e})", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("couldn't read {} ({e})", path.display()))?;
    Ok(hex(&hasher.finalize()))
}

/// Whether a 403 or 429 from GitHub's API means "slow down", and until when (Unix seconds, when
/// it says). `None`: refused for some other reason.
///
/// GitHub sends `x-ratelimit-reset` with every answer: it's when the hourly quota refills, which
/// only matters once `x-ratelimit-remaining` is 0. A secondary limit says how long to wait in
/// `Retry-After` (seconds) and leaves the hourly quota alone; without either, GitHub's advice is
/// to wait a minute.
fn refused_until(status: u16, remaining: Option<&str>, reset: Option<i64>, retry_after: Option<i64>, now: i64) -> Option<Option<i64>> {
    let retry = retry_after.map(|secs| now.saturating_add(secs.max(0)));
    if remaining.map(str::trim) == Some("0") {
        // `None` sorts first: whichever of the two is known, and the later if both are.
        return Some(reset.max(retry));
    }
    if retry.is_some() {
        return Some(retry);
    }
    (status == 429).then_some(Some(now.saturating_add(60)))
}

/// The host of `url` for error messages, or all of it when it has no scheme.
fn host(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else { return url };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Never `user:password@`.
    authority.rsplit_once('@').map_or(authority, |(_, host)| host)
}

/// Why a request failed, for a message. A hop refused for plain HTTP names only its host:
/// ureq's own message holds the whole link, and a redirect's link can carry a signed query.
fn call_failed(e: &ureq::Error) -> String {
    match e {
        ureq::Error::RequireHttpsOnly(to) => format!("refused plain HTTP to {}", host(to)),
        e => e.to_string(),
    }
}

/// Reads a text body, rejecting one whose *decoded* size exceeds `max`.
///
/// ureq's own `limit` guards the *compressed* stream: it sits below the transparent gzip decoder,
/// so a few hundred wire bytes can still expand far past the cap. So we cap the fully decoded
/// reader with `take` instead, one byte past the cap, and refuse an oversized answer outright —
/// before it is parsed — instead of truncating it into something that merely fails to parse
/// (CWE-409). Invalid UTF-8 stays an error, as before.
fn read_text_capped(resp: &mut ureq::http::Response<ureq::Body>, max: u64) -> Result<String, String> {
    let mut reader = resp.body_mut().with_config().reader().take(max.saturating_add(1));
    let mut text = String::new();
    reader.read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() as u64 > max {
        return Err(format!("the answer is larger than {max} bytes"));
    }
    Ok(text)
}

/// Reads a binary body, rejecting one whose decoded size exceeds `max` (CWE-409).
fn read_bytes_capped(resp: &mut ureq::http::Response<ureq::Body>, max: usize) -> Result<Vec<u8>, String> {
    let mut reader = resp.body_mut().with_config().reader().take((max as u64).saturating_add(1));
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(format!("the answer is larger than {max} bytes"));
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests;
