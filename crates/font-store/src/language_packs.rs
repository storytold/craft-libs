//! The store's second item kind, `language-pack`: a language's spelling dictionary, hyphenation
//! patterns and licence texts, published by storytold/craft-i18n as `<locale>.tar.zst` release
//! assets listed in an `index.json`, and read at runtime by `craft-proof` (`craft_proof::packs`).
//!
//! It reuses the store's folder, consent ([`FontStore::enable_network`]), HTTP client and sha256
//! checks: the app pins the index by URL and sha256 ([`PackIndexPin`]), each archive is checked
//! against the size and sha256 the index gives, then unpacked into `.staging/` and moved into
//! `language-packs/<locale>/` with one rename. Packs have no update checks of their own: a new
//! pin is a new index.
//!
//! ```text
//! <store>/language-packs/<locale>/   pack.toml, spelling.aff, spelling.dic, hyphenation.pat, LICENCES/…
//! <store>/language-packs.json        which packs are installed, from which index entry
//! <store>/indexes/language-packs.json  the last pack index fetched
//! ```
//!
//! **Developer override (testing only):** `CRAFT_LANGUAGE_PACK_INDEX=<url or path>` replaces the
//! pinned index URL with an `http://` URL on a loopback host (`127.0.0.1`, `localhost`, `[::1]`),
//! an `https://` URL or a local file path, and skips the index's sha256 pin. Archives are still
//! checked against the sha256 and size in that index, and may then also be loopback `http://`
//! URLs or local paths. It is off unless the variable is set, and logged as a warning when used.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::http::{Cancel, Client, Timeouts};
use crate::index::IndexError;
use crate::store::{Error, FontStore, INDEX_CACHE, Progress, Stage, io, read_limited};
use crate::verify::{VerifyError, sha256_hex};

/// The store's folder of installed Language packs.
pub const LANGUAGE_PACKS: &str = "language-packs";
/// The developer override of the pinned pack index (see the module docs). Testing only.
pub const LANGUAGE_PACK_INDEX_ENV: &str = "CRAFT_LANGUAGE_PACK_INDEX";
/// Largest pack index read.
pub const MAX_PACK_INDEX_BYTES: u64 = 4 << 20;
/// Largest pack archive downloaded.
pub const MAX_PACK_ARCHIVE_BYTES: u64 = 128 << 20;
/// Largest file unpacked from a pack (craft-proof's dictionary cap).
pub const MAX_PACK_FILE_BYTES: u64 = 64 << 20;
/// Largest total unpacked size of a pack.
pub const MAX_PACK_UNPACKED_BYTES: u64 = 256 << 20;
/// Most files and folders in a pack archive.
pub const MAX_PACK_ENTRIES: usize = 256;
/// Most packs in an index.
const MAX_PACKS: usize = 1024;
const INSTALLED: &str = "language-packs.json";
const INDEX_CACHE_NAME: &str = "language-packs.json";

/// The pack index an app trusts: `index.json` of a craft-i18n `language-packs-<n>` release, by
/// URL and sha256 (the app's Pin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackIndexPin {
    /// `https://github.com/storytold/craft-i18n/releases/download/language-packs-<n>/index.json`.
    pub url: &'static str,
    /// The sha256 of that file, lowercase hex.
    pub sha256: &'static str,
}

/// `index.json` of a pack release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIndex {
    /// The format; 1.
    pub format: u32,
    /// The release (`language-packs-1`).
    pub release: String,
    /// The packs, by locale.
    pub packs: Vec<PackEntry>,
}

/// One pack of a [`PackIndex`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackEntry {
    /// Lowercase BCP 47 tag (`nl`, `pt-br`); the pack's folder name.
    pub locale: String,
    /// The language's own name (`Nederlands`).
    pub name: String,
    /// The craft-i18n commit it was built from.
    #[serde(default)]
    pub version: String,
    /// The archive (`<locale>.tar.zst`).
    pub url: String,
    /// The archive's sha256, lowercase hex.
    pub sha256: String,
    /// The archive's size in bytes.
    pub size: u64,
    /// SPDX licence expression.
    pub licence: String,
    /// The licence class: `Embeddable` or `Download-only`.
    #[serde(default)]
    pub class: String,
    /// Whether it has a spelling dictionary.
    #[serde(default)]
    pub spelling: bool,
    /// Whether it has hyphenation patterns.
    #[serde(default)]
    pub hyphenation: bool,
    /// Where the data comes from (an `https://` page), for display; optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// An installed pack, as recorded when it was installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledLanguagePack {
    /// Lowercase BCP 47 tag; the folder under [`LANGUAGE_PACKS`].
    pub locale: String,
    /// The language's own name.
    pub name: String,
    /// The release it came from.
    pub release: String,
    /// The craft-i18n commit it was built from.
    pub version: String,
    /// SPDX licence expression.
    pub licence: String,
    /// The archive's sha256.
    pub sha256: String,
    /// The archive's size in bytes.
    pub size: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Installed {
    version: u32,
    packs: Vec<InstalledLanguagePack>,
}

