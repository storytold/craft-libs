//! BIFF5 differences, from the published Excel file format specification (OpenOffice.org,
//! revision 1.42, §§2.5.2, 3.3.3, 3.4, 5.45, 5.49, 5.115):
//! https://www.openoffice.org/sc/excelfileformat.pdf . No implementation source was consulted.
//! The container, record loop and BIFF8 expression decoder remain shared. Tokens whose older
//! representation is not translated are refused as a whole: the caller keeps the saved result.

use crate::formula::{Links, Mode, Unsupported, decode};
use crate::records::Cursor;
use crate::{CellRef, Error};

pub(crate) fn check_codepage(cp: u16) -> Result<(), Error> {
    if craft_codepage::is_byte_code_page(cp) { Ok(()) } else { Err(Error::Invalid(format!("BIFF5 code page {cp} is not supported"))) }
}

fn text(c: &mut Cursor<'_, '_>, n: usize, cp: u16) -> Result<String, Error> {
    let mut out = String::with_capacity(n);
    for _ in 0..n {
        out.push(craft_codepage::code_page_byte(cp, c.u8()?));
    }
    Ok(out)
}

pub(crate) fn short_string(c: &mut Cursor<'_, '_>, cp: u16) -> Result<String, Error> {
    let n = usize::from(c.u8()?);
    text(c, n, cp)
}

pub(crate) fn string(c: &mut Cursor<'_, '_>, cp: u16) -> Result<String, Error> {
    let n = usize::from(c.u16()?);
    text(c, n, cp)
}

fn address(row: u16, col: u8, base: CellRef, relative: bool) -> (u16, u16) {
    let mut r = row & 0x3FFF;
    let mut c = u16::from(col);
    if relative {
        if row & 0x8000 != 0 {
            r = ((u32::from(r) + base.row) & 0x3FFF) as u16;
        }
        if row & 0x4000 != 0 {
            c = ((u32::from(c) + base.col) & 0xFF) as u16;
        }
    }
    (r, c | (row & 0xC000))
}

/// Translate the common BIFF5 tokens into the BIFF8 representation before decoding them.
/// Shared references are resolved in the 14-bit grid before asking the cell-mode decoder.
/// Array constants, names and 3-D links deliberately keep their cached results for now.
pub(crate) fn formula(bytes: &[u8], base: CellRef, cp: u16, links: &Links) -> Result<crate::expr::Expr, Unsupported> {
    if bytes.len() > 16 * 1024 {
        return Err(Unsupported("formula too long"));
    }
    let parts = [bytes];
    let mut c = Cursor::new(&parts);
    let mut out = Vec::new();
    let read = (|| -> Result<(), Error> {
        while c.remaining() > 0 {
            let ptg = c.u8()?;
            if ptg >= 0x80 {
                return Err(Error::Invalid("invalid formula token".into()));
            }
            let id = if ptg & 0x60 != 0 { 0x20 | (ptg & 0x1F) } else { ptg };
            // An unsupported token must stop translation: its payload is not token data.
            let n = match id {
                0x03..=0x16 => 0,
                0x1C | 0x1D => 1,
                0x1E | 0x21 | 0x29 => 2,
                0x22 => 3,
                0x1F => 8,
                0x27 | 0x28 => 6,
                0x17 | 0x19 | 0x24 | 0x25 | 0x2A..=0x2D => 0,
                _ => return Err(Error::Invalid("unsupported BIFF5 formula token".into())),
            };
            out.push(if id == 0x2C || id == 0x2D { ptg - 8 } else { ptg });
            match id {
                0x17 => {
                    let s = short_string(&mut c, cp)?;
                    let units: Vec<u16> = s.encode_utf16().collect();
                    out.push(units.len() as u8);
                    out.push(1);
                    for u in units {
                        out.extend(u.to_le_bytes());
                    }
                }
                0x19 => {
                    let kind = c.u8()?;
                    let n = c.u16()?;
                    out.push(kind);
                    out.extend(n.to_le_bytes());
                    if kind & 4 != 0 {
                        for _ in 0..=n {
                            out.extend(c.u16()?.to_le_bytes());
                        }
                    }
                }
                0x24 | 0x2C => {
                    let (r, col) = address(c.u16()?, c.u8()?, base, id == 0x2C);
                    out.extend(r.to_le_bytes());
                    out.extend(col.to_le_bytes());
                }
                0x25 | 0x2D => {
                    let (r0, r1) = (c.u16()?, c.u16()?);
                    let (c0, c1) = (c.u8()?, c.u8()?);
                    let (r0, c0) = address(r0, c0, base, id == 0x2D);
                    let (r1, c1) = address(r1, c1, base, id == 0x2D);
                    let r1 = if r0 == 0 && r1 == 0x3FFF { 0xFFFF } else { r1 };
                    for v in [r0, r1, c0, c1] {
                        out.extend(v.to_le_bytes());
                    }
                }
                0x2A | 0x2B => {
                    c.skip(if id == 0x2A { 3 } else { 6 })?;
                    out.extend(vec![0; if id == 0x2A { 4 } else { 8 }]);
                }
                _ => {
                    for _ in 0..n {
                        out.push(c.u8()?);
                    }
                }
            }
        }
        Ok(())
    })();
    read.map_err(|_| Unsupported("BIFF5 formula tokens that are unsupported or damaged"))?;
    decode(&out, &[], base, Mode::Cell, links)
}
