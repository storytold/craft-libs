//! Id checks shared by the instance record, the moniker and the link record.
//! `std` only, so the pure modules can use it.

/// App id: `^[a-z][a-z0-9-]{1,31}$`.
pub fn valid_app_id(app: &str) -> bool {
    let bytes = app.as_bytes();
    (2..=32).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// UUID text in 8-4-4-4-12 form. Either hex case.
pub fn valid_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| if matches!(index, 8 | 13 | 18 | 23) { *byte == b'-' } else { byte.is_ascii_hexdigit() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids() {
        for ok in ["ec", "filmcraft", "a-b-1", &"a".repeat(32)] {
            assert!(valid_app_id(ok), "{ok}");
        }
        for bad in ["", "a", "A1", "1a", "a_b", "Film", &"a".repeat(33)] {
            assert!(!valid_app_id(bad), "{bad}");
        }
    }

    #[test]
    fn uuids() {
        assert!(valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(valid_uuid("550E8400-E29B-41D4-A716-446655440000"));
        assert!(!valid_uuid("550e8400e29b41d4a716446655440000"));
        assert!(!valid_uuid("550e8400-e29b-41d4-a716-44665544000g"));
        assert!(!valid_uuid(""));
    }
}
