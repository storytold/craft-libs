# craft-mime

Internet messages and MIME, read and written: `.eml` files, what a mail server hands over, MHT /
MHTML web archives and mbox files.

- **Reader.** `parse(bytes, &Limits)` gives a `Message`: a tree of `Part`s, each with its header
  fields in order, its `Content-Type` and parameters, and its body with the transfer encoding
  undone. Accessors give the media type, disposition, file name (RFC 2231 and encoded-words
  decoded), `Content-ID`, `Content-Location` and the charset-decoded text.
- **Writer.** `Entity::text`, `Entity::file` and `Entity::multipart` build the parts,
  `write_message` writes the message. Headers are folded at 78 columns, text is 7bit or
  quoted-printable, files are base64 in 76-column lines, and boundaries are derived from the
  content so the same input gives the same bytes.
- **The pieces**, for callers that work below the tree: `split_headers`, `write_field`,
  `parse_params`, `decode_words` / `encode_words`, `transfer::{base64, base64_decode, qp_encode,
  qp_decode}`, `charset::{decode, guess, is_known}`, with the `detect` feature `detect::{is_text, decode, legacy, legacy_label}` for bytes that name no character set, `parse_address_list` / `format_address_list`,
  `parse_date` / `format_date`, `split_multipart`, `mbox_messages` / `write_mbox_message`, and
  `Resources` / `start_part` / `resolve_location` for `multipart/related`.
- **Beside MIME**, what the same callers need: `parse_mailto` reads a `mailto:` link (RFC 6068:
  recipients, subject and body only, sizes capped, no line break into a header), `base64` is
  strict RFC 4648 (both alphabets, padded or not, one spelling per value) for keys, tokens and
  credentials, and `charset::code_page_label` names the character set of a Windows code page.

Written from the public RFCs: 5322 (messages), 2045–2049 (MIME), 2047 (encoded-words), 2231
(parameter values), 2387 and 2557 (`multipart/related`, MHTML), 4155 (mbox, `mboxrd` quoting).

```rust
use craft_mime::{Limits, Resources, parse, start_part};

let message = parse(bytes, &Limits::default());
let subject = message.root.headers.get("subject").map(craft_mime::decode_words);
for part in message.root.leaves() {
    println!("{} {:?} {} bytes", part.media_type(), part.file_name(), part.data().len());
}
// An MHT archive: the page and a picture it names.
let page = start_part(&message.root);
let picture = Resources::of(&message.root).find("images/logo.png", "https://example.test/page.html");
```

## Hostile input

The reader never fails and never panics. What can't be read is skipped; garbage gives a message
with no header fields and the bytes as its body. The work is linear in the input, and every
size, count and depth is capped by `Limits`. `Message::truncated` says that a limit left
something out.

| Limit | Default | What happens past it |
|---|---|---|
| `max_input` | 1 GiB | the rest of the input is not read |
| `max_header_fields` | 2000 per header block | further fields are read past |
| `max_header_value` | 256 KiB | continuation lines are dropped |
| `max_header_block` | 4 MiB per header block | further fields are read past |
| `max_depth` | 20 (never more than 100) | a deeper multipart has no parts |
| `max_parts` | 1000, multiparts included | further parts are not read |
| `max_decoded` | 1 GiB of decoded bodies in all | a part that would pass it has an empty body |
| `reserved_fields` | none | named fields are kept past the field and block caps, `max_reserved_each` (4) of each, each cut at `max_reserved_value` (64) bytes with a `…` |
| `MAX_PARAMS` (constant) | 64 per header value | further parameters are not read |
| `MAX_ADDRESSES` (constant) | 10,000 per address list | further addresses are not read |

The reader is tolerant of what real senders write: bare LF line ends, a missing blank line after
the headers, raw 8-bit header values, a multipart cut short, look-alike boundaries, missing
base64 padding and junk between base64 groups, stray `=` in quoted-printable, unquoted file
names with spaces, encoded-words where RFC 2231 belongs, comments in `Content-Type`, obsolete
date forms, addresses without commas between them.

