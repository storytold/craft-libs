# craft-timeline

Timelines for Craft apps, from FilmCraft (Premiere Pro), EffectCraft (After Effects) and SoundCraft (Pro Tools).

The core is egui-free:

- `time`: `Tick` (254,016,000,000 per second, exact for every broadcast frame rate and common sample rate), `TimeRange`, and `TimeView` (the time-to-pixel map with zoom and scroll).
- `rates`: `FrameRate` (rational), `SampleRate`, `PixelAspect` and their menu presets.
- `timecode`: SMPTE timecode (drop and non-drop), frames, feet + frames, samples and seconds, formatted and parsed the way Premiere's fields do.
- `clock`: `PlaybackClock` (play, stop, preroll, loop, stop point, an external audio clock) and `Follow` (the view follows the playhead while playing).
- `cache` and `frames`: a byte-budgeted `FrameCache` that spares the frames a playing preview reaches next, and a `FrameServer` that renders frames on worker threads by priority with playback prefetch (on wasm, `pump` runs them on the UI thread).
- `clips`: clip edges, trims and snapping rules; `markers`: timeline markers; `model`: the `TimelineModel` trait the app implements on its sequence.

With the `ui` feature, `craft_timeline::ui` has the `Timeline` panel (track headers, rows, playhead), `TimelineRuler`, `TimelineScrollBar`, `ClipLane`, `KeyframeLane` and `KeyframeNavigator`, `GraphEditor`, `TransportBar` and `MediaControls`, `TimecodeField` and `TimecodePanel`, `FramePreview`, `MarkerStrip` and `MarkerDialog`, `TimelineOverview` and `RenderQueueList`. Edits go out as events; the app applies them to its document and undo stack.

Stories are `#[story]` functions in `src/ui/*/stories.rs`; they register in `craft_ui::testing::stories()` and the `craft-ui-gallery` viewer lists them by the type they show.

## Interchange

Editorial timeline interchange: the parts every reader and writer of an edit shares, and the text formats
that are documents of their own. Times are the core's `Tick` and `FrameRate`.

| Module | What |
|---|---|
| `rate` | Frame-rate conventions of documents: `timebase` + `ntsc`, floating rates snapped to exact ones, the highest rate an importer accepts (`MAX_RATE`). |
| `report` | `Report`: what an import or export could not represent exactly, identical messages merged with a count. |
| `path` | Media references: `file://` URLs (`localhost`, drive letters, UNC hosts, percent-encoding) to and from paths, and relative references joined to a document's folder. Lexical only. |
| `interchange` | A neutral timeline: a `Project` of bins and items (media with what a document says about it, sequences, subclips), sequences of video and audio `Track`s holding `TrackItem`s (speed, reverse, freeze frame, links, effects with keyframes, markers) and `Transition`s. `Catalog` is the application's effects, supplied by the caller. Every object has an `ext` slot for application JSON a format can carry unread. |
| `fx` | The effect and parameter ids the formats name themselves (motion, opacity, time remap, volume, pan, the common dissolves) and `Standard`, a catalog of just those. |
| `build` | Building an `interchange::Project` while reading a document (`Builder`, media deduplicated by key, track placement bounded at 256 tracks) and the bounds every import ends with (`Imported::bound`: implausible rates and start timecodes reset and reported). |
| `ale` | Avid Log Exchange: `AleDoc` (heading, columns, rows), `parse`, `write`, the standard heading and columns. |
| `edl` | CMX 3600 edit decision lists: `parse` to the list as written (events, their lines and timecode fields, `M2`, `AUD`, clip-name, source-file, effect-name and locator comments), `write` for events the caller built, channel codes, reel names, the frame-rate guess. |
| `fcp7` (feature `xml`) | Final Cut Pro 7 XML (`xmeml` 4 and 5, the dialect Premiere Pro reads and writes) to and from `interchange`. Read with craft-xml, strictly and bounded. |
| `fcpxml` (feature `xml`) | FCPXML 1.9–1.11 to and from `interchange`: the primary storyline as V1, connected clips by lane, `ref-clip` compounds as nested sequences, rational times exact, transforms, opacity, volume and their keyframes, retiming through `timeMap`. |
| `otio` (feature `otio`) | OpenTimelineIO JSON to and from `interchange`: timelines, collections, stacks as nested sequences, gaps, transitions, markers, linear time warps and freeze frames. What OTIO has no schema for goes in `metadata.<namespace>`, and the model's `ext` is merged into it, so an application's own data round-trips. |

No file I/O. EDL and ALE give the document as the format writes it. The formats that describe a
whole edit read into and write from `interchange`; mapping it onto an application's own project
stays in the application.
AAF and OMF are in `craft-aaf`.

Feature `xml`: the XML formats, on craft-xml. Feature `otio`: OpenTimelineIO, on serde_json.

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
