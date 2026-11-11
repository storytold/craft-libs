//! The Windows Installer context of an installed copy, from Windows Installer's own registration,
//! never inferred from a display registry entry. An existing copy must never fall back to a fresh
//! installation when discovery fails.
//!
//! Shared by the launcher's installs and the apps' self-updates, so both keep a copy in the same
//! context by the same rule.

#![cfg_attr(not(windows), allow(dead_code))]

/// The Windows Installer context a copy is installed in. A per-machine copy elevates and writes
/// HKLM registrations; a per-user one must not silently become a second, per-machine copy (or the
/// other way round).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsiScope {
    #[allow(missing_docs)]
    PerUser,
    #[allow(missing_docs)]
    PerMachine,
}

/// The `msiexec` properties that ask a dual-scope package for `scope`. `ALLUSERS=1` installs for
/// everyone (Windows asks for permission); `ALLUSERS=2` with `MSIINSTALLPERUSER=1` is the documented
/// single-package way to ask for a per-user install.
pub fn scope_arguments(scope: MsiScope) -> &'static [&'static str] {
    match scope {
        MsiScope::PerUser => &["ALLUSERS=2", "MSIINSTALLPERUSER=1"],
        MsiScope::PerMachine => &["ALLUSERS=1"],
    }
}

const UNKNOWN_CONTEXT: &str = "Windows could not confirm who this app is installed for. No changes were made. Try the app's installer instead.";
const CONFLICTING_CONTEXT: &str = "This app is installed for both your account and everyone. Use its installer to choose which copy to update.";

/// Why the Windows Installer lookup gave no context.
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The lookup ran but gave no usable answer. The context is never guessed from that.
    Unknown,
    /// The same product is registered for this account and for everyone.
    Conflicting,
    /// The lookup could not run (a registration key it may not read, no account identity). Carries
    /// the real error so it can be logged.
    Unavailable(String),
}

/// The context an update must keep. Windows Installer's own registration decides. Only when that
/// lookup cannot run at all does the context come from the uninstall record the copy was found by
/// (`record_scope`): the machine's records are per-machine and the account's own are per-user. A
/// lookup that ran and gave no answer, or a copy registered twice, still stops the update.
pub fn update_scope(
    is_msi: bool,
    product_code: Option<&str>,
    probe: impl FnOnce(&str) -> Result<MsiScope, ProbeError>,
    record_scope: impl FnOnce(&str) -> Option<MsiScope>,
) -> Result<Option<MsiScope>, String> {
    if !is_msi {
        return Ok(None);
    }
    let code = product_code.filter(|code| valid_product_code(code)).ok_or(UNKNOWN_CONTEXT)?;
    match probe(code) {
        Ok(scope) => Ok(Some(scope)),
        Err(ProbeError::Conflicting) => Err(CONFLICTING_CONTEXT.into()),
        Err(ProbeError::Unknown) => Err(UNKNOWN_CONTEXT.into()),
        Err(ProbeError::Unavailable(reason)) => {
            log::warn!("Windows Installer context lookup for {code} could not run: {reason}");
            let scope = record_scope(code).ok_or(UNKNOWN_CONTEXT)?;
            log::info!("{code}: keeping the {scope:?} context of its uninstall record");
            Ok(Some(scope))
        }
    }
}

/// The one context every uninstall record of a product agrees on. Records that disagree (the copy is
/// registered for this account and for everyone) or no records at all give `None`, so the update stops.
pub fn single_scope(scopes: impl IntoIterator<Item = MsiScope>) -> Option<MsiScope> {
    let mut scopes = scopes.into_iter();
    let first = scopes.next()?;
    scopes.all(|scope| scope == first).then_some(first)
}

/// The context to install in. An update keeps `updated`, the context of the copy it replaces. A fresh
/// install (no copy at all) is machine-wide, so every app lands in `C:\Program Files\ArtCraft\<App>`
/// (or the folder the person chose) and not in one account's profile; a dual-scope package would
/// otherwise take its own default, which is per-user. Windows asks for permission once, for that
/// install. Anything else (a copy that is not an MSI) takes the package's own default.
pub fn install_scope(updated: Option<MsiScope>, fresh: bool) -> Option<MsiScope> {
    updated.or_else(|| fresh.then_some(MsiScope::PerMachine))
}

