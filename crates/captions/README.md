# craft-captions

Caption and subtitle files behind one neutral model: bytes in, a `Document` of `Cue`s out, and
back. One module per format. Depends on `craft-timeline` (exact time),
`craft-xml` (TTML) and `thiserror`.

```rust
use craft_captions::{Format, WriteOptions, detect, parse, write};

let bytes = b"1\n00:00:01,000 --> 00:00:02,500\nHello\n";
let format = detect(bytes, Some("srt")).ok_or(craft_captions::Error::NotFormat("caption"))?;
let mut doc = parse(bytes, format)?;
doc.snap_to_frames(craft_captions::FrameRate::FPS_25);
let scc = write(&doc, Format::Scc, WriteOptions::default());
assert!(scc.starts_with(b"Scenarist_SCC V1.0"));
# Ok::<(), craft_captions::Error>(())
```

- `detect(bytes, extension)` guesses the format; `parse(bytes, format)` reads; `write(doc,
  format, options)` writes; `charset_warnings` says what an SCC or MCC export cannot carry.
- `Cue`: start and end (`Tick`), text with inline `<i>` / `<b>` / `<u>` tags kept, and the
  WebVTT speaker, cue identifier and cue settings. `Document`: the cues, WebVTT
  STYLE / REGION / NOTE blocks, the language and the reader's warnings.
- `cea608` has the CEA-608 character sets, control codes, parity and preamble address codes;
  `scc::decode_pairs` turns timed 608 byte pairs (from a file, or from a video stream's caption
  data) into cues, and `scc::schedule` does the reverse. `mcc` has the CEA-708 service coding
  and the SMPTE ST 334-2 caption distribution packet.
- `plain_text` strips inline tags and decodes the common entities, for display or for formats
  without markup.

## Time

`Tick` is an integer count of 1/254 016 000 000 s: a whole number of ticks per millisecond and
per frame at every broadcast rate, so no format's times are rounded on the way in. `FrameRate`
is an exact rational. Both are `craft-timeline`'s (`craft_timeline::time`), re-exported here as
`craft_captions::time`, so a caller that already uses that crate passes its values straight in.
No format computes with floating-point seconds.

## Limits

`parse` refuses input over `MAX_INPUT` (64 MiB) and keeps at most `MAX_CUES` (1,000,000) cues,
with a warning. TTML is read by `craft-xml`: no DTD processing or entity expansion, at most 128
levels of nesting, and its node and attribute limits. A TTML speaker label is at most 4096 bytes
from at most 64 agent references; a language tag at most 35 characters. `charset_warnings`
lists at most 20 characters per stream.

## Formats