impl PackIndex {
    /// Parses and checks an index: format 1, every locale a lowercase tag and listed once, every
    /// archive at most [`MAX_PACK_ARCHIVE_BYTES`] with a sha256. Archive URLs must be `https://`
    /// (with `local`, the developer override, also loopback `http://` or a path).
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse(bytes, false)
    }

    fn parse(bytes: &[u8], local: bool) -> Result<Self, Error> {
        let bad = |why: String| Error::Index(IndexError::Invalid(format!("Language pack index: {why}")));
        let mut index: PackIndex = serde_json::from_slice(bytes).map_err(|e| Error::Index(IndexError::Json(format!("Language pack index: {e}"))))?;
        if index.format != 1 {
            return Err(bad(format!("format {} is not supported", index.format)));
        }
        if index.packs.len() > MAX_PACKS {
            return Err(bad("too many packs".into()));
        }
        for (i, p) in index.packs.iter().enumerate() {
            if !is_locale(&p.locale) || index.packs.iter().take(i).any(|q| q.locale == p.locale) {
                return Err(bad(format!("pack {:?}: not a lowercase language tag, or listed twice", p.locale.chars().take(40).collect::<String>())));
            }
            if p.size > MAX_PACK_ARCHIVE_BYTES || !is_sha256(&p.sha256) {
                return Err(bad(format!("pack {}: bad size or sha256", p.locale)));
            }
            if !(p.url.starts_with("https://") || local && local_source(&p.url).is_some()) {
                return Err(bad(format!("pack {}: {} is not an https URL", p.locale, p.url.chars().take(200).collect::<String>())));
            }
            if let Some(src) = &p.source
                && !(src.starts_with("https://") && src.len() <= 2048 && !src.chars().any(|c| c.is_control() || is_format(c) || c == ' '))
            {
                return Err(bad(format!("pack {}: source {:?} is not an https URL", p.locale, src.chars().take(200).collect::<String>())));
            }
        }
        // Text shown to people loses control and bidi/format characters.
        index.release = plain_text(&index.release);
        for p in &mut index.packs {
            for t in [&mut p.name, &mut p.version, &mut p.licence, &mut p.class] {
                *t = plain_text(t);
            }
        }
        Ok(index)
    }

    /// The pack for `locale`.
    pub fn pack(&self, locale: &str) -> Option<&PackEntry> {
        self.packs.iter().find(|p| p.locale == locale)
    }
}

/// Where the pack index comes from: the pin, or the developer override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IndexSource {
    /// The pinned URL, checked against the pinned sha256.
    Pinned(PackIndexPin),
    /// `CRAFT_LANGUAGE_PACK_INDEX`: no index pin; loopback `http://` and paths allowed.
    Override(String),
}

impl IndexSource {
    /// The pin, unless [`LANGUAGE_PACK_INDEX_ENV`] is set.
    fn from_env(pin: &PackIndexPin) -> Self {
        match language_pack_index_overridden() {
            Some(v) => {
                log::warn!(
                    "{LANGUAGE_PACK_INDEX_ENV}={v}: the Language pack index is NOT the pinned one and its sha256 is not checked (testing only)"
                );
                Self::Override(v)
            }
            None => Self::Pinned(*pin),
        }
    }

    fn is_override(&self) -> bool {
        matches!(self, Self::Override(_))
    }
}

