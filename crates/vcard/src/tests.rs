//! Whole-crate properties: what is written reads back the same, and no text makes the reader panic.

use crate::{Contact, Labeled, PostalAddress};
use chrono::NaiveDate;
use proptest::prelude::*;

use crate::{parse, write};

/// What reading back gives for a written contact: the name the card was written under.
fn as_read(c: &Contact) -> Contact {
    Contact { display_name: c.name(), ..c.clone() }
}

fn round_trip(contacts: &[Contact]) -> Vec<Contact> {
    parse(&write(&contacts.iter().collect::<Vec<_>>()))
}

fn sample_contacts() -> Vec<Contact> {
    let nadia = Contact {
        display_name: "Dr. Nadia Okafor".into(),
        first: "Nadia".into(),
        middle: "Chidi Rose".into(),
        last: "Okafor".into(),
        nickname: "Nadi, \"the planner\"".into(),
        company: "Example Research, Inc.".into(),
        title: "Lead Scientist; Field".into(),
        department: "Field Lab \\ Unit 7".into(),
        emails: vec![
            Labeled::new("work", "nadia.okafor@example.org"),
            Labeled::new("", "nadia@example.org"),
            Labeled::new("Family", "nadia@family.example.org"),
            Labeled::new("mobile", "nadia@phone.example.org"),
        ],
        phones: vec![
            Labeled::new("mobile", "+1 (555) 010-0142"),
            Labeled::new("work", "+1 555 010 0143"),
            Labeled::new("home", "+1 555 010 0144"),
            Labeled::new("fax", "+1 555 010 0145"),
            Labeled::new("pager", "+1 555 010 0146"),
            Labeled::new("other", "+1 555 010 0147"),
            Labeled::new("", "+1 555 010 0148"),
            Labeled::new("Car; old, blue", "+1 555 010 0149"),
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
            PostalAddress { label: "home".into(), street: "7; Rue de l'Étang".into(), city: "Saint-Rémy".into(), ..Default::default() },
            PostalAddress { label: "Cabin".into(), country: "Norway".into(), ..Default::default() },
            PostalAddress { postal_code: "12345".into(), ..Default::default() },
        ],
        websites: vec!["https://example.org/~nadia?a=1,2;b=3".into(), "example.org".into()],
        birthday: NaiveDate::from_ymd_opt(1985, 4, 12),
        notes: "Met at the 2024 symposium.\nPrefers e-mail, not calls; never on Fridays.\n\n\tC:\\notes\\nadia.txt\n".into(),
        categories: vec!["Research".into(), "Field Team, West".into()],
        favorite: true,
        uid: "7f1c2f6e-0001-4a5b-9c3d-000000000042".into(),
        ..Default::default()
    };
    let unnamed =
        Contact { first: "Jürgen".into(), last: "Müller".into(), phones: vec![Labeled::new("mobile", "+49 170 5550101")], ..Default::default() };
    let company =
        Contact { company: "東京デザイン株式会社".into(), websites: vec!["https://design.example.org".into()], ..Default::default() };
    let address_only = Contact { emails: vec![Labeled::new("other", "someone@example.org")], ..Default::default() };
    let bare = Contact { favorite: true, ..Default::default() };
    let emoji = Contact {
        display_name: "Zoë 🎉 “Zed” Ødegård".into(),
        notes: " leading and trailing spaces stay ".into(),
        birthday: NaiveDate::from_ymd_opt(2000, 2, 29),
        ..Default::default()
    };
    vec![nadia, unnamed, company, address_only, bare, emoji]
}

#[test]
fn sample_contacts_round_trip() {
    let contacts = sample_contacts();
    let back = round_trip(&contacts);
    assert_eq!(back, contacts.iter().map(as_read).collect::<Vec<_>>());
    assert_eq!(back[1].display_name, "Jürgen Müller");
    assert_eq!(back[3].display_name, "someone@example.org");
    assert_eq!(back[4].display_name, "");
    // What was read writes the same text again.
    let text = write(&back.iter().collect::<Vec<_>>());
    assert_eq!(parse(&text), back);
    assert_eq!(write(&parse(&text).iter().collect::<Vec<_>>()), text);
}

