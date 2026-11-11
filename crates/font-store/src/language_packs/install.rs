//! Installing and listing Language packs, fetching the pack index, and the record of installed
//! packs.

use super::*;

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

    /// The record of installed packs. One that can't be read is copied aside as
    /// `language-packs.json.bad` and rebuilt from the pack folders on disk (written atomically
    /// over it), so no installed pack is ever dropped from it; a missing one is rebuilt the same
    /// way when pack folders are there (none: empty).
    pub(super) fn read_installed(&self) -> Result<Installed, Error> {
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

    /// The record as written, if there is one that can be read. Readable pack metadata and
    /// valid receipts are kept when an optional receipt is malformed.
    pub(super) fn read_record(&self) -> Option<Installed> {
        let mut installed: Installed =
            read_limited(&self.dir().join(INSTALLED), MAX_PACK_INDEX_BYTES).and_then(|b| serde_json::from_slice(&b).ok())?;
        installed.committed_removals.retain(|name| Stage::valid_name(name));
        Some(installed)
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
        Installed { version: 1, packs, committed_removals: Vec::new() }
    }

    /// Prune receipts only through an available recovery capability; otherwise retain them.
    pub(super) fn write_installed(&self, installed: &Installed) -> Result<(), Error> {
        let recovery = if installed.committed_removals.is_empty() { None } else { PackRemovalRoot::open(self.dir()).ok() };
        self.write_installed_with_recovery(installed, recovery.as_ref())
    }

    /// Commit removal receipts against the same recovery directory used for the pack move.
    pub(super) fn write_installed_with_recovery(&self, installed: &Installed, recovery: Option<&PackRemovalRoot>) -> Result<(), Error> {
        #[cfg(test)]
        tests::fail_io(tests::IoFailure::RecordWrite).map_err(io("saving the Language pack list"))?;
        // Keep receipts until their backup has actually disappeared. A future locked sweep
        // retries failed cleanup, and stage creation excludes IDs whose receipts remain.
        let committed_removals = installed
            .committed_removals
            .iter()
            .filter(|id| Stage::valid_name(id) && recovery.is_none_or(|root| root.retains_receipt(id)))
            .cloned()
            .collect();
        let bytes = serde_json::to_vec_pretty(&Installed { version: 1, packs: installed.packs.clone(), committed_removals })
            .map_err(|e| io("saving the Language pack list")(std::io::Error::other(e)))?;
        self.write_file(INSTALLED, &bytes)
    }
}
