//! The error code a platform or protocol gave, kept beside the neutral classification.

use std::fmt;

/// The [`NativeCode::domain`] of a Windows HRESULT.
pub const HRESULT_DOMAIN: &str = "hresult";

/// A native error code and the domain that owns its numbering.
///
/// `domain` is a short lowercase name chosen by the backend: `"hresult"` (the code is the unsigned
/// HRESULT), `"win32"`, `"sane"`, `"http"`, `"ipp"`, `"wsd-fault"`, `"cups"`, `"nserror"`. `name`
/// is the symbolic name when the backend knows one (`WIA_ERROR_BUSY`, `SANE_STATUS_NO_DOCS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NativeCode {
    pub domain: &'static str,
    pub code: i64,
    pub name: Option<&'static str>,
}

impl NativeCode {
    /// A code in `domain`, with its symbolic `name` when known. For an HRESULT use [`Self::hresult`].
    pub const fn new(domain: &'static str, code: i64, name: Option<&'static str>) -> Self {
        Self { domain, code, name }
    }

    /// A Windows HRESULT (domain [`HRESULT_DOMAIN`]); the code is kept as the unsigned value.
    pub const fn hresult(code: u32, name: Option<&'static str>) -> Self {
        Self { domain: HRESULT_DOMAIN, code: code as i64, name }
    }

    /// Whether this code is an HRESULT.
    pub fn is_hresult(&self) -> bool {
        self.domain == HRESULT_DOMAIN
    }
}

/// `hresult 0x80004005 (E_FAIL)`, `http 503`, `sane 5 (SANE_STATUS_NO_DOCS)`.
impl fmt::Display for NativeCode {
    /// Writes `domain code (name)`; an HRESULT code is shown as eight hex digits when it fits in `u32`,
    /// otherwise as the decimal value it really holds (never a truncated one).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let (true, Ok(code)) = (self.is_hresult(), u32::try_from(self.code)) {
            write!(f, "{} {:#010x}", self.domain, code)?;
        } else {
            write!(f, "{} {}", self.domain, self.code)?;
        }
        match self.name {
            Some(name) => write!(f, " ({name})"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_hresult_agree_on_the_fields() {
        let a = NativeCode::new("http", 503, Some("Service Unavailable"));
        assert_eq!((a.domain, a.code, a.name), ("http", 503, Some("Service Unavailable")));
        let h = NativeCode::hresult(0x8000_4005, Some("E_FAIL"));
        assert_eq!(h, NativeCode::new(HRESULT_DOMAIN, 0x8000_4005, Some("E_FAIL")));
        assert!(h.is_hresult());
        assert!(!a.is_hresult());
        assert_eq!(h.code, 2_147_500_037, "kept unsigned, not sign-extended");
    }

    #[test]
    fn display_is_stable() {
        assert_eq!(NativeCode::hresult(0x8000_4005, Some("E_FAIL")).to_string(), "hresult 0x80004005 (E_FAIL)");
        assert_eq!(NativeCode::hresult(0x0000_0005, None).to_string(), "hresult 0x00000005");
        assert_eq!(NativeCode::new("http", 503, None).to_string(), "http 503");
        assert_eq!(NativeCode::new("sane", 5, Some("SANE_STATUS_NO_DOCS")).to_string(), "sane 5 (SANE_STATUS_NO_DOCS)");
        assert_eq!(NativeCode::new("win32", -1, None).to_string(), "win32 -1");
    }

    #[test]
    fn an_hresult_domain_code_outside_u32_is_shown_whole_not_truncated() {
        assert_eq!(NativeCode::new(HRESULT_DOMAIN, -1, None).to_string(), "hresult -1");
        assert_eq!(NativeCode::new(HRESULT_DOMAIN, 0x1_0000_0005, None).to_string(), "hresult 4294967301");
        assert_eq!(NativeCode::new(HRESULT_DOMAIN, i64::from(u32::MAX), None).to_string(), "hresult 0xffffffff");
    }

    #[test]
    fn is_copy_and_hashable() {
        let a = NativeCode::new("ipp", 1, None);
        let b = a;
        let set: std::collections::HashSet<_> = [a, b].into_iter().collect();
        assert_eq!(set.len(), 1);
    }
}
