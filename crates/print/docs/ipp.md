# The IPP backend

`craft_print::ipp::IppBackend` (cargo feature `ipp`, on by default, not built for `wasm32`) is a
`BackendClass::Network` backend named `ipp`. This note is the reference for what it sends, what it reads,
how errors are classified, and what it does not do. The overview is in the crate README.

Protocol sources: RFC 8010 (encoding), RFC 8011 (model and operations), PWG 5100.x (job and printer
attributes: 5100.13 `print-color-mode`, `print-quality`, `printer-resolution`, `print-content-optimize`).
The codec, the operations and the mappings are this crate's own code.

## Ids and policy

| What | Form |
|---|---|
| Printer | `ipp:<printer uri>`; the native id is the canonical URI `ipp://<ip>:<port><path>` (the port is always written) |
| Job | `ipp:<printer uri>#<job id>`; `IppBackend::job_id(uri, n)`, `parse_native`, `parse_job_id` (typed, no string concatenation) |
| Accepted URIs | `ipp://` and `http://`, a numeric loopback, link-local or private address, port 631 when none is named. `ipp://h/p` and `http://h:631/p` are the same printer |
| Refused | host names, public addresses, user information, queries, fragments (`craft-wsd`'s `DeviceUrl` policy); `ipps://` and `https://` with a TLS-specific error (`IppError::Tls`, `ErrorKind::InvalidRequest`); no redirects, no proxies |

The app adds printers (`add`, `add_named`, `with_printers`); nothing is discovered, no multicast is sent,
and no connection is opened by building a backend, adding a printer or listing printers. A connection is
made by `printer_details` (one `Get-Printer-Attributes`) and by a job. Every job needs a `UserConfirmed`.

IPP is `POST application/ipp` over the transport of `craft-wsd`. That transport also sends a `SOAPAction`
header and a SOAP `Accept` header, which IPP servers ignore. The reply must be HTTP 200 with an IPP body
whose request id matches.

## Requests

Every request starts with `attributes-charset` (utf-8), `attributes-natural-language` (en), `printer-uri`,
then `job-id` where the operation names one, then `requesting-user-name` (default `craft`, never the account
name; `with_user_name`). The IPP version is negotiated, see below.

### IPP version

The default is **IPP 2.0** (every IPP Everywhere printer supports it). Version rules, per printer:

| Situation | What is sent | Remembered |
|---|---|---|
| nothing known | 2.0 | |
| the printer's `ipp-versions-supported` was read (`printer_details`, the first step of every submit) | the highest of 2.0 and 1.1 it lists (versions this client does not speak, such as 1.0 or 2.1, are ignored; a list naming neither changes nothing) | yes, per printer, never raised |
| the printer answers `server-error-version-not-supported` to a 2.0 request | **the same request once more at 1.1**; no third attempt | see the next rows |
| the printer answers the 1.1 request with success or any status other than `server-error-version-not-supported` (a busy printer, a refused attribute, anything) | that answer is the result | 1.1 for that printer, for the life of the backend (clones share it): a printer that answers a request at 1.1 with any status other than version-not-supported is remembered as speaking 1.1 |
| the printer rejects 1.1 too | the second failure is the result: `Failed`, `Safe`, `NativeCode { ipp, 0x0503, server-error-version-not-supported }` | nothing |
| the 1.1 attempt gets no answer at all (connection refused, timeout) | that failure | nothing (no answer shows nothing) |
| `with_ipp_version(v)` | exactly `v`, always; an advertised list does not move it | |
| `with_ipp_version(v)` and `with_version_fallback(true)` | `v`, lowered to 1.1 once on a rejection (and to what is advertised) | as above |

Why the downgrade is safe even for `Print-Job`, `Create-Job` and `Send-Document`: a printer that does not
support the request's version-number rejects the message before it acts on it, so a version-rejected request
creates nothing and sending it again at 1.1 cannot produce two jobs. The repeat is a new request id with the
same attributes and document bytes. `IppBackend::version_of(uri)` says which version the next request to a
printer uses; `Submitted::effective` names the version the job went out at.

| Step | Operation | Notes |
|---|---|---|
| details, and the first step of every submit | `Get-Printer-Attributes` | `requested-attributes` lists the attributes below; also learns `multiple-operation-time-out` |
| optional (`with_validate_first`) | `Validate-Job` | what the printer would change comes back as warnings |
| submit, small document | `Print-Job` | `job-name`, `document-format`, `ipp-attribute-fidelity` (strict only), the job template group, then the document |
| submit, large document or no `Print-Job` listed | `Create-Job` then `Send-Document` | `last-document` true; a refused `Send-Document` is followed by a cleanup `Cancel-Job`; a lost reply is not (the job may exist) |
| `job_status` | `Get-Job-Attributes` | one request |
| `list_jobs` / `find_job` | `Get-Jobs` twice (`not-completed`, `completed`) | `limit` (default 100), `my-jobs` and `requesting-user-name` when the filter names a user; a status reply to the `completed` listing means "no history" |
| `cancel_job` | `Cancel-Job` | tolerant, see below |

`JobFlow::Auto` (default) uses `Print-Job` up to 8 MiB (`with_large_document`) and `Create-Job` +
`Send-Document` above that, or when the printer lists operations without `Print-Job`.

## Printer attributes to `PrinterDetails`

| IPP attribute | `PrinterDetails` |
|---|---|
| `printer-name`, else the name given to `add_named`, else `printer-info`, else the URI | `name` |
| `document-format-supported` (via `DocumentFormat::from_mime`: pdf, pwg-raster, postscript, octet-stream, jpeg, png, tiff, text/plain, xps; others such as `image/urf` are ignored) | `formats` |
| `sides-supported` has any `two-sided-*` | `two_sided` |
| `print-color-mode-supported` has `color`, else `color-supported` | `colour` |
| `media-supported` entries that spell a size (`..._210x297mm`, `..._8.5x11in`), plus fixed `media-size-supported` sizes that no name covers (`custom_100x150mm`) | `paper` |
| `copies-supported` upper bound | `max_copies` |
| `printer-resolution-supported` (dots per inch; dots per centimetre converted) | `resolutions` |
| `printer-state` (3 idle, 4 processing, 5 stopped; stopped with `offline-report` is offline), `printer-state-reasons` (without `none`), `printer-is-accepting-jobs` false adds `not-accepting-jobs` | `status` |
| the settings the backend applies | `honoured`: copies, duplex, colour, media, orientation, quality, resolution |

`formats_per_printer()` is true, so the registry reads the printer's own list before deciding on a
conversion through the app's `DocumentConverter`. A document whose MIME type the printer does not list is
refused with `ErrorKind::UnsupportedFormat` before it is sent.

## Job settings to job attributes

| Neutral setting | IPP | Checked against |
|---|---|---|
| `Job::copies` (not 1) | `copies` | `copies-supported`; over the maximum is lowered with a warning (strict: refused) |
| `Job::duplex` | `sides` = `one-sided`, `two-sided-long-edge`, `two-sided-short-edge` | `sides-supported` |
| `JobOptions::orientation` | `orientation-requested` = 3 portrait, 4 landscape, 5 reverse-landscape, 6 reverse-portrait | `orientation-requested-supported` |
| `JobOptions::quality` | `print-quality` = 3 draft, 4 normal, 5 high; photo is 5, plus `print-content-optimize` = `photo` when the printer lists it | `print-quality-supported` |
| `JobOptions::resolution` | `printer-resolution` (dpi) | `printer-resolution-supported` |
| `JobOptions::media` | `media` (the PWG name) | `media-supported`, or a `media-size-supported` entry of the size the name spells (half a millimetre tolerance) |
| `JobOptions::colour`, `Job::grayscale` | `print-color-mode` = `color`, `monochrome`; auto sends nothing | `print-color-mode-supported`, `color-supported` (a colour request to a monochrome printer prints grey with a warning; strict: refused) |
| title and idempotency tag | `job-name` = title + ` [ref xxxxxxxx]`, cut to 255 bytes with the tag kept; control characters (C0, DEL and C1) are removed from the title first, and a title left empty is sent as `Craft` | |
| (backend) | `requesting-user-name` | |
| `Job::scaling` | `print-scaling` (PWG 5100.13) = `auto-fit` for shrink-to-fit, `none` for actual size | `print-scaling-supported` / `print-scaling-default`. Sent only to a printer that lists the keyword. Shrink-to-fit to a printer that publishes no list, or a list without `auto-fit`, is not sent: warning "scaling is not applied: the document is sent as it is" (strict: refused when a list is published without the keyword; a printer that publishes nothing is still a warning). `fit` is not substituted (it also enlarges). Actual size is the neutral default and is sent as `none` only when the printer lists it and its `print-scaling-default` is something else. `JobHint::Scaling` is in `honoured` exactly when the printer lists `auto-fit`; otherwise scaling is reported as not honoured |
| collation, driver options | not applied (a warning for driver options) | |

The rule for "listed" is the WS-Print layer's: a setting is dropped (with a warning) or refused (strict)
only when the printer **publishes** the list and the value is not in it. A list the printer does not publish
is not checked; the printer answers when the job arrives, and what it ignores or substitutes comes back
in the unsupported-attributes group (`successful-ok-ignored-or-substituted-attributes`) as a warning. In
strict mode the request carries `ipp-attribute-fidelity` so that the printer refuses instead.

## Job state to `JobState`

| `job-state` | `JobState` |
|---|---|
| 3 pending | `Pending`; with reason `job-hold-until-specified`, `Held` |
| 4 pending-held | `Held` |
| 5 processing | `Processing` |
| 6 processing-stopped | `NeedsAttention` |
| 7 canceled | `Cancelled` |
| 8 aborted | `Aborted` |
| 9 completed | `Completed`; with `job-completed-with-errors`, `CompletedWithErrors`; with `job-completed-with-warnings`, `Completed` plus a warning in `JobStatus::warnings` |
| anything else, or missing | `Unknown` |

`job-state-reasons` (without `none`) are `JobStatus::reasons`. `job-media-sheets-completed` is `sheets_done`,
`job-impressions-completed` is `pages_done`, `job-media-sheets` and `job-impressions` are the totals when
positive. A job the printer answers `client-error-not-found` or `client-error-gone` for is
`ErrorKind::NotFound` ("printers forget finished jobs"; a watch reports it as forgotten).

`cancel_job` is `Cancel-Job`; `not-possible` is confirmed with a `Get-Job-Attributes` and is success only
when the job is terminal or gone. `not-found` is disambiguated, see below.

### Not found: the job or the printer?

A wrong printer path looks like a forgotten job (many servers answer `client-error-not-found`, or HTTP
404/410, for an unknown path). So when `job_status`, `cancel_job` or the job listing behind `list_jobs` /
`find_job` gets `client-error-not-found`, `client-error-gone` or HTTP 404/410, the backend sends **one**
`Get-Printer-Attributes` with `requested-attributes` = `printer-state` to the same printer URI (never
repeated, never looped):

| The extra read | Verdict | `job_status` | `cancel_job` | job listing |
|---|---|---|---|---|
| answers normally | the job is forgotten | `ErrorKind::NotFound`, `Safe`, "printers forget finished jobs" (a watch reports it as forgotten) | success | the original error |
| is also not-found / gone / HTTP 404 or 410 | the printer path is wrong | `ErrorKind::NotFound`, `Safe`, "the printer was not found at <uri> ... not a forgotten job", `NativeCode { ipp, 0x0406, client-error-not-found }` | the same error | the same error |
| fails for another reason | could not tell | `ErrorKind::Failed`, `Idempotent` (a read may be repeated), message says "could not tell", the original not-found as the native code | the same | the same |

The error also carries `PrintError::subject()`: `Job` for the forgotten row, `Printer` for the wrong-path row.
A `JobWatch` reads only a `Job` not-found as forgotten; a `Printer` one ends the watch `Failed` at once (a
configuration error, no retry).

## Listing

`list_jobs` fills `RemoteJob` from `job-id`, `job-name` (the tag is read from its suffix), `job-originating-user-name`,
`job-state`, `job-state-reasons` and the counters. `job_query_caps` is `list`, `created_time`, `user`, `tag`
and `history` all true. The creation time is `date-time-at-creation` when the printer has a clock, else
`time-at-creation` related to `printer-up-time` (one more `Get-Printer-Attributes`, only when needed; without it the
job is listed with no creation time). `find_job` reconciles by tag, else by title, this backend's user name and
the fingerprint's time window.

`open_job_timeout()` is the largest `multiple-operation-time-out` any printer of this backend published
plus 5 s, else 240 s (the upper bound RFC 8011 recommends). `watch_support()` is `Poll`, recommended interval 2 s
(`with_poll_interval`); IPP event subscriptions are a possible later option.

## Status codes and retry facts

Applies to the status of any reply. "Acted on" means the printer may have changed something before
failing: for `Print-Job`, `Create-Job` and `Send-Document` the retry safety is then `OutcomeUnknown` (reconcile
before resending), for reads and `Cancel-Job` `Idempotent`. Every other row is `Safe` (the printer declined).
The wait is the HTTP `Retry-After` when the printer sent one, else the default shown.

| Status | Code | `ErrorKind` | Retryable | Acted on | Default wait |
|---|---|---|---|---|---|
| successful-ok, -ignored-or-substituted-attributes (warnings), -conflicting-attributes | 0x0000 to 0x0002 | success | | | |
| client-error-bad-request | 0x0400 | InvalidRequest | no | no | |
| client-error-forbidden, -not-authenticated, -not-authorized | 0x0401 to 0x0403 | Rejected (authentication is not implemented) | no | no | |
| client-error-not-possible | 0x0404 | Rejected | no | no | |
| client-error-timeout | 0x0405 | Timeout | yes | no | 2 s |
| client-error-not-found, -gone | 0x0406, 0x0407 | NotFound | no | no | |
| client-error-request-entity-too-large, -request-value-too-long | 0x0408, 0x0409 | InvalidRequest | no | no | |
| client-error-document-format-not-supported, -compression-not-supported | 0x040A, 0x040F | UnsupportedFormat | no | no | |
| client-error-attributes-or-values-not-supported, -uri-scheme-not-supported, -charset-not-supported, -conflicting-attributes, -attributes-not-settable, -document-access-error | 0x040B to 0x040E, 0x0412, 0x0413 | InvalidRequest | no | no | |
| client-error-compression-error, -document-format-error, -ignored-all-subscriptions, -too-many-subscriptions, -ignored-all-notifications, -print-support-file-not-found, -document-password-error, -document-permission-error, -document-security-error, -document-unprintable-error, any other 0x04xx | 0x0410, 0x0411, 0x0414 to 0x0417, 0x0418 to 0x041B, others | Rejected | no | no | |
| server-error-internal-error, -device-error | 0x0500, 0x0504 | Failed | no | **yes** | |
| server-error-operation-not-supported, -version-not-supported | 0x0501, 0x0503 | Failed | no | no | |
| server-error-service-unavailable | 0x0502 | Unavailable | yes | no | 5 s |
| server-error-temporary-error, -busy | 0x0505, 0x0507 | Busy | yes | no | 5 s |
| server-error-not-accepting-jobs | 0x0506 | Busy | yes | no | 15 s |
| server-error-too-many-jobs | 0x050B | Busy | yes | no | 10 s |
| server-error-job-canceled | 0x0508 | Cancelled | no | no | |
| server-error-multiple-document-jobs-not-supported | 0x0509 | InvalidRequest | no | no | |
| server-error-printer-is-deactivated, -too-many-documents | 0x050A, 0x050C | Rejected | no | no | |
| any other 0x05xx, informational (0x01xx), redirection (0x03xx), 0x0600 and above | | Failed | no | **yes** | |

The error's `NativeCode` is `{ domain: "ipp", code, name }`. HTTP failures use `{ domain: "http", code }`:
401 and 403 are Rejected (authentication), 413 InvalidRequest, 426 Failed (the printer wants TLS), 408 Timeout,
425, 429 Busy and 503 Unavailable (all `Safe`, with their waits), other 5xx Unavailable and acted on, other 4xx
Failed. Transport failures follow `craft-wsd`'s typed kinds and stages: a connect failure is `Safe`; a closed
connection, timeout or unreadable reply after the request was written is `OutcomeUnknown` for a send and
`Idempotent` for a read. A reply that does not decode, answers another request id or is not
`application/ipp` is `Failed`. The table is pinned by `ipp::error` tests.

## The codec

`ipp::codec` is the RFC 8010 encoding: typed values (integer, boolean, enum, octetString, dateTime,
resolution, rangeOfInteger, text and name with or without language, keyword, uri, uriScheme, charset,
naturalLanguage, mimeMediaType, collections, the out-of-band values; an unknown syntax is kept as
`Value::Other`). The reader is for bytes from the network and refuses, with a typed `CodecError`: a message above
the response cap, more than 1024 groups, 16384 attributes, 65536 values or 4096 values in one attribute,
collections nested deeper than 6, bad delimiter tags, values outside a group, additional values with nothing to add
to, wrong fixed lengths, a missing end-of-attributes tag, bad names and truncation anywhere. Tests cover
every prefix of a message and every single-byte corruption.

## Not implemented

* **TLS.** `ipps://` and `https://` are refused (`craft-wsd` has no TLS). A numeric local address has no
  certificate to check anyway.
* **Authentication** (HTTP Basic or Digest, `client-error-not-authenticated`): reported, not answered.
* **IPP subscriptions** (`Create-Job-Subscription`, `Get-Notifications`): status is polled.
* **Discovery** (mDNS, DNS-SD, SNMP): the app adds printers.
* Compression of the document, `Send-Uri`, multi-document jobs, `Hold-Job` and the other operations.
* Collation, the other `print-scaling` keywords (`auto`, `fit`, `fill`) and vendor job options.
* Not verified against real printers or a CUPS server (the Windows development machines have none).

## Tests, and where the fake printer is

The unit tests in this crate need no printer: the codec on static byte vectors, request builders against
expected bytes, canned replies through a small scripted transport (a mock transport that returns exactly the
replies a test queues, not a printer), the status and retry tables, and registry-level tests (routing,
consent, format negotiation and conversion, tracked jobs, reconcile by tag and by time).

The fake printer is fakeprint. There is deliberately no in-memory IPP printer in this crate: one strict
implementation, shared by every client library and used for conformance against real clients, is better than
one more fake per library. Protocol-level verification against fakeprint is done through the Python bindings
in a separate harness (outside this repository).
