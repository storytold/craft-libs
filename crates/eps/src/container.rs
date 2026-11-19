//! The binary header of an EPS file with a preview ("DOS EPS binary file header", EPSF 3.0
//! section 5.2): 30 bytes, little-endian.
//!
//! | bytes | |
//! |---|---|
//! | 0–3 | `C5 D0 D3 C6` |
//! | 4–7, 8–11 | offset and length of the PostScript |
//! | 12–15, 16–19 | offset and length of the Windows metafile preview (0 for none) |
//! | 20–23, 24–27 | offset and length of the TIFF preview (0 for none) |
//! | 28–29 | checksum of bytes 0–27 (XOR of the 16-bit words), or `FFFF` for none |

use crate::Error;

/// The first bytes of an EPS file with a binary header.
pub const DOS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];
/// The binary header's size.
pub const DOS_HEADER: usize = 30;

/// The parts of an EPS file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sections<'a> {
    /// The PostScript program: the whole file when it has no binary header.
    pub postscript: &'a [u8],
    /// The TIFF preview.
    pub tiff: Option<&'a [u8]>,
    /// The Windows metafile preview.
    pub metafile: Option<&'a [u8]>,
}

/// The sections of an EPS file: its PostScript and previews behind a binary header, or the whole
/// file as PostScript. A header that points outside the file is an error, whichever section it
/// is: such a file is cut short or not an EPS.
pub fn sections(bytes: &[u8]) -> Result<Sections<'_>, Error> {
    if !bytes.starts_with(&DOS_MAGIC) {
        return Ok(Sections { postscript: bytes, tiff: None, metafile: None });
    }
    let word = |at: usize| -> Result<usize, Error> {
        let b = bytes.get(at..at.saturating_add(4)).and_then(|b| <[u8; 4]>::try_from(b).ok()).ok_or(Error::OutsideFile)?;
        usize::try_from(u32::from_le_bytes(b)).map_err(|_| Error::OutsideFile)
    };
    let part = |at: usize| -> Result<&[u8], Error> {
        let (start, len) = (word(at)?, word(at + 4)?);
        start.checked_add(len).and_then(|end| bytes.get(start..end)).ok_or(Error::OutsideFile)
    };
    // A preview with no offset or no length is absent.
    let preview = |at: usize| -> Result<Option<&[u8]>, Error> { if word(at)? == 0 || word(at + 4)? == 0 { Ok(None) } else { part(at).map(Some) } };
    Ok(Sections { postscript: part(4)?, metafile: preview(12)?, tiff: preview(20)? })
}

/// `postscript` behind a binary header, followed by its previews. With neither preview the
/// PostScript is returned as it is: a header would only hide it from readers of plain EPS.
pub fn wrap(postscript: &[u8], tiff: Option<&[u8]>, metafile: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    let (tiff, metafile) = (tiff.filter(|t| !t.is_empty()), metafile.filter(|m| !m.is_empty()));
    if tiff.is_none() && metafile.is_none() {
        return Ok(postscript.to_vec());
    }
    let size = |part: Option<&[u8]>| u32::try_from(part.map_or(0, <[u8]>::len)).map_err(|_| Error::TooLarge);
    let ps_len = u32::try_from(postscript.len()).map_err(|_| Error::TooLarge)?;
    let (metafile_len, tiff_len) = (size(metafile)?, size(tiff)?);
    let metafile_at = (DOS_HEADER as u32).checked_add(ps_len).ok_or(Error::TooLarge)?;
    let tiff_at = metafile_at.checked_add(metafile_len).ok_or(Error::TooLarge)?;
    let total = tiff_at.checked_add(tiff_len).ok_or(Error::TooLarge)?;
    let mut out = Vec::with_capacity(usize::try_from(total).map_err(|_| Error::TooLarge)?);
    out.extend(DOS_MAGIC);
    out.extend((DOS_HEADER as u32).to_le_bytes());
    out.extend(ps_len.to_le_bytes());
    for (at, len) in [(metafile_at, metafile_len), (tiff_at, tiff_len)] {
        out.extend((if len == 0 { 0 } else { at }).to_le_bytes());
        out.extend(len.to_le_bytes());
    }
    // No checksum.
    out.extend(0xFFFFu16.to_le_bytes());
    out.extend(postscript);
    out.extend(metafile.unwrap_or_default());
    out.extend(tiff.unwrap_or_default());
    Ok(out)
}
