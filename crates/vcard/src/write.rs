//! Writing contacts as vCard 3.0.

use crate::Contact;
use chrono::{Datelike, NaiveDate};

use crate::lines::fold;
use crate::text::{escape, multi, single};

/// Write vCard 3.0 cards, CRLF line endings, lines folded at 75 octets on char boundaries.
///
/// `FN` is the contact's [`name`](Contact::name). Values are escaped. Every field but the notes is
/// one line: its line breaks and other control characters become spaces. A label the format has a
/// `TYPE` for (`work`, `home`, `other`; for phones also `mobile`, `fax`, `pager`) is written as
/// that type; any other label is written as the `X-ABLabel` of a group (`item1.TEL` +
/// `item1.X-ABLabel:Car`), which is how address books exchange custom labels. E-mail addresses,
/// phone numbers, postal addresses and web sites with nothing in them are left out. Keys are
/// written as `KEY;TYPE=X509;ENCODING=b:` and `KEY;TYPE=PGP;ENCODING=b:`; one that would not read
/// back as itself is left out, and so is the address of a key, which vCard 3.0 has no form for.
pub fn write(contacts: &[&Contact]) -> String {
    let mut out = String::new();
    for contact in contacts {
        card(&mut out, contact);
    }
    out
}

/// A one-line field, escaped.
fn field(s: &str) -> String {
    escape(&single(s))
}

/// `NAME:value` when there is a value.
fn optional(out: &mut String, name: &str, value: &str) {
    let value = field(value);
    if !value.is_empty() {
        fold(out, &format!("{name}:{value}"));
    }
}

/// The `TYPE` of an e-mail or postal address label, when the format has one for it.
fn place_type(label: &str) -> Option<&'static str> {
    match label {
        "work" => Some("WORK"),
        "home" => Some("HOME"),
        "other" => Some("OTHER"),
        _ => None,
    }
}

fn phone_type(label: &str) -> Option<&'static str> {
    match label {
        "mobile" => Some("CELL"),
        "fax" => Some("FAX"),
        "pager" => Some("PAGER"),
        _ => place_type(label),
    }
}

/// One labelled property. `kind` is the type the property always has (`INTERNET`), `known` the
/// type its label stands for; a label without one goes into an `X-ABLabel` of the same group.
fn labeled(out: &mut String, groups: &mut u32, name: &str, kind: Option<&str>, known: Option<&str>, label: &str, value: &str) {
    let types: Vec<&str> = kind.into_iter().chain(known).collect();
    let params = if types.is_empty() { String::new() } else { format!(";TYPE={}", types.join(",")) };
    let custom = if known.is_some() { String::new() } else { field(label) };
    if custom.is_empty() {
        fold(out, &format!("{name}{params}:{value}"));
    } else {
        *groups = groups.saturating_add(1);
        fold(out, &format!("item{groups}.{name}{params}:{value}"));
        fold(out, &format!("item{groups}.X-ABLabel:{custom}"));
    }
}

fn card(out: &mut String, c: &Contact) {
    fold(out, "BEGIN:VCARD");
    fold(out, "VERSION:3.0");
    fold(out, &format!("N:{};{};{};;", field(&c.last), field(&c.first), field(&c.middle)));
    fold(out, &format!("FN:{}", field(&c.name())));
    optional(out, "NICKNAME", &c.nickname);
    let (company, department) = (field(&c.company), field(&c.department));
    if !department.is_empty() {
        fold(out, &format!("ORG:{company};{department}"));
    } else if !company.is_empty() {
        fold(out, &format!("ORG:{company}"));
    }
    optional(out, "TITLE", &c.title);
    let mut groups = 0u32;
    for email in &c.emails {
        let value = field(&email.value);
        if !value.is_empty() {
            labeled(out, &mut groups, "EMAIL", Some("INTERNET"), place_type(&email.label), &email.label, &value);
        }
    }
    for phone in &c.phones {
        let value = field(&phone.value);
        if !value.is_empty() {
            labeled(out, &mut groups, "TEL", None, phone_type(&phone.label), &phone.label, &value);
        }
    }
    for address in c.addresses.iter().filter(|a| !a.is_empty()) {
        // Post office box and extended address stay empty: the model keeps them in the street.
        let value = format!(
            ";;{};{};{};{};{}",
            field(&address.street),
            field(&address.city),
            field(&address.region),
            field(&address.postal_code),
            field(&address.country)
        );
        labeled(out, &mut groups, "ADR", None, place_type(&address.label), &address.label, &value);
    }
    for site in &c.websites {
        optional(out, "URL", site);
    }
    if let Some(day) = c.birthday {
        // A date that exists, whatever the fields say.
        // A year a card can say (1 to 9999), whatever the date's is.
        let year = day.year().clamp(1, 9999);
        let day = day.with_year(year).or_else(|| NaiveDate::from_ymd_opt(year, day.month(), 28)).unwrap_or_default();
        fold(out, &format!("BDAY:{}", day.format("%Y-%m-%d")));
    }
    let notes = escape(&multi(&c.notes));
    if !notes.is_empty() {
        fold(out, &format!("NOTE:{notes}"));
    }
    let categories: Vec<String> = c.categories.iter().map(|name| field(name)).filter(|name| !name.is_empty()).collect();
    if !categories.is_empty() {
        fold(out, &format!("CATEGORIES:{}", categories.join(",")));
    }
    for key in c.keys.iter().filter_map(crate::key::line).take(Contact::MAX_KEYS) {
        fold(out, &key);
    }
    optional(out, "UID", &c.uid);
    if c.favorite {
        fold(out, "X-MAILCRAFT-FAVORITE:TRUE");
    }
    fold(out, "END:VCARD");
}

