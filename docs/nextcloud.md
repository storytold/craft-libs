# Nextcloud

**Status:** implemented in `crates/nextcloud` (2026-10-13). The first consumer is LightCraft. Its integration spec lives on
LightCraft's `feat/nextcloud-source` branch and is unchanged.

`craft-nextcloud` lets a Craft app treat files on a Nextcloud server as documents it can list, preview and open,
without mounting the server and without downloading files the user only browses.

**The shared layer owns:**
- signing in (Login Flow v2) and trusting a self-signed certificate;
- storing the app password in the OS keyring;
- the remote-path format;
- WebDAV listing, server previews, Range reads and downloads;
- the local download cache of originals;
- optional metadata from the Memories app.

**The application owns:**
- where remote files appear (catalog, recent files, file dialogs);
- when a download is allowed;
- what it renders from a preview;
- its commands, dialogs, progress UI and persistence of account settings.

## Scope

A blocking, native-only Rust API: the browser build would also need CORS on the server. It stays useful to more than
one app:
- **LightCraft:** browse a raw archive, develop selected shots.
- **PhotoCraft and DesignCraft:** open a PSD or layout file.
- **PdfCraft:** open a PDF.
- **FilmCraft:** pick a clip.

Not in scope:
- writing back (PUT, XMP sidecars);
- sync or conflict resolution;
- push notifications;
- other WebDAV servers without Nextcloud's preview endpoint.

## HTTP and TLS

- **HTTP:** `ureq` 3 with `default-features = false` and `rustls-no-provider` + `platform-verifier`, the stack
  `craft-launcher`, `filmcraft` and `pdfcraft` already use.
- **Crypto provider:** passed in by the application. LightCraft passes `rustls-rustcrypto` (pure Rust); an app that
  accepts `ring` may pass that instead.
- **Certificates:** the OS verifies them. Nothing bundles webpki-roots or webpki-root-certs (CDLA-licensed). The OS
  verifier is built only for unpinned https accounts; with no CA store the user gets the trust prompt.
- **Connections:** one `ureq::Agent` per account gives connection reuse. It keeps up to 16 idle connections (ureq's
  default is 3 per host), so an app's worker threads keep theirs. `allow_non_standard_methods` is on for `PROPFIND`.
- **Timeouts** are set per request kind: short for previews and listings, generous for downloads, which have no
  overall limit but a stall limit (default 30 s, `Client::with_stall_limit` adjusts it for tests).
- **Credentials stay on the server:** the app password goes only to URLs under the account's server URL. `http://` is
  accepted only for loopback hosts.
- **Redirects** are disabled; a 3xx is `Malformed`.
- **No proxy support:** an environment proxy is ignored.

**Trust on first use.** When the OS rejects the certificate (unknown issuer, expiry or name), the error carries the
presented leaf certificate's SHA-256 fingerprint. After the user confirms it, `Account::trust` pins that certificate.
- A custom connector chain on `ureq::unversioned::transport` (TCP, then our rustls TLS, then a stall guard), with the
  ureq minor pinned (`~3.4`). ureq has no verifier hook and its rustls error drops the leaf, so capturing the
  fingerprint needs our own `ServerCertVerifier`. `RootCerts::Specific` is not used.
- A pin matches the leaf's SHA-256 exactly and skips name and expiry checks. Handshake signatures are verified with the
  provider's algorithms.
- A changed certificate is a hard error, never a silent re-pin.

## Remote paths

`RemotePath` parses and formats `nextcloud://<user>@<host>[:port]/<path below the user's files root>`, for example
`nextcloud://root@cloud.dev-null.me/Sony A7R VI/2026/06/28/_DSC1748.ARW`.

- **Names** are kept decoded and are percent-encoded only when a URL is built.
- **Parsing rejects:**
  - names that decode to `/`, `\`, NUL or control characters;
  - `.` and `..`;
  - empty segments;
  - an authority without a user;
  - more than a capped number of segments.
- **`RemotePath::account_key()`** (`user@host[:port]`) identifies the account.
- **`RemotePath::is_remote(&str)`** is the cheap check apps use to route a path away from `std::fs`.
- **No UNC form** (`//host/…`): on Windows `std::fs` would try SMB against the server.