#[test]
fn an_empty_contact_does_not_come_back() {
    assert!(round_trip(&[Contact::default(), Contact::default()]).is_empty());
    // Entries with nothing in them are left out.
    let hollow = Contact {
        display_name: "Ada".into(),
        emails: vec![Labeled::new("work", ""), Labeled::new("home", "ada@example.org")],
        phones: vec![Labeled::new("mobile", " ")],
        addresses: vec![PostalAddress { label: "home".into(), ..Default::default() }],
        websites: vec![String::new()],
        categories: vec![" ".into()],
        ..Default::default()
    };
    let back = round_trip(std::slice::from_ref(&hollow));
    assert_eq!(back, [Contact { display_name: "Ada".into(), emails: vec![Labeled::new("home", "ada@example.org")], ..Default::default() }]);
}

#[test]
fn cards_from_other_programs_survive_a_round_trip() {
    // A 2.1 card read, written as 3.0 and read again says the same.
    let old = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:M=C3=BCller;J=C3=BCrgen\r\nTEL;WORK;VOICE:+49 30 5550100\r\nTEL;CELL:+49 170 5550101\r\n\
EMAIL;PREF;INTERNET:jm@example.org\r\nADR;HOME:;;Gartenweg 12, Hinterhaus;Berlin;;10115;Deutschland\r\nNOTE;ENCODING=QUOTED-PRINTABLE:Zeile eins=0D=0AZeile =\r\nzwei\r\nEND:VCARD\r\n";
    let first = parse(old);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].notes, "Zeile eins\nZeile zwei");
    assert_eq!(first[0].addresses[0].street, "Gartenweg 12, Hinterhaus");
    assert_eq!(round_trip(&first), first.iter().map(as_read).collect::<Vec<_>>());
}

/// One line of text as a contact field holds it: printable, trimmed.
fn line() -> impl Strategy<Value = String> {
    prop_oneof!["(\\PC|[;,\\\\:\"^=.]){0,24}", "[ -~]{0,60}", Just(String::new())].prop_map(|s| s.trim().to_string())
}

/// A line with something in it: an empty one gets a character.
fn filled_line() -> impl Strategy<Value = String> {
    (line(), "[!-~]").prop_map(|(line, fallback)| if line.is_empty() { fallback } else { line })
}

fn label(known: &'static [&'static str]) -> impl Strategy<Value = String> {
    prop_oneof![proptest::sample::select(known).prop_map(str::to_string), Just(String::new()), line()]
}

fn date() -> impl Strategy<Value = NaiveDate> {
    (1i32..=9999, 1u32..=12, 1u32..=28).prop_map(|(y, m, day)| NaiveDate::from_ymd_opt(y, m, day).unwrap_or_default())
}

fn address() -> impl Strategy<Value = PostalAddress> {
    // One part, any of them, always says something.
    (label(&["work", "home", "other"]), [line(), line(), line(), line(), line()], 0usize..5, filled_line()).prop_map(
        |(label, mut parts, filled, text)| {
            parts[filled] = text;
            let [street, city, region, postal_code, country] = parts;
            PostalAddress { label, street, city, region, postal_code, country }
        },
    )
}