/// The developer override of the pack index, when [`LANGUAGE_PACK_INDEX_ENV`] is set: the URL or
/// path that replaces the pinned index, whose sha256 is then **not** checked. Apps show a warning
/// in their Language pack interface while it is set. It works in release builds too, for
/// end-to-end testing against a local index.
pub fn language_pack_index_overridden() -> Option<String> {
    std::env::var(LANGUAGE_PACK_INDEX_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A source the developer override may read: `Ok(url)` for an `https://` URL or an `http://`
/// one on a loopback host, `Err(path)` for a local file path; `None` for anything else.
fn local_source(s: &str) -> Option<Result<&str, PathBuf>> {
    if s.starts_with("https://") {
        return Some(Ok(s));
    }
    if let Some(rest) = s.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        let name = if host.starts_with('[') {
            host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
        } else {
            host.split(':').next().unwrap_or_default().to_string()
        };
        return ["127.0.0.1", "localhost", "[::1]"].contains(&name.as_str()).then_some(Ok(s));
    }
    if s.contains("://") {
        return None;
    }
    Some(Err(PathBuf::from(s)))
}

/// A lowercase BCP 47 tag as packs are named (craft-proof's rule): ASCII lowercase letters and
/// digits in parts of 1 to 8 joined by `-`, at most 35 characters.
fn is_locale(s: &str) -> bool {
    !windows_reserved(s)
        && !s.is_empty()
        && s.len() <= 35
        && s.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl FontStore {
    /// The folder of installed Language packs (`<store>/language-packs`): give it to
    /// `craft_proof::packs::set_packs_dir`.
    pub fn language_packs_dir(&self) -> PathBuf {
        self.dir().join(LANGUAGE_PACKS)
    }

    /// The pack index saved by the last [`FontStore::fetch_language_pack_index`], when it is
    /// `pin`'s (or the developer override is set).
    pub fn cached_language_pack_index(&self, pin: &PackIndexPin) -> Option<PackIndex> {
        self.cached_pack_index_from(&IndexSource::from_env(pin))
    }

    pub(crate) fn cached_pack_index_from(&self, source: &IndexSource) -> Option<PackIndex> {
        let bytes = read_limited(&self.dir().join(INDEX_CACHE).join(INDEX_CACHE_NAME), MAX_PACK_INDEX_BYTES)?;
        match source {
            IndexSource::Pinned(pin) if sha256_hex(&bytes) != pin.sha256 => None,
            _ => PackIndex::parse(&bytes, source.is_override()).ok(),
        }
    }

    /// Downloads the pack index `pin` names, checks it against the pin and saves it for
    /// [`FontStore::cached_language_pack_index`]. Needs the network on.
    pub fn fetch_language_pack_index(&self, pin: &PackIndexPin, cancel: &Cancel) -> Result<PackIndex, Error> {
        self.fetch_pack_index_from(&IndexSource::from_env(pin), cancel)
    }

    pub(crate) fn fetch_pack_index_from(&self, source: &IndexSource, cancel: &Cancel) -> Result<PackIndex, Error> {
        self.require_network()?;
        let bytes = match source {
            IndexSource::Pinned(pin) => {
                let check = |b: &[u8]| if sha256_hex(b) == pin.sha256 { Ok(()) } else { Err(VerifyError::Sha256) };
                self.fetch(&[pin.url.to_string()], MAX_PACK_INDEX_BYTES, "the Language pack index", cancel, &mut |_| {}, &check)?
            }
            IndexSource::Override(at) => self.get_local(at, MAX_PACK_INDEX_BYTES, "the Language pack index", cancel, &mut |_| {}, &|_| Ok(()))?,
        };
        let index = PackIndex::parse(&bytes, source.is_override())?;
        std::fs::create_dir_all(self.dir().join(INDEX_CACHE)).map_err(io("creating the index folder"))?;
        self.write_file(&format!("{INDEX_CACHE}/{INDEX_CACHE_NAME}"), &bytes)?;
        Ok(index)
    }

    /// A download under the developer override: an https or loopback http URL, or a file.
    fn get_local(
        &self,
        at: &str,
        max: u64,
        what: &str,
        cancel: &Cancel,
        on_bytes: &mut dyn FnMut(u64),
        check: &dyn Fn(&[u8]) -> Result<(), VerifyError>,
    ) -> Result<Vec<u8>, Error> {
        let bytes = match local_source(at) {
            Some(Ok(url)) if url.starts_with("https://") => return self.fetch(&[url.to_string()], max, what, cancel, on_bytes, check),
            Some(Ok(url)) => Client::allowing_http(Timeouts::default())
                .get(url, max, cancel, on_bytes)
                .map_err(|e| Error::Download { what: what.to_string(), reasons: vec![e.to_string()] })?,
            Some(Err(path)) => {
                let mut buf = Vec::new();
                std::fs::File::open(&path)
                    .and_then(|f| f.take(max.saturating_add(1)).read_to_end(&mut buf))
                    .map_err(io(format!("reading {}", path.display())))?;
                if buf.len() as u64 > max {
                    return Err(Error::Download { what: what.to_string(), reasons: vec![format!("larger than {max} bytes")] });
                }
                on_bytes(buf.len() as u64);
                buf
            }
            None => return Err(Error::BadUrl(at.chars().take(200).collect())),
        };
        check(&bytes).map_err(|error| Error::Verify { file: what.to_string(), error })?;
        Ok(bytes)
    }

    /// Downloads `pack` from `index`, checks its size and sha256, unpacks it and moves it into
    /// `language-packs/<locale>/` in one rename, replacing an older one. Needs the network on.
    /// Then call `craft_proof::packs::reload()`.
    pub fn install_language_pack(
        &self,
        index: &PackIndex,
        pack: &PackEntry,
        cancel: &Cancel,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstalledLanguagePack, Error> {
        let local = language_pack_index_overridden().is_some();
        self.install_pack_from(local, index, pack, cancel, progress)
    }

    pub(crate) fn install_pack_from(
        &self,
        local: bool,
        index: &PackIndex,
        pack: &PackEntry,
        cancel: &Cancel,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstalledLanguagePack, Error> {
        self.require_network()?;
        // The entry is checked again here: callers may build one by hand.
        if !is_locale(&pack.locale) || !is_sha256(&pack.sha256) || pack.size > MAX_PACK_ARCHIVE_BYTES {
            let locale: String = pack.locale.chars().take(40).collect();
            return Err(Error::Index(IndexError::Invalid(format!("Language pack {locale:?}: bad locale, sha256 or size"))));
        }
        let what = format!("{}.tar.zst", pack.locale);
        let (size, sha) = (pack.size, pack.sha256.clone());
        let check = move |b: &[u8]| {
            if b.len() as u64 != size {
                Err(VerifyError::Size { expected: size, actual: b.len() as u64 })
            } else if sha256_hex(b) != sha {
                Err(VerifyError::Sha256)
            } else {
                Ok(())
            }
        };
        let mut on_bytes = |done: u64| progress(Progress { file: 0, files: 1, done, total: size });
        let bytes = if local {
            self.get_local(&pack.url, size, &what, cancel, &mut on_bytes, &check)?
        } else if pack.url.starts_with("https://") {
            self.fetch(std::slice::from_ref(&pack.url), size, &what, cancel, &mut on_bytes, &check)?
        } else {
            return Err(Error::BadUrl(pack.url.chars().take(200).collect()));
        };
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.sweep_staging();
        let stage = Stage::new(self.dir())?;
        let unpacked = stage.dir.join(&pack.locale);
        unpack(&bytes, &unpacked, pack).map_err(|e| Error::Io { what: format!("unpacking {what}"), source: std::io::Error::other(e) })?;
        let record = InstalledLanguagePack {
            locale: pack.locale.clone(),
            name: plain_text(&pack.name),
            release: plain_text(&index.release),
            version: plain_text(&pack.version),
            licence: plain_text(&pack.licence),
            sha256: pack.sha256.clone(),
            size: pack.size,
        };
        let _lock = self.lock()?;
        // The record first: rebuilding a damaged one must not count the pack being installed.
        let mut installed = self.read_installed()?;
        let packs = self.language_packs_dir();
        std::fs::create_dir_all(&packs).map_err(io("creating the Language packs folder"))?;
        let dest = packs.join(&pack.locale);
        let old = stage.dir.join(format!("{}.old", pack.locale));
        let had = std::fs::symlink_metadata(&dest).is_ok();
        if had {
            std::fs::rename(&dest, &old).map_err(io(format!("replacing the {} pack", pack.locale)))?;
        }
        if let Err(e) = std::fs::rename(&unpacked, &dest) {
            if had {
                let _ = std::fs::rename(&old, &dest);
            }
            return Err(io(format!("installing the {} pack", pack.locale))(e));
        }
        installed.packs.retain(|p| p.locale != record.locale);
        installed.packs.push(record.clone());
        installed.packs.sort_by(|a, b| a.locale.cmp(&b.locale));
        if let Err(e) = self.write_installed(&installed) {
            // Put the old pack back, so folder and record agree.
            let _ = std::fs::remove_dir_all(&dest);
            if had {
                let _ = std::fs::rename(&old, &dest);
            }
            return Err(e);
        }
        Ok(record)
    }

    /// The installed Language packs whose folder is there, by locale.
    pub fn installed_language_packs(&self) -> Vec<InstalledLanguagePack> {
        let dir = self.language_packs_dir();
        // Read only: without the store lock this never writes the record (an install or removal
        // may be under way); a record that can't be read is listed from the folders and repaired
        // by the next install or removal, which hold the lock.
        let installed = self.read_record().unwrap_or_else(|| self.installed_from_disk());
        installed
            .packs
            .into_iter()
            .filter(|p| is_locale(&p.locale) && dir.join(&p.locale).is_dir())
            .map(|p| InstalledLanguagePack {
                name: plain_text(&p.name),
                release: plain_text(&p.release),
                version: plain_text(&p.version),
                licence: plain_text(&p.licence),
                ..p
            })
            .collect()
    }

    /// Removes the pack for `locale`: its folder disappears at once (one rename) and is deleted
    /// after. No network needed. Then call `craft_proof::packs::reload()`.
    pub fn remove_language_pack(&self, locale: &str) -> Result<(), Error> {
        if !is_locale(locale) {
            return Err(Error::NotInstalled(locale.chars().take(40).collect()));
        }
        let stage = Stage::new(self.dir())?;
        let _lock = self.lock()?;
        let dest = self.language_packs_dir().join(locale);
        let mut installed = self.read_installed()?;
        let known = installed.packs.iter().any(|p| p.locale == locale);
        if !known && std::fs::symlink_metadata(&dest).is_err() {
            return Err(Error::NotInstalled(locale.to_string()));
        }
        if std::fs::symlink_metadata(&dest).is_ok() {
            std::fs::rename(&dest, stage.dir.join(locale)).map_err(io(format!("removing the {locale} pack")))?;
        }
        installed.packs.retain(|p| p.locale != locale);
        self.write_installed(&installed)
        // `stage` deletes the old folder when dropped.
    }

    /// The record of installed packs. One that can't be read is copied aside as
    /// `language-packs.json.bad` and rebuilt from the pack folders on disk (written atomically
    /// over it), so no installed pack is ever dropped from it; a missing one is rebuilt the same
    /// way when pack folders are there (none: empty).
    fn read_installed(&self) -> Result<Installed, Error> {
        let path = self.dir().join(INSTALLED);
        let there = std::fs::symlink_metadata(&path).is_ok();
        if let Some(i) = self.read_record() {
            return Ok(i);
        }
        let rebuilt = self.installed_from_disk();
        if !there && rebuilt.packs.is_empty() {
            return Ok(Installed::default());
        }
        log::warn!("{}: {}; rebuilt from the Language pack folders", path.display(), if there { "can't be read" } else { "missing" });
        if there {
            let _ = std::fs::copy(&path, self.dir().join(format!("{INSTALLED}.bad")));
        }
        self.write_installed(&rebuilt)?;
        Ok(rebuilt)
    }

    /// The record as written, if there is one that can be read.
    fn read_record(&self) -> Option<Installed> {
        read_limited(&self.dir().join(INSTALLED), MAX_PACK_INDEX_BYTES).and_then(|b| serde_json::from_slice::<Installed>(&b).ok())
    }

    /// The installed packs as their folders say: each folder named by a locale, with the name
    /// and licence from its `pack.toml` (release, version and sha256 unknown).
    fn installed_from_disk(&self) -> Installed {
        let mut packs: Vec<InstalledLanguagePack> = std::fs::read_dir(self.language_packs_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|l| is_locale(l))
            .take(MAX_PACKS)
            .map(|locale| {
                let info = read_limited(&self.language_packs_dir().join(&locale).join("pack.toml"), MAX_PACK_TOML_BYTES)
                    .and_then(|b| String::from_utf8(b).ok())
                    .and_then(|t| t.parse::<toml::Table>().ok());
                let field = |k: &str| info.as_ref().and_then(|t| t.get(k)).and_then(toml::Value::as_str).map(plain_text);
                InstalledLanguagePack {
                    name: field("name").unwrap_or_else(|| locale.clone()),
                    licence: field("licence").unwrap_or_default(),
                    locale,
                    release: String::new(),
                    version: String::new(),
                    sha256: String::new(),
                    size: 0,
                }
            })
            .collect();
        packs.sort_by(|a, b| a.locale.cmp(&b.locale));
        Installed { version: 1, packs }
    }

    fn write_installed(&self, installed: &Installed) -> Result<(), Error> {
        let bytes = serde_json::to_vec_pretty(&Installed { version: 1, packs: installed.packs.clone() })
            .map_err(|e| io("saving the Language pack list")(std::io::Error::other(e)))?;
        self.write_file(INSTALLED, &bytes)
    }
}

/// The zstd frames of `archive` one after another, as one stream (`zstd` writes one frame, but
/// a concatenation of frames is a valid `.zst` too, and must not stop after the first).
struct Frames<'a> {
    decoder: Option<ruzstd::decoding::StreamingDecoder<&'a [u8], ruzstd::decoding::FrameDecoder>>,
    /// Frames started so far, empty ones included (at most [`MAX_PACK_FRAMES`]).
    frames: usize,
}

/// Most zstd frames in a pack archive (`zstd` writes one; empty frames count too).
const MAX_PACK_FRAMES: usize = 4096;

impl<'a> Frames<'a> {
    fn new(archive: &'a [u8]) -> Result<Self, String> {
        Ok(Self { decoder: Some(ruzstd::decoding::StreamingDecoder::new(archive).map_err(|e| format!("not zstd: {e}"))?), frames: 1 })
    }
}

impl Read for Frames<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let Some(d) = self.decoder.as_mut() else { return Ok(0) };
            let n = d.read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            let rest: &[u8] = d.get_ref();
            if !rest.is_empty() {
                self.frames += 1;
                if self.frames > MAX_PACK_FRAMES {
                    return Err(std::io::Error::other(format!("more than {MAX_PACK_FRAMES} zstd frames")));
                }
            }
            self.decoder = if rest.is_empty() { None } else { Some(ruzstd::decoding::StreamingDecoder::new(rest).map_err(std::io::Error::other)?) };
        }
    }
}

