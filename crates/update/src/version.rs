//! Release versions: `MAJOR.MINOR.PATCH` with an optional `-prerelease`, as the Craft apps tag
//! them (`v0.6.1`). Build metadata (`+…`) is accepted and ignored, as SemVer says.

use std::cmp::Ordering;
use std::fmt;

/// A parsed release version. Ordering follows SemVer precedence: numbers compare numerically,
/// a prerelease sorts before its release, and prerelease identifiers compare left to right
/// (numeric ones numerically and below alphanumeric ones).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// The dot-separated prerelease identifiers (`rc.1` → `["rc", "1"]`); empty for a release.
    pub pre: Vec<String>,
}

/// Longest version string accepted; tags are short and anything longer is not a version.
const MAX_LEN: usize = 64;

impl Version {
    /// Parses `0.6.1`, `v0.6.1`, `V0.6.1`, `0.7.0-rc.1` or `0.6.1+build`. Leading zeros, empty
    /// parts, missing parts and characters outside `[0-9A-Za-z.-]` are refused.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_LEN {
            return None;
        }
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(core, _build)| core);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let major = number(parts.next()?)?;
        let minor = number(parts.next()?)?;
        let patch = number(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let ids: Vec<String> = pre.split('.').map(str::to_string).collect();
                let valid = |id: &String| {
                    !id.is_empty()
                        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        && !(id.len() > 1 && id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
                };
                if !ids.iter().all(valid) {
                    return None;
                }
                ids
            }
        };
        Some(Version { major, minor, patch, pre })
    }

    /// Whether this is a prerelease (`0.7.0-rc.1`).
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Whether `self` is newer than `other`.
    pub fn is_newer_than(&self, other: &Version) -> bool {
        self > other
    }
}

fn number(part: &str) -> Option<u64> {
    if part.is_empty() || (part.len() > 1 && part.starts_with('0')) || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => compare_pre(&self.pre, &other.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_pre(a: &[String], b: &[String]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn tags_and_plain_versions_parse() {
        assert_eq!(v("v0.6.1"), Version { major: 0, minor: 6, patch: 1, pre: vec![] });
        assert_eq!(v("0.6.1"), v("V0.6.1"));
        assert_eq!(v(" 1.2.3 "), v("1.2.3"));
        assert_eq!(v("1.2.3+linux.x86"), v("1.2.3"));
        assert_eq!(v("0.7.0-rc.1").pre, vec!["rc".to_string(), "1".to_string()]);
        assert_eq!(v("0.7.0-rc.1").to_string(), "0.7.0-rc.1");
        assert_eq!(v("v10.20.30").to_string(), "10.20.30");
    }

    #[test]
    fn malformed_versions_are_refused() {
        for bad in [
            "",
            "v",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.x",
            "a.b.c",
            "1.2.3-",
            "1.2.3-rc..1",
            "1.2.3-01",
            "1.2.3-rc_1",
            "-1.2.3",
            "1.2.3 ; rm -rf /",
            "1.2.3\n4",
            "1..3",
            "99999999999999999999.0.0",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::parse(&format!("1.2.3-{}", "a".repeat(80))), None);
    }

    #[test]
    fn ordering_follows_semver_precedence() {
        let ordered = [
            "0.4.0",
            "0.5.0",
            "0.6.0-alpha",
            "0.6.0-alpha.1",
            "0.6.0-alpha.beta",
            "0.6.0-beta.2",
            "0.6.0-beta.11",
            "0.6.0-rc.1",
            "0.6.0",
            "0.6.1",
            "0.10.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[1]).is_newer_than(&v(pair[0])), "{} should be newer than {}", pair[1], pair[0]);
            assert!(!v(pair[0]).is_newer_than(&v(pair[1])));
        }
        assert!(!v("0.6.1").is_newer_than(&v("v0.6.1")));
        assert!(v("0.6.1").is_newer_than(&v("0.4.0")));
        assert!(v("0.7.0-rc.1").is_prerelease() && !v("0.7.0").is_prerelease());
    }
}
