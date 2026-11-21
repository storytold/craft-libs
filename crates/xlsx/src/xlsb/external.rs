//! External link parts ([MS-XLSB] 2.1.7.25, `externalLinkN.bin`) into the [`ExternalLink`] an
//! `externalLinkN.xml` reads as, and what formulas need of them to spell `[1]Sheet!A1`.

use craft_opc::Rels;
use craft_sheet::{CellRef, MAX_COLS, MAX_ROWS, Value, push_col};

use super::formula::{ExternalInfo, ExternalNameInfo};
use super::{Budget, Bytes, Records, error, invalid};
use crate::external::{ExternalBook, ExternalLink, ExternalName, ExternalSheetData};
use crate::{Error, Limits, Warnings};

/// The content type of an external link part.
pub(crate) const EXTERNAL_LINK: &str = "application/vnd.ms-excel.externalLink";

/// Sheet names and names one external link holds at most ([MS-XLSB] 2.4.827: fewer than 65,535
/// sheets).
const MAX_ITEMS: usize = 65_535;

/// A sheet as `refersTo` writes it: always quoted, `'` doubled.
fn quoted(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

/// One end of a reference in an external name (`RgceLocSmall`'s column: 14 bits, then the
/// column-relative and row-relative flags).
fn cell(row: u16, col: u16) -> Result<String, Error> {
    let c = u32::from(col & 0x3FFF);
    if c >= MAX_COLS {
        return Err(invalid("external name outside the grid"));
    }
    let mut s = String::new();
    if col & 0x4000 == 0 {
        s.push('$');
    }
    push_col(&mut s, c);
    if col & 0x8000 == 0 {
        s.push('$');
    }
    s.push_str(&(u32::from(row) + 1).to_string());
    Ok(s)
}

/// `BrtSupNameFmla`'s value ([MS-XLSB] 2.4.818) as `refersTo`: an `ExtPtgRef3D` or
/// `ExtPtgArea3D` (`='Data'!$A$1:$A$3`, a span `='A:C'!$A$1`), or `#REF!` for the error forms, as
/// Excel writes them. `None` when the other workbook doesn't define the name (no value).
fn refers_to(val: &[u8], sheets: &[String]) -> Result<Option<String>, Error> {
    let mut b = Bytes::new(val);
    if val.is_empty() {
        return Ok(None);
    }
    let ptg = b.u8()?;
    if matches!(ptg, 0x1C | 0x3C | 0x3D) {
        return Ok(Some("#REF!".into()));
    }
    let (first, last) = (usize::from(b.u16()?), usize::from(b.u16()?));
    let sheet = |i: usize| sheets.get(i).ok_or_else(|| invalid("external name on a missing sheet"));
    let tabs = if first == last { quoted(sheet(first)?) } else { quoted(&format!("{}:{}", sheet(first)?, sheet(last)?)) };
    let area = match ptg {
        0x3A => {
            let (r, c) = (b.u16()?, b.u16()?);
            cell(r, c)?
        }
        0x3B => {
            let (r0, r1, c0, c1) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
            format!("{}:{}", cell(r0, c0)?, cell(r1, c1)?)
        }
        _ => return Err(invalid("unknown external name formula")),
    };
    b.finish()?;
    Ok(Some(format!("={tabs}!{area}")))
}

/// An external link part: the link, and what formulas need of it. `rels` are the part's own
/// relationships (the other workbook's path).
pub(crate) fn external_link(
    bytes: &[u8],
    rels: &Rels,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
) -> Result<(ExternalLink, ExternalInfo), Error> {
    let mut records = Records::new(bytes, limits, budget);
    let Some((360, payload)) = records.next(warnings)? else { return Err(invalid("missing external link begin record")) };
    // BrtBeginSupBook: the link's type, then its strings.
    let mut b = Bytes::new(payload);
    let sbt = b.u16()?;
    let first = b.string(32_767, true)?;
    b.string(32_767, true)?;
    b.finish()?;
    let dde = sbt != 0;
    if sbt > 2 {
        return Err(invalid("unknown external link type"));
    }
    let mut book = ExternalBook {
        target: if dde { None } else { first.as_deref().and_then(|id| rels.by_id(id)).map(|r| r.target_raw.clone()) },
        ..ExternalBook::default()
    };
    let mut info = ExternalInfo { dde, ..ExternalInfo::default() };
    let mut name: Option<ExternalName> = None;
    let mut table: Option<ExternalSheetData> = None;
    let mut row: Option<u32> = None;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            // BrtSupTabs.
            359 => {
                let count = b.u32()? as usize;
                if count > MAX_ITEMS || !book.sheet_names.is_empty() {
                    return Err(invalid("invalid external sheet list"));
                }
                for _ in 0..count {
                    let s = b.text(31)?;
                    budget.text(s.len(), limits)?;
                    book.sheet_names.push(s);
                }
                b.finish()?;
            }
            // BrtSupNameStart.
            577 => {
                if name.is_some() || info.names.len() >= MAX_ITEMS {
                    return Err(invalid("invalid external name"));
                }
                let text = b.text(255)?;
                b.finish()?;
                budget.text(text.len(), limits)?;
                name = Some(ExternalName { name: text, ..ExternalName::default() });
            }
            // BrtSupNameFmla.
            585 => {
                let n = name.as_mut().ok_or_else(|| invalid("external name formula outside a name"))?;
                let cb = b.u32()? as usize;
                let val = b.take(cb)?;
                b.finish()?;
                n.refers_to = refers_to(val, &book.sheet_names)?;
            }
            // BrtSupNameBits: an external name's ExternalNameProperties (fBuiltIn, then iSheet,
            // 1-based); a DDE or OLE item's properties.
            586 => {
                let n = name.as_mut().ok_or_else(|| invalid("external name properties outside a name"))?;
                b.u16()?;
                let sheet = b.u32()?;
                b.u8()?;
                b.finish()?;
                if !dde && sheet > 0 {
                    if sheet as usize > book.sheet_names.len() {
                        return Err(invalid("external name scoped to a missing sheet"));
                    }
                    n.sheet_id = Some(sheet - 1);
                }
            }
            // BrtSupNameEnd.
            587 => {
                let n = name.take().ok_or_else(|| invalid("external name end without a name"))?;
                info.names.push(ExternalNameInfo { name: n.name.clone(), sheet: n.sheet_id.map(|s| s as usize) });
                if !dde {
                    book.defined_names.push(n);
                }
            }
            // BrtExternTableStart: a sheet's cached cells.
            363 => {
                if table.is_some() {
                    return Err(invalid("nested external cell cache"));
                }
                let sheet_id = b.u32()?;
                let flags = b.u8()?;
                b.finish()?;
                if sheet_id as usize >= book.sheet_names.len() {
                    return Err(invalid("external cell cache for a missing sheet"));
                }
                table = Some(ExternalSheetData { sheet_id, refresh_error: flags & 1 != 0, cells: vec![] });
                row = None;
            }
            // BrtExternRowHdr.
            366 => {
                let r = b.u32()?;
                b.finish()?;
                if r >= MAX_ROWS {
                    return Err(invalid("external cell outside the grid"));
                }
                row = Some(r);
            }
            // External cells: blank, Boolean, error, number, string.
            367..=371 => {
                let t = table.as_mut().ok_or_else(|| invalid("external cell outside its cache"))?;
                let r = row.ok_or_else(|| invalid("external cell outside a row"))?;
                let c = b.u32()?;
                if c >= MAX_COLS {
                    return Err(invalid("external cell outside the grid"));
                }
                let value = match id {
                    367 => Value::Empty,
                    369 => Value::Bool(b.u8()? != 0),
                    370 => Value::Error(error(b.u8()?)?),
                    368 => Value::number(b.f64()?),
                    _ => {
                        let s = b.text(32_767)?;
                        budget.text(s.len(), limits)?;
                        Value::Text(s.into())
                    }
                };
                b.finish()?;
                budget.cell(limits)?;
                t.cells.push((CellRef::new(r, c), value));
            }
            // BrtExternTableEnd.
            364 => {
                b.finish()?;
                book.sheet_data.push(table.take().ok_or_else(|| invalid("external cell cache end without a cache"))?);
            }
            // BrtEndSupBook.
            588 => {
                if name.is_some() || table.is_some() {
                    return Err(invalid("incomplete external link"));
                }
                records.end(payload, warnings)?;
                info.sheets = book.sheet_names.clone();
                let link = if dde { ExternalLink::Other } else { ExternalLink::Book(book) };
                return Ok((link, info));
            }
            _ => {}
        }
    }
    Err(invalid("missing external link end record"))
}