Apps that keep their own folder trees (LightCraft's catalog) parse `nextcloud://<authority>` as a volume prefix.
That stays in the app.

## Accounts and sign-in

`Account { server, user, trust: Option<Fingerprint> }` holds no secret. The app persists it with its other settings.
`user` is the Login Flow `loginName`, used for Basic auth and the DAV path. A login name that differs from the user id
(email or LDAP logins) is a known limitation.

`SignIn` is a state machine with no UI:

```text
SignIn::start(server, client_name, provider)
  -> NeedsTrust { fingerprint }   // unknown issuer; app asks, then confirm_trust(true/false)
  -> Waiting { login_url }        // app opens login_url in the browser, then calls poll() every ~2 s
  -> Done { account, app_password }
  -> Failed(Error)                // also after a 20-minute timeout or cancel
```

The API is `SignIn::start`, `SignIn::start_for`, `state()`, `confirm_trust(bool)`, `poll()` and `cancel()`.
- `SignIn::start_for(&account, client_name, provider)` signs in again (e.g. after `Unauthorized`) to the account's
  server, keeping its pinned certificate: a different certificate ends as `Failed(CertificateChanged)`, with no trust
  prompt.
- `client_name` (e.g. "LightCraft (Linux)") is every sign-in request's User-Agent, which Nextcloud shows as the app
  password's name on the grant page and in Devices & sessions. An empty name or one with control characters becomes
  "Craft". `Client` keeps ureq's default User-Agent.
- A server address that can't be used ends as `Failed(InvalidAddress)`.
- Transient poll errors (unreachable, rate limited) keep `Waiting`.
- The 20-minute timeout and a declined certificate end as `Failed(Cancelled)`.
- The poll endpoint must be on the same origin as the server.

The app owns the dialog, the browser launch, the polling timer and any progress row. CLI and agent front ends print
`login_url` instead.

## Client

`Client::new(account, app_password, provider)`. The password is kept only inside the `Client`; this crate never logs
it, `Debug`-prints it or puts it in an error message. `ureq` and `ureq_proto` do log raw HTTP bytes at TRACE, including
the `Authorization` header and the sign-in poll's `appPassword`: an app must cap those two log targets below TRACE in
its logger.

| Call | Request | Returns |
|---|---|---|
| `list(dir)` | `PROPFIND`, `Depth: 1`; Nextcloud forbids `infinity` by default | `Vec<Entry { name, is_dir, size, etag, fileid, mtime, content_type, has_preview }>`; `Malformed` when no href is `dir` itself (a proxy rewriting paths) |
| `stat(path)` | `PROPFIND`, `Depth: 0` | one `Entry`, used for freshness checks |
| `preview(path, max_edge)` | `GET /index.php/core/preview.png?file=<path>&x=&y=&a=1` | The server's image bytes (JPEG or PNG), or `None` on 404 |
| `read_range(path, range)` | `GET` with `Range` | Bytes. Requires `206` with a matching `Content-Range`; on `200` it reads only the needed prefix and drops the response (only when the range starts within 64 MiB, else `Malformed`); `416` returns empty |
| `download_to(path, writer, cancel: &AtomicBool, progress: &mut dyn FnMut(done, Option<total>))` | `GET` | Size. Streams and checks `Content-Length` |
| `memories()` | `GET /apps/memories/api/…` with `OCS-APIRequest: true` | `Option<Memories>`: `None` when the app isn't installed |

`memories()` detects the app with `GET /index.php/apps/memories/api/describe`; a 404 or a non-JSON 200 means "not
installed". `Memories::days()` returns the user's configured timeline (`fileid`, `dayid`, `w`, `h`, `etag`, `basename`,
`size`, `epoch`, `mimetype` per photo): it fetches `/api/days`, then `/api/days/<id,id,…>` with 100 day ids per request.
`Memories::info(fileid)` maps `exif.{Make, Model, LensModel, ExposureTime, FNumber, ISO, FocalLength, Orientation}`
plus `datetaken`, `w` and `h` into a neutral `RemoteMeta`; numbers may be strings or fractions.

