//! One-click model download: resumable, `.part`-suffixed while in flight
//! (a half-downloaded GGUF is never mistaken for a valid one), SHA-256
//! verified, then atomically renamed.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::Digest;

use crate::error::AiError;
use crate::registry::ModelEntry;

/// Progress + lifecycle events, fed to the UI's status line.
#[derive(Clone, Debug)]
pub enum Event {
    /// We know the total; download is starting or resuming.
    Started { total: u64, resumed_from: u64 },
    /// Bytes on disk so far. `bytes_per_sec` is a running average.
    Progress { done: u64, total: u64, bytes_per_sec: u64 },
    /// Body finished; hashing the file before we trust it.
    Verifying,
    /// Download is complete. `verified` is false when no reference digest
    /// existed anywhere (registry table and the host's ETag) — the file is
    /// kept but the UI should say "unverified" rather than pretend.
    Done { verified: bool },
}

/// A handle to abort a download: `cancel` returns false → `AiError::Cancelled`.
pub type Cancel<'a> = &'a dyn Fn() -> bool;

/// Download `entry` into `cache_dir`, calling `on` with progress events.
/// Resumes an existing `*.part` via HTTP Range. Returns the final path.
pub fn download(entry: &ModelEntry, cache_dir: &Path, on: impl Fn(&Event), cancel: Option<Cancel<'_>>) -> Result<PathBuf, AiError> {
    fs::create_dir_all(cache_dir)?;
    let final_path = cache_dir.join(entry.file);
    if final_path.is_file() {
        // Already on disk — verify once, reuse if good.
        on(&Event::Verifying);
        if let Some(expected) = expected_sha256(entry)? {
            let got = sha256_file(&final_path)?;
            if got.eq_ignore_ascii_case(&expected) {
                on(&Event::Done { verified: true });
                return Ok(final_path);
            }
            return Err(AiError::Checksum { expected, got });
        }
        on(&Event::Done { verified: false });
        return Ok(final_path);
    }

    let part_path = cache_dir.join(format!("{}.part", entry.file));
    let resumed_from = part_path.metadata().map_or(0, |m| m.len());

    let expected = expected_sha256(entry)?;

    let mut req = ureq::get(entry.url);
    if resumed_from > 0 {
        req = req.header("Range", &format!("bytes={resumed_from}-"));
    }
    let mut resp = req.call().map_err(http_err)?;

    // A 200 to a Range request means the server ignored resume — truncate.
    let honored_resume = resumed_from > 0 && resp.status() == 206;
    let total = content_length(&resp).map(|len| if honored_resume { len + resumed_from } else { len });

    on(&Event::Started { total: total.unwrap_or(0), resumed_from: if honored_resume { resumed_from } else { 0 } });

    let mut file = if honored_resume { OpenOptions::new().append(true).open(&part_path)? } else { File::create(&part_path)? };

    let mut body = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = if honored_resume { resumed_from } else { 0 };
    let started = Instant::now();
    loop {
        if cancel.is_some_and(|c| !c()) {
            return Err(AiError::Cancelled);
        }
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(AiError::Io(e)),
        };
        file.write_all(&buf[..n])?;
        done += n as u64;
        let secs = started.elapsed().as_secs_f64().max(0.001);
        on(&Event::Progress { done, total: total.unwrap_or(0), bytes_per_sec: (done.saturating_sub(resumed_from) as f64 / secs) as u64 });
    }
    file.flush()?;
    drop(file);

    // A short read means the connection dropped mid-body — keep the .part
    // for resume and report it as an HTTP failure, not a success.
    if let Some(t) = total
        && done < t
    {
        return Err(AiError::Http(format!("connection dropped at {done}/{t} bytes — retry to resume")));
    }

    on(&Event::Verifying);
    let verified = if let Some(expected) = expected {
        let got = sha256_file(&part_path)?;
        if !got.eq_ignore_ascii_case(&expected) {
            // A corrupt download is deleted — it can never silently pass.
            let _ = fs::remove_file(&part_path);
            return Err(AiError::Checksum { expected, got });
        }
        true
    } else {
        false
    };

    fs::rename(&part_path, &final_path)?;
    on(&Event::Done { verified });
    Ok(final_path)
}

/// The digest we hold this file to: our registry hash if pinned, else the
/// ETag of the resolved URL (Hugging Face serves the LFS sha256 there).
fn expected_sha256(entry: &ModelEntry) -> Result<Option<String>, AiError> {
    if let Some(s) = entry.sha256 {
        return Ok(Some(s.to_string()));
    }
    match ureq::head(entry.url).call() {
        Ok(resp) => Ok(etag(&resp)),
        // HEAD unsupported/unreachable → proceed without verification.
        Err(_) => Ok(None),
    }
}

fn etag(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    // Prefer the redirect target's etag (the LFS object), fall back to
    // the response's own. Quotes stripped; weak etags ("W/…") ignored.
    for name in ["x-linked-etag", "etag"] {
        if let Some(v) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            let v = v.trim().trim_matches('"');
            if !v.starts_with("W/") && v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn content_length(resp: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        // A redirect's own length is meaningless; only trust it after
        // ureq followed redirects (it does — so the final body length).
        .filter(|&n: &u64| n > 0)
}

fn sha256_file(path: &Path) -> Result<String, AiError> {
    let mut file = File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn http_err(e: ureq::Error) -> AiError {
    match e {
        ureq::Error::StatusCode(code) => AiError::Rejected(format!("HTTP {code}")),
        other => AiError::Http(other.to_string()),
    }
}

/// Where a model would land once downloaded.
pub fn model_path(entry: &ModelEntry, cache_dir: &Path) -> PathBuf {
    cache_dir.join(entry.file)
}

/// Is `entry` already fully downloaded?
pub fn is_downloaded(entry: &ModelEntry, cache_dir: &Path) -> bool {
    cache_dir.join(entry.file).is_file()
}

/// `tokenizer.json` beside the GGUF: `<file>.tokenizer.json`.
pub fn tokenizer_path(entry: &ModelEntry, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{}.tokenizer.json", entry.file))
}

/// The companion tokenizer — small (a few MB), so no resume loop, but the
/// same `.part`-then-rename rule applies.
pub fn download_tokenizer(entry: &ModelEntry, cache_dir: &Path) -> Result<PathBuf, AiError> {
    let dest = tokenizer_path(entry, cache_dir);
    if dest.is_file() {
        return Ok(dest);
    }
    let tmp = dest.with_extension("part");
    let mut resp = ureq::get(entry.tokenizer_url).call().map_err(http_err)?;
    let bytes = resp.body_mut().read_to_vec().map_err(|e| AiError::Http(e.to_string()))?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &dest)?;
    Ok(dest)
}

/// Remove `entry`'s file (and any `.part` remnant). Missing is fine.
pub fn delete(entry: &ModelEntry, cache_dir: &Path) -> Result<(), AiError> {
    for p in [cache_dir.join(entry.file), cache_dir.join(format!("{}.part", entry.file))] {
        match fs::remove_file(&p) {
            Ok(()) | Err(_) => {} // absent or locked — nothing to surface
        }
    }
    Ok(())
}
