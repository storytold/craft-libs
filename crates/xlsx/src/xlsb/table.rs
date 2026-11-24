//! Table parts ([MS-XLSB] 2.1.7.51, `tableN.bin`) into the [`Table`] a `.xlsx` table part reads
//! as. The table's AutoFilter and sort state are held as `.xlsx` elements, so they are built here
//! as Excel writes them. A filter column the reader can't spell is left out with a warning, as
//! is a sort state; the table keeps the rest.

use craft_sheet::{CellRef, MAX_COLS, MAX_ROWS, Range};
use craft_xml::El;

use super::formula::{self, ListInfo, Mode};
use super::{Budget, Bytes, Links, Records, expr, invalid};
use crate::features::{Table, TableColumn, TableStyleInfo};
use crate::{Error, Limits, Warnings};

/// The content type of a table part.
pub(crate) const TABLE: &str = "application/vnd.ms-excel.table";

/// `RfX` / `UncheckedRfX` ([MS-XLSB] 2.5.118, 2.5.154): first and last row, first and last column.
fn rfx(b: &mut Bytes<'_>) -> Result<Range, Error> {
    let (r0, r1, c0, c1) = (b.u32()?, b.u32()?, b.u32()?, b.u32()?);
    if r0 > r1 || c0 > c1 || r1 >= MAX_ROWS || c1 >= MAX_COLS {
        return Err(invalid("invalid table range"));
    }
    Ok(Range::new(CellRef::new(r0, c0), CellRef::new(r1, c1)))
}

/// A `DXFId`, `None` for 0xFFFFFFFF (no differential format).
fn dxf(id: u32) -> Option<u32> {
    (id != u32::MAX).then_some(id)
}

/// An `XLNullableWideString` of at most `max` characters, with the empty string as none: Excel
/// writes an empty comment where `.xlsx` has no attribute.
fn text(b: &mut Bytes<'_>, max: u32) -> Result<Option<String>, Error> {
    Ok(b.string(max, true)?.filter(|s| !s.is_empty()))
}

/// What a table part holds of formulas, for the reader's index of tables: its id, display name
/// and column names.
fn list_info(bytes: &[u8], limits: &Limits, budget: &Budget) -> Result<(u32, ListInfo), Error> {
    let t = table(bytes, None, limits, budget, &mut Warnings::default())?;
    let id = t.id.ok_or_else(|| invalid("table without an id"))?;
    let name = t.display_name.or(t.name).ok_or_else(|| invalid("table without a name"))?;
    // A column without a name can't be spelled in a structured reference.
    let columns = t
        .columns
        .into_iter()
        .map(|c| c.name.filter(|n| !n.is_empty()))
        .collect::<Option<_>>()
        .ok_or_else(|| invalid("table column without a name"))?;
    Ok((id, ListInfo { name, columns }))
}

/// Adds a table part to `links`, so formulas can name it. Two tables with one id name neither.
pub(crate) fn index_table(links: &mut Links, bytes: &[u8], limits: &Limits, budget: &Budget) -> Result<(), Error> {
    let (id, info) = list_info(bytes, limits, budget)?;
    links.tables.entry(id).and_modify(|t| *t = None).or_insert(Some(info));
    Ok(())
}

