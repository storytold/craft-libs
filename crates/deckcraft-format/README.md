# craft-deckcraft-format

The DeckCraft `.deckcraft` package. A file is a ZIP that holds, in this order:

- `mimetype`, stored: `application/vnd.storyteller.deckcraft+zip`. Files from before the
  SlideCraft rename carry the legacy type, and `sniff` accepts both.
- `presentation.json`, deflated: `{"format":"deckcraft","version":1,"generator":…,"presentation":…}`.
  It is compact JSON, and non-finite numbers are written as `0` (`Finite`).
- `media/<id>` and `chart-parts/<index>` binary members. A member whose content type is already
  compressed (JPEG, PNG, video, MP3/MP4 audio, anything `zip`) is stored, and any other member is deflated.

The presentation is a type parameter: DeckCraft owns the model, decides which media and chart
parts become members, and repairs a loaded deck. This crate owns the bytes:

- `write(presentation, generator, members)` writes the package with `craft-zip`'s deterministic writer.
- `Package::open_manifest(bytes, max)` reads and checks the manifest. A file with no
  `presentation.json` or a different `format` is `NotOurs`. `parse_manifest` reads files written
  before `Finite`, where serde_json saved NaN as `null`, as 0 within fixed patch and re-parse budgets.
- `Package::read_max` refuses a member that declares more than its limit and stops one that
  inflates past it. `read_charged` charges a shared budget for every byte it inflates, including
  the bytes of a read that then fails its CRC.
- `Budget` and the `MAX_*` constants are the format's limits: 2 GiB per member, 4 GiB of media and
  chart parts, 100,000 items and a 256 MiB manifest.

There is no `unsafe` and no panics, and it builds for `wasm32-unknown-unknown`.
