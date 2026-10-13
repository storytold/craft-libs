//! Contacts over CardDAV, for the share picker.
//!
//! Nextcloud serves address books at `/remote.php/dav/addressbooks/users/<user>/`. Listing the
//! books is a `PROPFIND` with `Depth: 1`; reading one is a `REPORT` carrying an
//! `addressbook-query`, which answers `207 Multi-Status` with a vCard per entry.
//!
//! Only the fields a picker needs are read: the identifier, a display name, and the addresses.
//! Everything else in the vCard is skipped. Written from [RFC 6352] and [RFC 6350].
//!
//! Contacts come from CardDAV. CalDAV serves calendars and has no part in this.
//!
//! [RFC 6352]: https://www.rfc-editor.org/rfc/rfc6352
//! [RFC 6350]: https://www.rfc-editor.org/rfc/rfc6350

use crate::error::{Error, Result};
use crate::http::{Http, Method};
use crate::{Account, send, xml};

/// Most vCards one `REPORT` will be read from, as a bound on a very large address book.
const MAX_CARDS: usize = 20_000;
/// Longest vCard line accepted after unfolding.
const MAX_LINE: usize = 8 * 1024;

const BOOKS_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
    <cs:getctag/>
  </d:prop>
</d:propfind>"#;

const QUERY_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<card:addressbook-query xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <d:prop>
    <d:getetag/>
    <card:address-data/>
  </d:prop>
</card:addressbook-query>"#;

/// One address book.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressBook {
    /// Server path, as returned. Pass it straight back to [`Contacts::list`].
    pub href: String,
    pub display_name: String,
}

/// One contact, reduced to what a picker shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    /// vCard `UID`. Stable across edits, and the right key for deduplication.
    pub uid: String,
    /// vCard `FN`, the formatted name.
    pub name: String,
    pub emails: Vec<String>,
    /// Nextcloud user id, when this contact is a local account. That is the value a user share
    /// needs as `shareWith`, so a picker should prefer it over an address.
    pub user_id: String,
}

impl Contact {
    /// What to hand the OCS Share API as `shareWith`.
    ///
    /// A local user id is preferred, because a user share needs one. An address is the fallback,
    /// which suits an email share.
    pub fn share_target(&self) -> Option<&str> {
        if !self.user_id.is_empty() {
            return Some(&self.user_id);
        }
        self.emails.first().map(String::as_str)
    }
}

/// Contact lookups against one account.
pub struct Contacts<'a> {
    account: &'a Account,
}

