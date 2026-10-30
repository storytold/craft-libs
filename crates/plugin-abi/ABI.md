# craft-abi 1

The package format that `craft-plugin-abi` checks. The crate does not run guest code. A host (an app with an engine) loads a package only after `gate::check_package` accepts it. This file lists the rules the crate encodes. Run time behavior of a host (budgets, retries, reload, panels) is not part of this crate and is not described here.

## 1. Package

- One core WebAssembly module (`.wasm`).
- Custom section `craft.manifest`: UTF-8 JSON, at most 256 KiB, described by [schemas/plugin-manifest.schema.json](schemas/plugin-manifest.schema.json). Required.
- Custom section `craft.sig`: reserved for a detached signature. Absent in v1. Hosts ignore it in v1.
- Plugin id: `[A-Za-z0-9._-]{1,64}`. The prefixes `ec.`, `pc.`, `vc.`, `craft.`, `app.` and `plugin.`, and the whole ids `app` and `plugin`, are reserved for the host and the suite, in any letter case. The gate rejects them with "reserved". A host registers its own management commands as `plugin.app.<app>.<verb>`, so a plugin id of `app.<app>` would own the same command ids. Two installed ids may not differ only by letter case, because they would share a directory on a case-insensitive file system.

## 2. Policy gate

The gate runs before any guest code and gives the same answer on every engine. It rejects the package unless all of these hold:

- Size is at most 32 MiB. The manifest is present and valid. `abi.craft == 1`. Every `requires.ops` entry and every permission name is known to this crate.
- The module has exactly one memory, exported as `memory`, with a declared maximum at or below the host cap (`HostPolicy::memory_cap_mb`) and at or below `limits.memory_mb` if set.
- At most 4 tables, each with at most 100,000 elements. At most 10,000 functions. No start function.
- The only import is `craft_host.call : (i32, i32, i32, i32, i32) -> i64`, or there is no import.
- The exports in section 3 are present with these exact signatures.
- Features are limited to MVP, bulk-memory, sign-ext, multi-value, non-trapping float-to-int, mutable globals and reference types. `simd128` is allowed only when `requires.features` lists it and the host policy reports `SIMD128` (off by default). Relaxed-simd, threads, memory64, GC, exceptions, tail calls and multi-memory are rejected.

The gate lists every violation, not only the first.

Unknown contribution kinds. A `kind` this crate does not know is not a rejection. The manifest parser keeps the entry with its exact name and every other field, a host loads the entries it knows, and the plugin is shown as "partly supported". The entry gets every common check (`id`, `entry`, enum fields such as `phase`, `params`, `process`). Only the kind specific rules (a `hook` needs `phase`, a `before` hook needs `match`) bind the kinds that define them. The name must match `[a-z][a-z0-9-]{0,31}`. An unknown kind needs neither `craft_invoke` nor `craft_process`. A host never runs, registers or rewrites an unknown entry, and saving the plugin keeps the manifest bytes whole so a newer host can run them. Kinds listed as reserved (`function`, `provider`, `tool`) take this path until a release defines them.

Legacy v1 modules (exports prefixed `ec_`, `pc_`, `vc_`) skip the export, manifest and import checks of this section. A host loads them through a v1 shim. They keep the resource caps: the size cap, at most one memory, at most 4 tables of 100,000 elements and at most 10,000 functions. The shim refuses imports itself. A start function is allowed for them, because the shim runs it under the module's fuel and wall clock.

## 3. Exports

| Export | Signature | Required | Meaning |
|---|---|---|---|
| `memory` | memory | yes | linear memory |
| `craft_abi` | `() -> i32` | yes | returns 1 |
| `craft_alloc` | `(len: i32, align: i32) -> i32` | yes | 0 on failure |
| `craft_free` | `(ptr: i32, len: i32, align: i32)` | yes | frees a block from `craft_alloc` |
| `craft_invoke` | `(entry: i32, req_ptr: i32, req_len: i32, blob_ptr: i32, blob_len: i32) -> i64` | if any control-plane contribution | `entry` indexes `manifest.contributions`. Returns `len << 32 \| ptr` of a JSON response in guest memory, or a negative status |
| `craft_process` | `(desc_ptr: i32, desc_len: i32) -> i32` | if any data-plane contribution | processes one band or block. 0 is ok, non-zero is an error and the host discards the output |
| `craft_state_save` | `(out_ptr_ptr: i32) -> i32` | no | writes a pointer at `out_ptr_ptr`, returns the length |
| `craft_state_load` | `(ptr: i32, len: i32, from_ver_ptr: i32, from_ver_len: i32) -> i32` | no | restores state after a reload |

## 4. Import and ops

`craft_host.call(op, in_ptr, in_len, out_ptr, out_cap) -> i64`. The input is JSON at `in_ptr`. The host writes at most `out_cap` bytes of JSON at `out_ptr`. The return value has the status in the low 32 bits and the byte count in the high 32 bits (bytes written, or bytes required when `E_TOO_SMALL`).

