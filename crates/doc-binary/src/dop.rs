//! The document properties a `.doc` keeps in its Dop ([MS-DOC] §2.7) that the model has a place
//! for: the footnote and endnote options, and different odd and even pages. Word 97 and Word
//! 6.0/95 files share the start of the Dop (`DopBase`): the offsets below were seen in files of
//! both, each read the same way by Word.

use craft_wordmodel::Settings;
use craft_wordmodel::section::{MAX_NOTE_START, NotePos, NoteProps, NoteRestart};

/// Byte 0: bit 0 fFacingPages, bits 5-6 fpc (where footnotes go).
const OFF_FLAGS: usize = 0;
/// u16: rncFtn (bits 0-1), nFtn (bits 2-15).
const OFF_FTN: usize = 2;
/// u16: rncEdn (bits 0-1), nEdn (bits 2-15).
const OFF_EDN: usize = 0x34;
/// u16: epc (bits 0-1, where endnotes go), then a Word 6.0/95 file's nfcFtnRef (bits 2-5) and
/// nfcEdnRef (bits 6-9).
const OFF_EPC: usize = 0x36;
/// `Dop97.nfcFtnRef` and `nfcEdnRef` (u16 MSONFCs): a Word 97 file's note number formats, where
/// the 4-bit ones above are 0 (Word 2016 writes them both ways: lower roman and upper letter
/// here, 0 at 0x36).
const OFF_NFC97: usize = 0x1EC;

/// Read the note options and facing pages from `dop` into `settings`. Values Word takes as its
/// defaults stay unset, as the `.docx` reader leaves them when `settings.xml` doesn't name them.
pub(crate) fn read(dop: &[u8], word6: bool, settings: &mut Settings) {
    let u16_at = |at: usize| dop.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let Some(&flags) = dop.get(OFF_FLAGS) else { return };
    settings.even_odd_headers = flags & 1 != 0;
    // fpc: 1 the bottom of the page (Word's default), 2 beneath the text.
    if (flags >> 5) & 3 == 2 {
        settings.footnote_pr.pos = Some(NotePos::BeneathText);
    }
    if let Some(v) = u16_at(OFF_FTN) {
        numbering(&mut settings.footnote_pr, v);
    }
    if let Some(v) = u16_at(OFF_EDN) {
        numbering(&mut settings.endnote_pr, v);
    }
    let formats = if word6 {
        u16_at(OFF_EPC).map(|v| (((v >> 2) & 0xF) as u8, ((v >> 6) & 0xF) as u8))
    } else {
        u16_at(OFF_NFC97).zip(u16_at(OFF_NFC97 + 2)).map(|(f, e)| (f.min(0xFF) as u8, e.min(0xFF) as u8))
    };
    if let Some((ftn, edn)) = formats {
        settings.footnote_format = crate::list::num_format(ftn);
        settings.endnote_format = crate::list::num_format(edn);
    }
    // epc: 0 the end of each section, 3 the end of the document (Word's default).
    if u16_at(OFF_EPC).is_some_and(|v| v & 3 == 0) {
        settings.endnote_pr.pos = Some(NotePos::SectEnd);
    }
}

/// A restart rule (bits 0-1: 0 continuous, 1 each section, 2 each page) and a first number (bits
/// 2-15), as the Dop and the section sprms store them.
fn numbering(pr: &mut NoteProps, v: u16) {
    pr.num_restart = restart(v & 3);
    let start = u32::from(v >> 2);
    if start > 1 {
        pr.num_start = Some(start.min(MAX_NOTE_START));
    }
}

/// A restart rule (`Rnc`): continuous is Word's default and stays unset.
pub(crate) fn restart(rnc: u16) -> Option<NoteRestart> {
    match rnc {
        1 => Some(NoteRestart::EachSect),
        2 => Some(NoteRestart::EachPage),
        _ => None,
    }
}
