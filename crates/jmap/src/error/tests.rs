use super::*;
use std::time::Duration;

// RFC 8620 §3.6.1: the limit property names the limit; only short identifiers are kept.
#[test]
fn limit_names() {
    assert_eq!(LimitName::new("maxSizeRequest").unwrap().as_str(), "maxSizeRequest");
    assert!(LimitName::new("maxConcurrentRequests").unwrap().is_concurrency());
    assert!(!LimitName::new("maxCallsInRequest").unwrap().is_concurrency());
    for bad in ["", "max size", "x\ny", &"a".repeat(65), "<script>"] {
        assert_eq!(LimitName::new(bad), None, "{bad:?}");
    }
}

// Requirement 2.4: messages are the crate's own and one line.
#[test]
fn messages() {
    let busy = Error::Busy { retry_after: Some(RetryAfter::Delay(Duration::from_secs(30))) };
    assert_eq!(busy.to_string(), "the server is busy and asks to wait 30 s before the next attempt");
    assert_eq!(Error::from(TransportError::Overloaded), Error::Busy { retry_after: None });
    assert_eq!(Error::from(TransportError::TimedOut), Error::Transport(TransportError::TimedOut));
    assert_eq!(Error::from(TransportError::BodyTooLarge), Error::Unusable(Unusable::BodyTooLarge));
    // The cause and the remedy, not a stock phrase.
    let refused = Error::CredentialsRefused.to_string();
    assert!(refused.contains("user name and password") && refused.contains("sign in again"));
    let long = Error::Address(AddressError::TooLong).to_string();
    assert!(long.contains("8 KiB") && long.contains("pasted"));
    for error in [
        Error::CredentialsRefused,
        Error::NotFound,
        Error::NoMailAccount,
        Error::Address(AddressError::UserInfo),
        Error::Transport(TransportError::Other),
        Error::Build(BuildError::TooManyCalls { limit: 16 }),
    ] {
        let text = error.to_string();
        assert!(!text.contains('\n') && text.len() > 40, "{text}");
    }
}