/// A table part. `links` decodes its calculated-column and totals-row formulas; without them
/// (or when one can't be decoded) a formula is left out, with a warning.
pub(crate) fn table(bytes: &[u8], links: Option<&Links>, limits: &Limits, budget: &Budget, warnings: &mut Warnings) -> Result<Table, Error> {
    let mut records = Records::new(bytes, limits, budget);
    let mut table = match records.next(warnings)? {
        Some((343, payload)) => begin_list(payload)?,
        _ => return Err(invalid("missing table begin record")),
    };
    let mut cols_open = false;
    let mut cols_seen = false;
    let mut column: Option<TableColumn> = None;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            161 if !cols_seen && table.auto_filter.is_none() => {
                let range = rfx(&mut b)?;
                b.finish()?;
                table.auto_filter = Some(auto_filter(range, &mut records, warnings)?);
            }
            530 if !cols_seen && table.sort_state.is_none() => table.sort_state = sort_state(payload, &mut records, warnings)?,
            345 if !cols_seen => {
                b.u32()?;
                b.finish()?;
                cols_open = true;
                cols_seen = true;
            }
            346 if cols_open && column.is_none() => {
                b.finish()?;
                cols_open = false;
            }
            347 if cols_open && column.is_none() => {
                if table.columns.len() >= MAX_COLS as usize {
                    return Err(invalid("too many table columns"));
                }
                column = Some(begin_list_col(payload)?);
            }
            351 | 352 if column.is_some() => {
                let f = list_formula(payload, links, limits, budget, warnings)?;
                if let Some(c) = column.as_mut() {
                    *(if id == 351 { &mut c.calculated_formula } else { &mut c.totals_row_formula }) = f;
                }
            }
            348 => {
                b.finish()?;
                table.columns.push(column.take().ok_or_else(|| invalid("table column end without a column"))?);
            }
            349 => {
                let c = column.as_mut().ok_or_else(|| invalid("XML column properties outside a table column"))?;
                c.other.push(xml_column(payload)?);
            }
            513 if !cols_open && cols_seen => {
                let flags = b.u16()?;
                let name = b.string(255, true)?;
                b.finish()?;
                let bit = |n: u16| Some(flags & (1 << n) != 0);
                table.style = Some(TableStyleInfo {
                    name,
                    show_first_column: bit(0),
                    show_last_column: bit(1),
                    show_row_stripes: bit(2),
                    show_column_stripes: bit(3),
                    ..TableStyleInfo::default()
                });
            }
            344 => {
                if cols_open || !cols_seen || column.is_some() {
                    return Err(invalid("incomplete table"));
                }
                records.end(payload, warnings)?;
                return Ok(table);
            }
            // The end of an XML map's column; anything else out of place is damage.
            350 => b.finish()?,
            _ => return Err(invalid(format!("unexpected record {id} in a table"))),
        }
    }
    Err(invalid("missing table end record"))
}

/// `BrtBeginList` ([MS-XLSB] 2.4.100). `.xlsx` leaves out what is the default: one header row,
/// no totals row, a totals row that has been shown, a worksheet table.
fn begin_list(payload: &[u8]) -> Result<Table, Error> {
    let mut b = Bytes::new(payload);
    let range = rfx(&mut b)?;
    let table_type = match b.u32()? {
        0 => None,
        2 => Some("xml".to_string()),
        3 => Some("queryTable".to_string()),
        _ => return Err(invalid("invalid table type")),
    };
    let id = b.u32()?;
    if id == 0 || id == u32::MAX {
        return Err(invalid("invalid table id"));
    }
    let header = b.u32()?;
    let totals = b.u32()?;
    let flags = b.u32()?;
    // fSingleCell: a single-cell table belongs in its own part type, not here.
    if flags & 2 != 0 {
        return Err(invalid("single-cell table in a table part"));
    }
    let dxfs: Vec<Option<u32>> = (0..6).map(|_| b.u32().map(dxf)).collect::<Result<_, _>>()?;
    let connection = b.u32()?;
    let name = b.string(255, true)?;
    let display_name = b.string(255, true)?;
    let comment = text(&mut b, 255)?;
    let header_row_cell_style = b.string(255, true)?;
    let data_cell_style = b.string(255, true)?;
    let totals_row_cell_style = b.string(255, true)?;
    b.finish()?;
    let set = |bit: u32| (flags & bit != 0).then_some(true);
    Ok(Table {
        id: Some(id),
        name,
        display_name,
        comment,
        table_type,
        header_row_count: (header != 1).then_some(header),
        insert_row: set(4),
        insert_row_shift: set(8),
        totals_row_count: (totals != 0).then_some(totals),
        totals_row_shown: (flags & 1 == 0).then_some(false),
        published: set(16),
        header_row_dxf_id: dxfs[0],
        data_dxf_id: dxfs[1],
        totals_row_dxf_id: dxfs[2],
        table_border_dxf_id: dxfs[3],
        header_row_border_dxf_id: dxfs[4],
        totals_row_border_dxf_id: dxfs[5],
        header_row_cell_style,
        data_cell_style,
        totals_row_cell_style,
        connection_id: (connection != 0).then_some(connection),
        range,
        ..Table::default()
    })
}

