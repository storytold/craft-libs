use super::*;
use crate::test_support::made_secret;

// RFC 7617 §2: the "Aladdin" / "open sesame" example.
#[test]
fn basic_example() {
    let credentials = Credentials::basic("Aladdin", &made_secret("open sesame")).unwrap();
    assert_eq!(credentials.authorization().expose(), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
}

// RFC 7617 §2.1: the UTF-8 example ("test" / "123£").
#[test]
fn basic_utf8_example() {
    let credentials = Credentials::basic("test", &made_secret("123\u{A3}")).unwrap();
    assert_eq!(credentials.authorization().expose(), "Basic dGVzdDoxMjPCow==");
}

// RFC 4648 §10: the base64 test vectors.
#[test]
fn base64_vectors() {
    for (input, expected) in
        [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
    {
        let mut out = String::new();
        base64_into(input.as_bytes(), &mut out);
        assert_eq!(out, expected);
    }
}

// RFC 7617 §2: no colon in the user name, no control characters anywhere.
#[test]
fn basic_refusals() {
    assert_eq!(Credentials::basic("", &made_secret("x")).unwrap_err(), CredentialsError::EmptyUser);
    assert_eq!(Credentials::basic("a:b", &made_secret("x")).unwrap_err(), CredentialsError::ColonInUser);
    assert_eq!(Credentials::basic("user", &made_secret("pa\nss")).unwrap_err(), CredentialsError::ControlCharacter);
    assert_eq!(Credentials::basic("us\u{7f}er", &made_secret("pass")).unwrap_err(), CredentialsError::ControlCharacter);
    assert!(Credentials::basic("user", &made_secret("with:colon")).is_ok());
}

// RFC 6750 §2.1: the specification's example token, and an empty token.
#[test]
fn bearer_tokens() {
    let credentials = Credentials::bearer("mF_9.B5f-4.1JqM").unwrap();
    assert_eq!(credentials.authorization().expose(), "Bearer mF_9.B5f-4.1JqM");
    assert_eq!(Credentials::bearer("").unwrap_err(), CredentialsError::EmptyToken);
}

// RFC 6750 §2.1, rule 1: the body is made of ALPHA, DIGIT and the six marks - . _ ~ + /.
#[test]
fn token_body_octets() {
    for token in ["AZaz09", "a-b", "a.b", "a_b", "a~b", "a+b", "a/b", "-._~+/"] {
        assert!(is_bearer_token(token), "{token}");
    }
    for octet in 0u8..=255 {
        let expected = octet.is_ascii_alphanumeric() || b"-._~+/".contains(&octet);
        assert_eq!(token_octet(octet) == TokenOctet::Body, expected, "{octet}");
    }
}

// RFC 6750 §2.1, rule 2: at least one body octet ("1*").
#[test]
fn token_needs_a_body() {
    for token in ["", "=", "==="] {
        assert!(!is_bearer_token(token), "{token:?}");
    }
    assert_eq!(Credentials::bearer("==").unwrap_err(), CredentialsError::InvalidToken);
}

// RFC 6750 §2.1, rule 3: "=" only at the end, any number of times ("*\"=\"").
#[test]
fn token_padding_only_at_the_end() {
    for token in ["abc=", "abc==", "abc===="] {
        assert!(is_bearer_token(token), "{token}");
    }
    for token in ["=abc", "a=b", "ab=c=", "ab==c"] {
        assert!(!is_bearer_token(token), "{token}");
    }
}

// RFC 6750 §2.1, rule 4: nothing else, which also rules out header injection (RFC 9110 §5.5).
#[test]
fn token_refused_octets() {
    for token in ["a b", "a\r\nX-Injected: 1", "a,b", "a\"b", "a:b", "tok\u{e9}n", "a\tb", "a;b"] {
        assert!(!is_bearer_token(token), "{token:?}");
        assert_eq!(Credentials::bearer(token).unwrap_err(), CredentialsError::InvalidToken, "{token:?}");
    }
}

// Requirement 2.3: no secret and no user name in the debug form.
#[test]
fn debug_shows_no_secret() {
    let basic = Credentials::basic("john@example.com", &made_secret("hunter2-long")).unwrap();
    let shown = format!("{basic:?}");
    assert_eq!(shown, "Credentials(Basic, <redacted>)");
    let bearer = Credentials::bearer("tok-123456").unwrap();
    assert!(!format!("{bearer:?}").contains("tok-123456"));
}

// Requirement 3.5: the redactor learns the user name, the password and the encoded header value.
#[test]
fn registers_secrets() {
    let credentials = Credentials::basic("john@example.com", &made_secret("open sesame")).unwrap();
    let mut redactor = Redactor::new();
    credentials.register_secrets(&mut redactor);
    let excerpt = redactor.excerpt("john@example.com open sesame am9obkBleGFtcGxlLmNvbTpvcGVuIHNlc2FtZQ==");
    assert_eq!(excerpt.as_str(), "[redacted] [redacted] [redacted]");
}
