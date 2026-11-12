# craft-timeline

Editorial timeline interchange: the parts every reader and writer of an edit shares, and the
text formats that are documents of their own.

| Module | What |
|---|---|
| `time` | Exact media time. A `Tick` is 1/254 016 000 000 s, which divides evenly into every broadcast frame duration and every common audio sample duration; `FrameRate` is an exact rational; SMPTE timecode (drop and non-drop), feet+frames and typed-timecode parsing are displays of ticks. Every conversion is total: a zero or negative rate gives zero, not a division by zero, and results saturate at the storage limits. |
| `rate` | Frame-rate conventions of documents: `timebase` + `ntsc`, floating rates snapped to exact ones, the highest rate an importer accepts (`MAX_RATE`). |
| `report` | `Report`: what an import or export could not represent exactly, identical messages merged with a count. |
| `path` | Media references: `file://` URLs (`localhost`, drive letters, UNC hosts, percent-encoding) to and from paths, and relative references joined to a document's folder. Lexical only. |
| `model` | A neutral timeline: a `Project` of bins and items (media with what a document says about it, sequences, subclips), sequences of video and audio `Track`s holding `TrackItem`s (speed, reverse, freeze frame, links, effects with keyframes, markers) and `Transition`s. `Catalog` is the application's effects, supplied by the caller. Every object has an `ext` slot for application JSON a format can carry unread. |
| `fx` | The effect and parameter ids the formats name themselves (motion, opacity, time remap, volume, pan, the common dissolves) and `Standard`, a catalog of just those. |
| `build` | Building a `Project` while reading a document (`Builder`, media deduplicated by key, track placement bounded at 256 tracks) and the bounds every import ends with (`Imported::bound`: implausible rates and start timecodes reset and reported). |
| `ale` | Avid Log Exchange: `AleDoc` (heading, columns, rows), `parse`, `write`, the standard heading and columns. |
| `edl` | CMX 3600 edit decision lists: `parse` to the list as written (events, their lines and timecode fields, `M2`, `AUD`, clip-name, source-file, effect-name and locator comments), `write` for events the caller built, channel codes, reel names, the frame-rate guess. |
| `fcp7` (feature `xml`) | Final Cut Pro 7 XML (`xmeml` 4 and 5, the dialect Premiere Pro reads and writes) to and from `model`. Read with craft-xml, strictly and bounded. |
| `fcpxml` (feature `xml`) | FCPXML 1.9–1.11 to and from `model`: the primary storyline as V1, connected clips by lane, `ref-clip` compounds as nested sequences, rational times exact, transforms, opacity, volume and their keyframes, retiming through `timeMap`. |
| `otio` (feature `otio`) | OpenTimelineIO JSON to and from `model`: timelines, collections, stacks as nested sequences, gaps, transitions, markers, linear time warps and freeze frames. What OTIO has no schema for goes in `metadata.<namespace>`, and the model's `ext` is merged into it, so an application's own data round-trips. |
| crate root | `Format` (EDL, xmeml, FCPXML, OTIO, AAF, OMF), `detect_text`, `decode_text`. |

No file I/O. EDL and ALE give the document as the format writes it. The formats that describe a
whole edit read into and write from `model`; mapping `model` onto an application's own project
stays in the application.
AAF and OMF are in `craft-aaf`.

Feature `xml`: the XML formats, on craft-xml. Feature `otio`: OpenTimelineIO, on serde_json. Feature `serde`: `Serialize` / `Deserialize` for the time, report and `Format` types.

## Specifications

Written from public descriptions only: SMPTE ST 12-1 (timecode); the CMX 3600 EDL format as
documented by its users (event lines, `FCM`, `M2`, and the common `* FROM CLIP NAME:`,
`* SOURCE FILE:`, `* LOC:` comment extensions); Avid's published description of ALE; RFC 8089
(`file` URIs).

## Origin

Moved from FilmCraft's `filmcraft-time` (whole) and `filmcraft-interchange` (`ale`, the parser and
line layout of `edl`, the path and rate helpers of `common`, `Report`, `Format`).

## Hostile input

Parsers are line-oriented over text the caller already holds, so memory is bounded by the input.
Timecode fields beyond `i64` are refused; audio channels outside 1..=99 are refused and reported;
an `FPS` heading that is not a positive finite rate up to 100 000 is no rate. Tests cover each.
