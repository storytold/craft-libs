//! Reading vCards (2.1, 3.0 and 4.0) into contacts.

use std::collections::HashMap;

use crate::{Contact, Labeled, PostalAddress};
use chrono::NaiveDate;

use crate::lines::{ContentLine, Lines};
use crate::text::{base64_bytes, component, multi, quoted_printable, single, split, unescape};

/// Most cards read from one text.
const MAX_CARDS: usize = 100_000;
/// Most e-mail addresses, phone numbers, postal addresses or web sites kept per card.
const MAX_ITEMS: usize = 100;
const MAX_CATEGORIES: usize = 500;
/// Most group labels (`item1.X-ABLabel`) kept per card.
const MAX_LABELS: usize = 400;
/// Deepest nesting of cards followed (a 2.1 `AGENT` holds a card of its own).
const MAX_DEPTH: usize = 8;
/// Most bytes of keys (`KEY`) kept from one text, all its cards together.
pub(crate) const MAX_KEY_BYTES: usize = 64 << 20;

/// Parse one or many vCards (2.1, 3.0, 4.0). Never fails: junk gives an empty list, and a card
/// with nothing in it is skipped. Ids are left 0.
///
/// `FN` gives the display name; without one it stays empty and the model derives a name. A label
/// comes from `TYPE` (`work`, `home`, `other`, and for phones `mobile`, `fax`, `pager`), or from the
/// `X-ABLabel` of the property's group (`item1.X-ABLabel:Family`); it is empty when neither says
/// anything. An S/MIME certificate or OpenPGP public key (`KEY`) is kept as bytes when its bytes
/// have the shape of one, and the address of one as an address, never fetched; secret key
/// material is never kept (`crate::key`). A card with nothing but keys is skipped. Photos, logos,
/// sounds and unknown properties are skipped.
///
/// Work is bounded: at most 100 000 cards are read, 100 e-mail addresses, phone numbers, postal
/// addresses and web sites per card, 32 keys of at most 64 KiB per card and 64 MiB of keys in all,
/// and what a line holds beyond 1 MiB is dropped.
///
/// A file's bytes go through [`parse_bytes`], which reads raw 8-bit values in their `CHARSET`.
pub fn parse(text: &str) -> Vec<Contact> {
    parse_keeping(text, MAX_KEY_BYTES)
}

/// [`parse`], keeping at most `key_room` bytes of keys.
pub(crate) fn parse_keeping(text: &str, mut key_room: usize) -> Vec<Contact> {
    let mut lines = Lines::new(text);
    // The cards that are open, outermost first, and the finished ones with the order they began in.
    let mut open: Vec<Card> = Vec::new();
    let mut done: Vec<(usize, Contact)> = Vec::new();
    let mut begun = 0usize;
    while done.len() < MAX_CARDS {
        let v21 = open.last().is_some_and(|card| card.v21);
        let Some(raw) = lines.next_line(v21) else { break };
        let Some(line) = ContentLine::parse(&raw) else { continue };
        let is_card = line.value.trim().eq_ignore_ascii_case("VCARD");
        match (line.name.as_str(), open.last_mut()) {
            ("BEGIN", _) if is_card => {
                // A card that never ended ends where too many are open.
                if open.len() >= MAX_DEPTH {
                    done.extend(open.pop().and_then(Card::finish));
                }
                open.push(Card { order: begun, ..Default::default() });
                begun = begun.saturating_add(1);
            }
            ("END", _) if is_card => done.extend(open.pop().and_then(Card::finish)),
            ("BEGIN", Some(card)) => card.foreign = card.foreign.saturating_add(1),
            ("END", Some(card)) => card.foreign = card.foreign.saturating_sub(1),
            (_, Some(card)) if card.foreign == 0 => card.apply(&line, &mut key_room),
            _ => {}
        }
    }
    // Cards without an END are still cards.
    while let Some(card) = open.pop() {
        done.extend(card.finish());
    }
    done.sort_by_key(|(order, _)| *order);
    done.into_iter().map(|(_, contact)| contact).take(MAX_CARDS).collect()
}

/// Parse a vCard file as [`parse`] does, from its bytes: a value written as raw 8-bit bytes
/// (vCard 2.1, `FN;CHARSET=WINDOWS-1250:…`) is read in the `CHARSET` of its property, and one
/// without a `CHARSET` as UTF-8 when it is valid UTF-8, else as Windows-1252.
pub fn parse_bytes(bytes: &[u8]) -> Vec<Contact> {
    parse(&crate::lines::text_from_bytes(bytes))
}

/// A card being read.
#[derive(Default)]
struct Card {
    /// Its place among the cards of the text.
    order: usize,
    /// `VERSION:2.1`: other folding, no comma lists.
    v21: bool,
    /// How deep inside a component that isn't a card the reader is.
    foreign: u32,
    contact: Contact,
    named: bool,
    /// E-mail addresses, phones and postal addresses, each with the group it was written in.
    emails: Vec<(String, Labeled)>,
    phones: Vec<(String, Labeled)>,
    addresses: Vec<(String, PostalAddress)>,
    /// `X-ABLabel` by group.
    labels: HashMap<String, String>,
}

