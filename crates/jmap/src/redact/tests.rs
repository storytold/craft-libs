use super::*;

fn redactor(secrets: &[&str]) -> Redactor {
    let mut redactor = Redactor::new();
    for secret in secrets {
        redactor.add(secret);
    }
    redactor
}

// Requirement 3.5: one line, no control characters, no invisible characters.
#[test]
fn one_line_no_controls() {
    let excerpt = Redactor::new().excerpt("  line one\r\nline\ttwo\u{0}\u{1b}[31m red \u{202E}evil\u{2028}end\u{200B}\u{FEFF}!  ");
    assert_eq!(excerpt.as_str(), "line one line two [31m red evil end!");
}

// Requirement 3.5: bounded length, with an ellipsis when cut.
#[test]
fn bounded_length() {
    let excerpt = Redactor::new().excerpt(&"word ".repeat(500));
    assert!(excerpt.as_str().chars().count() <= EXCERPT_MAX_CHARS + 1);
    assert!(excerpt.as_str().ends_with('…'));
    let marks = redactor(&["secret"]).excerpt(&"a secret ".repeat(100));
    assert!(marks.as_str().chars().count() <= EXCERPT_MAX_CHARS + 1);
    assert_eq!(Redactor::new().excerpt("short").as_str(), "short");
    let multibyte = "é".repeat(EXCERPT_MAX_CHARS * 2);
    assert_eq!(Redactor::new().excerpt(&multibyte).as_str().chars().count(), EXCERPT_MAX_CHARS + 1);
}

// Requirement 3.5: every known secret is replaced, whatever its case or written form.
#[test]
fn secrets_are_replaced() {
    let mut redactor = redactor(&["s3cret-pass", "open sesame"]);
    redactor.add_personal("john@example.com");
    redactor.add_personal("https://jmap.example.com/session?t=abc");
    let excerpt = redactor.excerpt("bad password s3cret-pass for JOHN@EXAMPLE.COM (john%40example.com)");
    assert_eq!(excerpt.as_str(), "bad password [redacted] for [redacted] ([redacted])");
    let excerpt = redactor.excerpt("fetch https://jmap.example.com/session?t=abc failed");
    assert_eq!(excerpt.as_str(), "fetch [redacted] failed");
    for disguised in ["open\nsesame", "open  sesame", "open\u{a0}sesame", "OPEN\tSESAME", "open+sesame", "open%20sesame", "open\u{200B} sesame"] {
        let excerpt = redactor.excerpt(&format!("got {disguised} here"));
        assert_eq!(excerpt.as_str(), "got [redacted] here", "{disguised:?}");
    }
}

// Requirement 3.5: secrets are replaced before cutting, so one straddling the cut never shows in part.
#[test]
fn secret_across_the_cut_is_not_shown() {
    let secret = "VerySecretToken0123456789";
    let redactor = redactor(&[secret]);
    for prefix_len in 140..170 {
        let text = format!("{}{}{}", "x".repeat(prefix_len), secret, "y".repeat(50));
        let excerpt = redactor.excerpt(&text);
        for cut in 4..secret.len() {
            assert!(!excerpt.as_str().contains(&secret[..cut]), "prefix {prefix_len} shows {cut} octets");
        }
    }
    // A long secret repeated so that the scan window ends inside one of its occurrences, while the output
    // (each occurrence shrinking to "[redacted]") still reaches that far.
    let long: String = (0..900u32).map(|i| char::from(b'a' + u8::try_from(i * 7 % 26).unwrap())).collect();
    let redactor = redactor_with(&long);
    let text = format!("{long} ").repeat(20);
    let excerpt = redactor.excerpt(&text);
    for start in 0..long.len() - 8 {
        assert!(!excerpt.as_str().contains(&long[start..start + 8]), "{}", excerpt.as_str());
    }
}

fn redactor_with(secret: &str) -> Redactor {
    let mut redactor = Redactor::new();
    redactor.add(secret);
    redactor
}

// Requirement 3.5: overlapping secrets are covered together.
#[test]
fn overlapping_secrets() {
    let both = redactor(&["abcdef", "defghi"]);
    assert_eq!(both.excerpt("xx abcdefghi yy").as_str(), "xx [redacted] yy");
    let repeated = redactor(&["aaaa"]);
    assert_eq!(repeated.excerpt("baaaaab").as_str(), "b[redacted]b");
}

// Requirement 3.5 (RFC 7617 §2 passwords may be short): a one- or two-octet secret cannot be hidden by
// replacement, so the text is withheld; short personal data is simply not looked for.
#[test]
fn short_secrets() {
    assert_eq!(redactor(&["ab"]).excerpt("nothing to see").as_str(), WITHHELD);
    let mut personal = Redactor::new();
    personal.add_personal("x");
    assert_eq!(personal.excerpt("x marks the spot").as_str(), "x marks the spot");
}

// RFC 8620 §8.5 (resource exhaustion): matching is linear, even for a periodic secret and text.
#[test]
fn matching_is_linear() {
    let mut redactor = Redactor::new();
    redactor.add_personal(&"a".repeat(1024));
    let text = "a".repeat(64 * 1024);
    for _ in 0..200 {
        assert_eq!(redactor.excerpt(&text).as_str(), "[redacted]…");
    }
}

// Requirement 3.5: identifiers that echo a secret can be detected.
#[test]
fn mentions_secret() {
    let redactor = redactor(&["hunter22"]);
    assert!(redactor.mentions_secret("xhunter22x"));
    assert!(!redactor.mentions_secret("serverFail"));
}

// Requirement 2.3: the redactor's debug form says nothing about its secrets.
#[test]
fn debug_hides_secrets() {
    let redactor = redactor(&["s3cret-pass"]);
    assert!(!format!("{redactor:?}").contains("s3cret"));
}

// KMP itself, against a naive search.
#[test]
fn kmp_finds_every_occurrence() {
    for (hay, needle) in [("aaaaa", "aa"), ("abababab", "abab"), ("xyz", "xyzw"), ("abcabcab", "cab"), ("", "a")] {
        let mut found = Vec::new();
        for_each_match(hay.as_bytes(), needle.as_bytes(), |start| found.push(start));
        let expected: Vec<usize> = (0..hay.len()).filter(|&i| hay[i..].starts_with(needle)).collect();
        assert_eq!(found, expected, "{hay} / {needle}");
    }
}
