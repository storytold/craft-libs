# craft-scraft-format

The native SoundCraft session document, `.scraft` JSON schema and typed reader/writer. `Session`
and its nested types are the wire representation; the library checks `format_version` before
parsing, deserializes the document directly (without building a `serde_json::Value` tree), then
sanitizes and validates the resulting session. `Session::to_json` writes the same pretty JSON
representation.

The crate retains SoundCraft's serialized enum names, defaults, compatibility behavior for older
fields, 256 MiB session byte limit, character limits, and duplicate/exhausted ID checks. Its time
fields use `craft_time::soundcraft`, the exact sample-index time API retained by `craft-time`.
MIDI sequence values use `craft-midi`'s serde representation.

## Format versions

A session file declares the lowest `format_version` able to read it back, not the writer's own
version: `Session::to_json` writes 1 unless the document holds a `Route::Instrument` (an
instrument's auxiliary output bus, used as a track input), which serializes an enum tag v1 cannot
parse — those files declare 2. A reader refuses anything above `Session::FORMAT_VERSION` with
`SessionError::Version`, so an older build fails such files with the newer-version message rather
than a generic parse error.

The application still maps sessions to UI state, engine state, media paths and audio pools. The
session schema and its format-specific parsing, writing, sanitization and validation live here.
