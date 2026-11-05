# Craft Link protocol, version 1

Extends the existing Craft control channel (the control protocol document of each app). Every
existing method keeps its exact behaviour. New methods are additive. `craft-ipc` implements this
protocol.

## 1. Transport and framing

- **Endpoint**: from the instance record (`schemas/instance-record.schema.json`). Unix domain socket file
  (mode 0600, directory 0700) on Linux and macOS; named pipe with an owner-only SDDL descriptor on
  Windows; loopback TCP only when the user opts in (`--control <port>`, legacy).
- **Control framing**: one UTF-8 JSON object per line, max 4 MiB per line. A request has `method`
  (string), optional `id` (number or string), optional `params` (object). A reply has the request's
  `id`, `ok` (bool), and `result` or `error`. Malformed input gets one error line and the connection is
  closed (existing behaviour).
- **Push events** (new): lines with `event` and `seq` and no `id`:
  `{"event": "<name>", "seq": <u64>, "data": {...}}`. Sent only on connections that called
  `events.subscribe`. Per-subscriber queue of 256; on overflow the server sends
  `{"event":"events.overflow","seq":n,"data":{"since":<seq>}}` and drops events until the client
  catches up.
- **Bulk connection** (new): a second connection to the same endpoint, used only for frames (section 6).
- **Connection limit**: 16 concurrent connections per app (existing). Bulk connections count. At most
  4 of the 16 may be unauthenticated at one time; the other 12 slots go only to peers that sent the
  right token. A peer past either limit gets one line `{"ok":false,"error":"too many control
  connections"}` or `"too many unauthenticated connections"` and a close.
- **Time limits** (server defaults): a connection must send a valid `auth` line within 2 s of the
  accept; a request line that has started must end within 10 s; an authenticated control
  connection with no input for 1 h, no event subscription and no bulk role is closed with
  `{"id":null,"ok":false,"error":"idle connection closed"}`. A client that keeps a quiet connection
  longer must call `events.subscribe` or reconnect. A request may arrive in any number of pieces
  with pauses between them; the server keeps the bytes of an unfinished line.
- **Launch on demand** (library rule, not wire): `craft_ipc::launch` runs an app binary only if the
  path is absolute and the file is regular and executable, and the file and its directory belong to
  the user or root and are not writable by every user. Empty and relative `PATH` entries are
  skipped. A fork that starts an app by another route must call `craft_ipc::trust::check_executable`.
  Caches use `craft_ipc::cache::private_cache_dir` (`0700`, owner checked, no symlink, no
  fallback to a shared temp directory); `CRAFT_LINK_CACHE_DIR` must be absolute.

## 2. Authentication (required on every transport: sockets, pipes, loopback TCP)

The first line on every connection must be:

```json
{"id": 0, "method": "auth", "params": {"token": "<64 hex from the instance record>"}}
```