The Memories API is not documented as stable. It is detected at run time, and callers must work without it.

**Errors** are one `Error` enum: `Unreachable`, `Unauthorized`, `NotFound`, `CertificateChanged`,
`UnknownCertificate { fingerprint }`, `RateLimited`, `TooLarge`, `Malformed`, `InvalidAddress`, `Cancelled`, `Io`.
`Unreachable(String)`, `Malformed(String)` and `InvalidAddress(String)` carry a short detail; `Io(io::Error)`.
`InvalidAddress` is a server address that can't be used (e.g. `http://` for a host other than localhost), from
`SignIn::start` or `Client::new`. Each has a `Display` the user can act on, e.g. "Nextcloud rejected the app password;
sign in again". `429` and `503` are retried with backoff, at most 3 times. Nextcloud's maintenance mode (`503` with
`X-Nextcloud-Maintenance-Mode: 1`) is `RateLimited` at once, without retries.

## Originals cache

`OriginalsCache::open(dir, budget)` keeps downloaded originals on local disk for apps that need a real file (a raw
decoder, a video demuxer).

- **Layout:** `<dir>/<hex16(sha256(account_key))>/<hex64(sha256(path))>-<hex16(sha256(etag))>.<ext>`. `ext` is the
  lowercased remote extension if it matches `[a-z0-9]{1,8}`, else `bin`. Names are hex only, so a hostile remote name
  can't escape `dir`.
- **`cached(path, etag)`** → `Option<PathBuf>`, with no network.
- **`newest(path)`** → `Option<PathBuf>`: the newest local copy, any etag, with no network; `None` when none is on disk.
  For offline lookups when the app has no etag. Counts as a use.
- **`fetch(client, path, cancel, progress)`:**
  - `stat` for the current etag;
  - a cache hit returns the local path;
  - a miss streams the download into `craft_atomic::atomic_write_with`, so a crash, cancel or full disk never leaves
    a partial file under the final name;
  - a stale etag deletes the old copy.

  If the server is unreachable or busy (`Unreachable`, `RateLimited`) or its certificate isn't trusted
  (`CertificateChanged`, `UnknownCertificate`), it returns the newest cached copy with `stale: true`: none of these
  mean the file is gone. Other errors, and these with no local copy, are returned as they are.
- **Eviction:**
  - least recently used above `budget`;
  - entries the app marks with `pin(path)` (offline copies) or `hold(path)` (in use right now, released on drop) are
    never evicted;
  - pinned bytes are reported separately;
  - `set_budget` changes the budget.
- **Size, last use and pins** live in a small index file, also written with `craft-atomic`. `open` rebuilds entries from
  the files on disk (the index keeps last use and pins), deletes leftover `*.craft_tmp` stage files, and never fails on
  index problems. Hits update last use in memory; it is saved with the next miss, pin, unpin or budget change.
- **Holds:** take `hold` before `fetch`, `cached` or `newest`. A newer fetch of the same path still replaces older-etag copies
  while held.
- **One open per directory:** `open` takes an exclusive lock on `<dir>/.lock` and keeps it until the last clone of the
  cache (and any `Hold`) is dropped. Opening the same directory meanwhile, from another process or the same one, fails
  with an `Error::Io` of kind `WouldBlock`; clone the open cache to share it. The lock is taken before the stage-file
  sweep, so a refused opener never deletes the first one's in-flight files. The lock is best-effort: where `.lock`
  can't be opened (even read-only) or the filesystem can't lock, `open` goes on without it, so a read-only cache stays
  usable.