#[cfg(test)]
mod tests {
    use crate::{Labeled, PostalAddress};

    use super::*;

    fn lines(c: &Contact) -> Vec<String> {
        let out = write(&[c]);
        assert!(out.ends_with("END:VCARD\r\n"));
        let mut cursor = crate::lines::Lines::new(&out);
        std::iter::from_fn(|| cursor.next_line(false)).collect()
    }

    fn nadia() -> Contact {
        Contact {
            display_name: "Dr. Nadia Okafor".into(),
            first: "Nadia".into(),
            middle: "Chidi".into(),
            last: "Okafor".into(),
            nickname: "Nadi".into(),
            company: "Example Research, Inc.".into(),
            title: "Lead Scientist; Field".into(),
            department: "Field Lab".into(),
            emails: vec![
                Labeled::new("work", "nadia.okafor@example.org"),
                Labeled::new("", "nadia@example.org"),
                Labeled::new("Family", "nadia@family.example.org"),
                Labeled::new("home", "  "),
            ],
            phones: vec![
                Labeled::new("mobile", "+1 (555) 010-0142"),
                Labeled::new("fax", "+1 555 010 0144"),
                Labeled::new("Car, old", "+1 555 010 0145"),
                Labeled::new("", "+1 555 010 0146"),
            ],
            addresses: vec![
                PostalAddress {
                    label: "work".into(),
                    street: "Suite 300, 41 Harbor Road".into(),
                    city: "Riverton".into(),
                    region: "CA".into(),
                    postal_code: "94000".into(),
                    country: "USA".into(),
                },
                PostalAddress { label: "home".into(), ..Default::default() },
                PostalAddress { label: "Cabin".into(), city: "Pinewood".into(), ..Default::default() },
            ],
            websites: vec!["https://example.org/~nadia".into(), "".into()],
            birthday: NaiveDate::from_ymd_opt(1985, 4, 12),
            notes: "Met at the 2024 symposium.\r\nPrefers e-mail, not calls.".into(),
            categories: vec!["Research".into(), "Field Team, West".into(), " ".into()],
            favorite: true,
            uid: "7f1c2f6e-0001-4a5b-9c3d-000000000042".into(),
            ..Default::default()
        }
    }

    #[test]
    fn writes_a_whole_card() {
        assert_eq!(
            lines(&nadia()),
            [
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Okafor;Nadia;Chidi;;",
                "FN:Dr. Nadia Okafor",
                "NICKNAME:Nadi",
                "ORG:Example Research\\, Inc.;Field Lab",
                "TITLE:Lead Scientist\\; Field",
                "EMAIL;TYPE=INTERNET,WORK:nadia.okafor@example.org",
                "EMAIL;TYPE=INTERNET:nadia@example.org",
                "item1.EMAIL;TYPE=INTERNET:nadia@family.example.org",
                "item1.X-ABLabel:Family",
                "TEL;TYPE=CELL:+1 (555) 010-0142",
                "TEL;TYPE=FAX:+1 555 010 0144",
                "item2.TEL:+1 555 010 0145",
                "item2.X-ABLabel:Car\\, old",
                "TEL:+1 555 010 0146",
                "ADR;TYPE=WORK:;;Suite 300\\, 41 Harbor Road;Riverton;CA;94000;USA",
                "item3.ADR:;;;Pinewood;;;",
                "item3.X-ABLabel:Cabin",
                "URL:https://example.org/~nadia",
                "BDAY:1985-04-12",
                "NOTE:Met at the 2024 symposium.\\nPrefers e-mail\\, not calls.",
                "CATEGORIES:Research,Field Team\\, West",
                "UID:7f1c2f6e-0001-4a5b-9c3d-000000000042",
                "X-MAILCRAFT-FAVORITE:TRUE",
                "END:VCARD",
            ]
        );
    }

