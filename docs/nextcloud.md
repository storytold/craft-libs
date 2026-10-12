# Nextcloud

**Status:** design, not implemented yet (2026-10-12). The first consumer is LightCraft. Its integration spec lives on
LightCraft's `feat/nextcloud-source` branch.

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
- **Certificates:** the OS verifies them. Nothing bundles webpki-roots or webpki-root-certs (CDLA-licensed).
- **Connections:** one `ureq::Agent` per account gives connection reuse. `allow_non_standard_methods` is on for
  `PROPFIND`.
- **Timeouts** are set per request kind: short for previews and listings, generous for downloads, which have no
  overall limit but a stall limit.

**Trust on first use.** When verification fails because the issuer is unknown, the error carries the presented leaf
certificate's SHA-256 fingerprint. After the user confirms it, `Account::trust` pins that certificate.
- The first implementation attempt is `RootCerts::Specific([pinned leaf])`.
- If rustls-webpki rejects a self-signed leaf as its own anchor, the fallback is a custom connector on ureq's
  `unversioned::transport::Connector`, with the ureq minor version pinned.
- Either way, handshake signatures are verified as usual.
- A changed certificate is a hard error, never a silent re-pin.
- This is the first thing prototyped.

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

`SignIn` is a state machine with no UI:

```text
SignIn::start(server, provider)
  -> NeedsTrust { fingerprint }   // unknown issuer; app asks, then confirm_trust(true/false)
  -> Waiting { login_url }        // app opens login_url in the browser, then calls poll() every ~2 s
  -> Done { account, app_password }
  -> Failed(Error)                // also after a 20-minute timeout or cancel
```

The app owns the dialog, the browser launch, the polling timer and any progress row. CLI and agent front ends print
`login_url` instead.

## Client

`Client::new(account, app_password, provider)`. The password is kept only inside the `Client`; it is never logged,
`Debug`-printed or put in an error message.

| Call | Request | Returns |
|---|---|---|
| `list(dir)` | `PROPFIND`, `Depth: 1`; Nextcloud forbids `infinity` by default | `Vec<Entry { name, is_dir, size, etag, fileid, mtime, content_type, has_preview }>` |
| `stat(path)` | `PROPFIND`, `Depth: 0` | one `Entry`, used for freshness checks |
| `preview(path, max_edge)` | `GET /index.php/core/preview.png?file=<path>&x=&y=&a=1` | JPEG bytes, or `None` on 404 |
| `read_range(path, range)` | `GET` with `Range` | Bytes. Requires `206` with a matching `Content-Range`; on `200` it reads only the needed prefix and drops the response |
| `download_to(path, writer, cancel, progress)` | `GET` | Size. Streams and checks `Content-Length` |
| `memories()` | `GET /apps/memories/api/…` with `OCS-APIRequest: true` | `Option<Memories>`: `None` when the app isn't installed |

`Memories::days()` returns the timeline (`fileid`, `dayid`, `w`, `h`, `etag`, `basename`, `size`, `epoch`,
`mimetype` per photo). `Memories::info(fileid)` returns EXIF mapped to a neutral `RemoteMeta`.

The Memories API is not documented as stable. It is detected at run time, and callers must work without it.

**Errors** are one `Error` enum: `Unreachable`, `Unauthorized`, `NotFound`, `CertificateChanged`,
`UnknownCertificate { fingerprint }`, `RateLimited`, `TooLarge`, `Malformed`, `Cancelled`, `Io`. Each has a `Display`
the user can act on, e.g. "Nextcloud rejected the app password; sign in again". `429` and `503` are retried with
backoff, at most 3 times.

## Originals cache

`OriginalsCache::open(dir, budget)` keeps downloaded originals on local disk for apps that need a real file (a raw
decoder, a video demuxer).

- **Layout:** `<dir>/<account_key>/<sha256(path)>-<etag>.<ext>`. Names are hex only and the extension is allow-listed,
  so a hostile remote name can't escape `dir`.
- **`cached(path, etag)`** → `Option<PathBuf>`, with no network.
- **`fetch(client, path, cancel, progress)`:**
  - `stat` for the current etag;
  - a cache hit returns the local path;
  - a miss streams the download into `craft_atomic::atomic_write_with`, so a crash, cancel or full disk never leaves
    a partial file under the final name;
  - a stale etag deletes the old copy.

  If the server is unreachable, it returns the newest cached copy with `stale: true`.
- **Eviction:**
  - least recently used above `budget`;
  - entries the app marks with `pin(path)` (offline copies) or `hold(path)` (in use right now, released on drop) are
    never evicted;
  - pinned bytes are reported separately.
- **Size, last use and pins** live in a small index file, also written with `craft-atomic`. A missing or corrupt index
  is rebuilt from the directory listing, never fatal.

The cache never decides when to download: an app that must not download during background work simply doesn't call
`fetch` there.

## Credentials

The `keyring` feature stores and loads app passwords: `store_password(account)`, `load_password(account)`,
`delete_password(account)`. It uses `keyring` 4.2 with `default-features = false` and native stores only:
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

- **`testing` feature:** exports `FakeServer`, an in-process, plain-HTTP, scripted Nextcloud on `std::net::TcpListener`.
  - It serves the login flow, `PROPFIND`, previews, Range, downloads and Memories.
  - Failure injection: 401, 429, truncation, a stalled body, a dropped connection, a server that ignores Range,
    oversized or malformed XML/JSON, hostile names (`..`, NUL, `%2F`).
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
