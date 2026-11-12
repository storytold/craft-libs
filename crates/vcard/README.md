# craft-vcard

vCard: contacts as vCard text (`.vcf` files, CardDAV address data).

- `parse(text)` reads vCard 2.1, 3.0 and 4.0, one card or many in a text. `parse_bytes(bytes)`
  reads a file: UTF-8, UTF-16 with a byte order mark, and vCard 2.1 values each in their own
  `CHARSET` (quoted-printable or raw), decoded by `craft_mime::charset`.
- `write(&[&Contact])` writes vCard 3.0, the version every address book reads. Lines are folded
  at 75 octets, values are escaped, and no value can start a new property or card.
- `Contact` is this crate's own plain model: names, organisation, labelled e-mail addresses and
  phone numbers, postal addresses, web sites, birthday (`chrono::NaiveDate`), notes, categories,
  favourite, `UID` and `KEY`.
- Labels come from `TYPE` in all three versions' spellings and from Apple's `itemN.X-ABLabel`
  groups, and are written back the same way.
- `KEY` keeps S/MIME certificates and OpenPGP public keys in every form the three versions have
  (base64, `data:` URIs, PEM, ASCII armour); a key given as an address is kept as an address and
  never fetched. Anything with the shape of secret key material is dropped.

## Hostile input

Nothing panics on any text. What makes no sense is skipped.

| Limit | Value |
|---|---|
| cards in one text | 100,000 |
| nesting of `BEGIN:VCARD` | 8 |
| unfolded line | 1 MiB |
| parameters on a line | 64 |
| e-mail addresses, phones, addresses, web sites per card | 100 each |
| categories per card | 500 |
| group labels per card | 400 |
| keys per card (`Contact::MAX_KEYS`) | 32 |
| one key (`ContactKey::MAX_BYTES`) | 64 KiB |
| key URI (`ContactKey::MAX_URI`) | 2048 bytes |
| key bytes kept from one text | 64 MiB |

## What stays in the apps

Ids, where a contact came from, merging and de-duplication, photos (not read), and the mapping
of `Contact` onto the app's own contact type.

## Origin

MailCraft `crates/vcard` at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), with its tests. The model
types are the vCard fields of MailCraft's `Contact`; the birthday is a `chrono::NaiveDate` where
MailCraft has its own date type.