/// `BrtBeginListCol` ([MS-XLSB] 2.4.101): `stName` is `.xlsx`'s `uniqueName`, `stCaption` its
/// `name`, and the insert row's format and style are the data's.
fn begin_list_col(payload: &[u8]) -> Result<TableColumn, Error> {
    let mut b = Bytes::new(payload);
    let id = b.u32()?;
    let function = match b.u32()? {
        0 => None,
        1 => Some("average"),
        2 => Some("count"),
        3 => Some("countNums"),
        4 => Some("max"),
        5 => Some("min"),
        6 => Some("sum"),
        7 => Some("stdDev"),
        8 => Some("var"),
        9 => Some("custom"),
        _ => return Err(invalid("invalid totals row function")),
    };
    let header_row_dxf_id = dxf(b.u32()?);
    let data_dxf_id = dxf(b.u32()?);
    let totals_row_dxf_id = dxf(b.u32()?);
    let query = b.u32()?;
    let unique_name = b.string(255, true)?;
    let name = b.string(255, true)?;
    let totals_row_label = b.string(8189, true)?;
    let header_row_cell_style = b.string(255, true)?;
    let data_cell_style = b.string(255, true)?;
    let totals_row_cell_style = b.string(255, true)?;
    b.finish()?;
    Ok(TableColumn {
        id: Some(id),
        unique_name,
        name,
        totals_row_function: function.map(str::to_string),
        totals_row_label,
        query_table_field_id: (query != 0).then_some(query),
        header_row_dxf_id,
        data_dxf_id,
        totals_row_dxf_id,
        header_row_cell_style,
        data_cell_style,
        totals_row_cell_style,
        ..TableColumn::default()
    })
}

/// `XmlDataType` ([MS-XLSB] 2.5.170) from 1, as `.xlsx` names the XML Schema types.
const XML_TYPES: [&str; 45] = [
    "string",
    "normalizedString",
    "token",
    "byte",
    "unsignedByte",
    "base64Binary",
    "hexBinary",
    "integer",
    "positiveInteger",
    "negativeInteger",
    "nonPositiveInteger",
    "nonNegativeInteger",
    "int",
    "unsignedInt",
    "long",
    "unsignedLong",
    "short",
    "unsignedShort",
    "decimal",
    "float",
    "double",
    "boolean",
    "time",
    "dateTime",
    "duration",
    "date",
    "gMonth",
    "gYear",
    "gYearMonth",
    "gDay",
    "gMonthDay",
    "Name",
    "QName",
    "NCName",
    "anyURI",
    "language",
    "ID",
    "IDREF",
    "IDREFS",
    "ENTITY",
    "ENTITIES",
    "NOTATION",
    "NMTOKEN",
    "NMTOKENS",
    "anyType",
];

/// `BrtBeginListXmlCPr` ([MS-XLSB] 2.4.104): a column's place in an XML map, as `xmlColumnPr`.
/// `fCanBeSingle` (one node for every row) is `.xlsx`'s `denormalized`.
fn xml_column(payload: &[u8]) -> Result<El, Error> {
    let mut b = Bytes::new(payload);
    let map = b.u32()?;
    let flags = b.u32()?;
    let kind = b.u32()?;
    let xpath = b.text(31_999)?;
    b.finish()?;
    let kind = kind.checked_sub(1).and_then(|k| XML_TYPES.get(k as usize)).ok_or_else(|| invalid("invalid XML data type"))?;
    let mut el = El::new("xmlColumnPr").with_attr("mapId", map).with_attr("xpath", xpath);
    if flags & 2 != 0 {
        el.set_attr("denormalized", 1);
    }
    el.set_attr("xmlDataType", kind);
    Ok(el)
}