Reply `{"id":0,"ok":true,"result":{"authenticated":true}}`, or
`{"id":0,"ok":false,"error":"authentication required"}` followed by close. The server also checks
the peer's user id (Linux/macOS euid; Windows token user) and refuses other users. Any other first
line gets the same refusal. (Shape copied from PhotoCraft's existing handshake.)

Loopback TCP (`--control <port>`, legacy) runs the same handshake, the same unauthenticated cap and
the same 2 s auth deadline. It binds 127.0.0.1 only. There is no uid check on TCP, so the token is
the only proof. The instance record (0600) publishes the port as `tcp_port` next to the `token`;
a TCP client reads both from the record (`Client::connect_tcp`). Never print the token or put it
in a config file or on a command line. A TCP client without a token gets the auth refusal.

## 3. Session methods (answered on the connection thread, no UI frame needed)

### `link.hello`
```json
{"id":1,"method":"link.hello","params":{"client":{"app":"filmcraft","app_version":"0.9.1","pid":4242,"instance":"<uuid>"},"protocol":{"min":1,"max":1}}}
-> {"id":1,"ok":true,"result":{"app":"effectcraft","app_version":"0.8.3","pid":5151,"instance":"<uuid>","protocol":1,"caps":["events","frames","activate","link.receive"]}}
```
Errors: `protocol_unsupported{server_min, server_max}`. The client must check `pid` and `instance`
equal the instance record. Legacy clients that never call `link.hello` keep today's behaviour.

`caps` is the fixed base set (`events`, `activate`, `link.receive`, `frames`) plus the `caps` array of
the instance record (`schemas/instance-record.schema.json`), without duplicates. Roles and optional verbs use
this array. Defined names:

| Cap | Meaning |
|---|---|
| `hub` | The peer is a browser or launcher hub. It owns no document, it only reads and opens assets (section 9). |
| `assets` | The peer answers `link.resolve` and `asset.describe` itself, on the connection thread (section 4). |

A client that does not know a cap ignores it.

### `events.subscribe` / `events.unsubscribe`
```json
{"id":2,"method":"events.subscribe","params":{"events":["doc.changed","link.changed","app.closing"]}}
-> {"id":2,"ok":true,"result":{"seq":0}}
```

### `app.activate`
```json
{"id":3,"method":"app.activate","params":{"token":"<xdg activation token or null>","from_pid":4242,"open":{"link_id":"<uuid>"}}}
-> {"id":3,"ok":true,"result":{"activated":"requested"}}
```
`activated` has three values:

| Value | Meaning |
|---|---|
| `true` | The window has focus now. |
| `"requested"` | The app asked the OS to focus the window. The OS decides later. Nobody confirmed it, and the reply does not say it will happen. |
| `false` | Focus was not taken. The app asked for user attention instead (taskbar flash, dock bounce). |

On Linux, `"requested"` needs a token that the app could apply to its existing window. An app whose
toolkit can mint a token but cannot apply one (stock winit 0.30 on Wayland) answers `false`. The
sender can still pass `token`. Old senders and receivers keep working: the three values did not change.
`activated` is distinct from the existing `ui.focus` method, which never steals focus by design.

`open` is an object that names what to show. It carries `link_id` (open a link), `asset` (a moniker,
section 9: open that asset in its document) or both. A hub or launcher uses `asset` to send the user
to the app and document that owns the asset, with no link in between. The server rejects an `asset`
that is not a valid moniker with `bad_params`. An `open` with neither key is passed on unchanged
(existing behaviour). The receiving app decides what "show" means for the asset.

### `link.bye`
Clean close. The server drops subscriptions and bulk tickets of this connection.

## 4. Link methods (run on the UI thread through the app's existing request channel)

### `link.open` (FilmCraft -> EffectCraft)
```json
{"id":10,"method":"link.open","params":{
  "link_id":"<uuid>",
  "payload":{"format":"otio-json","data":"<OTIO JSON string>"},
  "place":{"seconds":12.48,"selection":[{"start_s":10.0,"end_s":14.0,"track_index":0}],"view":{"zoom":1.0}},
  "result_path":"/abs/path/Project Links/<link_id>.mov",
  "source":{"app":"filmcraft","doc_id":"<uuid>","item":"<nest item id>"}}}
-> {"id":10,"ok":true,"result":{"comp":"<comp id>","project":"/abs/path/Project Links/<link_id>.ecproj",
     "format":{"w":1920,"h":1080,"rate_num":24,"rate_den":1,"duration_ticks":...,"alpha":true},
     "unsupported":[{"what":"effect","name":"Lumetri Color","kept_as":"none"}]}}
```
Errors: `payload_invalid`, `media_missing`, `payload_too_large` (OTIO string > 4 MiB), `io_error`.
A request line longer than 4 MiB (`MAX_LINE`) is not an error reply: the server answers once in text
and closes the connection. `unsupported` lists parts that
did not survive the hand-off.

**Payload formats.** `payload.format` selects the shape of the rest of `payload`:

| `format` | Other fields | Use |
|---|---|---|
| `otio-json` | `data`: OTIO JSON string | Video: a nested sequence (above). |
| `image-doc` | `asset`: moniker (section 9), `path`: absolute path of the producer document or image file | Stills, vectors and other single images. `result_path` is the last-known flattened image. `place`, `source` and `format` in the reply stay as above. `rate_num`, `rate_den` and `duration_ticks` are absent from `format` in the reply. |

An unknown `format` gets `payload_invalid`. A new `format` value is additive and needs no protocol
bump. The server checks the shape of `payload` with `craft_ipc::payload::OpenPayload` before it
forwards `link.open` to the app. A missing `payload`, a `payload` that is not an object, a missing or
unknown `format`, a missing field, a relative `path` or a bad `asset` moniker gets `payload_invalid`
and the app never sees the request. `params` that is not an object stays `bad_params`. The server
checks shape, and for `otio-json` it also opens every `ExternalReference` whose `target_url` is a
`file://` URL or an absolute path. A file that does not open gets `media_missing` with
`data.missing` (the paths) and the app never sees the request. Relative targets are left to the app.

Path rules. `result_path`, `payload.path` and every absolute media `target_url` must
be absolute, hold no NUL byte and no `..` component, or the request gets `payload_invalid`. The
server does not canonicalise and does not confine paths to a root: footage lives anywhere and
may be a symlink. The media check is the only file probe. It looks only at paths the payload
names, at most 4096 distinct ones, and reports a path as missing when it is not a regular file
(after symlinks) or does not open for reading. A missing file, a directory, a FIFO, a device and a
file without read access all look the same, and the probe never opens a FIFO. An OTIO document
with more than 4096 distinct files, more than 2^20 JSON values or nesting over 128 gets
`payload_invalid`. The app must apply its own checks to `result_path` before it writes there;
the server check is a first filter.
An app still answers `payload_invalid` when the content is bad (for example OTIO that does not
parse).

### `link.attach` (FilmCraft -> EffectCraft, reopen an existing link)
`{"link_id","project","comp"}` -> `{"format", "epoch"}`. Errors: `comp_missing` (link becomes `Broken`).
The receiver makes the comp of the link the active one. It reads `project` (the sidecar written by
`link.open`) when it holds no such link and has no unsaved edits, otherwise `busy`.
FilmCraft sends this when `link.editIn` runs on a clip that plays a linked item already. It nests
nothing and creates no item. On `comp_missing` or `io_error` (no sidecar, damaged sidecar) it sends
`link.open` again with the same `link_id` and `result_path`, built from the nest in `nested_sequence`.

### `link.focus` (either direction, used by Back)
`{"link_id","place"}` -> `{}`. The receiver moves to `place` and is then activated with `app.activate`.

### `link.close`
`{"link_id"}` -> `{}`. EffectCraft drops the binding; the sidecar project stays on disk.

### `link.refresh` (consumer side, any app that holds a link)
`{"link_id"}` -> `{"link_id","epoch","state":"current"|"modified"|"missing","result_path","hash"}`.
The receiver compares the last-known result file with the disk (rules in section 10), bumps the epoch
when the content changed, and answers the state. `hash` is `sha256:<64 hex>` and is absent when the
state is `missing`. A link that exists in the receiver but whose file is gone is not an error. The
answer is `state:"missing"`. Errors: `link_unknown`, `bad_params`, `io_error`.

### `link.resolve` (producer or hub, any app that owns assets)
`{"asset":"craft://..."}` -> `{"asset","path","epoch","size","hash","mtime_ms"}`. `path` is an
absolute path of the last-known flattened result that a consumer can read now. `hash` is
`sha256:<64 hex>` of that file. `epoch` is the producer epoch. Errors: `link_unknown` (no such asset,
or the asset has no result file yet), `bad_params` (not a moniker), `io_error`.

### `asset.describe` (producer or hub)
`{"asset":"craft://..."}` -> `{"asset","name","kind","epoch","w","h","hash","thumb_path","modified_ms"}`.
`kind` is `still`, `vector`, `sequence` or `comp`. `w`, `h`, `hash` and `thumb_path` are `null` when
unknown. `thumb_path` is an absolute path of a PNG, at most 512 px on the long side, that the
producer keeps in a per-document sidecar. A hub browses assets with this call and never opens the
document. Errors as `link.resolve`.

`link.resolve` and `asset.describe` have no side effects and never need a UI frame. A server that was
started with an asset provider (`Server::start_with` and `Extensions`) answers both on the connection thread
and lists `assets` in `caps`. `link.refresh` is answered by the provider the same way. A server
started without a provider forwards all three to the app like any other link method (existing
behaviour).

### Events
- `link.changed`: `{"link_id","epoch"}`. Sent by EffectCraft when the bound comp changes (any
  `bump()` whose change touches the comp).
- `link.result`: `{"link_id","epoch","path"}`. A fresh last-known result was written.
- `doc.changed`: `{"doc_id","revision","cause"}`. Generic change feed (from `craft-command`).
- `app.closing`: `{}`. Sent before a clean exit.

## 5. Command passthrough (existing, unchanged)

`engine.execute {command, params}`, `engine.commands`, `ui.*`, `perf.stats` keep their current
contracts. `engine.commands` now also lists plugin commands and carries a `schema` per command
(`x-craft-source`: `doc` | `derived` | `opaque`).

## 6. Bulk frames

1. Control connection: `frames.open {"link_id","format":"rgba8_premul"|"rgba16f_premul","scale":1.0}`
   -> `{"ticket":"<32 hex>","w","h","stride","len"}`.
2. New connection: `auth`, then `{"method":"bulk.attach","params":{"ticket":"..."}}` -> `{"ok":true}`.
   The connection is now binary in both directions.
3. Client request record (binary): `'CFRQ' | u32 header_len | header JSON` with header
   `{"req":<u64>,"time_s":<f64>,"epoch":<u64>,"region":[x,y,w,h]|null}`.
4. Server reply record: `'CFRM' | u32 header_len | u64 payload_len | header JSON | payload`, header
   `{"req","epoch","w","h","stride","format","render_ms"}`. `epoch` is the epoch the frame was
   rendered at; a client that asked for an older epoch may get a newer one.
5. Errors: header `{"req","error":"..."}` with `payload_len` 0.
All integers little endian. Tickets expire when the control connection closes.

## 7. Error codes (in `error` objects for new methods)

`{"code": "<snake_case>", "message": "<human text>", "data": {...}}`. Codes: `auth_required`,
`protocol_unsupported`, `unknown_method`, `bad_params`, `not_ready`, `payload_invalid`,
`media_missing`, `payload_too_large`, `comp_missing`, `link_unknown`, `io_error`, `busy`, `internal`.
`link_unknown` also covers an unknown asset moniker.
Existing methods keep their current string `error` field.

## 8. Versioning

- `protocol` is an integer. A server serves any client version in `[min, max]`. New methods and new
  optional fields do not bump the version. Removing or changing the meaning of a field does.
- Method names are never reused with a different meaning.

## 9. Asset identity: the moniker

An asset is addressed by a moniker, a string that stays valid across restarts:

```
craft://<app>/<doc>/<kind>/<id>
```

| Part | Rule |
|---|---|
| `app` | Same pattern as the instance record `app`: `^[a-z][a-z0-9-]{1,31}$`. The app that owns the document. |
| `doc` | The document id, a UUID in 8-4-4-4-12 hex form. Parsers accept either hex case and keep the text as written. |
| `kind` | `^[a-z][a-z0-9-]{0,31}$`. Known values: `comp`, `image`, `vector`, `sequence`, `layer`. An unknown value is valid and is kept. |
| `id` | Non-empty, at most 512 bytes after decoding, valid UTF-8. Characters other than `A-Z a-z 0-9 - . _ ~` are percent-encoded as `%XX` (uppercase hex) by the formatter. The parser accepts either hex case. A raw `/`, `?`, `#` or `%` not followed by two hex digits is invalid. |

A moniker has no query and no fragment. It names an asset, not a state: the state is the epoch.
A moniker can outlive its asset. Resolving a dead moniker gives `link_unknown`. Parsing then
formatting a moniker gives the canonical text (uppercase `%XX`). Formatting then parsing gives back
an equal moniker. `craft_ipc::moniker::Moniker` implements both. The module uses only `serde` and
`std`, so an app can copy it to a wasm build. The `craft-ipc` crate as a whole still does not build
for wasm32 (local sockets).

## 10. Refresh and watch rules for linked files

These rules apply to any linked asset with a `result_path` (a still, a rendered movie).

1. **Producer writes are atomic.** The producer writes the file to a temporary name in the same
   directory (`.<name>.<pid>.<n>.tmp`), flushes it, renames it over `result_path`, and then flushes
   the directory. A reader sees the old file or the new file, never a half file. A reader ignores
   names that start with `.` and end with `.tmp`.
2. **Epoch.** `result_epoch` is a counter of result versions. The producer adds 1 on every saved
   change and emits `link.changed` and `link.result`. A consumer that detects a change by polling
   (rule 4) adds 1 to its own stored epoch. When the producer is reachable again, the consumer keeps
   the larger of the two values. An epoch never decreases.
3. **Fingerprint.** `(size, mtime_ms, sha256)` of the file. The hash is of the full content.
4. **Polling when the producer is offline.** The consumer calls `poll` about once per second
   (`craft_ipc::watch::Watcher` runs this schedule with a thread and an injectable ticker). A poll
   reads size and mtime. It hashes the file only when size or mtime differ from the last fingerprint.
   Some file systems have coarse mtime, so the consumer also calls a deep poll (always hash) about
   once every 30 seconds. A touch that leaves the hash equal does not bump the epoch.
5. **States.** `current` (disk equals what the consumer loaded), `modified` (disk differs from what
   the consumer loaded; the epoch was bumped), `missing` (the file does not exist). The consumer
   moves `modified` to `current` when it has loaded the new content. A file that returns with the
   hash it had when loaded goes back to `current` with no epoch bump. A file that returns with a
   different hash is `modified` and bumps the epoch.
6. **Other read errors** (permission denied, I/O error) are errors, not `missing`. The state does not
   change and the next poll tries again.
7. **Relink.** A consumer can point a link at a new path. The new path is polled at once. The state
   is `current` when its hash equals what the consumer loaded and `modified` (epoch bump) otherwise.
   A missing new path is rejected and the old path stays.