/// Whether `code` is a braced Windows Installer product GUID, the only text ever used to name a
/// registration key.
pub fn valid_product_code(code: &str) -> bool {
    code.len() == 38
        && code.bytes().enumerate().all(|(index, byte)| match index {
            0 => byte == b'{',
            37 => byte == b'}',
            9 | 14 | 19 | 24 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// The one context the product is registered in: 1 and 2 (managed and unmanaged, this account) are
/// per-user, 4 is the machine. None, any other value, or both kinds is never guessed from.
fn scope_of(contexts: &[u32]) -> Result<MsiScope, ProbeError> {
    let mut scope = None;
    for context in contexts {
        let context = match context {
            1 | 2 => MsiScope::PerUser,
            4 => MsiScope::PerMachine,
            _ => return Err(ProbeError::Unknown),
        };
        if scope.is_some_and(|previous| previous != context) {
            return Err(ProbeError::Conflicting);
        }
        scope = Some(context);
    }
    scope.ok_or(ProbeError::Unknown)
}

/// Windows Installer's packed form of a validated product code, the name of its registration keys:
/// the first three groups reversed, then each remaining byte's two digits swapped.
fn packed_code(code: &str) -> Option<String> {
    if !valid_product_code(code) {
        return None;
    }
    let hex: Vec<char> = code.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_uppercase()).collect();
    let (groups, rest) = hex.split_at_checked(16)?;
    let mut packed = String::with_capacity(32);
    for range in [0..8, 8..12, 12..16] {
        packed.extend(groups.get(range)?.iter().rev());
    }
    for pair in rest.chunks(2) {
        packed.extend(pair.iter().rev());
    }
    Some(packed)
}

/// Asks Windows Installer, read-only, which context `product_code` is installed in, for this
/// account and the machine, as `ProductsEx` does, from Windows Installer's own keys: no other
/// account, no other program started, and never `Win32_Product` (which repairs).
#[cfg(windows)]
pub fn probe(product_code: &str) -> Result<MsiScope, ProbeError> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

    let packed = packed_code(product_code).ok_or(ProbeError::Unknown)?;
    let sid = current_user_sid().map_err(ProbeError::Unavailable)?;
    let keys = [
        (1, HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\Managed\{sid}\Installer\Products\{packed}")),
        (2, HKEY_CURRENT_USER, format!(r"Software\Microsoft\Installer\Products\{packed}")),
        (4, HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\Classes\Installer\Products\{packed}")),
    ];
    let mut contexts = Vec::new();
    for (context, hive, path) in keys {
        match RegKey::predef(hive).open_subkey_with_flags(&path, KEY_READ | KEY_WOW64_64KEY) {
            Ok(_) => contexts.push(context),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // A key we may not read is a lookup that didn't run, not a product that isn't there.
            Err(e) => return Err(ProbeError::Unavailable(format!("couldn't read the Windows Installer registration ({e})"))),
        }
    }
    scope_of(&contexts)
}

/// This account's SID, which names its managed (policy-installed) registrations.
#[cfg(windows)]
fn current_user_sid() -> Result<String, String> {
    use winsafe::{HACCESSTOKEN, TokenInfo, co};
    let failed = |e: winsafe::co::ERROR| format!("couldn't identify this account ({e})");
    match HACCESSTOKEN::GetCurrentProcessToken().GetTokenInformation(co::TOKEN_INFORMATION_CLASS::User).map_err(failed)? {
        TokenInfo::User(user) => user.User.Sid().map(|sid| sid.to_string()).ok_or_else(|| "couldn't identify this account".into()),
        _ => Err("couldn't identify this account".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    const CODE: &str = "{12345678-1234-1234-1234-123456789ABC}";

    #[trial]
    fn a_product_code_packs_the_way_windows_installer_names_its_keys() {
        assert_eq!(packed_code("{12345678-ABCD-EF01-2345-6789ABCDEF01}").as_deref(), Some("87654321DCBA10FE32547698BADCFE10"));
        assert_eq!(packed_code("{12345678-abcd-ef01-2345-6789abcdef01}").as_deref(), Some("87654321DCBA10FE32547698BADCFE10"));
        for code in ["", "12345678-ABCD-EF01-2345-6789ABCDEF01", "{12345678-ABCD-EF01-2345-6789ABCDEF0Z}", r"{1234567\-ABCD-EF01-2345-6789ABCDEF01}"]
        {
            assert_eq!(packed_code(code), None, "{code:?}");
        }
    }

    #[trial]
    fn unavailable_context_never_becomes_a_fresh_install() {
        let unavailable = || Err(ProbeError::Unavailable("query failed".into()));
        assert!(update_scope(true, Some(CODE), |_| unavailable(), |_| None).is_err());
    }
}
