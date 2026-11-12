use super::*;

// 1994-10-29T19:43:31Z, the If-Modified-Since example of RFC 9110 §13.1.3.
const MODIFIED_SINCE: u64 = 783_459_811;
// 2009-07-27T12:28:53Z, the Date of the example exchange of RFC 9110 §3.9.
const EXCHANGE_DATE: u64 = 1_248_697_733;
// 2012-01-20T15:41:54Z, the Date of the 416 example of RFC 9110 §15.5.17.
const RANGE_EXAMPLE: u64 = 1_327_074_114;

// RFC 9110 §5.6.7: the three forms of one instant (the §13.1.3 date), all read alike.
#[test]
fn parses_three_forms() {
    let now = UnixTime::from_secs(RANGE_EXAMPLE);
    for text in ["Sat, 29 Oct 1994 19:43:31 GMT", "Saturday, 29-Oct-94 19:43:31 GMT", "Sat Oct 29 19:43:31 1994"] {
        let date = HttpDate::parse(text).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(date.to_unix(now), Some(UnixTime::from_secs(MODIFIED_SINCE)), "{text}");
    }
}

// RFC 9110 §3.9 and §8.8.3.3 example dates, an asctime day padded with a space, and a 29 February in the
// century leap year 2400 (divisible by 400).
#[test]
fn converts_dates() {
    let now = UnixTime::from_secs(RANGE_EXAMPLE);
    for (text, expected) in [
        ("Mon, 27 Jul 2009 12:28:53 GMT", EXCHANGE_DATE),
        ("Wed, 22 Jul 2009 19:15:56 GMT", 1_248_290_156),
        ("Fri, 26 Mar 2010 00:05:00 GMT", 1_269_561_900),
        ("Wed Jul  1 00:00:00 2009", 1_246_406_400),
        ("Tue, 29 Feb 2400 06:41:17 GMT", 13_574_587_277),
    ] {
        let date = HttpDate::parse(text).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(date.to_unix(now), Some(UnixTime::from_secs(expected)), "{text}");
    }
}

// RFC 5322 §3.3 day ranges (which RFC 9110 §5.6.7 refers to), Gregorian leap years: every month's length.
#[test]
fn month_lengths() {
    let common: Vec<u8> = (1..=12).map(|month| month_length(2009, month)).collect();
    assert_eq!(common, [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]);
    assert_eq!(month_length(2400, 2), 29);
    assert_eq!(month_length(1600, 2), 29);
    assert_eq!(month_length(2012, 2), 29);
    assert_eq!(month_length(2100, 2), 28);
    assert_eq!(month_length(1700, 2), 28);
    assert_eq!(month_length(2009, 0), 0);
    assert_eq!(month_length(2009, 13), 0);
}

// RFC 9110 §5.6.7: HTTP-date is case sensitive and has no extra white space; days must exist.
#[test]
fn refuses_malformed_dates() {
    for text in [
        "sat, 29 Oct 1994 19:43:31 GMT",
        "Sat, 29 oct 1994 19:43:31 GMT",
        "Sat, 29 Oct 1994 19:43:31 UTC",
        "Sat,  29 Oct 1994 19:43:31 GMT",
        "Mon, 6 Jul 2009 12:28:53 GMT",
        "Thu, 31 Sep 2009 12:28:53 GMT",
        "Mon, 29 Feb 2100 09:14:26 GMT",
        "Mon, 29 Feb 1700 21:03:58 GMT",
        "Mon, 27 Jul 2009 24:30:10 GMT",
        "Mon, 27 Jul 2009 12:60:41 GMT",
        "Mon, 27 Jul 2009 12:28:61 GMT",
        "Mon, 27 Jul 2009 12:28:53 GMT x",
        "Monday, 27-Jul-2009 12:28:53 GMT",
        "Wed Jul 1 00:00:00 2009",
        "",
    ] {
        assert_eq!(HttpDate::parse(text), None, "{text}");
    }
    assert!(HttpDate::parse("Sat, 31 Dec 2016 23:59:60 GMT").is_some());
}

// RFC 9110 §5.6.7: a two-digit year more than 50 years ahead is in the past century.
#[test]
fn two_digit_years() {
    let now = UnixTime::from_secs(RANGE_EXAMPLE); // 2012
    let date = HttpDate::parse("Monday, 01-Jan-62 00:00:00 GMT").unwrap();
    assert_eq!(date.year(now), 2062);
    let date = HttpDate::parse("Monday, 01-Jan-63 00:00:00 GMT").unwrap();
    assert_eq!(date.year(now), 1963);
    let date = HttpDate::parse("Friday, 20-Jan-12 15:41:54 GMT").unwrap();
    assert_eq!(date.to_unix(now), Some(UnixTime::from_secs(RANGE_EXAMPLE)));
    // 00 is 2000 here, a century leap year (divisible by 400), so its 29 February exists; 62 is 2062,
    // which has none.
    let date = HttpDate::parse("Tuesday, 29-Feb-00 06:41:17 GMT").unwrap();
    assert_eq!(date.to_unix(now), Some(UnixTime::from_secs(951_806_477)));
    let date = HttpDate::parse("Wednesday, 29-Feb-62 00:00:00 GMT").unwrap();
    assert_eq!(date.to_unix(now), None);
}

// RFC 9110 §10.2.3: delay-seconds, clamped to the one maximum.
#[test]
fn delay_seconds() {
    assert_eq!(RetryAfter::parse("75"), Some(RetryAfter::Delay(Duration::from_secs(75))));
    assert_eq!(RetryAfter::parse(" 0 "), Some(RetryAfter::Delay(Duration::ZERO)));
    assert_eq!(RetryAfter::parse("99999999999999999999999"), Some(RetryAfter::Delay(MAX_RETRY_DELAY)));
    for bad in ["-1", "1.5", "12s", "", "  ", "0x10"] {
        assert_eq!(RetryAfter::parse(bad), None, "{bad:?}");
    }
}

// RFC 9110 §10.2.3: a date is judged against the caller's "now"; past dates mean no wait.
#[test]
fn date_delays() {
    let now = UnixTime::from_secs(EXCHANGE_DATE);
    let retry = RetryAfter::parse("Mon, 27 Jul 2009 12:31:53 GMT").unwrap();
    assert_eq!(retry.delay_from(now), Duration::from_secs(180));
    let past = RetryAfter::parse("Wed, 22 Jul 2009 19:15:56 GMT").unwrap();
    assert_eq!(past.delay_from(now), Duration::ZERO);
    let far = RetryAfter::parse("Thu, 30 Dec 9999 17:05:44 GMT").unwrap();
    assert_eq!(far.delay_from(now), MAX_RETRY_DELAY);
    let old = RetryAfter::parse("Sat, 14 Jul 1900 03:27:09 GMT").unwrap();
    assert_eq!(old.delay_from(now), Duration::ZERO);
    assert_eq!(far.delay_from(UnixTime::from_secs(u64::MAX)), Duration::ZERO);
}