fn contact() -> impl Strategy<Value = Contact> {
    let names = (line(), line(), line(), line(), line(), line(), line(), line());
    let emails = proptest::collection::vec((label(&["work", "home", "other"]), filled_line()), 0..4);
    let phones = proptest::collection::vec((label(&["mobile", "work", "home", "fax", "pager", "other"]), filled_line()), 0..4);
    let lists = (
        emails,
        phones,
        proptest::collection::vec(address(), 0..3),
        proptest::collection::vec(filled_line(), 0..3),
        proptest::collection::vec(filled_line(), 0..3),
    );
    let rest = (proptest::option::of(date()), "(\\PC|[\n\t;,\\\\:]){0,120}", any::<bool>(), line());
    (names, lists, rest).prop_map(|(names, lists, rest)| {
        let (display_name, first, middle, last, nickname, company, title, department) = names;
        let (emails, phones, addresses, websites, categories) = lists;
        let (birthday, notes, favorite, uid) = rest;
        let labeled = |list: Vec<(String, String)>| list.into_iter().map(|(label, value)| Labeled { label, value }).collect();
        Contact {
            display_name,
            first,
            middle,
            last,
            nickname,
            company,
            title,
            department,
            emails: labeled(emails),
            phones: labeled(phones),
            addresses,
            websites,
            birthday,
            notes,
            categories,
            favorite,
            uid,
            ..Default::default()
        }
    })
}

/// Text that looks like a card without being one.
fn soup() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        proptest::sample::select(vec![
            "BEGIN:VCARD",
            "END:VCARD",
            "BEGIN:",
            "END:",
            "BEGIN:X",
            "END:X",
            "VERSION:2.1",
            "VERSION:3.0",
            "VERSION:4.0",
            "FN",
            "N",
            "NICKNAME",
            "ORG",
            "TITLE",
            "EMAIL",
            "TEL",
            "ADR",
            "URL",
            "BDAY",
            "NOTE",
            "CATEGORIES",
            "UID",
            "PHOTO",
            "X-ABLabel",
            "X-FAVORITE",
            "item1.",
            "item2.",
            ".",
            ":",
            ";",
            ",",
            "=",
            "\"",
            "\\",
            "\r\n",
            "\n",
            "\r\n ",
            "\n\t",
            " ",
            "=\r\n",
            "=\n",
            "=C3=A9",
            "=C3",
            "=ZZ",
            "\\n",
            "\\;",
            "\\,",
            ";TYPE=WORK,VOICE",
            ";TYPE=\"cell,pref\"",
            ";HOME",
            ";CELL",
            ";ENCODING=QUOTED-PRINTABLE",
            ";QUOTED-PRINTABLE",
            ";ENCODING=b",
            ";BASE64",
            ";CHARSET=ISO-8859-1",
            ";CHARSET=UTF-8",
            ";CHARSET=x",
            "mailto:",
            "tel:",
            "a@example.org",
            "+1 555 0100",
            "1985-04-12",
            "19850412T101010Z",
            "--0412",
            "0000-00-00",
            "TRUE",
            "SGVsbG8=",
            "_$!<Work>!$_",
            "_$!<",
        ])
        .prop_map(str::to_string),
        "\\PC{0,6}",
        any::<String>(),
    ];
    proptest::collection::vec(piece, 0..60).prop_map(|pieces| pieces.concat())
}

fn valid_text() -> String {
    write(&sample_contacts().iter().collect::<Vec<_>>())
}

/// Whatever the reader gives must also be writable and readable again.
fn survives(text: &str) -> Vec<Contact> {
    let contacts = parse(text);
    let again = write(&contacts.iter().collect::<Vec<_>>());
    let _ = parse(&again);
    contacts
}

