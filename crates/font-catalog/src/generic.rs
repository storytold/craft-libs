//! Generic families: the installed family to use for sans-serif, serif, monospace or interface
//! text when a caller has no family name, or the name it has is not installed.

use crate::{Catalog, Os};

/// A generic family ([`Catalog::generic_family`]), as CSS names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Generic {
    /// `sans-serif`.
    SansSerif,
    /// `serif`.
    Serif,
    /// `monospace`.
    Monospace,
    /// `system-ui`: the face the platform draws its own interface with.
    SystemUi,
}

impl Generic {
    /// Every generic family.
    pub const ALL: [Generic; 4] = [Generic::SansSerif, Generic::Serif, Generic::Monospace, Generic::SystemUi];

    /// The CSS keyword (`sans-serif`, `serif`, `monospace`, `system-ui`).
    pub fn css_name(self) -> &'static str {
        match self {
            Generic::SansSerif => "sans-serif",
            Generic::Serif => "serif",
            Generic::Monospace => "monospace",
            Generic::SystemUi => "system-ui",
        }
    }
}

/// The platforms in the order their lists are tried after the catalog's own.
const OSES: [Os; 4] = [Os::Windows, Os::MacOs, Os::Unix, Os::Haiku];

/// The families tried for `generic` on `os`, most likely first.
fn names(generic: Generic, os: Os) -> &'static [&'static str] {
    match (generic, os) {
        (Generic::SansSerif, Os::Windows) => &["Arial", "Segoe UI", "Tahoma", "Verdana"],
        (Generic::SansSerif, Os::MacOs) => &["Helvetica", "Helvetica Neue", "Arial"],
        (Generic::SansSerif, Os::Unix) => &["DejaVu Sans", "Noto Sans", "Liberation Sans", "Cantarell", "Ubuntu", "FreeSans"],
        (Generic::SansSerif, Os::Haiku) => &["Noto Sans", "DejaVu Sans"],
        (Generic::Serif, Os::Windows) => &["Times New Roman", "Cambria", "Georgia"],
        (Generic::Serif, Os::MacOs) => &["Times", "Times New Roman", "New York"],
        (Generic::Serif, Os::Unix) => &["DejaVu Serif", "Noto Serif", "Liberation Serif", "FreeSerif"],
        (Generic::Serif, Os::Haiku) => &["Noto Serif", "DejaVu Serif"],
        (Generic::Monospace, Os::Windows) => &["Consolas", "Cascadia Mono", "Courier New"],
        (Generic::Monospace, Os::MacOs) => &["Menlo", "SF Mono", "Monaco", "Courier", "Courier New"],
        (Generic::Monospace, Os::Unix) => &["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono", "FreeMono"],
        (Generic::Monospace, Os::Haiku) => &["Noto Sans Mono", "DejaVu Sans Mono"],
        (Generic::SystemUi, Os::Windows) => &["Segoe UI", "Tahoma"],
        (Generic::SystemUi, Os::MacOs) => &[".SF NS", "SF Pro", "SF Pro Text", "Helvetica Neue", "Lucida Grande"],
        (Generic::SystemUi, Os::Unix) => &["Cantarell", "Ubuntu", "Noto Sans", "DejaVu Sans"],
        (Generic::SystemUi, Os::Haiku) => &["Noto Sans"],
        (_, Os::None) => &[],
    }
}

/// The names [`Catalog::generic_family`] tries, in order: the font service's (`service`) and the
/// list for `os`, then the other platforms' lists. For [`Generic::SystemUi`], the font service's
/// and `os`'s sans-serif names (`sans_service`) come before the other platforms' lists, and their
/// sans-serif lists come last.
fn candidates<'a>(generic: Generic, os: Os, service: &'a [String], sans_service: &'a [String]) -> Vec<&'a str> {
    let others = |generic: Generic| OSES.into_iter().filter(move |o| *o != os).flat_map(move |o| names(generic, o).iter().copied());
    let mut out: Vec<&str> = service.iter().map(String::as_str).chain(names(generic, os).iter().copied()).collect();
    if generic == Generic::SystemUi {
        out.extend(sans_service.iter().map(String::as_str).chain(names(Generic::SansSerif, os).iter().copied()));
        out.extend(others(generic).chain(others(Generic::SansSerif)));
    } else {
        out.extend(others(generic));
    }
    out
}

impl Catalog {
    /// The installed family for `generic`, as the catalog names it.
    ///
    /// With the font service on ([`Catalog::with_font_service`]), the families the platform's
    /// service lists for `generic` come first (fontconfig's configured aliases on Linux and
    /// FreeBSD, fixed names on macOS and Windows). A fixed list for the catalog's platform
    /// ([`crate::FontEnv::os`]) follows, then the other platforms' lists. [`Generic::SystemUi`]
    /// tries the service's and the platform's sans-serif families before the other platforms'
    /// lists. The first of these names that [`Catalog::resolve`] finds wins, so a legacy family
    /// name finds its family. Faces added with [`Catalog::add_bytes`] or [`Catalog::add_file`]
    /// count as installed. Returns `None` when none of the names is installed. Pass the family
    /// to [`Catalog::resolve`] for a face in a style.
    pub fn generic_family(&self, generic: Generic) -> Option<String> {
        let service = |generic| if self.uses_font_service() { crate::system::generic_family_names(generic) } else { Vec::new() };
        let (own, sans) = (service(generic), if generic == Generic::SystemUi { service(Generic::SansSerif) } else { Vec::new() });
        candidates(generic, self.env().os, &own, &sans).into_iter().find_map(|name| self.resolve(name, "").map(|(face, _)| face.family))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_put_the_service_first_then_this_platform_then_the_others() {
        let service = ["Configured Sans".to_string()];
        let tried = candidates(Generic::SansSerif, Os::Unix, &service, &[]);
        assert_eq!(&tried[..3], ["Configured Sans", "DejaVu Sans", "Noto Sans"]);
        let windows = tried.iter().position(|n| *n == "Arial");
        let mac = tried.iter().position(|n| *n == "Helvetica");
        assert!(windows.is_some() && mac.is_some() && windows < mac, "the other platforms' lists follow in a fixed order");
        let ui = candidates(Generic::SystemUi, Os::Windows, &[], &service);
        assert_eq!(&ui[..4], ["Segoe UI", "Tahoma", "Configured Sans", "Arial"], "this platform's sans-serif names follow its system-ui names");
        let (cantarell, dejavu) = (ui.iter().position(|n| *n == "Cantarell"), ui.iter().rposition(|n| *n == "DejaVu Sans"));
        assert!(cantarell < dejavu, "the other platforms' system-ui names come before their sans-serif names");
        assert!(candidates(Generic::Serif, Os::None, &[], &[]).contains(&"Times New Roman"));
    }

    #[test]
    fn every_generic_has_names_on_every_platform_with_font_folders() {
        for generic in Generic::ALL {
            for os in OSES {
                assert!(!names(generic, os).is_empty(), "{generic:?} on {os:?}");
            }
            assert!(names(generic, Os::None).is_empty());
        }
        assert_eq!(Generic::ALL.map(Generic::css_name), ["sans-serif", "serif", "monospace", "system-ui"]);
    }
}