## Hostile values

Nothing a caller gives the writer can add a header field, a part or a message:

- `write_field` drops CR, LF and NUL from every value and writes nothing for a name that is not
  a field name. `write_message` and `Entity::to_bytes` write every field through it.
- `format_address` strips from an address every character that could end it or start another;
  `clean_id` does the same for message and content ids; `safe_media_type` turns anything that is
  not `type/subtype`, and any `multipart/*`, into `application/octet-stream`.
- A multipart's subtype and parameter names must be tokens; parameter values are quoted. The
  boundary is checked against the encoded parts and changed until none of them holds it.
- `write_mbox_message` strips the sender of white space and quotes `From ` lines.

## Dates

`parse_date` and `format_date` work on `DateTime`, a plain struct (year, month, day, hour,
minute, second, offset in minutes east of UTC). No date library is in the public API, so an app
converts it to whatever it keeps time in (`chrono`, `jiff`, a Unix timestamp). `chrono` is a
private dependency, used for the weekday and for whether a date exists. The raw `Date` text is
always there as `headers.get("date")`.

## What stays in the apps

- What a message means: which text part is the body, which parts are attachments or inline
  pictures, how alternatives are chosen, HTML to text.
- Mail semantics: `Status` / `X-Status` flags, importance, keywords, kept-header policy, which
  header fields have encoded-words decoded, message-id generation, recipient caps.
- S/MIME and PGP: detecting protected messages, canonical CRLF form, keeping the original
  bytes. `split_headers` and `split_multipart` give the raw slices those need.
- File-name-to-media-type tables and default attachment names.
- For MHT: picking the page, reading its HTML, and the budget for pictures loaded.

## Migration

No app uses this crate yet. These tables say what replaces what.

### MailCraft (`crates/mime`, `crates/charset`)

| MailCraft | craft-mime |
|---|---|
| `header::split(bytes, top)` | `split_headers(bytes, top, &limits)`; the Exchange stamp fields go in `Limits::reserved_fields` |
| `header::Headers`, `Field`, `Params`, `parse_params` | the same names |
| `header::clean`, `write_field`, `is_token` | `clean`, `write_field` (now refuses a bad name), `is_field_name` |
| `words::decode_words`, `encode_words`, `encode_b`, `is_plain`, `WORD_BYTES` | the same names |
| `words::has_words` | stays (policy) |
| `transfer::base64_decode`, `base64`, `base64_lines`, `qp_decode`, `qp_encode` | `transfer::` the same names |
| `parse::decode(body, encoding)` | `transfer::decode` |
| `parse::split_multipart` | `split_multipart` |
| `parse::Walk` (the recursion over parts, depth and part caps) | `parse` / `parse_entity`, then a walk over `Part`s |
| `parse::is_media_type`, content id and file name extraction | `Part::declared_media_type`, `content_id`, `file_name`, `disposition` |
| `charset::decode`, `guess`, `unix_lines` (`mailcraft-charset`) | `charset::` the same names |
| `mailcraft_charset::base64` (strict) | `base64::` the same names (`STANDARD`, `STANDARD_NO_PAD`, `URL_SAFE`, `URL_SAFE_NO_PAD`, `Config`, `DecodeError`) |
| `mailcraft_pst::code_page_label` | `charset::code_page_label` |
| `address::parse_list`, `format_address`, `format_list` | `parse_address_list`, `format_address`, `format_address_list`, on `Mailbox { name, email }` |
| `mailcraft_time::parse_rfc5322`, `format_rfc5322` | `parse_date`, `format_date`, on `DateTime` |
| `mbox::mbox_messages`, `parse_mbox` | `mbox_messages` gives each message's bytes; MailCraft parses them |
| `mbox::write_mbox` | `write_mbox_message(out, sender, &date, bytes)` per message |
| `write::unstructured`, `kept` | `encode_unstructured`, `seven_bit_value` |
| `write::message_id` | `clean_id` |
| `write::Part`, `text_part`, `attachment_part`, `multipart` | `Entity`, `Entity::text`, `Entity::file(&File)`, `Entity::multipart` |
| `write::boundary`, `free_boundary` (`=_mc_` prefix, `counter`) | `Boundaries::new("=_mc_")` gives the same boundaries |
| `write::quoted`, `file_name_params`, `is_seven_bit_message` | `quoted`, `file_name_params`; the last is inside `Entity::file` |
| `media::safe_media_type` | `safe_media_type` |
| `media::media_type_for`, `default_name` | stay |
| `parse::read_headers`, `write::write_header`, `calendar_form`, `body` | stay (model mapping) |
| `mailto::{Mailto, is_mailto, parse_mailto}` | the same names; `mailto::cut`, which `rich` borrows, stays |
| `protected`, `signed`, `rich` | stay |

