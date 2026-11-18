# craft-ews

The SOAP layer of Exchange Web Services, without sockets.

- **Requests**: `Element` builds an operation's elements (`Element::new("m:GetFolder").child(…)`)
  and escapes everything it is given: attribute values lose every control character, text keeps
  its tabs and line feeds and writes a carriage return as `&#13;`, names keep only what a
  qualified name is made of. `envelope(Version, &operation)` writes the SOAP 1.1 envelope with
  `RequestServerVersion`.
- **Answers**: `Answer::parse` reads the envelope with `craft-xml`. A fault is the error it names;
  `Answer::messages("GetFolder")` gives the `GetFolderResponseMessage`s with their
  `ResponseClass` and `ResponseCode`, and `ResponseMessage::ok` the element or the error. A busy
  server (`ErrorServerBusy`, or that code inside `ErrorInternalServerError`) gives its wait,
  clamped to 1 s to 5 min.

Names in an answer carry fixed prefixes whatever the server used: `s:` envelope, `m:` messages,
`t:` types, `e:` errors.

Written from Microsoft's public documentation: the EWS reference, [MS-OXWSCDATA] and
[MS-OXWSCORE].

## Hostile input

A document type declaration, malformed markup, more than 40 levels, more than 100,000 nodes or
more than 64 MiB is `Error::Malformed`; an entity XML does not define is left out. Errors hold no
text a server wrote except a response code that is a short identifier (64 letters and digits at
most). `Debug` output of an answer or a message shows nothing a server chose.

## What stays in the apps

HTTP and its status codes, Autodiscover, NTLM and Basic authentication, the operations (which
elements to send, what to make of what comes back), sync state and the mapping onto a mailbox.

## Origin

MailCraft `crates/ews` (`soap.rs`, `reply.rs`, the response codes of `error.rs`, the escaping of
`xml.rs`) at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), with the tests of that layer.
MailCraft's own bounded XML reader (`xml.rs`) is replaced by `craft-xml`.