/// `BrtListCCFmla` / `BrtListTrFmla` ([MS-XLSB] 2.4.706, 2.4.708): a flag byte (`fArray`, bit 1)
/// and a `ListParsedFormula`. Its references hold coordinates, as a cell formula's do.
fn list_formula(
    payload: &[u8],
    links: Option<&Links>,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
) -> Result<Option<(String, bool)>, Error> {
    let mut b = Bytes::new(payload);
    let array = b.u8()? & 2 != 0;
    let cce = b.u32()?;
    if !(1..=16384).contains(&cce) {
        return Err(invalid("invalid formula token length"));
    }
    let rgce = b.take(cce as usize)?;
    let cb = b.u32()?;
    let rgcb = b.take(cb as usize)?;
    b.finish()?;
    let Some(links) = links else { return Ok(None) };
    let text = match formula::decode(rgce, rgcb, links, Mode::Cell) {
        Ok(e) => expr::print(&e),
        Err(formula::Unsupported(why)) => {
            warnings.warn(format!("XLSB: some table column formulas were left out ({why})"));
            return Ok(None);
        }
    };
    if !budget.try_formula_text(text.len(), limits) {
        warnings.warn("XLSB: some table column formulas were left out (more formula text than the limits allow)");
        return Ok(None);
    }
    Ok(Some((text, array)))
}

/// A number in a filter, as Excel writes it in `.xlsx`, which differs from a formula's: 17
/// significant digits (`1234.5678901234501`, `1.0000000000000001E-5`), written out from 0.01 up to
/// 10^16 unless that takes more than 17 decimals, and otherwise with an exponent of no fixed
/// width (`1E-4`, `1E+20`). Excel writes a few values with fewer digits (`0.1`, not
/// `0.10000000000000001`); both read back as the same number.
fn filter_number(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".into();
    }
    let sci = format!("{:.16e}", n.abs());
    let Some((mantissa, exp)) = sci.split_once('e') else { return sci };
    let e: i32 = exp.parse().unwrap_or(0);
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let decimals = i32::try_from(digits.len()).unwrap_or(i32::MAX).saturating_sub(1).saturating_sub(e);
    let mut s = String::from(if n < 0.0 { "-" } else { "" });
    if !(-2..=15).contains(&e) || decimals > 17 {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push_str(&format!("E{}{}", if e < 0 { '-' } else { '+' }, e.unsigned_abs()));
    } else if e >= 0 {
        let whole = e as usize + 1;
        if digits.len() <= whole {
            s.push_str(digits);
            s.push_str(&"0".repeat(whole - digits.len()));
        } else {
            s.push_str(&digits[..whole]);
            s.push('.');
            s.push_str(&digits[whole..]);
        }
    } else {
        s.push_str("0.");
        s.push_str(&"0".repeat((-e - 1) as usize));
        s.push_str(digits);
    }
    s
}

/// Warns that a table's filter column was left out, and why.
fn left_out(warnings: &mut Warnings, why: &str) {
    warnings.warn(format!("XLSB: some table filters were left out ({why})"));
}

/// The records after `BrtBeginAFilter` ([MS-XLSB] 2.4.8) up to `BrtEndAFilter`, as `autoFilter`.
fn auto_filter(range: Range, records: &mut Records<'_, '_>, warnings: &mut Warnings) -> Result<El, Error> {
    let mut el = El::new("autoFilter").with_attr("ref", range.a1());
    while let Some((id, payload)) = records.next(warnings)? {
        match id {
            163 => {
                if let Some(c) = filter_column(payload, records, warnings)? {
                    el.push(c);
                }
            }
            162 => {
                Bytes::new(payload).finish()?;
                return Ok(el);
            }
            _ => return Err(invalid(format!("unexpected record {id} in a table filter"))),
        }
    }
    Err(invalid("missing table filter end record"))
}

