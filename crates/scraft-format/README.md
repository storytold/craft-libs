# craft-scraft-format

The native SoundCraft session document, `.scraft` JSON schema and typed reader/writer. `Session`
and its nested types are the wire representation; the library checks `format_version` before
parsing, deserializes the document directly (without building a `serde_json::Value` tree), then
sanitizes and validates the resulting session. `Session::to_json` writes the same pretty JSON
representation.

Named hardware inputs compare ASCII letters case-insensitively and leave non-ASCII characters distinct, while preserving their stored spelling. Duplicate path names and keys over 256 characters are rejected.
The session format remains version 1. Older SoundCraft releases can discard unused named paths on save and cannot
open tracks assigned to the new named input route. Keep the original project when opening a copy in an older release.

The crate retains SoundCraft's serialized enum names, defaults, compatibility behavior for older
fields, 256 MiB session byte limit, character limits, and duplicate/exhausted ID checks. Its time
fields use `craft_time::soundcraft`, the exact sample-index time API retained by `craft-time`.
MIDI sequence values use `craft-midi`'s serde representation.

The application still maps sessions to UI state, engine state, media paths and audio pools. The
session schema and its format-specific parsing, writing, sanitization and validation live here.