impl Card {
    /// Take in one property; `key_room` is what keys may still take of the text's allowance.
    fn apply(&mut self, line: &ContentLine, key_room: &mut usize) {
        let v21 = self.v21;
        let contact = &mut self.contact;
        // One line of text, decoded and unescaped.
        let one = |line: &ContentLine| single(&unescape(&decoded(line)));
        match line.name.as_str() {
            "VERSION" => self.v21 = line.value.trim().starts_with("2."),
            "FN" if contact.display_name.is_empty() => contact.display_name = one(line),
            "N" if !self.named => {
                self.named = true;
                let value = decoded(line);
                let parts = split(&value, ';', 8);
                let part = |i: usize| parts.get(i).map(|p| component(p, " ", v21)).unwrap_or_default();
                (contact.last, contact.first, contact.middle) = (part(0), part(1), part(2));
            }
            "NICKNAME" if contact.nickname.is_empty() => contact.nickname = one(line),
            "ORG" if contact.company.is_empty() && contact.department.is_empty() => {
                let value = decoded(line);
                let parts = split(&value, ';', 16);
                contact.company = parts.first().map(|p| component(p, ", ", v21)).unwrap_or_default();
                contact.department = parts.iter().skip(1).map(|p| component(p, ", ", v21)).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ");
            }
            "TITLE" if contact.title.is_empty() => contact.title = one(line),
            "EMAIL" if self.emails.len() < MAX_ITEMS => {
                let value = without_scheme(&one(line), "mailto:");
                if !value.is_empty() {
                    self.emails.push((line.group.clone(), Labeled::new(place_label(&line.types()), value)));
                }
            }
            "TEL" if self.phones.len() < MAX_ITEMS => {
                let value = without_scheme(&one(line), "tel:");
                if !value.is_empty() {
                    self.phones.push((line.group.clone(), Labeled::new(phone_label(&line.types()), value)));
                }
            }
            "ADR" if self.addresses.len() < MAX_ITEMS => {
                let value = decoded(line);
                let parts = split(&value, ';', 8);
                let part = |i: usize| parts.get(i).map(|p| component(p, ", ", v21)).unwrap_or_default();
                // Post office box, extended address, street: the lines above the city.
                let street = [part(0), part(1), part(2)].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ");
                let address = PostalAddress {
                    label: place_label(&line.types()).into(),
                    street,
                    city: part(3),
                    region: part(4),
                    postal_code: part(5),
                    country: part(6),
                };
                if !address.is_empty() {
                    self.addresses.push((line.group.clone(), address));
                }
            }
            "URL" if contact.websites.len() < MAX_ITEMS => {
                let value = one(line);
                if !value.is_empty() {
                    contact.websites.push(value);
                }
            }
            "BDAY" if contact.birthday.is_none() => contact.birthday = birthday(&line.value),
            "NOTE" => {
                let note = multi(&unescape(&decoded(line)));
                if !note.is_empty() {
                    if !contact.notes.is_empty() {
                        contact.notes.push('\n');
                    }
                    contact.notes.push_str(&note);
                }
            }
            "CATEGORIES" => {
                let value = decoded(line);
                let room = MAX_CATEGORIES.saturating_sub(contact.categories.len());
                contact
                    .categories
                    .extend(split(&value, ',', MAX_CATEGORIES).into_iter().map(|c| single(&unescape(c))).filter(|c| !c.is_empty()).take(room));
            }
            "UID" if contact.uid.is_empty() => contact.uid = one(line),
            "KEY" if contact.keys.len() < Contact::MAX_KEYS && *key_room > 0 => {
                if let Some(key) = crate::key::read(line) {
                    let cost = key.data.len().saturating_add(key.uri.len());
                    if cost <= *key_room && !contact.keys.contains(&key) {
                        *key_room = key_room.saturating_sub(cost);
                        contact.keys.push(key);
                    }
                }
            }
            "X-FAVORITE" | "X-MAILCRAFT-FAVORITE" => {
                contact.favorite |= matches!(line.value.trim().to_ascii_uppercase().as_str(), "TRUE" | "YES" | "1")
            }
            "X-ABLABEL" if !line.group.is_empty() && self.labels.len() < MAX_LABELS => {
                let label = one(line);
                // The built-in labels of the program that invented this are wrapped: `_$!<Work>!$_`.
                let label = label.strip_prefix("_$!<").and_then(|l| l.strip_suffix(">!$_")).map(str::to_lowercase).unwrap_or(label);
                self.labels.entry(line.group.clone()).or_insert(label);
            }
            _ => {}
        }
    }

    /// The contact with the place it began at; `None` for a card with nothing in it.
    fn finish(self) -> Option<(usize, Contact)> {
        let Card { order, mut contact, emails, phones, addresses, labels, .. } = self;
        // A label given to a group says more than the property's type.
        let label = |group: &str, own: String| labels.get(group).filter(|_| !group.is_empty()).cloned().unwrap_or(own);
        contact.emails = emails.into_iter().map(|(group, e)| Labeled { label: label(&group, e.label), value: e.value }).collect();
        contact.phones = phones.into_iter().map(|(group, p)| Labeled { label: label(&group, p.label), value: p.value }).collect();
        contact.addresses = addresses.into_iter().map(|(group, a)| PostalAddress { label: label(&group, a.label.clone()), ..a }).collect();
        // Keys alone make no contact: a card of nothing but keys is skipped as an empty one is.
        let keys = std::mem::take(&mut contact.keys);
        let empty = contact == Contact::default();
        contact.keys = keys;
        (!empty).then_some((order, contact))
    }
}