/// A `BrtBeginFilterColumn` ([MS-XLSB] 2.4.84) and its criteria, up to `BrtEndFilterColumn`, as
/// `filterColumn`. `None` when its criteria can't be spelled as `.xlsx` has them, or are kept
/// in an extension the reader skips (Excel's date-group filters).
fn filter_column(payload: &[u8], records: &mut Records<'_, '_>, warnings: &mut Warnings) -> Result<Option<El>, Error> {
    let mut b = Bytes::new(payload);
    let col = b.u32()?;
    let flags = b.u16()?;
    b.finish()?;
    let mut el = El::new("filterColumn").with_attr("colId", col);
    if flags & 1 != 0 {
        el.set_attr("hiddenButton", 1);
    }
    if flags & 2 != 0 {
        el.set_attr("showButton", 0);
    }
    let skipped = records.skipped;
    let mut keep = true;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            165 => {
                let blank = b.u32()?;
                b.u32()?;
                b.finish()?;
                let mut filters = El::new("filters");
                if blank == 1 {
                    filters.set_attr("blank", 1);
                }
                loop {
                    let (id, payload) = records.next(warnings)?.ok_or_else(|| invalid("missing filter list end record"))?;
                    let mut b = Bytes::new(payload);
                    match id {
                        167 => {
                            let val = b.text(255)?;
                            b.finish()?;
                            filters.push(El::new("filter").with_attr("val", val));
                        }
                        175 => filters.push(date_group_item(payload)?),
                        166 => {
                            b.finish()?;
                            break;
                        }
                        _ => return Err(invalid(format!("unexpected record {id} in a filter list"))),
                    }
                }
                el.push(filters);
            }
            172 => {
                let and = b.u32()?;
                b.finish()?;
                // Excel writes 1 where `.xlsx` has `and="1"` ([MS-XLSB]'s table has them the other way round).
                let mut custom = El::new("customFilters");
                if and == 1 {
                    custom.set_attr("and", 1);
                }
                loop {
                    let (id, payload) = records.next(warnings)?.ok_or_else(|| invalid("missing custom filter end record"))?;
                    match id {
                        174 => match custom_filter(payload)? {
                            Some(f) => custom.push(f),
                            None => {
                                left_out(warnings, "custom filters on a Boolean value");
                                keep = false;
                            }
                        },
                        173 => {
                            Bytes::new(payload).finish()?;
                            break;
                        }
                        _ => return Err(invalid(format!("unexpected record {id} in custom filters"))),
                    }
                }
                el.push(custom);
            }
            170 => {
                let flags = b.u8()?;
                let val = b.f64()?;
                let filter_val = b.f64()?;
                b.finish()?;
                let mut top = El::new("top10");
                if flags & 1 == 0 {
                    top.set_attr("top", 0);
                }
                if flags & 2 != 0 {
                    top.set_attr("percent", 1);
                }
                top.set_attr("val", filter_number(val));
                if flags & 4 != 0 {
                    top.set_attr("filterVal", filter_number(filter_val));
                }
                el.push(top);
            }
            171 => el.push(dynamic_filter(payload)?),
            168 => {
                let dxf_id = b.u32()?;
                let cell = b.u32()?;
                b.finish()?;
                let mut color = El::new("colorFilter").with_attr("dxfId", dxf_id);
                if cell == 0 {
                    color.set_attr("cellColor", 0);
                }
                el.push(color);
            }
            169 => {
                let set = b.u32()?;
                let icon = b.u32()?;
                b.finish()?;
                // KPINIL: no filter.
                if set != u32::MAX {
                    let name = ICON_SETS.get(set as usize).ok_or_else(|| invalid("invalid filter icon set"))?;
                    el.push(El::new("iconFilter").with_attr("iconSet", name).with_attr("iconId", icon));
                }
            }
            164 => {
                b.finish()?;
                if records.skipped != skipped {
                    left_out(warnings, "filters kept in extensions the reader skips, such as date groups");
                    return Ok(None);
                }
                return Ok(keep.then_some(el));
            }
            _ => return Err(invalid(format!("unexpected record {id} in a filter column"))),
        }
    }
    Err(invalid("missing filter column end record"))
}

/// `KPISets` ([MS-XLSB] 2.5.86), as `.xlsx` names them.
const ICON_SETS: [&str; 17] = [
    "3Arrows",
    "3ArrowsGray",
    "3Flags",
    "3TrafficLights1",
    "3TrafficLights2",
    "3Signs",
    "3Symbols",
    "3Symbols2",
    "4Arrows",
    "4ArrowsGray",
    "4RedToBlack",
    "4Rating",
    "4TrafficLights",
    "5Arrows",
    "5ArrowsGray",
    "5Rating",
    "5Quarters",
];

