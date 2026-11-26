//! A stable, crate-prefixed semantic key for presence, automation, undo names and programmatic focus.

/// A document key such as `"craft-ui.find-field"`.
// F2-MERGE: props (`DocKey` is a contract base type no F2 track owns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocKey(&'static str);

impl DocKey {
    /// A key `crate-prefix.name`; evaluated in a const context it rejects a malformed name at compile time. The
    /// prefix is lowercase ASCII letters, digits, hyphens and underscores.
    pub const fn new(name: &'static str) -> DocKey {
        let bytes = name.as_bytes();
        let mut prefix = 0;
        while prefix < bytes.len() && bytes[prefix] != b'.' {
            let byte = bytes[prefix];
            assert!(
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_',
                "a DocKey's crate prefix is lowercase ASCII, digits, hyphens and underscores"
            );
            prefix += 1;
        }
        assert!(prefix > 0 && prefix + 1 < bytes.len(), "a DocKey is `crate-prefix.name`");
        DocKey(name)
    }

    /// The name the key was declared with.
    pub const fn name(self) -> &'static str {
        self.0
    }
}