### WordCraft (`crates/formats/src/mht.rs`)

| WordCraft | craft-mime |
|---|---|
| `split_headers`, `header` | `parse`, `Headers::get` |
| `param` | `parse_params` / `Part::content_type` (comments and quoted strings handled the same) |
| `collect`, `multipart_bodies`, `Part` | `parse`, `Part::leaves` |
| `quoted_printable`, `model::base64_decode` | done by `parse` (`transfer::qp_decode`, `transfer::base64_decode`) |
| the `start` lookup | `start_part` |
| the `by_location` / `by_id` maps and the loader's lookup | `Resources::of`, `Resources::find(src, base)` |
| `resolve`, `UriParts`, `remove_dots` | `resolve_location` |
| `html::decode_as(body, charset)` | stays, or `Part::text` |
| `is_mht`, `import`, `import_on`, the picture budget | stay |

Differences a switch brings to WordCraft: the header block ends at the first line that is not a
header field (not only at a blank line); a delimiter line must hold nothing after the boundary
but white space; base64 with junk or missing padding decodes; boundaries longer than 70
characters are accepted; the limits are the ones above (depth 20, 1000 parts by default, both
settable).

## Origin

| Part | Source |
|---|---|
| Headers, parameters, encoded-words, transfer encodings, address lists, multipart splitting, mbox, the writer's parts and boundaries, their tests | MailCraft `crates/mime` [a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5) |
| Character sets, strict base64 | MailCraft `crates/charset`, same commit |
| `mailto:` links | MailCraft `crates/mime` (`mailto.rs`), same commit |
| Code page labels | MailCraft `crates/pst` (`codepage.rs`), same commit |
| Date parsing and formatting | MailCraft `crates/time` (`text.rs`), same commit; the calendar arithmetic replaced by `chrono` |
| `multipart/related` lookups, location resolving, comment-aware parameters, the MHT tests | WordCraft `crates/formats/src/mht.rs` [639b813](https://github.com/storytold/wordcraft/commit/639b8133ea31efb63aac381a7ced8b8aed32aa30) |

MailCraft's implementation is the base: it was the fuller one (the whole message grammar, a
writer, limits and property tests). From WordCraft come the parts MailCraft had no use for
(`Content-Location`, `start`, URI resolving) and one fix: a `;` or `"` inside a comment in
`Content-Type` no longer ends the value or supplies a false `boundary`.

New here: the neutral `Part` tree, `Limits` as a caller's setting with `truncated` reporting,
the input and decoded-size caps, `write_field` refusing bad names, and the truncation, mutation
and injection tests in `tests/hostile.rs`.

## Dependencies

`encoding_rs` ((Apache-2.0 OR MIT) AND BSD-3-Clause) for the character sets of the WHATWG
Encoding Standard, and `chrono` (MIT OR Apache-2.0). No `unsafe`.