/// `BrtCustomFilter` ([MS-XLSB] 2.4.348) as `customFilter`; `None` for a Boolean value, whose
/// `.xlsx` spelling no Excel file has shown. A blank or non-blank test is `val=" "`, as Excel
/// writes it.
fn custom_filter(payload: &[u8]) -> Result<Option<El>, Error> {
    let mut b = Bytes::new(payload);
    let vts = b.u8()?;
    let operator = match b.u8()? {
        1 => Some("lessThan"),
        2 => None,
        3 => Some("lessThanOrEqual"),
        4 => Some("greaterThan"),
        5 => Some("notEqual"),
        6 => Some("greaterThanOrEqual"),
        _ => return Err(invalid("invalid custom filter operator")),
    };
    let union = b.take(8)?;
    let val = match vts {
        4 => {
            let n = f64::from_le_bytes(union.try_into().map_err(|_| invalid("truncated number"))?);
            if !n.is_finite() {
                return Err(invalid("nonfinite numeric value"));
            }
            filter_number(n)
        }
        6 => b.text(255)?,
        8 => return Ok(None),
        0x0C | 0x0E => " ".to_string(),
        _ => return Err(invalid("invalid custom filter type")),
    };
    b.finish()?;
    let mut el = El::new("customFilter");
    if let Some(op) = operator {
        el.set_attr("operator", op);
    }
    el.set_attr("val", val);
    Ok(Some(el))
}

/// `BrtDynamicFilter` ([MS-XLSB] 2.4.362) as `dynamicFilter`: the computed values only once the
/// filter has been applied, and the maximum only for date periods.
fn dynamic_filter(payload: &[u8]) -> Result<El, Error> {
    let mut b = Bytes::new(payload);
    let cft = b.u32()?;
    let applied = b.u8()? & 1 != 0;
    let val = b.f64()?;
    let max = b.f64()?;
    b.finish()?;
    const PERIODS: [&str; 16] = [
        "tomorrow",
        "today",
        "yesterday",
        "nextWeek",
        "thisWeek",
        "lastWeek",
        "nextMonth",
        "thisMonth",
        "lastMonth",
        "nextQuarter",
        "thisQuarter",
        "lastQuarter",
        "nextYear",
        "thisYear",
        "lastYear",
        "yearToDate",
    ];
    let kind = match cft {
        0 => "null".to_string(),
        1 => "aboveAverage".to_string(),
        2 => "belowAverage".to_string(),
        8..=0x17 => PERIODS[(cft - 8) as usize].to_string(),
        0x18..=0x1B => format!("Q{}", cft - 0x17),
        0x1C..=0x27 => format!("M{}", cft - 0x1B),
        _ => return Err(invalid("invalid dynamic filter type")),
    };
    let mut el = El::new("dynamicFilter").with_attr("type", kind);
    if applied && (1..=0x17).contains(&cft) {
        el.set_attr("val", filter_number(val));
        if cft >= 8 {
            el.set_attr("maxVal", filter_number(max));
        }
    }
    Ok(el)
}

/// `BrtAFilterDateGroupItem` ([MS-XLSB] 2.4.5) as `dateGroupItem`, with the parts of the date
/// its grouping compares.
fn date_group_item(payload: &[u8]) -> Result<El, Error> {
    let mut b = Bytes::new(payload);
    let year = b.u16()?;
    let month = b.u16()?;
    let day = b.u32()?;
    let (hour, minute, second) = (b.u16()?, b.u16()?, b.u16()?);
    b.take(6)?;
    let grouping = b.u32()?;
    b.finish()?;
    const GROUPS: [&str; 6] = ["year", "month", "day", "hour", "minute", "second"];
    let name = GROUPS.get(grouping as usize).ok_or_else(|| invalid("invalid date group"))?;
    let mut el = El::new("dateGroupItem").with_attr("year", year);
    let parts = [("month", u32::from(month)), ("day", day), ("hour", u32::from(hour)), ("minute", u32::from(minute)), ("second", u32::from(second))];
    for (name, value) in parts.iter().take(grouping as usize) {
        el.set_attr(name, value);
    }
    el.set_attr("dateTimeGrouping", name);
    Ok(el)
}

