# craft-plotter

The command languages of vinyl cutters and pen plotters. A job (polylines in millimetres, tool
settings, registration marks) goes in; the exact command bytes of one language come out. The
device queries and the parsers of their replies are here too. The crate does no I/O and builds
for wasm: opening a port, writing to it and waiting for the device stay with the caller.

| Language | Module | What it writes |
|---|---|---|
| HP-GL (the `IN SP PU PD` subset, also valid HP-GL/2) | `hpgl` | `IN;` `SP<n>;` then `PUx,y;` / `PDx,y,…;` per path, `PU;` `SP0;`. 40 units per mm, origin bottom-left, y flipped about the design's bottom edge. One command per line. |
| DMPL | `dmpl` | `;:` `EC1` then `Ux,y` / `Dx,y,…` per path, `U0,0`, `@`. 0.001 in units, origin bottom-left. One command per line. Experimental. |
| GP-GL, plain | `gpgl` | `!<speed>` when set, `M<down>,<right>` / `D<down>,<right>` at 0.1 mm steps, park `M<below>,0`. Each command ends with ETX. |
| GP-GL, Silhouette dialect | `gpgl::silhouette` | Setup preamble per generation (legacy, Cameo 3, Cameo 4 line), registration marks, `M`/`D` at 0.05 mm units, park `M<below>,0` `SO0`. Each command ends with ETX. Status, firmware-version and tool-setup queries and their reply parsers. |

G-code is not here: it is [`craft-gcode`](../gcode/src/lib.rs).

## Use

```rust
use craft_plotter::{hpgl, Job, Operation};

let job = Job {
    operations: vec![Operation { holder: 1, paths: vec![vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]], ..Operation::default() }],
    ..Job::default()
};
let program = hpgl::encode(&job)?;
let bytes = program.to_stream();      // what to write to the device
let lines = program.transcript();     // one command per line, for a dry run
```

Paths are `(x right, y down)` in millimetres from the media's top-left corner. `ToolSettings` has
force and speed in the device's own steps, passes, line-segment overcut at sharp corners, and the
blade settings of the Silhouette dialect. The Silhouette encoder also takes a `Setup`: the
generation, the cutting region, the mat and the media preset.

A program with registration marks has `registration_wait`: send the first part
(`Program::split_at_registration`), wait until the device reports the marks found
(`parse_status` gives `Status::Ready`), then send the rest.

The optional `serde` feature gives `gpgl::silhouette::Mat` `Serialize` and `Deserialize`, as its
short name (`"12x24"`).

## Limits

Input is not trusted. A job is refused, and nothing encoded, past 64 operations
(`MAX_OPERATIONS`), 2,000,000 points (`MAX_POINTS`), 10 passes (`MAX_PASSES`) or a 2 mm overcut
(`MAX_OVERCUT_MM`), or when a coordinate, offset or setting is not finite. A device reply is read
only up to 256 bytes (`gpgl::silhouette::MAX_REPLY_LEN`) and reports at most 8 tool holders
(`MAX_TOOL_HOLDERS`). Coordinates past the integer range saturate. Nothing panics.

What a particular machine accepts (cutting area, force and speed range, which tools and mats) is
not checked here. That is a table of products, and it belongs to the caller.

## Where the protocol facts come from

None of these languages has a vendor specification that covers what current machines accept, so
the facts are from public documentation and from community descriptions of the command sets.
They were not verified on hardware by this crate's authors.

- Silhouette dialect (units, framing, the escape queries and their replies, the preamble of each
  generation, mat and tool codes, registration-mark commands): the command documentation and
  device notes published by the community driver projects
  [`fablabnbg/inkscape-silhouette`](https://github.com/fablabnbg/inkscape-silhouette) (GPL-2.0),
  [`virtualabs/inkscape-cutcutgo`](https://github.com/virtualabs/inkscape-cutcutgo) (GPL-3.0),
  `pklaus/Silhouette-Cutting-Plotters`, and the `robocut` lineage they credit.
- GP-GL: Graphtec's GP-GL command list (CraftRobo Pro S / CE5000):
  <https://web.archive.org/web/20160801023226/http://ohthehugemanatee.net/uploads/2011/07/GP-GL.pdf>.
  The plain encoder assumes the panel's GP-GL step size is 0.1 mm and the Silhouette axis order
  (feed first).
- HP-GL: the long-published `IN`, `SP`, `PU`, `PD` instructions of the HP-GL and HP-GL/2
  references.
- DMPL: no published manual was found for the `;:`, `EC1`, `U`/`D`, `@` framing. It is
  experimental: test on scrap.

Those drivers were read as specifications only: for the facts of what a device accepts, not for
code. No GPL code was ported or adapted; the Rust here was written from the facts, and the crate
is MIT OR Apache-2.0 like the rest of the repository. Vendor binaries were not disassembled,
decompiled or debugged. The line-segment overcut is this crate's own geometric rule, not a
captured vendor stream. When extending the crate, keep to this: public documentation and
observed device behaviour, never GPL driver source.
