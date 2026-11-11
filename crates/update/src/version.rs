//! Versions: release tags (`v0.5.0`), installed versions from many sources, and ordering.

use std::cmp::Ordering;

pub use semver::Version;

/// Reads a version from a tag or an installed-version string.
///
/// Accepts `v0.5.0`, `0.5.0`, `0.2.0-rc.1`, Debian's `0.2.0~rc.1-1`, and the two-part `0.5`
/// some platforms report. Build metadata is kept but never affects ordering.
pub fn parse(text: &str) -> Option<Version> {
    let t = text.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    // Debian/RPM spell pre-releases with `~` and append a package revision after `-`.
    let debianish = t.contains('~');
    let t = if debianish { t.split_once('-').map_or(t, |(v, _)| v).replace('~', "-") } else { t.to_string() };
    if let Ok(v) = Version::parse(&t) {
        return Some(v);
    }
    // `0.5` → `0.5.0`
    let (core, rest) = match t.find(['-', '+']) {
        Some(i) => t.split_at(i),
        None => (t.as_str(), ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return Version::parse(&format!("{core}.0{rest}")).ok();
    }
    None
}

/// Reads a version as a system package manager reports it, dropping what the packaging adds:
/// an epoch (`1:`), the package revision (`-1`, `-2.fc40`), Debian's `~` pre-release spelling, and
/// a VCS snapshot suffix (`0.3.0.r47.g47f9306` from an AUR `-git` package, read as `0.3.0`).
pub fn parse_package(text: &str) -> Option<Version> {
    let t = text.trim();
    // Epoch.
    let t = match t.split_once(':') {
        Some((epoch, rest)) if !epoch.is_empty() && epoch.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => t,
    };
    // Package revision: whatever follows the last '-' when the part before it is a version.
    let t = match t.rsplit_once('-') {
        Some((upstream, rev)) if !rev.is_empty() && rev.starts_with(|c: char| c.is_ascii_digit()) && upstream.contains('.') => upstream,
        _ => t,
    };
    let t = t.replace('~', "-");
    if let Some(v) = parse(&t) {
        return Some(v);
    }
    // VCS snapshots and other suffixes: the leading X.Y.Z.
    let core: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let parts: Vec<&str> = core.split('.').filter(|p| !p.is_empty()).take(3).collect();
    match parts.as_slice() {
        [a, b, c] => Version::parse(&format!("{a}.{b}.{c}")).ok(),
        [a, b] => Version::parse(&format!("{a}.{b}.0")).ok(),
        // Text around the version, as in `--version` output (`PhotoCraft 0.5.0 (abc, 2026)`):
        // the first `X.Y.Z` run anywhere.
        _ => first_xyz(&t),
    }
}

/// The first `major.minor.patch` run anywhere in `text`.
fn first_xyz(text: &str) -> Option<Version> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes.get(start).is_some_and(u8::is_ascii_digit) || (start > 0 && bytes.get(start - 1).is_some_and(u8::is_ascii_digit)) {
            continue;
        }
        let Some((major, i)) = digits(text, start) else { continue };
        if bytes.get(i) != Some(&b'.') {
            continue;
        }
        let Some((minor, j)) = digits(text, i + 1) else { continue };
        if bytes.get(j) != Some(&b'.') {
            continue;
        }
        let Some((patch, _)) = digits(text, j + 1) else { continue };
        return Some(Version::new(major, minor, patch));
    }
    None
}

/// A run of ASCII digits from `from`, as a number and the index just past it.
fn digits(text: &str, from: usize) -> Option<(u64, usize)> {
    let bytes = text.as_bytes();
    let mut end = from;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == from {
        return None;
    }
    text.get(from..end)?.parse().ok().map(|n| (n, end))
}

/// A version that's a plain `X.Y.Z` with no pre-release suffix.
pub fn is_stable(v: &Version) -> bool {
    v.pre.is_empty()
}

/// Orders by precedence, ignoring build metadata.
pub fn compare(a: &Version, b: &Version) -> Ordering {
    a.cmp_precedence(b)
}

/// True when `latest` is newer than `installed`.
pub fn is_newer(latest: &Version, installed: &Version) -> bool {
    compare(latest, installed) == Ordering::Greater
}

/// The pre-release labels the suite publishes, which sort against releases as semver says
/// (`0.17.0-rc.2` is before `0.17.0`).
const RELEASE_CHANNELS: [&str; 4] = ["alpha", "beta", "rc", "nightly"];

/// Whether `v` is a build made from source rather than a published release: it has a pre-release
/// label the suite never publishes (`0.17.0-integration.20261029.f5b3f123`, `0.18.0-dev`).
pub fn is_local_build(v: &Version) -> bool {
    let label = v.pre.as_str().split('.').next().unwrap_or_default();
    !v.pre.is_empty() && !RELEASE_CHANNELS.iter().any(|channel| label.eq_ignore_ascii_case(channel))
}

/// Whether a copy running `running` should be offered `latest`. As [`is_newer`], except for a
/// local build: semver puts `0.17.0-integration…` before `0.17.0`, but a build from `main` already
/// holds 0.17.0's changes and more, so replacing it with 0.17.0 is a downgrade. A local build is
/// only offered a strictly higher `X.Y.Z`.
pub fn offers_update(latest: &Version, running: &Version) -> bool {
    if is_local_build(running) {
        return (latest.major, latest.minor, latest.patch) > (running.major, running.minor, running.patch);
    }
    is_newer(latest, running)
}

/// Platforms that only record `X.Y.Z` (the MSI ProductVersion, `CFBundleShortVersionString`)
/// drop a pre-release suffix. A numeric-only installed version therefore counts as current when it
/// matches a pre-release's numbers, rather than nagging forever.
pub fn is_update(latest: &Version, installed: &Version) -> bool {
    if installed.pre.is_empty()
        && !latest.pre.is_empty()
        && (latest.major, latest.minor, latest.patch) == (installed.major, installed.minor, installed.patch)
    {
        return false;
    }
    is_newer(latest, installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    fn v(s: &str) -> Version {
        parse(s).unwrap()
    }

    #[trial]
    fn a_build_from_main_is_never_offered_its_own_release() {
        let running = v("0.17.0-integration.20261029.f5b3f123");
        assert!(is_local_build(&running));
        assert!(!offers_update(&v("0.17.0"), &running));
        assert!(!offers_update(&v("0.16.4"), &running));
        assert!(!offers_update(&v("0.17.0-rc.3"), &running));
        assert!(offers_update(&v("0.17.1"), &running));
        assert!(offers_update(&v("0.18.0"), &running));
        assert!(offers_update(&v("1.0.0"), &v("0.18.0-dev")));
        assert!(!offers_update(&v("0.18.0"), &v("0.18.0-DEV.4")));
    }

    #[trial]
    fn orders_versions_and_spots_updates() {
        assert!(is_newer(&v("0.10.0"), &v("0.9.9")));
        assert!(is_newer(&v("0.2.0"), &v("0.2.0-rc.5")));
        assert!(!is_newer(&v("0.2.0+abc"), &v("0.2.0")));
        assert!(is_update(&v("0.5.0"), &v("0.4.1")));
        assert!(!is_update(&v("0.5.0"), &v("0.5.0")));
        assert!(!is_update(&v("0.5.0"), &v("0.6.0")));
        // An MSI that recorded 0.3.0 for 0.3.0-rc.2 is not out of date against 0.3.0-rc.2.
        assert!(!is_update(&v("0.3.0-rc.2"), &v("0.3.0")));
        assert!(is_stable(&v("1.0.0")) && !is_stable(&v("1.0.0-beta.1")));
    }
}