/// Unpacks the `.tar.zst` of `pack` into `dest` (created). Only the contract's files are allowed:
/// `pack.toml`, `spelling.aff`, `spelling.dic`, `hyphenation.pat` at the root and
/// `LICENCES/<name>` ([`licence_name`]); regular files and that folder only, each file at most
/// [`MAX_PACK_FILE_BYTES`], all of them [`MAX_PACK_UNPACKED_BYTES`], at most [`MAX_PACK_ENTRIES`].
/// Every zstd frame is read, and nothing but tar padding may follow the archive's end. `pack.toml`
/// must name `pack`'s locale, and the files the index promises (`spelling`, `hyphenation`) must
/// be there.
pub(crate) fn unpack(archive: &[u8], dest: &Path, pack: &PackEntry) -> Result<(), String> {
    // The decompressed stream is capped too: tar padding and headers beyond the files' bytes.
    let cap = MAX_PACK_UNPACKED_BYTES.saturating_add((MAX_PACK_ENTRIES as u64 + 2) * 1024 + 20 * 512);
    let mut tar = tar::Archive::new(Frames::new(archive)?.take(cap));
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let (mut total, mut count) = (0u64, 0usize);
    let mut written: Vec<String> = Vec::new();
    for entry in tar.entries().map_err(|e| format!("not a tar archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("damaged archive: {e}"))?;
        count += 1;
        if count > MAX_PACK_ENTRIES {
            return Err("too many files".into());
        }
        let raw = entry.path().map_err(|e| format!("bad file name: {e}"))?.into_owned();
        let rel = safe_entry_path(&raw).ok_or_else(|| format!("{} is outside the pack", raw.display()))?;
        let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            match parts.as_slice() {
                [] => {}
                [LICENCES_DIR] => std::fs::create_dir_all(dest.join(LICENCES_DIR)).map_err(|e| e.to_string())?,
                _ => return Err(format!("{}: not a folder a pack has", rel.display())),
            }
            continue;
        }
        if !kind.is_file() {
            return Err(format!("{}: only files and folders are allowed", rel.display()));
        }
        let allowed = match parts.as_slice() {
            [name] => ROOT_FILES.contains(name),
            [LICENCES_DIR, name] => licence_name(name),
            _ => false,
        };
        let key = parts.join("/");
        if !allowed || written.contains(&key) {
            return Err(format!("{}: not a file a pack has, or listed twice", rel.display()));
        }
        let size = entry.header().size().map_err(|e| e.to_string())?;
        if size > MAX_PACK_FILE_BYTES {
            return Err(format!("{}: larger than {MAX_PACK_FILE_BYTES} bytes", rel.display()));
        }
        total = total.saturating_add(size);
        if total > MAX_PACK_UNPACKED_BYTES {
            return Err(format!("larger than {MAX_PACK_UNPACKED_BYTES} bytes unpacked"));
        }
        let path = dest.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::options().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", rel.display()))?;
        let copied = std::io::copy(&mut (&mut entry).take(MAX_PACK_FILE_BYTES), &mut out).map_err(|e| format!("{}: {e}", rel.display()))?;
        if copied != size {
            return Err(format!("{}: truncated", rel.display()));
        }
        written.push(key);
    }
    // Only zeros (the tar end blocks and padding) may follow: not a second archive, not a frame
    // that wasn't read.
    let mut rest = tar.into_inner();
    let mut buf = [0u8; 8192];
    loop {
        let n = rest.read(&mut buf).map_err(|e| format!("damaged archive: {e}"))?;
        if n == 0 {
            break;
        }
        if buf.get(..n).is_some_and(|b| b.iter().any(|&x| x != 0)) {
            return Err("data after the end of the archive".into());
        }
    }
    let has = |f: &str| written.iter().any(|w| w == f);
    if !has("pack.toml") {
        return Err("no pack.toml".into());
    }
    let toml = std::fs::read_to_string(dest.join("pack.toml")).map_err(|e| format!("pack.toml: {e}"))?;
    let named = toml.parse::<toml::Table>().ok().and_then(|t| t.get("locale").and_then(toml::Value::as_str).map(str::to_string));
    if named.as_deref() != Some(pack.locale.as_str()) {
        return Err(format!("pack.toml names locale {named:?}, not {:?}", pack.locale));
    }
    if pack.spelling && !(has("spelling.aff") && has("spelling.dic")) {
        return Err("the index promises a spelling dictionary the archive doesn't have".into());
    }
    if pack.hyphenation && !has("hyphenation.pat") {
        return Err("the index promises hyphenation patterns the archive doesn't have".into());
    }
    Ok(())
}

