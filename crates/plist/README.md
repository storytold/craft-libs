# craft-plist

A cross-platform bounded binary (`bplist00`) reader, extracted from Craft Anything's existing implementation and hardening history. Typed dictionaries retain order and duplicate keys; `get` returns the first match. Strings retain the source reader's lossy decoding policy. Shared references expand under node and decoded-byte budgets, and cycles are rejected.

`parse_binary_with_limits` accepts explicit input, object, expanded-node, nesting and decoded-byte limits. Recursion always has a hard ceiling of 256 even if a caller supplies a larger depth. Counts are checked before container reservation; allocations can return a resource error. Limits are per parse, not a promise of a precise process RSS cap.

This first PR contains reading only. XML parsing/writing and keyed-archive interpretation are not implemented. JSON projection, date formatting, OS file access and preference/locale selection are not part of this crate. Existing updater/Photo streaming `plist` users remain unchanged.

Initial consumers: PhotoCraft's AppleLanguages binary-reader path; later Craft Anything binary parsing and LightCraft XML presets (after XML support/parity). Photo must retain its 4 MiB input policy and first-language selection.

Source: `storytold/craft-anything` at `88d1d9cac852aec1c7fd97886db451e005e49d2a`, `crates/formats/src/plist.rs` and checked byte reader. Source license files retained. Imported changes retain Brandon Thomas, BFlat, RRFZ and Afloat as primary Git authors; imported regression tests are BFlat's. No external specification implementation copied.