- **Known ceiling:** concurrent fetches of one path download twice.

The cache never decides when to download: an app that must not download during background work simply doesn't call
`fetch` there.

## Credentials

The `keyring` feature stores and loads app passwords: `store_password(app, account, password)`,
`load_password(app, account) -> Option`, `delete_password(app, account)`. The keyring service is the app's name and the
entry user is `Account::key()`, so two apps signed in to one account keep separate passwords. It uses `keyring` 4.2
with `default-features = false` and native stores only:
- Apple Keychain;
- Windows Credential Manager;
- `zbus-secret-service-keyring-store` with `crypto-rust`, so no OpenSSL.

With no keyring available, these return an error that says so; there is no plaintext fallback. The feature is
optional, for headless tools that receive the password another way.

## Limits (never crash)

Every number and name from the server is hostile:
- response size caps: e.g. 64 MiB per `PROPFIND`, 256 MiB for the Memories timeline;
- XML element depth and entry-count caps;
- checked or saturating arithmetic on sizes and ranges;
- no indexing by server-supplied offsets.

XML is parsed with `quick-xml`, JSON with `serde_json`. `lib.rs` carries the suite's
`#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo,
clippy::unreachable)]`.

## Testing

- **`testing` feature:** exports `FakeServer`, an in-process scripted Nextcloud on `std::net::TcpListener`: plain HTTP
  with `start()`, TLS with `start_tls(provider)` (a self-signed `CA:TRUE` P-256 fixture).
  - It serves the login flow, `PROPFIND`, previews, Range, downloads and Memories.
  - Failure injection: 401, 429, maintenance mode, truncation, a stalled body, a dropped connection, a server that
    ignores Range, a hostile `Content-Range`, oversized or malformed XML/JSON, hostile names (`..`, NUL, `%2F`).
  - Failures are queued with
    `fail_next(Fault::{Status, Drop, Stall, Truncate, IgnoreRange, Oversize, Garbage, ContentRange, Maintenance})`.
  - Content and state: `add_file`, `add_dir`, `remove`, `add_raw_entry`, `set_preview`, `set_href_prefix`,
    `approve_login`, `enable_memories`, `set_photo`.
  - What the client did: `connections()`, `requests()`, `last_user_agent()`, and `downloads()`, the whole-file `GET`s
    below the user's WebDAV files root (not `PROPFIND`s on the same paths, previews or Range reads), for tests that
    assert a path never downloads.
  - Apps use it for their own integration tests.
- **Unit tests:**
  - `RemotePath` round-trips and rejections;
  - parsers on synthetic fixtures;
  - every `SignIn` transition;
  - Range semantics;
  - connection reuse (requests per socket);
  - trust-on-first-use: accept, mismatch, no pin;
  - cache hit, miss, stale etag, eviction with pins and holds, a corrupt index, cancel mid-download.
- **Manual:** the real keyring.
- **Before merging:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.

## Measurements

Against a Nextcloud 35 server on the LAN (`cloud.dev-null.me`, 2026-10-12; previewgenerator and camerarawpreviews
installed). Network: TCP ~1 ms, TLS ~10 ms.

| Request | Result |
|---|---|
| 256 px preview, new connection each (`ureq`) | 41 ms |
| 256 px preview, reused connection (`ureq`) | 24 ms |
| Preview by path vs by fileid | same bytes, 36–39 ms |
| `PROPFIND` of a folder with 115 entries (46 KB) | 32 ms |
| Full 26 MB ARW | 29 MB/s (curl: 22–29 MB/s) |
| Memories `days` (1,472 days, 52,414 photos) | 0.74 s |
| Memories `/days/<id>` | ~45 ms |
| Memories `image/info/<fileid>` | ~60 ms |

The server's PHP (~25–30 ms per preview) and its ~30 MB/s download rate are the limits, not the client. Nextcloud's
own `nc:metadata-photos-*` properties are empty for raw files, which is why Memories, or a header Range read in the
app, supplies photo metadata.