| Format | Read | Write | Notes |
|---|---|---|---|
| SubRip `.srt` | yes | yes | BOM, UTF-16, Windows-1252 fallback, CRLF/CR, missing or wrong indexes, `,` or `.` milliseconds, 1–9 fraction digits, missing hours, SSA coordinates, cues not separated by blank lines. Inline tags are kept as text. |
| WebVTT `.vtt` | yes | yes | Cue identifiers, cue settings (verbatim), a leading `<v Speaker>` voice span (as the caption's speaker) and STYLE / REGION / NOTE blocks are kept. A missing `WEBVTT` signature is tolerated. |
| Scenarist SCC `.scc` | yes | yes | CEA-608 at 29.97 fps, drop-frame or not. Reads pop-on, roll-up and paint-on (channel 1); writes pop-on. |
| MacCaption MCC `.mcc` | yes | yes | One SMPTE ST 334-2 caption distribution packet (CDP) per frame carrying caption data, omitting empty frames (or on every frame, on request), in MacCaption's hex text with letter abbreviations. Writes 29.97 fps (`30DF` / `30`) with CEA-608 field 1 (the SCC schedule) **and** CEA-708 service 1 (two-window pop-on emulation). Reads CEA-608 field 1 when present, else CEA-708 service 1; time code rates 30DF, 30, 60DF, 60, 25, 24, 50. |
| EBU STL `.stl` | yes | yes | EBU Tech 3264-E: GSI block + 128-byte TTI blocks, `STL25.01` (25 fps) or `STL30.01` (read/written as 29.97 fps, non-drop labels), ISO/IEC 6937 Latin text (accents as diacritic + letter), italics / underline codes as `<i>` / `<u>`, extension blocks for long subtitles, start-of-programme (TCP) subtracted. |
| TTML `.ttml` (IMSC1) | yes | yes | Writes the IMSC1 Text profile (TTML2 namespaces) with the document's language as `xml:lang` (read back on import): `<p>` per cue, `<br/>`, styled `<span>`s for `<i>` / `<b>` / `<u>`, `ttm:agent` speakers, exact frame times (`123f`) at the sequence rate. Reads any TTML (TTML1, TTML2, the 2006 `ttaf1` drafts): clock / frame / offset / tick times, `dur`, nested time containers (`body` / `div` begins offset and ends clip their contents; `seq` containers are read as `par` with a warning), timed spans, referenced styles. |
| DFXP `.dfxp` | yes | yes | TTML1 with the DFXP presentation profile; clock times in milliseconds (frame-exact after `snap_to_frames`). |

All cue times are exact `Tick`s. Milliseconds convert exactly (1 ms = 254 016 000 ticks); SCC
frames map through `FrameRate::FPS_29_97`. `Document::snap_to_frames` snaps cues to a frame grid and
removes overlaps. SCC and MCC time codes count from `Document::start_timecode`, a time code
label read as a time (`label_time`), so the label carries across rates: 01:00:00:00 at 25 fps is
written as 01:00:00;00 drop-frame or 01:00:00:00 non-drop (`label_frame`). Readers leave it at
zero and keep the file's own time codes, noting the rate and drop-frame mode of its labels in
`Document::labels`; `Document::count_from` then counts the cues back from a start time code, when
every cue is at or after it.

### TTML tick timing

When `ttp:tickRate` is omitted and `ttp:frameRate` is present, tick offsets use the effective
frame rate (`frameRate` times `frameRateMultiplier`) times `subFrameRate`. The rate stays
rational, so a 30-tick offset at 30 times 1000/1001 fps is exactly 1.001 seconds. An explicit
`tickRate` takes precedence; without `frameRate`, the default is one tick per second.
This follows [W3C TTML1, third edition (2018), §6.2.10](https://www.w3.org/TR/2018/REC-ttml1-20181108/#parameter-attribute-tickRate).

### TTML speaker metadata

TTML and DFXP exports define each distinct speaker label once in the document head and refer to
it with a stable, unique `ttm:agent` ID. Names are XML-escaped full-name text with preserved
whitespace; punctuation, Unicode and literal underscores survive a round trip. Agent type is
`other` because a caption's speaker label does not say whether it denotes a person or a group.

On import, agent definitions directly under `head` or `head/metadata` resolve paragraph and timed
span speakers. Speaker metadata uses the standard TTML namespace or the April and October 2006
TTaf1 draft namespaces. Agent IDs use `xml:id`, except in the April draft, which uses unqualified
`id`. Agent definitions from unrelated application metadata are ignored. Recognized `ttm:agent`
attributes take precedence over the legacy local-name `agent` fallback. A full name is preferred,
otherwise the first nonempty name (or an explicitly empty name) is used. Multiple agent references become one display label, joined with `, ` in
reference order; the individual agent identities are not kept. Files that refer to an agent
without defining it read the reference itself as the label, with underscores as spaces. Other agent metadata, including actor relationships, is not retained.

This mapping follows [W3C TTML1, third edition (2018), §§12.1.5–12.2.1](https://www.w3.org/TR/2018/REC-ttml1-20181108/#metadata-vocabulary-agent).
The [April 2006 TTaf1 draft, §§7.2.1 and 12.1.5–12.2.1](https://www.w3.org/TR/2006/WD-ttaf1-dfxp-20060427/)
defines its earlier metadata namespace and agent ID form.

### TTML cue identifiers

TTML and DFXP writers keep valid explicit cue IDs verbatim unless a style, region or earlier cue
already uses them. Missing, invalid or repeated IDs receive deterministic replacements that also
avoid valid IDs on later cues. Emitted cue, style, region and speaker IDs are unique across the
document. Unicode spelling is preserved without normalization.

Identifier validation follows the `NCName` production in [Namespaces in XML 1.0, third edition
(2009), §3](https://www.w3.org/TR/2009/REC-xml-names-20091208/#NT-NCName), using the character
ranges in [XML 1.0, fifth edition (2008), §2.3](https://www.w3.org/TR/2008/REC-xml-20081126/#NT-NameStartChar).

### SCC writer

Each caption is sent as a pop-on load (RCL, ENM, then per row a preamble address code and tab
offset for centring, then text; control codes doubled) into non-displayed memory, followed by End
Of Caption exactly on the caption's first frame. The load is placed in the latest run of free
frames before the caption, so back-to-back captions load while the previous one is on screen. An
Erase Displayed Memory clears the caption at its out point unless the next caption replaces it.
Text is wrapped at 32 columns, bottom-aligned (last row 15) and centred; text past 15 rows is
cut. Characters come from the
608 basic, special and extended sets (extended characters are preceded by a basic fallback
character, as the standard requires); a few have a close stand-in (’ as ', – as -) and others are
dropped. `charset_warnings` names each dropped or replaced character and each caption cut to 15
rows, with how many characters it lost. If there is no room to load a caption
before its in point (for example a caption at 00:00:00:00), it appears as soon as it is loaded.

### MCC writer

The writer merges the scheduled CEA-608 and CEA-708 frames in timestamp order. Frames without
caption data are omitted, including during a displayed caption; its explicit erase/hide command
still lands at its out point. CDP sequence counters advance per emitted packet, wrapping at 16 bits.
CEA-708 text uses the G0 and G1 (Latin-1) sets and part of G2 (… ‘ ’ “ ” • ™ and a few more);
other characters are sent as `?`, and `charset_warnings` names them. The CEA-708 window holds 12
rows and the writer loads only those (`mcc::window_rows`), so a longer caption loses its last rows
there, and `charset_warnings` names it too.
Export work and output size therefore follow the scheduled caption data, not the time between
cues or their duration.

For consumers that need a packet on every video frame (SDI, frame-wrapped MXF with a SMPTE 436M
track), `WriteOptions::every_frame_until` also writes one on every frame without caption data,
from the start to that time: CEA-608 null pairs and CEA-708 padding. The packets are 29.97 fps,
so they line up with the video only at that rate; the caller decides whether to offer it. It is
off by default, covers at most 24 hours (`mcc::MAX_PADDED_FRAMES`), and can be streamed
(`write_to`).

### EBU STL writer

Text is encoded in ISO/IEC 6937 (character code table `00`). Characters it has no code for, such as
the ellipsis, the em dash, the euro sign and most scripts other than Latin, are written as `?`;
`charset_warnings` names each one with how often it occurs and the first caption that has it, at the
STL file's own time code.

### TTML inherited text styles

On import, italic, bold and underline styles from `<body>`, `<p>` and timed `<span>` roots appear in
the cue text as balanced `<i>`, `<b>` and `<u>` tags. Descendant overrides and explicit
`normal` / `noUnderline` resets are kept, and `inherit` restores the parent value. Styling
attributes are read from the TTML styling namespaces (and those of the 2006 drafts), with
unqualified attributes as a compatibility fallback; other namespaces are ignored. Named-style
chaining and `xml:space="preserve"` are not supported. The inline text collector does not
recurse. Content style inheritance follows [W3C TTML1, second edition (2013)](https://www.w3.org/TR/2013/REC-ttml1-20130924/#semantics-style-inheritance-content).

### Literal text and style markup

Cue text combines style tags with character entities, as WebVTT does. The TTML and EBU STL
readers keep a literal `<` as `&lt;` (and a literal `&` that would read as an entity as `&amp;`),
so it never turns into a tag. The TTML and STL writers decode the eight entities `entity_at`
knows: `&amp;`, `&lt;`, `&gt;`, `&nbsp;`, `&quot;`, `&apos;`, `&lrm;` and `&rlm;`; other named
entities and numeric references stay literal. The SRT writer keeps stored entities next to real
style tags, so a reimport does not turn a literal bracket into markup.
