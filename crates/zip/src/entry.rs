//! Validation and sanitization of ZIP archive entry paths.
//!
//! Rejects zip-slip path traversal vectors:
//! - Parent traversal components (`..`)
//! - Leading directory separators (`/` or `\`), indicating absolute paths
//! - Windows drive prefixes (`C:`, `D:`, etc.) at the start or within components
//! - Null bytes (`\0`)
//!
//! Checks slice tokens directly without allocating memory.

/// Whether `path` contains any path-traversal components or zip-slip vectors.
///
/// Rejects:
/// - Parent directory traversal (`..`) in any path segment
/// - Null bytes (`\0`)
/// - Leading separators (`/` or `\`)
/// - Windows drive letters (`C:`, etc.) at the start or in any component
///
/// This check inspects slice tokens directly without allocating heap memory.
pub fn has_traversal_component(path: &str) -> bool {
    has_traversal_component_bytes(path.as_bytes())
}

/// Byte-level check for path-traversal components without allocating heap memory.
pub fn has_traversal_component_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    // Reject leading separators: '/' or '\'
    if bytes.starts_with(b"/") || bytes.starts_with(b"\\") {
        return true;
    }
    // Tokenize by '/' and '\' directly on byte slices
    let mut rest = bytes;
    while !rest.is_empty() {
        let pos = rest.iter().position(|&b| b == b'/' || b == b'\\').unwrap_or(rest.len());
        let token = &rest[..pos];

        // Parent traversal component
        if token == b".." {
            return true;
        }

        // Windows drive prefix: e.g. "C:" or "C:test.txt" or in any component
        if token.len() >= 2 && token[0].is_ascii_alphabetic() && token[1] == b':' {
            return true;
        }

        if pos == rest.len() {
            break;
        }
        rest = &rest[pos + 1..];
    }
    false
}

/// Sanitize and validate an entry path, rejecting any zip-slip traversal vectors.
///
/// Returns `None` if `path` is empty or contains parent traversal components (`..`),
/// null bytes, or Windows drive prefixes (`C:`).
/// Strips leading separators (`/` or `\`) to ensure the path is relative.
pub fn sanitize_path(path: &str) -> Option<&str> {
    let trimmed = path.trim_start_matches(['/', '\\']);
    if trimmed.is_empty() || has_traversal_component(trimmed) { None } else { Some(trimmed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_parent_traversal() {
        assert!(has_traversal_component(".."));
        assert!(has_traversal_component("../"));
        assert!(has_traversal_component("..\\"));
        assert!(has_traversal_component("../../etc/passwd"));
        assert!(has_traversal_component(r"..\..\windows\system32"));
        assert!(has_traversal_component("foo/../bar"));
        assert!(has_traversal_component(r"foo\..\bar"));
        assert!(has_traversal_component("foo/bar/.."));
        assert!(has_traversal_component("foo/bar/../"));
        assert!(has_traversal_component("a/b/../../c"));
    }

    #[test]
    fn detects_absolute_and_leading_separators() {
        assert!(has_traversal_component("/etc/passwd"));
        assert!(has_traversal_component(r"\windows\system32"));
        assert!(has_traversal_component("//server/share"));
        assert!(has_traversal_component(r"\\server\share"));
        assert!(has_traversal_component("/"));
        assert!(has_traversal_component("\\"));
    }

    #[test]
    fn sanitizes_leading_separators() {
        assert_eq!(sanitize_path("/etc/passwd"), Some("etc/passwd"));
        assert_eq!(sanitize_path(r"\windows\system32"), Some(r"windows\system32"));
        assert_eq!(sanitize_path("//server/share"), Some("server/share"));
        assert_eq!(sanitize_path(r"\\server\share"), Some(r"server\share"));
        assert_eq!(sanitize_path("/"), None);
        assert_eq!(sanitize_path("\\"), None);
    }

    #[test]
    fn detects_windows_drive_letters() {
        assert!(has_traversal_component("C:/test.txt"));
        assert!(has_traversal_component(r"C:\test.txt"));
        assert!(has_traversal_component("c:/test.txt"));
        assert!(has_traversal_component(r"c:\windows\system32"));
        assert!(has_traversal_component("D:test.txt"));
        assert!(has_traversal_component("foo/C:/bar"));
        assert!(has_traversal_component(r"foo\D:\bar"));
    }

    #[test]
    fn detects_null_bytes() {
        assert!(has_traversal_component("foo\0bar"));
        assert!(has_traversal_component("\0"));
        assert!(has_traversal_component_bytes(b"foo/bar\x00baz"));
    }

    #[test]
    fn accepts_safe_relative_paths() {
        assert!(!has_traversal_component("mimetype"));
        assert!(!has_traversal_component("dir/file.txt"));
        assert!(!has_traversal_component(r"dir\file.txt"));
        assert!(!has_traversal_component("dir/"));
        assert!(!has_traversal_component("a/b/c/d.bin"));
        assert!(!has_traversal_component("."));
        assert!(!has_traversal_component("./file.txt"));
        assert!(!has_traversal_component("foo..bar"));
        assert!(!has_traversal_component("..foo"));
        assert!(!has_traversal_component("foo.."));
        assert!(!has_traversal_component(""));
    }

    #[test]
    fn sanitize_path_validation() {
        assert_eq!(sanitize_path("../../etc/passwd"), None);
        assert_eq!(sanitize_path(r"..\..\windows\system32"), None);
        assert_eq!(sanitize_path("C:/test.txt"), None);
        assert_eq!(sanitize_path("/etc/shadow"), Some("etc/shadow"));
        assert_eq!(sanitize_path(""), None);
        assert_eq!(sanitize_path("safe/path.txt"), Some("safe/path.txt"));
        assert_eq!(sanitize_path("mimetype"), Some("mimetype"));
    }
}
