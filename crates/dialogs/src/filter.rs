//! File-type filters that match extensions in any letter case.
//!
//! On Linux and the BSDs rfd turns each extension into a `*.ext` glob for the desktop portal, GTK
//! and zenity, which all match case-sensitively: a filter of `nef` hid `DSC_1234.NEF`. Windows and
//! macOS ignore case already.

/// A file type a dialog offers: its name and its extensions, without the dot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl Filter {
    pub fn new(name: impl Into<String>, extensions: &[&str]) -> Self {
        Self { name: name.into(), extensions: extensions.iter().map(|e| (*e).to_owned()).collect() }
    }

    /// The patterns to hand rfd on this platform.
    pub(crate) fn patterns(&self) -> Vec<String> {
        patterns(&self.extensions, cfg!(all(unix, not(target_vendor = "apple"))))
    }
}

/// The patterns for `extensions`. Without `globs` (Windows, macOS): each extension once, in lower
/// case. With `globs` (rfd formats each pattern as `*.{pattern}`): the lower-case form, the
/// upper-case form (for a portal backend without `[...]` classes) and a class per letter, such as
/// `[nN][eE][fF]`, that matches every mix.
pub(crate) fn patterns(extensions: &[String], globs: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    for ext in extensions {
        let lower = ext.trim_start_matches('.').to_lowercase();
        if !globs {
            push(lower);
            continue;
        }
        push(lower.clone());
        push(lower.to_uppercase());
        // An extension that is already glob syntax stays as it is.
        if lower.chars().any(char::is_alphabetic) && !lower.contains(['[', ']', '*', '?', '\\']) {
            push(
                lower
                    .chars()
                    .map(|c| {
                        let up: String = c.to_uppercase().collect();
                        if up.chars().count() == 1 && up != c.to_string() { format!("[{c}{up}]") } else { c.to_string() }
                    })
                    .collect(),
            );
        }
    }
    out
}
