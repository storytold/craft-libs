//! Semver precedence for the update check. Only what the check needs: parse
//! `MAJOR[.MINOR[.PATCH]][-pre][+build]` and say whether one version is strictly newer than another.
//! A missing minor or patch counts as 0, because v1 plugins write `1.0`. A string that does not
//! parse is never newer than anything, so the caller asks the user.

use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
struct Version<'a> {
    core: [u64; 3],
    pre: Vec<&'a str>,
}

fn numeric(s: &str) -> Option<u64> {
    // Semver forbids leading zeros in numeric parts.
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))).then(|| s.parse().ok()).flatten()
}

fn parse(text: &str) -> Option<Version<'_>> {
    let text = text.split('+').next().unwrap_or(text);
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let mut parts = core.split('.');
    let major = numeric(parts.next()?)?;
    let minor = parts.next().map_or(Some(0), numeric)?;
    let patch = parts.next().map_or(Some(0), numeric)?;
    if parts.next().is_some() {
        return None;
    }
    let pre = match pre {
        None => Vec::new(),
        Some(p) => {
            let ids: Vec<&str> = p.split('.').collect();
            if ids.iter().any(|i| i.is_empty() || !i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                return None;
            }
            ids
        }
    };
    Some(Version { core: [major, minor, patch], pre })
}

fn pre_cmp(a: &[&str], b: &[&str]) -> Ordering {
    // A release (no pre-release part) ranks above any pre-release of the same core.
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    for (x, y) in a.iter().zip(b) {
        let order = match (x.bytes().all(|c| c.is_ascii_digit()), y.bytes().all(|c| c.is_ascii_digit())) {
            (true, true) => match (x.parse::<u128>(), y.parse::<u128>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                _ => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
            },
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

/// True when `new` has strictly higher semver precedence than `old`. Build metadata is ignored.
/// False when either string is not a valid semver version.
pub fn is_newer(old: &str, new: &str) -> bool {
    match (parse(old), parse(new)) {
        (Some(o), Some(n)) => n.core.cmp(&o.core).then_with(|| pre_cmp(&n.pre, &o.pre)) == Ordering::Greater,
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn higher_core_is_newer() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.9.9", "2.0.0"));
        assert!(is_newer("1.2.3", "1.10.0"), "numeric, not text, order");
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn short_versions_pad_with_zeros() {
        assert!(is_newer("1.0", "2.0"));
        assert!(is_newer("1", "1.0.1"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(!is_newer("1", "1.0"));
        assert!(is_newer("1.9", "1.10"));
    }

    #[test]
    fn same_version_is_not_newer() {
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0+a", "1.0.0+b"), "build metadata does not count");
        assert!(!is_newer("1.0.0-rc.1", "1.0.0-rc.1"));
    }

    #[test]
    fn prerelease_ranks_below_release() {
        assert!(is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-alpha", "1.0.0-beta"));
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.10"));
        assert!(is_newer("1.0.0-1", "1.0.0-alpha"), "numeric ids rank below text ids");
        assert!(is_newer("1.0.0-alpha", "1.0.0-alpha.1"), "a longer id list ranks higher");
    }

    #[test]
    fn invalid_strings_are_never_newer() {
        for bad in [
            "",
            "1.0.0.0",
            "1..0",
            "1.",
            ".1",
            "01.0.0",
            "1.0.00",
            "v1.0.0",
            "1.0.x",
            "1.0.0-",
            "1.0.0-a..b",
            " 1.0.0",
            "1.0.0 ",
            "\u{661}.0.0",
            "-1.0.0",
        ] {
            assert!(!is_newer("1.0.0", bad), "{bad:?} as new");
            assert!(!is_newer(bad, "9.9.9"), "{bad:?} as old");
        }
    }

    #[test]
    fn huge_numbers_do_not_panic() {
        assert!(!is_newer("1.0.0", "99999999999999999999999.0.0"));
        assert!(is_newer("1.0.0-99999999999999999999999999999999999999999", "1.0.0-100000000000000000000000000000000000000000"));
    }
}