/// `BrtBeginSortState` ([MS-XLSB] 2.4.227) and its `BrtBeginSortCond`s, up to `BrtEndSortState`,
/// as `sortState`. `None`, with a warning, for a sort by another method than character order,
/// which `.xlsx` names (stroke or PinYin) and the record does not.
fn sort_state(payload: &[u8], records: &mut Records<'_, '_>, warnings: &mut Warnings) -> Result<Option<El>, Error> {
    let mut b = Bytes::new(payload);
    let flags = b.u16()?;
    let range = rfx(&mut b)?;
    b.finish()?;
    let mut el = El::new("sortState");
    if flags & 1 != 0 {
        el.set_attr("columnSort", 1);
    }
    if flags & 2 != 0 {
        el.set_attr("caseSensitive", 1);
    }
    el.set_attr("ref", range.a1());
    let keep = flags & 4 == 0;
    if !keep {
        warnings.warn("XLSB: some table sort states were left out (sorts by stroke or PinYin order)");
    }
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            532 => {
                let flags = b.u16()?;
                let range = rfx(&mut b)?;
                let (first, second) = (b.u32()?, b.u32()?);
                let list = b.string(32_767, true)?;
                b.finish()?;
                let mut cond = El::new("sortCondition");
                if flags & 1 != 0 {
                    cond.set_attr("descending", 1);
                }
                let on = (flags >> 1) & 0xF;
                match on {
                    0 => {}
                    1 => cond.set_attr("sortBy", "cellColor"),
                    2 => cond.set_attr("sortBy", "fontColor"),
                    3 => cond.set_attr("sortBy", "icon"),
                    _ => return Err(invalid("invalid sort condition")),
                }
                cond.set_attr("ref", range.a1());
                match on {
                    0 => {
                        if let Some(list) = list {
                            cond.set_attr("customList", list);
                        }
                    }
                    1 | 2 => cond.set_attr("dxfId", first),
                    _ => {
                        let name = ICON_SETS.get(first as usize).ok_or_else(|| invalid("invalid sort icon set"))?;
                        cond.set_attr("iconSet", name);
                        cond.set_attr("iconId", second);
                    }
                }
                el.push(cond);
            }
            533 => b.finish()?,
            531 => {
                b.finish()?;
                return Ok(keep.then_some(el));
            }
            _ => return Err(invalid(format!("unexpected record {id} in a sort state"))),
        }
    }
    Err(invalid("missing sort state end record"))
}

#[cfg(test)]
mod tests {
    use super::filter_number;

    #[test]
    fn filter_numbers_are_spelled_as_excel_writes_them() {
        // Excel's `.xlsx` text for each value.
        let cases = [
            (20.0, "20"),
            (23.25, "23.25"),
            (46340.0, "46340"),
            (-1.5, "-1.5"),
            (1e15, "1000000000000000"),
            (999_999_999_999_999.0, "999999999999999"),
            (1.234_567_890_123_45e17, "1.2345678901234499E+17"),
            (1e16, "1E+16"),
            (-1e16, "-1E+16"),
            (1e20, "1E+20"),
            (2.5e300, "2.5000000000000001E+300"),
            (1_234.567_890_123_45, "1234.5678901234501"),
            (12_345_678_901_234.5, "12345678901234.5"),
            (99_999_999_999_999.9, "99999999999999.906"),
            (1.1, "1.1000000000000001"),
            (0.57, "0.56999999999999995"),
            (0.01, "0.01"),
            (0.035, "3.5000000000000003E-2"),
            (0.001, "1E-3"),
            (1e-4, "1E-4"),
            (1e-5, "1.0000000000000001E-5"),
            (1.5e-10, "1.5E-10"),
            (1e-19, "9.9999999999999998E-20"),
        ];
        for (n, want) in cases {
            assert_eq!(filter_number(n), want, "{n}");
        }
    }
}