| Status | Name |
|---|---|
| 0 | `OK` |
| -1 | `E_DENIED` |
| -2 | `E_TOO_SMALL` |
| -3 | `E_UNSUPPORTED` |
| -4 | `E_BAD_ARG` |
| -5 | `E_CANCELLED` |
| -6 | `E_LIMIT` |

Op numbers are stable. New ops are additive.

| Op | Name | Permission |
|---|---|---|
| 0x0001 | `host.log` | none |
| 0x0002 | `host.info` | none |
| 0x0003 | `host.cancelled` | none |
| 0x0004 | `host.progress` | none |
| 0x0010 | `cmd.list` | `commands.run` |
| 0x0011 | `cmd.execute` | `commands.run[:glob]` |
| 0x0020 | `doc.query` | `doc.read[:metadata\|pixels]` |
| 0x0021 | `doc.apply` | `doc.write` |
| 0x0030 | `event.poll` | `commands.observe` |
| 0x0040 | `link.call` | `link.call:<app>`, plus `link.exec:<app>:<glob>` for `engine.execute` |
| 0x0041 | `link.apps` | `link.call:<app>` |
| 0x0050 | `state.get` | `state.local` |
| 0x0051 | `state.put` | `state.local` |
| 0x0060 | `fs.open_picked` | `fs.picked` |
| 0x0061 | `fs.read` | `fs.picked` |
| 0x0062 | `fs.write` | `fs.picked` |
| 0x0070 | `net.fetch` | `net.fetch:<origin>` |
| 0x0080 | `time.now` | `clock` (else returns 0) |
| 0x0090 | `random.bytes` | `random` (else a deterministic stream) |

Link calls. `link.call:<app>` forwards only the methods in `permissions::LINK_CALL_METHODS` (`link.resolve`, `asset.describe`, `link.refresh`, `link.focus`). The list is closed and exact, with no case folding. `engine.execute` (`LINK_EXEC_METHOD`) also needs `link.exec:<app>:<glob>`, where the glob matches the command.

Command ids. `permissions::plugin_may_name_command` refuses every id that starts with `plugin.app.` (any ASCII case), is empty, is longer than 256 bytes, or holds anything but printable ASCII. A host applies it to `cmd.execute`, `cmd.list` and `link.exec`, even under a bare `commands.run` or `link.exec:<app>:*`.

## 5. Permissions

Default deny. A plugin holds only the permissions its manifest lists and the user grants.

| Name | Grants | Prompt level |
|---|---|---|
| `commands.run[:glob]` | run matching app commands | normal |
| `commands.observe` | `after` hooks and the change feed | normal |
| `commands.intercept` | `before` hooks: veto or rewrite | high |
| `doc.read[:metadata\|pixels]` | snapshot queries | normal |
| `doc.write` | one transactional edit outside commands | high |
| `ui.contribute` | menus, panels, dialogs | low |
| `state.local` | key-value store with a quota | low |
| `link.call:<app>` | read-only link methods of the named app | high |
| `link.exec:<app>:<glob>` | run the named app's commands that match the glob. Needs `link.call:<app>` | high |
| `fs.picked` | files the user picks in a host dialog, never paths | on first use |
| `net.fetch:<origin>` | HTTPS requests to the listed origin through the host | high |
| `clock`, `random` | real time and entropy | low |

## 6. Format contributions

A `format` contribution reads or writes one file type. It needs `extensions` (1 to 16 lower case letters or digits each) and the `fs.picked` permission. The host gives it bytes, never a path. A response is JSON, so bytes in it are base64. Pixels are straight alpha RGBA8, top row first. The `format` module holds the request and response shapes.

## 7. Data plane descriptor

The host writes a little-endian descriptor into guest memory. The `size` field gives its version.

```
size:u32 kind:u32(image=1,audio=2) entry:u32 w:u32 h:u32 stride_bytes:u32 channels:u32
sample:u32(u8=1,u16=2,f16=3,f32=4) alpha:u32(none=0,straight=1,premultiplied=2) color_mode:u32 depth:u32
origin_x:i32 origin_y:i32 overlap:u32 scale:f64 time:f64 sample_rate:u32 frames:u32
in_ptr:u32 out_ptr:u32 params_ptr:u32 params_len:u32
```

## 8. Manifest rules for budgets and text

A contribution may declare `"budget": "batch"`. The class has no short deadline, so it may only run where a long call cannot freeze the UI thread or where the call waits on another process. `Manifest::validate` refuses `batch` on any other contribution, with an error that names the field.

| Kind | `batch` allowed |
|---|---|
| `format`, `filter`, `object-filter` | yes |
| `command` | only when the manifest asks for a `link.call:<app>` permission (required or optional) |
| `hook`, `panel`, `menu`, `effect`, `audio-effect`, `live-effect`, unknown kinds | no |

Plugin text. `Manifest::validate` refuses control, bidi and zero-width characters in `name`, `version`, `author`, `license`, `homepage`, `description` (newline and tab allowed), contribution `label`, `category`, `title`, `shortcut`, `menu` entries and param `label`, `group` and `options`. The `text` module holds the functions that clean runtime text (notices, veto reasons, log lines): one line, a length cap, controls turned into spaces, bidi and zero-width characters removed.
