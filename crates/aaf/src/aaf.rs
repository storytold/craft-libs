//! AAF (Advanced Authoring Format) import and export, following the AAF Edit Protocol.
//!
//! Written from the public AMWA specifications: *AAF Object Specification v1.1*, *AAF Low-Level
//! Container Specification v1.0.1* (structured storage mapping, see [`store`]) and *AAF Edit
//! Protocol v1.0*; the container is Microsoft's Compound File Binary format ([`craft_cfb`]).
//!
//! A written file holds one top-level **CompositionMob** (Edit Protocol usage code
//! "top level") with
//!
//! - a timecode slot (sequence start timecode, rounded rate, drop frame),
//! - one timeline slot per video track (picture) and per audio track (sound; one per channel when
//!   broken out to mono), each a **Sequence** of **Filler**s, **SourceClip**s and **Transition**s
//!   (Cross Dissolve → `VideoDissolve_2`, Dip to Black → `VideoFadeToBlack`, audio crossfades →
//!   `MonoAudioDissolve`; the model's transition name is kept in a tagged value),
//! - clip volume as a `MonoAudioGain` **OperationGroup** around the source clip with an
//!   `Amplitude` **ConstantValue** or **VaryingValue** (keyframes as control points),
//! - sequence markers as **CommentMarker**s in an event slot;
//!
//! a **CompositionMob** without a usage code for every nested sequence, built the same way (a
//! clip of the nested sequence is a **SourceClip** that points at that mob, at its first slot of
//! the clip's kind, with its start in that slot's edit units; Premiere Pro writes nested sequences
//! this way);
//!
//! and for every media item a **MasterMob**, a file **SourceMob** per essence (a CDCI or PCM
//! descriptor with a network locator to the file, or `EssenceData` holding embedded PCM) and a
//! tape **SourceMob** carrying the media's start timecode. Clip and media markers become comment
//! markers on the master mob.
//!
//! Import reads the same structures (and the common variations: sequences inside sequences,
//! selectors, operation groups around clips, legacy data definitions, essence groups, and Premiere
//! Pro's one video slot holding a nested scope with a segment per video track) into the model:
//! compositions (one that others use is a nested composition), sources (with their path, or their
//! embedded audio as PCM) and markers.
//!
//! Private tags. Three tagged values are not in any register; they were first written by FilmCraft
//! and keep their names so that existing files read back: `FilmCraft Effect` (a transition's
//! name), `FilmCraft Frame Size` and `FilmCraft Audio Sample Rate` (on the composition).

mod ids;
mod read;
pub(crate) mod store;
mod write;

use crate::model::Document;
use crate::{Identity, Report, Result};

/// How to write an AAF file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOptions<'a> {
    pub identity: Identity<'a>,
    /// Write 512-byte sectors (compound file version 3) instead of 4096-byte sectors.
    pub small_sectors: bool,
}

/// Write `doc` as an AAF file: its first composition is the top-level one, with every nested
/// composition and every source it uses. Times that are not on whole edit units are rounded and
/// reported.
pub fn write(doc: &Document, opts: &WriteOptions<'_>, report: &mut Report) -> Result<Vec<u8>> {
    let version = if opts.small_sectors { craft_cfb::Version::V3 } else { craft_cfb::Version::V4 };
    write::write(doc, version, &opts.identity, report)
}

/// Read an AAF file: its top-level compositions, the compositions nested in them and their
/// sources. What cannot be represented is skipped and named in `report`.
pub fn read(bytes: &[u8], report: &mut Report) -> Result<Document> {
    read::read(bytes, report)
}

/// Whether `bytes` look like an AAF file (a compound file whose root has the AAF root class or a
/// `Header-2` storage).
pub fn sniff(bytes: &[u8]) -> bool {
    if !craft_cfb::sniff(bytes) {
        return false;
    }
    match craft_cfb::CompoundFile::open(bytes) {
        Ok(cf) => cf.root().clsid == ids::ROOT || cf.find("Header-2").is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod tests;
