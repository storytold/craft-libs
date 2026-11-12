# craft-dav

WebDAV multistatus documents (RFC 4918 §13): what a server answers a `PROPFIND`, or a CalDAV
(RFC 4791) or CardDAV (RFC 6352) `REPORT`, with. No I/O.

```rust
let root = craft_dav::parse_multistatus(body)?;
for response in craft_dav::responses(&root) {
    let href = craft_dav::href(response);
    for property in craft_dav::properties(response) {
        match property.name.as_str() {
            "d:getetag" => { /* property.text() */ }
            "c:calendar-data" | "card:address-data" => { /* the iCalendar or vCard text */ }
            _ => {}
        }
    }
}
```

Names carry fixed prefixes whatever the document used: `d:` (`DAV:`), `c:` (CalDAV), `card:`
(CardDAV), `cs:` (CalendarServer), `oc:` (ownCloud), `nc:` (Nextcloud). Other namespaces start
with `?:`. `properties` gives only what a `200` propstat returned.

## Hostile input

The reader is built on `craft-xml` and fails closed: a document type declaration, an entity that
is not predefined, a character XML cannot carry, text that is not UTF-8, malformed or unbalanced
markup, a second root, text outside the root, or a root other than `DAV:multistatus` makes the
whole document an error.

| Limit | Value |
|---|---|
| `MAX_BYTES` | 16 MiB |
| `MAX_NODES` | 100,000 |
| `MAX_DEPTH` | 32 |
| `MAX_ATTRS` per element / `MAX_ATTRS_TOTAL` | 64 / 200,000 |

## What stays in the apps

Requests, credentials and HTTP; what a collection or resource means; reading the iCalendar and
vCard text (`craft-ical`, `craft-vcard`).

## Origin

MailCraft `crates/dav/src/xml.rs` at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), and its tests, re-based from
`quick-xml` onto `craft-xml`.