    #[test]
    fn an_empty_contact_is_still_a_card() {
        assert_eq!(write(&[&Contact::default()]), "BEGIN:VCARD\r\nVERSION:3.0\r\nN:;;;;\r\nFN:\r\nEND:VCARD\r\n");
        assert_eq!(write(&[]), "");
    }

    #[test]
    fn the_formatted_name_falls_back_as_the_model_does() {
        let named = Contact { first: "Ada".into(), last: "Lovelace".into(), ..Default::default() };
        assert!(lines(&named).contains(&"FN:Ada Lovelace".to_string()));
        let company = Contact { company: "Analytical Engines Ltd".into(), ..Default::default() };
        assert_eq!(lines(&company).get(2..5).unwrap(), ["N:;;;;", "FN:Analytical Engines Ltd", "ORG:Analytical Engines Ltd"]);
        let mail = Contact { emails: vec![Labeled::new("home", "x@example.org")], department: "Only a department".into(), ..Default::default() };
        assert_eq!(lines(&mail).get(3..6).unwrap(), ["FN:x@example.org", "ORG:;Only a department", "EMAIL;TYPE=INTERNET,HOME:x@example.org"]);
    }

    #[test]
    fn every_known_label_is_a_type() {
        let c = Contact {
            emails: ["work", "home", "other", "mobile", "Work"].map(|l| Labeled::new(l, "a@example.org")).to_vec(),
            phones: ["mobile", "work", "home", "fax", "pager", "other", "HOME"].map(|l| Labeled::new(l, "1")).to_vec(),
            ..Default::default()
        };
        let got = lines(&c);
        assert_eq!(
            got.get(4..20).unwrap(),
            [
                "EMAIL;TYPE=INTERNET,WORK:a@example.org",
                "EMAIL;TYPE=INTERNET,HOME:a@example.org",
                "EMAIL;TYPE=INTERNET,OTHER:a@example.org",
                "item1.EMAIL;TYPE=INTERNET:a@example.org",
                "item1.X-ABLabel:mobile",
                "item2.EMAIL;TYPE=INTERNET:a@example.org",
                "item2.X-ABLabel:Work",
                "TEL;TYPE=CELL:1",
                "TEL;TYPE=WORK:1",
                "TEL;TYPE=HOME:1",
                "TEL;TYPE=FAX:1",
                "TEL;TYPE=PAGER:1",
                "TEL;TYPE=OTHER:1",
                "item3.TEL:1",
                "item3.X-ABLabel:HOME",
                "END:VCARD",
            ]
        );
    }

    #[test]
    fn lines_end_in_crlf_and_stay_within_75_octets() {
        let mut c = nadia();
        c.notes = "Ünïcödé — 東京 🎉 ".repeat(60);
        c.display_name = "é".repeat(200);
        let out = write(&[&c, &c]);
        assert_eq!(out.matches("BEGIN:VCARD\r\n").count(), 2);
        assert!(!out.replace("\r\n", "").contains(['\r', '\n']));
        assert!(out.split("\r\n").all(|line| line.len() <= 75), "{out}");
        assert!(out.contains("\r\n "));
    }

    #[test]
    fn text_cannot_inject_lines() {
        let c = Contact {
            display_name: "Ada\r\nEND:VCARD\r\nBEGIN:VCARD".into(),
            first: "A\nTEL:666".into(),
            nickname: "x\rEMAIL:evil@example.org".into(),
            emails: vec![Labeled::new("work\r\nTEL:1", "a@example.org\r\nTEL:2")],
            notes: "one\r\nEND:VCARD\rtwo\u{0}".into(),
            categories: vec!["a\nb".into()],
            uid: "u\nUID:2".into(),
            ..Default::default()
        };
        let got = lines(&c);
        assert_eq!(got.iter().filter(|l| l.starts_with("BEGIN:") || l.starts_with("END:")).count(), 2, "{got:?}");
        assert!(!got.iter().any(|l| l.starts_with("TEL")), "{got:?}");
        assert_eq!(got.iter().filter(|l| l.starts_with("UID") || l.contains("EMAIL")).count(), 3, "{got:?}");
        assert!(got.contains(&"FN:Ada  END:VCARD  BEGIN:VCARD".to_string()));
        assert!(got.contains(&"NOTE:one\\nEND:VCARD\\ntwo".to_string()));
        assert!(got.contains(&"item1.X-ABLabel:work  TEL:1".to_string()));
    }

    #[test]
    fn hostile_birthdays_are_written_as_real_dates() {
        for hostile in [
            NaiveDate::MIN,
            NaiveDate::MAX,
            NaiveDate::from_ymd_opt(0, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(-4, 2, 29).unwrap(),
            NaiveDate::from_ymd_opt(10_000, 2, 29).unwrap(),
        ] {
            let c = Contact { birthday: Some(hostile), ..Default::default() };
            let back = crate::parse(&write(&[&c]));
            assert!(back[0].birthday.is_some(), "{hostile:?}");
        }
    }
}
