# craft-aep

After Effects projects (`.aep`, `.aepx`): the container as a lossless chunk tree, and a typed,
read-only view of what it holds. Bytes in, a neutral model out, and the tree back to bytes.
Mapping the view onto an application's document (which effect becomes which, units, what is
reported as lost) stays in the application.

No file access: callers pass bytes, so the crate builds for wasm32. No dependencies.

| Form | Read | Write |
|---|---|---|
| `.aep` (RIFX `Egg!`) | chunk tree and typed view | the chunk tree, byte for byte |
| `.aepx` (XML) | into the same chunk tree | no |

## Container

An `.aep` is a `RIFX` container (big-endian RIFF) with form type `Egg!`: chunks of a 4-byte id, a
big-endian `u32` size and a body padded to an even length; `LIST` chunks start with a 4-byte list
type followed by chunks. `parse` builds the tree, `write` serialises it, and for every input that
parses, `write(parse(x)) == x` byte for byte (container and chunk sizes are recomputed from the
tree; pad bytes, odd bodies and the bytes after the container, an XMP packet, are kept).

The XML form of a project (`.aepx`) is read into the same tree by `read_aepx` (a small bounded
XML tokenizer of its own); `parse_any` picks the reader by content, so the typed view takes
either form. The mapping is in [`docs/aep-format.md`](docs/aep-format.md), section 1.

A reader that only looks does not need the owned tree. `TreeIndex` is the same tree as one flat,
read-only array in depth-first order, borrowed from the input: `TreeIndex::parse` for an `.aep`,
`index_aepx` and `TreeIndex::from_drafts` for an `.aepx` (decoded into one buffer first). It
counts the same chunks against the same `Limits` and returns the same errors as `parse`, and the
typed view reads through either (`Project::new` for the owned tree, `Project::from_index` for the
index). An importer should use the index; it is about twice as fast and allocates far less.

Bodies that look like lists but are not chunk sequences (for example `LIST btdk`, raw text) stay
opaque rather than failing the parse. Input is untrusted: sizes are checked against the remaining
input, nesting and chunk count are bounded (`Limits`), nothing is allocated from a declared size,
and truncated or damaged files return `Err` (or, inside the container, an opaque body).

## Typed view

The container layer decodes no chunk body. On top of it, a typed view (`Project`, `Item`, `Comp`,
`Layer`, `Property`, ...) reads the fields whose layout was verified against the values the test
files were made with: layer and composition headers (switches, exact times and time stretch, size,
frame rate, work area, shutter phase, display start, renderer), static transform values in After
Effects' units with the "absent = default" rule, keyframe records (interpolation sides, ease per
component, plain and spatial templates), Time Remapping, expressions with their enable switch,
effect instances and parameter values (with the settings data of Curves, Levels, Hue/Saturation,
Cryptomatte, EXtractoR, IDentifier and Puppet meshes), masks and bezier paths with tangents, shape
layer groups, text documents (paragraph and style runs, box, justification, tracking, faux styles,
caps, leading), markers with duration, strings and cue data, cameras and lights with their
options, track mattes, project settings and the working space's name, the version that saved the
file, footage (stills, audio, image sequences, the layer of a Photoshop file an item shows), solids
and precomp links. Every accessor returns
`Option` or an empty list on damaged data and never panics. Which fields are verified, which are
not decoded, and how it was found is in [`docs/aep-format.md`](docs/aep-format.md).

Writing a project from an application's document (building every chunk in code) is not here: it
is EffectCraft's exporter, which is written against EffectCraft's own model.

## Specifications

Public specifications used for parts the format embeds:

- ICC.1:2010: the profile description of a working-space profile (a v2 `desc` or a v4 `mluc`
  tag), from which the working space's name is read (`settings.rs`).
- RFC 4648 (base64): the profile data in the project's colour settings JSON.

## Clean room

Everything here comes from our own observations: project files that we generated ourselves with
After Effects, one change per file, compared byte by byte. No source code or format
documentation of another `.aep` reader or writer was used, and no Adobe asset is used.

## Tests

The unit tests and the robustness tests build their input in code. The tests of the typed view
read generated projects (own test data): a few combined projects, one composition per case
([`tests/fixtures/CASES.md`](tests/fixtures/CASES.md)). They are not in this repository but in
[storytold/effectcraft-corpus](https://github.com/storytold/effectcraft-corpus), with their
attribution, the generation notes and the inspection of their embedded content.
`tests/fixtures/corpus.lock` pins the commit and lists the SHA-256 of every file used. To run
against them:

```sh
git clone https://github.com/storytold/effectcraft-corpus.git
git -C effectcraft-corpus checkout <commit from corpus.lock>
CRAFT_AEP_FIXTURES=$PWD/effectcraft-corpus/aep CRAFT_AEP_FIXTURES_REQUIRED=1 cargo test -p craft-aep
```

The tests reach the projects only through `tests/it/common/mod.rs`, which hands a test its case
as a project of its own cut from the combined file. Without `CRAFT_AEP_FIXTURES` such a test
prints `SKIPPED: fixture <name> missing` and passes; with `CRAFT_AEP_FIXTURES_REQUIRED` set, a
missing project fails it.

## Tools

- `examples/aepdump.rs`: prints the chunk tree of a file (ids, sizes, short data, match names):
  `cargo run -p craft-aep --example aepdump -- file.aep`.
- `examples/stubicc.rs`: replaces every embedded ICC profile of a saved project by a stub that
  keeps only the profile's description: the JSON `colorProfileData` of the working space in an
  `.aep` chunk or the `.aepx` text, and a raw profile of an imported image:
  `cargo run -p craft-aep --example stubicc -- in.aep out.aep`. A profile it cannot replace stops
  it before anything is written. Use it before a generated project becomes test data, so no
  vendor colour data stays in it, and check the result independently.
