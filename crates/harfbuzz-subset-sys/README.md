# harfbuzz-subset-sys

Raw bindings to the HarfBuzz subsetter, built from vendored upstream source.
`build.rs` compiles the `harfbuzz/src/harfbuzz-subset.cc` amalgamation with
the C++ compiler `cc` finds; it downloads nothing. `src/lib.rs` declares the
few C functions and constants craft-font-subset uses, by hand, and `src/safe.rs`
wraps them in a safe API. The crate is one of the places that allow `unsafe`
(design-lint rule `contract/unsafe-allow`); craft-font-subset has none.

## Vendored release

| Field | Value |
|---|---|
| Version | 15.0.0 |
| URL | https://github.com/harfbuzz/harfbuzz/releases/download/15.0.0/harfbuzz-15.0.0.tar.xz |
| sha256 | `968536e9abd782f07f7954044c0c4020199fcdd39b41d221ec42d2a3be4a44df` |

`harfbuzz/src/` is the tarball's `src/` without the Rust and WebAssembly
shapers, unit tests, generator scripts and inputs, meson files, and the
`harfbuzz.cc` and `harfbuzz-world.cc` amalgamations. `harfbuzz/COPYING` is
HarfBuzz's licence ("Old MIT", SPDX `MIT-Modern-Variant`). That identifier is
allowed for this vendored source only (an exception recorded 2026-11-12); it is
not on the house licence allowlist for other dependencies.

## Updating

1. Take the version and the `harfbuzz-<version>.tar.xz` sha256 from the
   [release page](https://github.com/harfbuzz/harfbuzz/releases).
2. Run `scripts/update-harfbuzz.sh <version> <sha256>` from the repository
   root. It checks the digest, replaces `harfbuzz/src/` and `COPYING`, and
   updates the table above.
3. Read the release notes (`NEWS` in the tarball) for subsetter changes, and
   check that the declarations in `src/lib.rs` still match `hb-subset.h`,
   `hb-blob.h`, `hb-face.h` and `hb-set.h`.
4. Run `tools/cl test craft-font-subset`.
