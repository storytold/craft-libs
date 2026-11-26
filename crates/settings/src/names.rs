//! The name rules shared by presets and profiles: a name becomes a file name on every platform.

/// The longest name, in characters.
pub(crate) const MAX_NAME_CHARS: usize = 100;

/// Why a name can't be used. Each store maps it to its own `BadName`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameFault {
    Empty,
    TooLong,
    /// A control character, a path separator or a character Windows reserves.
    ReservedCharacter,
    /// A leading or trailing dot.
    Dot,
    /// A Windows device name (`CON`, `COM1`, ...).
    DeviceName,
}

/// `name` trimmed, or the reason it can't be a file name on every platform.
pub(crate) fn checked_name(name: &str) -> Result<String, NameFault> {
    let name = name.trim();
    let reserved = |c: char| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let windows_device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    if name.is_empty() {
        Err(NameFault::Empty)
    } else if name.chars().count() > MAX_NAME_CHARS {
        Err(NameFault::TooLong)
    } else if name.contains(reserved) {
        Err(NameFault::ReservedCharacter)
    } else if name.ends_with('.') || name.starts_with('.') {
        Err(NameFault::Dot)
    } else if windows_device {
        Err(NameFault::DeviceName)
    } else {
        Ok(name.to_owned())
    }
}
