//! A bundle that was downloaded, opened and checked for an app, held until the person agrees to
//! install it: plain data with no session, store or UI in it, so a worker thread can build one and
//! hand it to the UI. The UI keys what it holds by [`Reviewed::sha256`] (the SHA-256 its
//! confirmation showed) and installs it with `Store::install_reviewed_for` (native builds), which
//! refuses it when the SHA-256 differs from the one the person agreed to.
//!
//! ```rust,ignore
//! // Worker thread:
//! let bytes = fetch::download_repo(&mut http, &plan)?;
//! let reviewed = Reviewed::new(&APP, bytes, Source::Github { repo, tag, asset })?;
//! send_to_ui(reviewed);
//!
//! // UI: show reviewed.manifest, reviewed.target, reviewed.source and reviewed.sha256, keep it in a
//! // map by reviewed.sha256; when the person agrees to the SHA-256 `agreed`:
//! store.install_reviewed_for(&APP, held.remove(&agreed)?, &agreed)?;
//! ```
//!
//! Adopting it is optional: `Store::install_for` with the bytes and source does the same checks.

use crate::bundle::{ModuleCheck, check_command_module, open_with, sha256_hex};
use crate::github::is_hex64;
use crate::manifest::Manifest;
use crate::source::Source;
use crate::target::{AppInfo, Target};

/// A bundle opened and checked for an app, with where it came from, ready to show in an install
/// confirmation and to install once the person agrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Reviewed {
    /// The `.craftplugin` file as downloaded or picked.
    pub bytes: Vec<u8>,
    /// Lower-case hex SHA-256 of [`Reviewed::bytes`]: what the confirmation shows and the key to
    /// hold it by.
    pub sha256: String,
    /// The bundle's manifest.
    pub manifest: Manifest,
    /// Where it came from, as it will be recorded.
    pub source: Source,
    /// How the plug-in targets the app it was reviewed for.
    pub target: Target,
}

impl Reviewed {
    /// Open the command plug-in bundle `bytes` from `source` and check it for `app`: the source
    /// can be recorded ([`Source::check`]), the bundle and its module are sound
    /// ([`crate::bundle::open`]) and the app runs it ([`Manifest::target`]). Nothing is written.
    pub fn new(app: &AppInfo<'_>, bytes: Vec<u8>, source: Source) -> Result<Reviewed, String> {
        Reviewed::new_with(app, bytes, source, &check_command_module)
    }

    /// [`Reviewed::new`] for an app that runs its own kinds: `check_module` checks the module of
    /// every kind the app runs (it gets the manifest, so it can tell them apart).
    pub fn new_with(app: &AppInfo<'_>, bytes: Vec<u8>, source: Source, check_module: &ModuleCheck) -> Result<Reviewed, String> {
        source.check()?;
        let (_, manifest) = open_with(&bytes, check_module)?;
        let target = manifest.target(app).map_err(|why| format!("{} can't be installed: {why}", manifest.name))?;
        let sha256 = sha256_hex(&bytes);
        Ok(Reviewed { bytes, sha256, manifest, source, target })
    }

    /// Why this isn't the bundle whose SHA-256 the person agreed to (`expected`, hex in either
    /// case), if it isn't: [`Reviewed::bytes`] must hash to both `expected` and
    /// [`Reviewed::sha256`].
    pub fn check_sha256(&self, expected: &str) -> Result<(), String> {
        let expected = expected.trim().to_ascii_lowercase();
        let actual = sha256_hex(&self.bytes);
        if !is_hex64(&expected) {
            return Err("the SHA-256 agreed to isn't 64 hex digits".into());
        }
        if actual != expected || actual != self.sha256 {
            return Err(format!("the plug-in held for installing has SHA-256 {actual}, not the {expected} that was agreed to: review it again"));
        }
        Ok(())
    }
}