/// The value with its `ENCODING` (quoted-printable or base64) and `CHARSET` undone; still escaped.
fn decoded(line: &ContentLine) -> String {
    let bytes = match line.param("ENCODING").unwrap_or("").to_ascii_uppercase().as_str() {
        "QUOTED-PRINTABLE" | "Q" => quoted_printable(&line.value),
        "BASE64" | "B" => base64_bytes(&line.value),
        _ => return line.value.clone(),
    };
    craft_mime::charset::decode(&bytes, line.param("CHARSET").unwrap_or(""))
}

/// `tel:+1-555-0100` and `mailto:a@example.org` (vCard 4.0 URIs) without the scheme.
fn without_scheme(value: &str, scheme: &str) -> String {
    match value.get(..scheme.len()) {
        Some(start) if start.eq_ignore_ascii_case(scheme) => value.get(scheme.len()..).unwrap_or("").trim().to_string(),
        _ => value.to_string(),
    }
}

/// The label of an e-mail or postal address.
fn place_label(types: &[String]) -> &'static str {
    let has = |t: &str| types.iter().any(|x| x == t);
    if has("WORK") {
        "work"
    } else if has("HOME") {
        "home"
    } else if has("OTHER") {
        "other"
    } else {
        ""
    }
}

/// The label of a phone number: what kind of device it is says more than where it stands.
fn phone_label(types: &[String]) -> &'static str {
    let has = |t: &str| types.iter().any(|x| x == t);
    if has("CELL") || has("MOBILE") || has("IPHONE") {
        "mobile"
    } else if has("FAX") {
        "fax"
    } else if has("PAGER") {
        "pager"
    } else {
        place_label(types)
    }
}

/// `1985-04-12`, `19850412`, or either with a time after it; `None` without a year (`--0412`).
fn birthday(value: &str) -> Option<NaiveDate> {
    parse_iso(value.trim().split(['T', 't', ' ']).next()?)
}

