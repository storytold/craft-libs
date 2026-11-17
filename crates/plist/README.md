# craft-plist

A cross-platform bounded binary (`bplist00`) reader, extracted from Craft Anything's existing implementation and hardening history. Typed dictionaries retain order and duplicate keys; `get` returns the first match. Strings retain the source reader's lossy decoding policy. Shared references expand under node and decoded-byte budgets, and cycles are rejected.

`parse_binary_with_limits` accepts explicit input, object, expanded-node, nesting and decoded-byte limits. Recursion always has a hard ceiling of 256 even if a caller supplies a larger depth. Counts are checked before container reservation; allocations can return a resource error. Limits are per parse, not a promise of a precise process RSS cap.

This crate contains reading only. Writing and keyed-archive interpretation are not implemented. JSON projection, date formatting, OS file access and preference/locale selection are not part of this crate. Existing updater/Photo streaming `plist` users remain unchanged.

Initial consumers: PhotoCraft's AppleLanguages binary-reader path; later Craft Anything binary parsing and LightCraft XML presets (after consumer parity). Photo must retain its 4 MiB input policy and first-language selection.

Source: `storytold/craft-anything` at `88d1d9cac852aec1c7fd97886db451e005e49d2a`, `crates/formats/src/plist.rs` and checked byte reader. Source license files retained. Imported changes retain Brandon Thomas, BFlat, RRFZ and Afloat as primary Git authors; imported regression tests are BFlat's. No external specification implementation copied.

## XML reader

`parse_xml` and `parse_xml_with_limits` now read UTF-8 and BOM-tagged UTF-16 XML into the same `Value` tree. XML input/decoded-text size, tokens, values, nesting and cumulative key/leaf text are bounded. The independent token ceiling is eight times the smaller node/object allowance plus eight; unusually fragmented text can exhaust it before the value limit. This is not a precise RSS limit. The decoded-XML limit also includes markup; `<data>` consumes its encoded text allowance, including whitespace. XML `max_objects` and `max_nodes` both cap values.

The attributed Craft Anything reader includes its complete-document, declaration-skipping (no external entity fetching), quoted-tag and normalized-line-ending behavior. Nonfinite reals are refused using contributor PR #1905. Unknown or malformed character references retain legacy literal behavior; scanning stops at the next non-entity byte, preserving LightCraft's valid zero-padded numeric references without suffix rescans. Base64 retains that reader's legacy optional-padding/ASCII-whitespace behavior, including unused trailing bits; it is not a signature/key-validation API. No writer, JSON projection, keyed-archive interpretation or app policy is added.

XML source: Craft Anything `8f3b775157d331dd31cfa4da911e26d8856b8c77`, plus #1905 `d85e104f093a334961048881bf003351e1ac4992`. The import credits Brandon Thomas and the existing hardening contributors; #1905 is authored by gmrmad1 and the quoted-tag/line-ending tests by RRFZ. Consumer cutovers require their own policy/parity tests, especially LightCraft's preset mapping and Craft Anything's projections.
