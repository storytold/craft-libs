# craft-jmap

The JMAP protocol core (RFC 8620) without sockets: a program brings an HTTP implementation
(`HttpExchange`), this crate describes each request and judges each answer.

- **Session**: `Client`, `fetch_session` / `read_session`, `Session`, `Account`, `CoreLimits`,
  the capabilities (`CAPABILITY_CORE`, `CAPABILITY_MAIL`).
- **Requests**: `ApiRequest` of method calls with `CallId`s and `ResultReference`s
  (back-references), checked against the session's limits.
- **Responses**: `ApiResponse`, `MethodResponse`, `MethodError` (RFC 8620 §3.6.2), per call.
- **Addresses**: `Address` (absolute `http(s)` URLs, RFC 3986 and the WHATWG URL Standard), the
  transport rule (`https`, or plain `http` to loopback only), `session_address` for what a person
  types, `AddressTemplate` (RFC 6570 level 1, the download and upload templates).
- **HTTP**: `HttpRequest`, `HttpResponse`, `judge_response` (status codes, problem details,
  JSON), `RetryAfter`, `Credentials` (Basic, Bearer), `Redactor` (excerpts of what a server says,
  with the credentials and addresses taken out).
- `Id`: the JMAP id alphabet.

Method calls' arguments and results are `serde_json::Value`s: what a program asks
(`Mailbox/get`, `Email/query`…) and what it makes of the answers stays in the program.

## Hostile input

Every size, count, nesting depth and string length a server sends is bounded; ids are held to
the JMAP alphabet; nothing indexes into what came over the wire, and the protocol modules are
checked with `clippy::indexing_slicing` and `clippy::arithmetic_side_effects`. Credentials and
the addresses of the session and the account never reach error text or `Debug` output.

## Origin

MailCraft `crates/jmap` (the protocol core: `address`, `classify`, `client`, `credentials`,
`discovery`, `error`, `host`, `http`, `id`, `percent`, `redact`, `request`, `response`, `retry`,
`session`, `template`) at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), with all of its tests. The mail
client that sits on it (the mailbox and email reads into MailCraft's sync model, the fetch of
whole messages, the in-process test server) stays in MailCraft.