proptest! {
    #[test]
    fn any_contact_round_trips(contacts in proptest::collection::vec(contact(), 0..4)) {
        let back = round_trip(&contacts);
        // A contact with nothing in it isn't written back as one.
        let want: Vec<Contact> = contacts.iter().map(as_read).filter(|c| *c != Contact::default()).collect();
        prop_assert_eq!(back, want);
    }

    #[test]
    fn any_notes_and_names_round_trip(notes in "(\\PC|[\n\t]){0,400}", name in "\\PC{0,200}", street in "\\PC{0,100}") {
        let name = name.trim().to_string();
        let street = street.trim().to_string();
        let c = Contact { display_name: name.clone(), last: name.clone(), company: name.clone(), notes, addresses: vec![PostalAddress { street, city: "Riverton".into(), ..Default::default() }], uid: "u".into(), ..Default::default() };
        prop_assert_eq!(round_trip(std::slice::from_ref(&c)), vec![as_read(&c)]);
    }

    #[test]
    fn text_with_control_characters_reads_back_clean_and_stable(name in any::<String>(), notes in any::<String>(), label in any::<String>(), value in any::<String>(), category in any::<String>()) {
        let c = Contact {
            display_name: name.clone(),
            first: name,
            notes,
            emails: vec![Labeled::new(label.clone(), value.clone())],
            phones: vec![Labeled::new(label.clone(), value.clone())],
            addresses: vec![PostalAddress { label, street: value.clone(), ..Default::default() }],
            websites: vec![value],
            categories: vec![category],
            uid: "u".into(),
            ..Default::default()
        };
        let once = round_trip(std::slice::from_ref(&c));
        prop_assert_eq!(once.len(), 1);
        // The first pass cleans (control characters, stray spaces); after that nothing changes but a
        // display name that was empty.
        prop_assert_eq!(round_trip(&once), once.iter().map(as_read).collect::<Vec<_>>());
        prop_assert!(!once[0].display_name.contains(['\r', '\n']) && !once[0].notes.contains('\r'));
    }

    #[test]
    fn arbitrary_text_never_panics(text in any::<String>()) {
        survives(&text);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let contacts = crate::parse_bytes(&bytes);
        let _ = parse(&write(&contacts.iter().collect::<Vec<_>>()));
    }

    #[test]
    fn card_shaped_bytes_never_panic(text in soup(), high in proptest::collection::vec((any::<prop::sample::Index>(), 0x80u8..=0xFF), 0..12), charset in proptest::sample::select(vec!["", "windows-1250", "UTF-8", "utf-16", "iso-2022-jp", "shift_jis", "x-unknown"])) {
        // Card-shaped text with a charset named and raw 8-bit bytes dropped in.
        let mut bytes = text.replace("FN:", &format!("FN;CHARSET={charset}:")).into_bytes();
        for (at, byte) in high {
            if !bytes.is_empty() {
                let i = at.index(bytes.len());
                bytes.insert(i, byte);
            }
        }
        let _ = crate::parse_bytes(&bytes);
    }

    #[test]
    fn utf8_bytes_read_as_the_text_does(text in soup()) {
        // (A value in UTF-8 that says it is Latin-1 is read as Latin-1 from bytes.)
        let text = text.replace("ISO-8859-1", "UTF-8");
        prop_assert_eq!(crate::parse_bytes(text.as_bytes()), parse(&text));
    }

    #[test]
    fn card_shaped_junk_never_panics(text in soup()) {
        survives(&text);
    }

    #[test]
    fn a_damaged_card_never_panics(cuts in proptest::collection::vec((any::<prop::sample::Index>(), 0usize..40, 0u8..5, "\\PC{0,8}|[\r\n:;,=\"\\\\ .]{1,4}"), 1..6)) {
        let mut chars: Vec<char> = valid_text().chars().collect();
        for (index, length, kind, junk) in cuts {
            let from = index.index(chars.len().max(1)).min(chars.len());
            let to = from.saturating_add(length).min(chars.len());
            match kind {
                // Cut a piece out, overwrite it, insert junk, double a piece, or stop the text there.
                0 => drop(chars.drain(from..to)),
                1 => drop(chars.splice(from..to, junk.chars())),
                2 => drop(chars.splice(from..from, junk.chars())),
                3 => {
                    let piece: Vec<char> = chars[from..to].to_vec();
                    drop(chars.splice(from..from, piece));
                }
                _ => chars.truncate(from),
            }
        }
        let damaged: String = chars.into_iter().collect();
        prop_assert!(survives(&damaged).len() <= 64);
    }
}

#[test]
fn a_card_cut_anywhere_never_panics() {
    let text = valid_text();
    for (cut, _) in text.char_indices().step_by(5) {
        survives(&text[..cut]);
        survives(&text[cut..]);
    }
}