impl<'a> Contacts<'a> {
    pub fn new(account: &'a Account) -> Contacts<'a> {
        Contacts { account }
    }

    /// List this user's address books.
    ///
    /// The home collection itself comes back first and is dropped, as is any child that is not an
    /// address book.
    pub fn books<H: Http>(&self, http: &H) -> Result<Vec<AddressBook>> {
        let root = self.account.addressbook_root();
        let req = self
            .account
            .request(Method::Propfind, format!("{root}/"))
            .header("depth", "1")
            .header("content-type", "application/xml; charset=utf-8")
            .body(BOOKS_BODY.as_bytes().to_vec());
        let resp = send(http, req)?;

        let mut out = Vec::new();
        for group in xml::groups(&resp.body, "response")? {
            let Some(href) = group.get("href") else { continue };
            // Only a real address book carries <card:addressbook/> in its resourcetype. This also
            // drops the home collection, which does not.
            if !group.has("addressbook") {
                continue;
            }
            let display_name = group.get("displayname").map(str::to_owned).unwrap_or_else(|| last_segment(href));
            out.push(AddressBook { href: href.to_owned(), display_name });
        }
        Ok(out)
    }

    /// Read every contact in one address book.
    ///
    /// `book_href` is the `href` from [`Contacts::books`]. It is a server path, so it is resolved
    /// against the account's base rather than re-encoded.
    pub fn list<H: Http>(&self, http: &H, book_href: &str) -> Result<Vec<Contact>> {
        let url = self.resolve(book_href)?;
        let req = self
            .account
            .request(Method::Report, url)
            .header("depth", "1")
            .header("content-type", "application/xml; charset=utf-8")
            .body(QUERY_BODY.as_bytes().to_vec());
        let resp = send(http, req)?;

        let groups = xml::groups(&resp.body, "response")?;
        let mut out = Vec::new();
        for group in groups.iter().take(MAX_CARDS) {
            let Some(card) = group.get("address-data") else { continue };
            if let Some(contact) = parse_vcard(card) {
                out.push(contact);
            }
        }
        if groups.len() > MAX_CARDS {
            log::warn!("address book has {} entries; read the first {MAX_CARDS}", groups.len());
        }
        Ok(out)
    }

    /// Every contact across every address book, deduplicated by `UID`.
    pub fn all<H: Http>(&self, http: &H) -> Result<Vec<Contact>> {
        let mut out: Vec<Contact> = Vec::new();
        for book in self.books(http)? {
            for contact in self.list(http, &book.href)? {
                if contact.uid.is_empty() || !out.iter().any(|c| c.uid == contact.uid) {
                    out.push(contact);
                }
            }
        }
        Ok(out)
    }

    /// Turn a server path into an absolute URL, leaving an already-absolute one alone.
    fn resolve(&self, href: &str) -> Result<String> {
        if href.starts_with("http://") || href.starts_with("https://") {
            return Ok(href.to_owned());
        }
        if !href.starts_with('/') {
            return Err(Error::invalid("address book href", format!("{href:?} is neither absolute nor rooted")));
        }
        Ok(format!("{}{}", self.account.base(), href))
    }
}

/// Pull the fields a picker needs out of one vCard.
///
/// Returns `None` when the card carries neither a name nor an address, because such an entry
/// cannot be shown or shared with. Unknown properties and parameters are skipped.
pub(crate) fn parse_vcard(card: &str) -> Option<Contact> {
    let mut contact = Contact::default();
    for line in unfold(card) {
        // A property line is NAME[;PARAM=VALUE...]:VALUE. Split on the first colon that is not
        // inside a quoted parameter.
        let Some((head, value)) = split_property(&line) else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let name = head.split(';').next().unwrap_or_default().trim().to_ascii_uppercase();
        match name.as_str() {
            "UID" => contact.uid = unescape_value(value),
            "FN" if contact.name.is_empty() => contact.name = unescape_value(value),
            "EMAIL" => {
                let addr = unescape_value(value);
                if !contact.emails.contains(&addr) {
                    contact.emails.push(addr);
                }
            }
            // Nextcloud stamps the local account id on contacts that are server users. The
            // property name has varied, so both spellings are accepted.
            "X-NC-USER-ID" | "X-NEXTCLOUD-USER-ID" => contact.user_id = unescape_value(value),
            _ => {}
        }
    }
    if contact.name.is_empty() && contact.emails.is_empty() {
        return None;
    }
    if contact.name.is_empty() {
        contact.name = contact.emails.first().cloned().unwrap_or_default();
    }
    Some(contact)
}

/// Unfold vCard line continuations.
///
/// A line beginning with a space or a tab continues the previous one, with that first character
/// dropped ([RFC 6350] section 3.2). Lines longer than [`MAX_LINE`] after unfolding are dropped,
/// so a pathological card cannot grow without bound.
///
/// [RFC 6350]: https://www.rfc-editor.org/rfc/rfc6350#section-3.2
fn unfold(card: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in card.split('\n') {
        let line = raw.trim_end_matches('\r');
        match line.strip_prefix([' ', '\t']) {
            Some(rest) => {
                if let Some(last) = out.last_mut()
                    && last.len() + rest.len() <= MAX_LINE
                {
                    last.push_str(rest);
                }
            }
            None => out.push(line.to_owned()),
        }
    }
    out.retain(|l| !l.is_empty() && l.len() <= MAX_LINE);
    out
}

/// Split a vCard property line into its head and value at the first unquoted colon.
fn split_property(line: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Undo vCard value escaping: `\n`, `\,`, `\;` and `\\`.
fn unescape_value(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn last_segment(path: &str) -> String {
    crate::dav::decode(path.trim_end_matches('/').rsplit('/').next().unwrap_or_default())
}
