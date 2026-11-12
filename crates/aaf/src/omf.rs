//! OMF Interchange 2.0 for audio post: written, and the same structures read.
//!
//! Written from the public *OMF Interchange Specification Version 2.0* (Avid Technology, 1995/97)
//! and Apple's *Bento Specification* revision 1.0d5 for the container ([`bento`]).
//!
//! The file holds a composition mob (`CMOB`) with a timecode slot and one sound slot per audio
//! track (one per channel when broken out to mono): sequences (`SEQU`) of fillers (`FILL`), source
//! clips (`SCLP`) and transitions (`TRAN` with a `SimpleMonoAudioDissolve` effect); clip volume is a
//! `MonoAudioGain` effect (`EFFE`) whose slot −1 is the clip and slot 1 the amplitude
//! (`CVAL` constant or `VVAL` keyframes). Every media item has a master mob (`MMOB`), a file source
//! mob (`SMOB` with a `WAVD` / `AIFD` descriptor: embedded essence in a `WAVE` media data object,
//! or a locator to a separate file) and a tape source mob carrying its timecode. Times are in
//! samples (sound slots use the sample rate as edit rate) so edits are sample-exact.
//!
//! Only sound is written: picture tracks and markers of the model are left out, and a nested
//! composition is not written as one (readers of OMF do not follow compositions). A caller with
//! nested sequences mixes their sound and passes it as media like any other; Premiere Pro's OMF
//! export does the same.
//!
//! Private properties. `FilmCraft:SCLP:ClipName` (a clip's name) and `FilmCraft:EFFE:EffectID` (a
//! transition's name) are not in the specification; they were first written by FilmCraft and keep
//! their names so that existing files read back.

pub(crate) mod bento;
mod read;
mod write;

use crate::model::Document;
use crate::{Identity, Report, Result};

/// Write the sound of `doc`'s first composition as an OMF 2.0 file.
pub fn write(doc: &Document, identity: &Identity<'_>, report: &mut Report) -> Result<Vec<u8>> {
    write::write(doc, identity, report)
}

/// Read an OMF 2.0 file (as written by [`write`] and by applications using the same structures).
pub fn read(bytes: &[u8], report: &mut Report) -> Result<Document> {
    read::read(bytes, report)
}

/// Whether `bytes` end with a Bento container label (OMF files do).
pub fn sniff(bytes: &[u8]) -> bool {
    bento::sniff(bytes)
}

#[cfg(test)]
mod tests;
