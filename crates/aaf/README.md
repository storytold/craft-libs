# craft-aaf

AAF and OMF, the two mob-based editorial interchange formats, read and written through one
composition model.

| Module | What |
|---|---|
| `model` | `Document`: compositions (tracks of fillers, source clips and overlapping transitions; clip gain, constant or keyframed; markers; start timecode), nested compositions, and the media `Source`s they reference (file path or embedded PCM, picture size and rate, sound channels / rate / bits, start timecode, markers). Times are exact `craft_timeline::time::Tick`s. |
| `aaf` | `read`, `write`, `sniff`: AAF Edit Protocol in structured storage (`craft-cfb`). |
| `omf` | `read`, `write`, `sniff`: OMF Interchange 2.0 in a Bento container. Sound only. |

No file I/O: documents are bytes and media references are strings. An application maps its own
timeline onto `model` once and gets both formats. Embedded audio is PCM in the model; the WAVE and
AIFF wrappers the formats use come from `craft-audio`.

## Specifications

Clean-room: written from public specifications only. The AAF SDK, pyaaf2, the OMF Toolkit and
other implementations were not consulted. Where a specification leaves a layout open (how a
nested sequence is written, how several video tracks share a slot), what Premiere Pro does was
read off files it exports.

| Format | Document | Edition | Used for |
|---|---|---|---|
| AAF | AMWA *AAF Object Specification* | v1.1 (2005) | classes, properties, mob / slot / segment model, sequences and transitions, operation groups, parameters, definitions, descriptors, locators, essence data |
| AAF | AMWA *AAF Low-Level Container Specification* | v1.0.1 | objects as structured storages, `properties` streams, stored forms, strong / weak reference collections and their index streams, `referenced properties` |
| AAF | AMWA *AAF Edit Protocol* | v1.0 (AMWA AS-01) | top-level composition, mob chains (composition → master → file → physical source), operation definitions, usage codes |
| AAF | SMPTE RP 224 / RP 210 registers | — | data definition, class and type labels |
| AAF | Microsoft [MS-CFB] | rev. 10.0 | the container, see `craft-cfb` |
| OMF | Avid *OMF Interchange Specification* | Version 2.0 (1997) | classes (`HEAD`, `CMOB`, `MMOB`, `SMOB`, `MSLT`, `TRKD`, `SEQU`, `SCLP`, `FILL`, `TRAN`, `EFFE`, `ESLT`, `CVAL`, `VVAL`, `TCCP`, `WAVD`, `AIFD`, `WAVE`, `AIFC`, locators), property and type names |
| OMF | Apple *Bento Specification* | revision 1.0d5 | container label, TOC encoding, objects / properties / types, references |

## What is read and written

AAF: a top-level composition mob with a timecode slot, a timeline slot per track (sequences of
fillers, source clips and transitions; `VideoDissolve`, `VideoFadeToBlack`, `SMPTEVideoWipe`,
`MonoAudioDissolve`), clip gain as a `MonoAudioGain` operation group with a constant or varying
amplitude, markers as comment markers in an event slot, a composition mob for every nested
composition, and per media item a master mob, file source mobs (CDCI or PCM descriptor with a
network locator, or `EssenceData` with embedded PCM) and a tape source mob with the start
timecode. The reader also takes sequences inside sequences, selectors, essence groups, legacy data
definitions, and Premiere Pro's layout (all video tracks as layers of one slot, nested compositions
tagged top-level). 512-byte or 4096-byte sectors.

OMF: a composition mob with a timecode slot and a sound slot per track, in samples; transitions as
`SimpleMonoAudioDissolve`; gain as `MonoAudioGain`; master, file (`WAVD` / `AIFD`, embedded `WAVE`
data or a locator) and tape mobs.

Not represented: speed changes, video effects other than transitions, OMF picture and markers.

Private tags, first written by FilmCraft and kept so existing files read back: AAF tagged values
`FilmCraft Effect`, `FilmCraft Frame Size`, `FilmCraft Audio Sample Rate`; OMF properties
`FilmCraft:SCLP:ClipName`, `FilmCraft:EFFE:EffectID`.

## Hostile input

A damaged or crafted file gives an error or a report entry, never a panic, a hang or an allocation
sized by a count in the file: weak-collection counts are checked against the index stream, zero-size
keys are refused, a storage owned twice is an error, nesting of segments and of compositions is
bounded, times are clamped to about three days. `src/aaf/hostile_tests.rs` and
`src/roundtrip_tests.rs` (truncation at every 97th byte, 400 random 8-byte corruptions per format)
cover them.

## Origin

Moved from FilmCraft's `filmcraft-interchange` (`aaf`, `omf`, the model types of `comp`). The
mapping between the model and an application's project stays in the application.