/// A date from `YYYY-MM-DD` or `YYYYMMDD`, years 1 to 9999.
fn parse_iso(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 8 || s.chars().any(|c| !c.is_ascii_digit() && c != '-') {
        return None;
    }
    let year: i32 = digits.get(0..4)?.parse().ok()?;
    if year < 1 {
        return None;
    }
    NaiveDate::from_ymd_opt(year, digits.get(4..6)?.parse().ok()?, digits.get(6..8)?.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// One contact from the properties of a 3.0 card.
    fn card(properties: &str) -> Contact {
        let got = parse(&format!("BEGIN:VCARD\r\nVERSION:3.0\r\n{properties}\r\nEND:VCARD\r\n"));
        assert_eq!(got.len(), 1, "{properties}");
        got.into_iter().next().unwrap()
    }

    /// A vCard 2.1 card as old phones and mail programs write them: bare type parameters,
    /// quoted-printable with soft line breaks, a charset.
    const V21: &str = "BEGIN:VCARD\r\n\
VERSION:2.1\r\n\
N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:M=C3=BCller;J=C3=BCrgen;;Dr.;\r\n\
FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Dr. J=C3=BCrgen M=C3=BCller\r\n\
ORG:Beispiel Werke, GmbH;Entwicklung\r\n\
TITLE;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Gesch=E4ftsf=FChrer\r\n\
TEL;WORK;VOICE:+49 30 5550100\r\n\
TEL;CELL:+49 170 5550101\r\n\
TEL;WORK;FAX:+49 30 5550199\r\n\
TEL;PAGER:+49 30 5550177\r\n\
EMAIL;PREF;INTERNET:juergen.mueller@example.org\r\n\
EMAIL;INTERNET;HOME:jm@privat.example.org\r\n\
ADR;HOME;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:;;Gartenstra=C3=9Fe 12;K=C3=B6ln;NW;50667;Deutschland\r\n\
LABEL;HOME;ENCODING=QUOTED-PRINTABLE:Gartenstra=C3=9Fe 12=0D=0A50667 K=C3=B6ln\r\n\
NOTE;ENCODING=QUOTED-PRINTABLE:Erste Zeile=0D=0AZweite Zeile, mit einem sehr langen Text der =\r\n\
umbrochen wird=3B und noch einmal =\r\n\
umbrochen.\r\n\
URL;WORK:http://example.org/~jm\r\n\
BDAY:19720314\r\n\
REV:20260901T101500Z\r\n\
END:VCARD\r\n";

    #[test]
    fn reads_a_2_1_card() {
        let got = parse(V21);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0],
            Contact {
                display_name: "Dr. Jürgen Müller".into(),
                first: "Jürgen".into(),
                last: "Müller".into(),
                company: "Beispiel Werke, GmbH".into(),
                department: "Entwicklung".into(),
                title: "Geschäftsführer".into(),
                emails: vec![Labeled::new("", "juergen.mueller@example.org"), Labeled::new("home", "jm@privat.example.org")],
                phones: vec![
                    Labeled::new("work", "+49 30 5550100"),
                    Labeled::new("mobile", "+49 170 5550101"),
                    Labeled::new("fax", "+49 30 5550199"),
                    Labeled::new("pager", "+49 30 5550177")
                ],
                addresses: vec![PostalAddress {
                    label: "home".into(),
                    street: "Gartenstraße 12".into(),
                    city: "Köln".into(),
                    region: "NW".into(),
                    postal_code: "50667".into(),
                    country: "Deutschland".into()
                }],
                websites: vec!["http://example.org/~jm".into()],
                birthday: Some(d(1972, 3, 14)),
                notes: "Erste Zeile\nZweite Zeile, mit einem sehr langen Text der umbrochen wird; und noch einmal umbrochen.".into(),
                ..Default::default()
            }
        );
    }

    /// A vCard 3.0 card as address books export them: grouped properties with custom labels, a
    /// photo, folded lines.
    const V30: &str = "BEGIN:VCARD\r\n\
VERSION:3.0\r\n\
PRODID:-//Example//Contacts 12.0//EN\r\n\
N:Okafor;Nadia;Chidi,Rose;Dr.;\r\n\
FN:Dr. Nadia Okafor\r\n\
NICKNAME:Nadi\r\n\
ORG:Example Research\\, Inc.;Field Lab;Unit 7\r\n\
TITLE:Lead Scientist\r\n\
item1.EMAIL;type=INTERNET;type=WORK;type=pref:nadia.okafor@example.org\r\n\
item2.EMAIL;type=INTERNET:nadia@family.example.org\r\n\
item2.X-ABLabel:Family\r\n\
TEL;type=CELL;type=VOICE;type=pref:+1 (555) 010-0142\r\n\
item3.TEL:+1 555 010 0143\r\n\
item3.X-ABLabel:_$!<Other>!$_\r\n\
TEL;TYPE=WORK,FAX:+1 555 010 0144\r\n\
item4.ADR;type=WORK;type=pref:;Suite 300;41 Harbor Road;Riverton;CA;94000;USA\r\n\
item4.X-ABADR:us\r\n\
item5.URL;type=pref:https\\://example.org/~nadia\r\n\
item5.X-ABLabel:_$!<HomePage>!$_\r\n\
BDAY:1985-04-12\r\n\
NOTE:Met at the 2024 symposium.\\nPrefers e-mail\\, not calls; see the long thread about the field s\r\n tation.\r\n\
CATEGORIES:Research,Field Team\\, West\r\n\
PHOTO;ENCODING=b;TYPE=JPEG:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1\r\n Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1u\r\n b3BxcnN0dXZ3eHl6e3x9fn8=\r\n\
UID:7f1c2f6e-0001-4a5b-9c3d-000000000042\r\n\
X-FAVORITE:TRUE\r\n\
REV:2026-09-01T10:15:00Z\r\n\
END:VCARD\r\n";

    #[test]
    fn reads_a_3_0_card_with_grouped_properties() {
        let got = parse(V30);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0],
            Contact {
                display_name: "Dr. Nadia Okafor".into(),
                first: "Nadia".into(),
                middle: "Chidi Rose".into(),
                last: "Okafor".into(),
                nickname: "Nadi".into(),
                company: "Example Research, Inc.".into(),
                department: "Field Lab, Unit 7".into(),
                title: "Lead Scientist".into(),
                emails: vec![Labeled::new("work", "nadia.okafor@example.org"), Labeled::new("Family", "nadia@family.example.org")],
                phones: vec![
                    Labeled::new("mobile", "+1 (555) 010-0142"),
                    Labeled::new("other", "+1 555 010 0143"),
                    Labeled::new("fax", "+1 555 010 0144")
                ],
                addresses: vec![PostalAddress {
                    label: "work".into(),
                    street: "Suite 300, 41 Harbor Road".into(),
                    city: "Riverton".into(),
                    region: "CA".into(),
                    postal_code: "94000".into(),
                    country: "USA".into()
                }],
                websites: vec!["https://example.org/~nadia".into()],
                birthday: Some(d(1985, 4, 12)),
                notes: "Met at the 2024 symposium.\nPrefers e-mail, not calls; see the long thread about the field station.".into(),
                categories: vec!["Research".into(), "Field Team, West".into()],
                favorite: true,
                uid: "7f1c2f6e-0001-4a5b-9c3d-000000000042".into(),
                ..Default::default()
            }
        );
    }

    /// A vCard 4.0 card: URIs for phones, quoted type lists, no `N` needed.
    const V40: &str = "BEGIN:VCARD\r\n\
VERSION:4.0\r\n\
KIND:individual\r\n\
FN:Mei Tanaka\r\n\
N:Tanaka;Mei;;;\r\n\
EMAIL;TYPE=work;PREF=1:mei.tanaka@example.org\r\n\
EMAIL;TYPE=home:mailto:mei@home.example.org\r\n\
TEL;VALUE=uri;TYPE=\"voice,cell\";PREF=1:tel:+81-90-5550-0101\r\n\
TEL;VALUE=uri;TYPE=work:tel:+81-3-5550-0102;ext=12\r\n\
ADR;TYPE=home;LABEL=\"1-2-3 Sakura\\nMinato, Tokyo\":;;1-2-3 Sakura;Minato;Tokyo;105-0000;Japan\r\n\
BDAY:19910704T080000Z\r\n\
ANNIVERSARY:20150620\r\n\
GENDER:F\r\n\
LANG;PREF=1:ja\r\n\
URL:https://example.org/mei\r\n\
URL;TYPE=work:https://design.example.org/team/mei\r\n\
NOTE:Speaks Japanese\\, English.\r\n\
NOTE:Second note.\r\n\
CATEGORIES:Design,Tokyo office\r\n\
CATEGORIES:Speakers\r\n\
UID:urn:uuid:0d5c7e0a-0000-4000-8000-0000000000a1\r\n\
X-MAILCRAFT-FAVORITE:true\r\n\
END:VCARD\r\n";

    #[test]
    fn reads_a_4_0_card() {
        let got = parse(V40);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0],
            Contact {
                display_name: "Mei Tanaka".into(),
                first: "Mei".into(),
                last: "Tanaka".into(),
                emails: vec![Labeled::new("work", "mei.tanaka@example.org"), Labeled::new("home", "mei@home.example.org")],
                phones: vec![Labeled::new("mobile", "+81-90-5550-0101"), Labeled::new("work", "+81-3-5550-0102;ext=12")],
                addresses: vec![PostalAddress {
                    label: "home".into(),
                    street: "1-2-3 Sakura".into(),
                    city: "Minato".into(),
                    region: "Tokyo".into(),
                    postal_code: "105-0000".into(),
                    country: "Japan".into()
                }],
                websites: vec!["https://example.org/mei".into(), "https://design.example.org/team/mei".into()],
                birthday: Some(d(1991, 7, 4)),
                notes: "Speaks Japanese, English.\nSecond note.".into(),
                categories: vec!["Design".into(), "Tokyo office".into(), "Speakers".into()],
                favorite: true,
                uid: "urn:uuid:0d5c7e0a-0000-4000-8000-0000000000a1".into(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn reads_many_cards_in_order() {
        let text = format!("{V21}\r\n{V30}junk between cards\r\n{V40}");
        let names: Vec<String> = parse(&text).into_iter().map(|c| c.display_name).collect();
        assert_eq!(names, ["Dr. Jürgen Müller", "Dr. Nadia Okafor", "Mei Tanaka"]);
        // LF only, lower case, no VERSION.
        let plain = parse("begin:vcard\nfn:Ada Lovelace\nemail:ada@example.org\nend:vcard\nBEGIN:VCARD\nFN:Grace Hopper\nEND:VCARD\n");
        assert_eq!(plain.len(), 2);
        assert_eq!((plain[0].display_name.as_str(), plain[0].emails[0].value.as_str()), ("Ada Lovelace", "ada@example.org"));
    }

    #[test]
    fn a_missing_name_leaves_the_display_name_empty() {
        let c = card("N:Lovelace;Ada;;;\nEMAIL:ada@example.org");
        assert_eq!(c.display_name, "");
        assert_eq!(c.name(), "Ada Lovelace");
        let org = card("ORG:Analytical Engines Ltd");
        assert_eq!((org.display_name.as_str(), org.name().as_str()), ("", "Analytical Engines Ltd"));
        // The first FN, N and so on count.
        let twice = card("FN:First\nFN:Second\nN:A;B\nN:C;D\nTITLE:One\nTITLE:Two\nUID:1\nUID:2\nBDAY:2000-01-02\nBDAY:1999-01-02");
        assert_eq!(
            (twice.display_name.as_str(), twice.last.as_str(), twice.title.as_str(), twice.uid.as_str(), twice.birthday),
            ("First", "A", "One", "1", Some(d(2000, 1, 2)))
        );
    }

    #[test]
    fn labels_come_from_types_and_groups() {
        let c = card(
            "EMAIL;TYPE=INTERNET,OTHER:o@example.org\nEMAIL;TYPE=x-custom:x@example.org\nEMAIL;TYPE=HOME,WORK:hw@example.org\n\
TEL;TYPE=HOME,VOICE:1\nTEL;TYPE=WORK,CELL:2\nTEL;TYPE=iPhone:3\nTEL;TYPE=HOME;TYPE=FAX:4\nTEL;TYPE=OTHER:5\nTEL;TYPE=VOICE:6\nTEL:7\n\
Item7.TEL;TYPE=CELL:8\nITEM7.X-ABLabel:Car\nitem8.TEL:9\nitem8.X-ABLabel:_$!<Mobile>!$_\nitem8.X-ABLabel:Ignored\nX-ABLabel:No group\nitem9.X-ABLabel:Nothing to label\n\
ADR;TYPE=dom,postal,parcel:;;1 Main St;Riverton\nitem2.ADR:;;2 Side St\nitem2.X-ABLabel:Cabin",
        );
        let labels = |list: &[Labeled]| list.iter().map(|l| l.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&c.emails), ["other", "", "work"]);
        assert_eq!(labels(&c.phones), ["home", "mobile", "mobile", "fax", "other", "", "", "Car", "mobile"]);
        assert_eq!(
            c.addresses.iter().map(|a| (a.label.as_str(), a.street.as_str())).collect::<Vec<_>>(),
            [("", "1 Main St"), ("Cabin", "2 Side St")]
        );
    }

    #[test]
    fn names_and_organisations_split_on_unescaped_separators() {
        let c = card("N:van der Berg\\; Jr.;Anna\\,Maria;Q.,R.;;\nORG:Semi\\;colon & Co;R\\&D;;Lab 2\nNICKNAME:Annie\\, Am");
        assert_eq!((c.last.as_str(), c.first.as_str(), c.middle.as_str()), ("van der Berg; Jr.", "Anna,Maria", "Q. R."));
        assert_eq!((c.company.as_str(), c.department.as_str()), ("Semi;colon & Co", "R\\&D, Lab 2"));
        assert_eq!(c.nickname, "Annie, Am");
        // Fewer parts than the format has, and none at all.
        assert_eq!(card("N:Lovelace").last, "Lovelace");
        assert_eq!(card("N:;;;;\nFN:x").first, "");
        assert_eq!(card("ORG:;Only a department").department, "Only a department");
        // In 2.1 a comma is a comma.
        let old = parse("BEGIN:VCARD\nVERSION:2.1\nN:Smith, Jr.;John\nADR:;;1 Main St, Apt 4;Riverton\nEND:VCARD\n").remove(0);
        assert_eq!((old.last.as_str(), old.addresses[0].street.as_str()), ("Smith, Jr.", "1 Main St, Apt 4"));
    }

    #[test]
    fn addresses_join_the_lines_above_the_city() {
        let c =
            card("ADR:PO Box 9;Floor 2;41 Harbor Road,Building C;Riverton;CA;94000;USA\nADR:;;;;;;\nADR;TYPE=HOME:;;;;;;Japan\nADR:;;Just a street");
        assert_eq!(c.addresses.len(), 3);
        assert_eq!(c.addresses[0].street, "PO Box 9, Floor 2, 41 Harbor Road, Building C");
        assert_eq!(c.addresses[0].one_line(), "PO Box 9, Floor 2, 41 Harbor Road, Building C, Riverton CA 94000, USA");
        assert_eq!((c.addresses[1].label.as_str(), c.addresses[1].country.as_str()), ("home", "Japan"));
        assert_eq!(c.addresses[2].street, "Just a street");
    }

    #[test]
    fn birthdays_in_every_form() {
        for (text, want) in [
            ("1985-04-12", Some(d(1985, 4, 12))),
            ("19850412", Some(d(1985, 4, 12))),
            ("1985-04-12T10:30:00Z", Some(d(1985, 4, 12))),
            ("19850412T103000", Some(d(1985, 4, 12))),
            (" 1985-04-12 10:30 ", Some(d(1985, 4, 12))),
            ("--0412", None),
            ("1985-02-30", None),
            ("spring", None),
            ("", None),
        ] {
            assert_eq!(birthday(text), want, "{text}");
        }
        assert_eq!(card("FN:x\nBDAY;VALUE=date:2000-02-29").birthday, Some(d(2000, 2, 29)));
        // A later valid one counts when the first is junk.
        assert_eq!(card("FN:x\nBDAY:--0412\nBDAY:1985-04-12").birthday, Some(d(1985, 4, 12)));
    }

    #[test]
    fn favorites_and_single_line_fields() {
        for (line, want) in [
            ("X-FAVORITE:TRUE", true),
            ("X-MAILCRAFT-FAVORITE:true", true),
            ("X-FAVORITE:1", true),
            ("X-FAVORITE: yes ", true),
            ("X-FAVORITE:FALSE", false),
            ("X-FAVORITE:", false),
        ] {
            assert_eq!(card(&format!("FN:x\n{line}")).favorite, want, "{line}");
        }
        assert!(card("FN:x\nX-FAVORITE:TRUE\nX-FAVORITE:FALSE").favorite);
        // Line breaks have no place in a name, an address or a number.
        let c =
            card("FN:  Ada\\nLovelace  \nEMAIL: ada@example.org \nTEL:+1 555\\n0100\nURL: https://example.org \nUID: abc \nCATEGORIES: a , ,b\\nc ");
        assert_eq!(c.display_name, "Ada Lovelace");
        assert_eq!(
            (c.emails[0].value.as_str(), c.phones[0].value.as_str(), c.websites[0].as_str(), c.uid.as_str()),
            ("ada@example.org", "+1 555 0100", "https://example.org", "abc")
        );
        assert_eq!(c.categories, ["a", "b c"]);
        // Empty values are not entries.
        let empty = card("FN:x\nEMAIL:\nEMAIL;TYPE=WORK:  \nTEL:\nURL:\nNOTE:\nCATEGORIES:,,");
        assert!(
            empty.emails.is_empty() && empty.phones.is_empty() && empty.websites.is_empty() && empty.categories.is_empty() && empty.notes.is_empty()
        );
    }

    #[test]
    fn base64_text_and_unknown_encodings() {
        // "Hello, world" as an old 2.1 writer and a 3.0 writer encode it.
        assert_eq!(card("NOTE;ENCODING=BASE64:SGVsbG8sIHdvcmxk").notes, "Hello, world");
        assert_eq!(card("NOTE;ENCODING=b:SGVsbG8s\n IHdvcmxk").notes, "Hello, world");
        assert_eq!(card("NOTE;ENCODING=8BIT:as it is").notes, "as it is");
        assert_eq!(card("FN:x\nNOTE;ENCODING=b:!!!").notes, "");
        // Quoted-printable in a named Western charset, and in none.
        assert_eq!(card("FN;CHARSET=windows-1252;ENCODING=QUOTED-PRINTABLE:Zo=EB =96 caf=E9").display_name, "Zoë – café");
        assert_eq!(card("FN;QUOTED-PRINTABLE:Zo=EB").display_name, "Zoë");
        assert_eq!(card("FN;QUOTED-PRINTABLE:Zo=C3=AB").display_name, "Zoë");
    }

    #[test]
    fn central_european_charsets() {
        // A 2.1 card from a Central European phone or mail program, in Windows-1250 and in
        // ISO-8859-2, was read as Windows-1252 or UTF-8.
        let cards = parse(
            "BEGIN:VCARD\r\n\
VERSION:2.1\r\n\
N;CHARSET=windows-1250;ENCODING=QUOTED-PRINTABLE:=8Eagar;=D0ur=F0ica;;;\r\n\
FN;CHARSET=windows-1250;ENCODING=QUOTED-PRINTABLE:=D0ur=F0ica =8Eagar\r\n\
ORG;CHARSET=windows-1250;ENCODING=QUOTED-PRINTABLE:Gvo=9E=F0e d.o.o.\r\n\
NOTE;CHARSET=windows-1250;ENCODING=QUOTED-PRINTABLE:We=92ve met =96 twice, in =8Aibenik =\r\n\
and =C8akovec.\r\n\
EMAIL;INTERNET:durdica@example.org\r\n\
END:VCARD\r\n\
BEGIN:VCARD\r\n\
VERSION:2.1\r\n\
N;CHARSET=ISO-8859-2;ENCODING=QUOTED-PRINTABLE:Kova=E8i=E6;=AEeljko;;;\r\n\
FN;CHARSET=ISO-8859-2;ENCODING=QUOTED-PRINTABLE:=AEeljko Kova=E8i=E6\r\n\
TITLE;CHARSET=iso-8859-2;ENCODING=QUOTED-PRINTABLE:Gvo=BE=F0e i =B9ljunak\r\n\
EMAIL;INTERNET:zeljko@example.org\r\n\
END:VCARD\r\n",
        );
        assert_eq!(cards.len(), 2);
        let (first, second) = (&cards[0], &cards[1]);
        assert_eq!((first.display_name.as_str(), first.first.as_str(), first.last.as_str()), ("Đurđica Žagar", "Đurđica", "Žagar"));
        assert_eq!(first.company, "Gvožđe d.o.o.");
        assert_eq!(first.notes, "We’ve met – twice, in Šibenik and Čakovec.");
        assert_eq!((second.display_name.as_str(), second.first.as_str(), second.last.as_str()), ("Željko Kovačić", "Željko", "Kovačić"));
        assert_eq!(second.title, "Gvožđe i šljunak");
        // A set no standard names is read as UTF-8, as mail is.
        assert_eq!(card("FN;CHARSET=x-unknown;ENCODING=QUOTED-PRINTABLE:Zo=C3=AB").display_name, "Zoë");
    }

    #[test]
    fn raw_8_bit_values_are_read_in_their_charset() {
        // Raw Windows-1250 bytes (not quoted-printable) were read as UTF-8 before the charset
        // was looked at, and đ ć č Š became U+FFFD.
        let cards = parse_bytes(
            b"BEGIN:VCARD\r\nVERSION:2.1\r\n\
N;CHARSET=WINDOWS-1250:Gvo\xf0\xe6i\xe8;Ivana;;;\r\n\
FN;CHARSET=WINDOWS-1250:Ivana Gvo\xf0\xe6i\xe8\r\n\
ORG;CHARSET=WINDOWS-1250:\x8akola \xe8e\x9a\xe6e\r\n\
NOTE;CHARSET=\"iso-8859-2\":Gvo\xbe\xf0e\r\n \xae\xb9\xe8\xe6\r\n\
TITLE;CHARSET=windows-1250;ENCODING=QUOTED-PRINTABLE:=8Aef\r\n\
X-SOMETHING:\xff\r\n\
EMAIL;INTERNET:ivana@example.org\r\n\
END:VCARD\r\n",
        );
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!((c.display_name.as_str(), c.last.as_str(), c.first.as_str()), ("Ivana Gvođćič", "Gvođćič", "Ivana"));
        assert_eq!(c.company, "Škola češće");
        // A folded line goes on in the charset of the property it continues (2.1 keeps the space).
        assert_eq!(c.notes, "Gvožđe Žščć");
        assert_eq!(c.title, "Šef");
        assert_eq!(c.emails[0].value, "ivana@example.org");
        // Without a charset: UTF-8 when it is UTF-8, else Windows-1252; in UTF-8 nothing changes.
        assert_eq!(parse_bytes(b"BEGIN:VCARD\nFN:Zo\xc3\xab\nEND:VCARD")[0].display_name, "Zoë");
        assert_eq!(parse_bytes(b"BEGIN:VCARD\nFN:Zo\xeb \x96 caf\xe9\nEND:VCARD")[0].display_name, "Zoë – café");
        assert_eq!(parse_bytes("\u{feff}BEGIN:VCARD\r\nFN;CHARSET=UTF-8:Đurđica\r\nEND:VCARD".as_bytes())[0].display_name, "Đurđica");
        // A file in UTF-16 with its byte order mark.
        let utf16: Vec<u8> = "\u{feff}BEGIN:VCARD\r\nFN:Željko\r\nEND:VCARD\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(parse_bytes(&utf16)[0].display_name, "Željko");
        // Bytes and text give the same cards.
        let text = crate::write(&cards.iter().collect::<Vec<_>>());
        assert_eq!(parse_bytes(text.as_bytes()), parse(&text));
    }

    #[test]
    fn skipped_properties_cost_nothing() {
        let photo = "A".repeat(3 << 20);
        let c = card(&format!(
            "FN:Ada\nPHOTO;ENCODING=b;TYPE=JPEG:{photo}\nLOGO;VALUE=uri:https://example.org/logo.png\nSOUND;ENCODING=QUOTED-PRINTABLE:=41=42\nKEY;TYPE=PGP:{photo}\nX-UNKNOWN;ENCODING=b:{photo}\nTITLE:After"
        ));
        assert_eq!((c.display_name.as_str(), c.title.as_str()), ("Ada", "After"));
        assert_eq!(c, Contact { display_name: "Ada".into(), title: "After".into(), ..Default::default() });
    }

    #[test]
    fn junk_gives_an_empty_list() {
        for junk in [
            "",
            "\r\n\r\n",
            "hello",
            "BEGIN",
            "BEGIN:",
            "END:VCARD",
            "BEGIN:VCARD",
            "BEGIN:VCARD\nEND:VCARD",
            "BEGIN:VCARD\nVERSION:3.0\nN:;;;;\nFN:\nEND:VCARD",
            "FN:Nobody\nEMAIL:n@example.org",
            "\u{0}\u{1}\u{2}\u{fffd}PK\u{3}\u{4}",
            "::::;;;;====",
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:x\nEND:VEVENT\nEND:VCALENDAR",
        ] {
            assert!(parse(junk).is_empty(), "{junk:?}");
        }
        let binary: String = (0u32..2000).filter_map(|i| char::from_u32(i.wrapping_mul(2_654_435_761) % 0x11_0000)).collect();
        assert!(parse(&binary).is_empty());
    }

    #[test]
    fn a_missing_end_loses_nothing() {
        // The text stops in the middle of the second card.
        let cut = parse("BEGIN:VCARD\nFN:Ada\nEND:VCARD\nBEGIN:VCARD\nFN:Grace\nEMAIL:grace@exam");
        assert_eq!(cut.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(), ["Ada", "Grace"]);
        assert_eq!(cut[1].emails[0].value, "grace@exam");
        // A card that never ends, and two that do: each keeps its own lines, in the order they began.
        let run_on = parse("BEGIN:VCARD\nFN:Ada\nBEGIN:VCARD\nFN:Grace\nTITLE:Admiral\nEND:VCARD\nBEGIN:VCARD\nFN:Mei\nEND:VCARD\n");
        assert_eq!(
            run_on.iter().map(|c| (c.display_name.as_str(), c.title.as_str())).collect::<Vec<_>>(),
            [("Ada", ""), ("Grace", "Admiral"), ("Mei", "")]
        );
    }

    #[test]
    fn nested_cards_and_foreign_components_are_contained() {
        // A 2.1 AGENT holds a whole card; the outer card goes on after it.
        let agent = parse(
            "BEGIN:VCARD\nVERSION:2.1\nFN:Nadia Okafor\nAGENT:\nBEGIN:VCARD\nVERSION:2.1\nFN:Tomas Lindqvist\nTEL;WORK:+1 555 0100\nEND:VCARD\nTITLE:Director\nEND:VCARD\n",
        );
        assert_eq!(agent.len(), 2);
        assert_eq!((agent[0].display_name.as_str(), agent[0].title.as_str(), agent[0].phones.len()), ("Nadia Okafor", "Director", 0));
        assert_eq!((agent[1].display_name.as_str(), agent[1].title.as_str(), agent[1].phones.len()), ("Tomas Lindqvist", "", 1));
        // Something that isn't a card keeps its lines to itself, ended or not.
        let foreign = parse(
            "BEGIN:VCARD\nFN:Ada\nBEGIN:X-EXTRA\nTITLE:Inner\nBEGIN:X-DEEPER\nNOTE:Inner\nEND:X-DEEPER\nEND:X-EXTRA\nTITLE:Outer\nBEGIN:X-OPEN\nNOTE:Inner\nEND:VCARD\n",
        );
        assert_eq!(foreign, [Contact { display_name: "Ada".into(), title: "Outer".into(), ..Default::default() }]);
        // Stray ENDs, and nesting deeper than is followed: every card still comes out once.
        assert!(parse("END:VCARD\nEND:VCARD\nEND:X\n").is_empty());
        let deep: String = (0..50).map(|i| format!("BEGIN:VCARD\nFN:Card {i}\n")).collect();
        let got = parse(&deep);
        assert_eq!(got.len(), 50);
        assert_eq!(got.iter().map(|c| c.display_name.clone()).collect::<Vec<_>>(), (0..50).map(|i| format!("Card {i}")).collect::<Vec<_>>());
    }

    #[test]
    fn sizes_and_counts_are_capped() {
        let note = "é".repeat(1 << 20);
        let c = card(&format!("FN:{}\nNOTE:{note}", "x".repeat(3 << 20)));
        assert!(c.display_name.len() <= 1 << 20 && c.display_name.len() > 1_000_000);
        assert!(c.notes.len() <= 1 << 20);
        let crowded = card(&format!(
            "FN:x\n{}{}{}{}CATEGORIES:{}\n{}",
            "EMAIL:someone@example.org\n".repeat(MAX_ITEMS + 20),
            "TEL:+1 555 0100\n".repeat(MAX_ITEMS + 20),
            "ADR:;;1 Main St\n".repeat(MAX_ITEMS + 20),
            "URL:https://example.org\n".repeat(MAX_ITEMS + 20),
            "a,".repeat(MAX_CATEGORIES + 20),
            (0..MAX_LABELS + 20).map(|i| format!("g{i}.X-ABLabel:L{i}\n")).collect::<String>(),
        ));
        assert_eq!(
            (crowded.emails.len(), crowded.phones.len(), crowded.addresses.len(), crowded.websites.len(), crowded.categories.len()),
            (MAX_ITEMS, MAX_ITEMS, MAX_ITEMS, MAX_ITEMS, MAX_CATEGORIES)
        );
        let many = "BEGIN:VCARD\nFN:x\nEND:VCARD\n".repeat(MAX_CARDS + 10);
        assert_eq!(parse(&many).len(), MAX_CARDS);
    }
}