/// The files a pack holds at its root.
const ROOT_FILES: [&str; 4] = ["pack.toml", "spelling.aff", "spelling.dic", "hyphenation.pat"];
/// The folder of a pack's licence texts.
const LICENCES_DIR: &str = "LICENCES";
/// Largest `pack.toml` read back.
const MAX_PACK_TOML_BYTES: u64 = 1 << 20;

/// A licence file's name: `[A-Za-z0-9._-]`, at most 100 characters, not starting with a dot or
/// ending with one, and not a name Windows reserves.
fn licence_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 100
        && !n.starts_with('.')
        && !n.ends_with('.')
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !windows_reserved(n)
}

/// A name Windows reserves for a device (`CON`, `nul.txt`, `COM1`, `lpt9.md`), with or without an
/// extension, in any case.
fn windows_reserved(n: &str) -> bool {
    let stem = n.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes().get(3).is_some_and(|d| (b'1'..=b'9').contains(d)))
}

/// Text from an index or `pack.toml` shown to people (a pack's name): without control,
/// bidirectional or other format characters (U+202E would reverse what follows), trimmed, at most
/// 200 characters.
fn plain_text(s: &str) -> String {
    s.chars().filter(|&c| !c.is_control() && !is_format(c)).take(200).collect::<String>().trim().to_string()
}

/// Unicode format characters (general category Cf): bidirectional controls, zero-width ones, the
/// soft hyphen, tags.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}'
        | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// `p` as a plain relative path (`./` prefixes dropped), or `None` for a root, a drive, `..` or a
/// name that isn't UTF-8 or holds a backslash, colon or control character.
fn safe_entry_path(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(n) => {
                let n = n.to_str()?;
                if n.is_empty() || n.contains(['\\', ':']) || n.chars().any(char::is_control) {
                    return None;
                }
                out.push(n);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
