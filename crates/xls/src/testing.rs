//! A minimal BIFF8 writer for tests and fixtures (not a general `.xls` writer).

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
/// Characters kept from one string: 4,000 UTF-16 units and the 3-byte header fit one record.
const MAX_STRING: usize = 4000;
/// The last zero-based row of the BIFF8 grid ([MS-XLS] 2.2.1).
const MAX_ROW: usize = 65_535;
/// The last zero-based column of the BIFF8 grid ([MS-XLS] 2.2.1).
const MAX_COLUMN: usize = 255;

fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(data.len().min(usize::from(u16::MAX)) as u16).to_le_bytes());
    out.extend_from_slice(data);
}

fn chars(out: &mut Vec<u8>, s: &str) {
    let wide = s.chars().any(|c| u32::from(c) > 0xFF);
    out.push(u8::from(wide));
    if wide {
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    } else {
        out.extend(s.chars().map(|c| u32::from(c) as u8));
    }
}

fn bof(dt: u16) -> Vec<u8> {
    let mut d = vec![0x00, 0x06];
    d.extend_from_slice(&dt.to_le_bytes());
    d.extend_from_slice(&[0; 12]);
    d
}

/// A compound file whose root holds one stream. A stream the file cannot hold, such as one with
/// an empty name or a name longer than 31 characters, returns the writer's error.
pub fn compound(stream: &str, data: &[u8]) -> Result<Vec<u8>, craft_cfb::Error> {
    craft_cfb::write_streams(craft_cfb::Version::V3, &[(stream, data)])
}

/// The XF index and value of a cell written as a Number record, or `None` for a cell written as
/// text: a cell `date:<serial>` takes the date XF when `<serial>` is a number, and any other cell
/// that is a number takes the general XF.
fn number(cell: &str) -> Option<(u16, f64)> {
    match cell.strip_prefix("date:") {
        Some(serial) => serial.parse().ok().map(|x| (1, x)),
        None => cell.parse().ok().map(|x| (0, x)),
    }
}

/// A minimal BIFF8 workbook: one worksheet per `(name, rows)`. Cells that read as numbers are
/// Number records, a cell `date:<serial>` is a Number with the date format `m/d/yyyy` (built-in
/// format 14), an empty cell is left out, and everything else, including a `date:` cell whose
/// serial is not a number, goes through the shared-string table (as UTF-16 when it isn't
/// Latin-1). No sheets, or more than the 4,096 sheets [`crate::read()`] accepts, returns
/// [`craft_cfb::Error::Invalid`] naming the count. A nonempty cell past row 65,535 or column 255
/// (zero-based) returns [`craft_cfb::Error::Invalid`] naming its sheet, row and column, and a
/// compound file that cannot be written returns the writer's error.
///
/// shortcut: characters outside the Basic Multilingual Plane are dropped, sheet names are cut at
/// 255 characters and strings at 4,000 (so each fits one record); lift when a fixture needs more.
pub fn workbook(sheets: &[(&str, Vec<Vec<String>>)]) -> Result<Vec<u8>, craft_cfb::Error> {
    if sheets.is_empty() {
        return Err(craft_cfb::Error::Invalid("fixture has no sheets; craft-xls reads a workbook only when it has a worksheet".into()));
    }
    if sheets.len() > crate::read::MAX_SHEETS {
        return Err(craft_cfb::Error::Invalid(format!(
            "fixture has {} sheets; craft-xls can read at most {}",
            sheets.len(),
            crate::read::MAX_SHEETS
        )));
    }
    let mut sst: Vec<&str> = Vec::new();
    let mut references = 0usize;
    for (name, rows) in sheets {
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if v.is_empty() {
                    continue;
                }
                if r > MAX_ROW || c > MAX_COLUMN {
                    return Err(craft_cfb::Error::Invalid(format!(
                        "fixture cell on sheet {name:?} at zero-based row {r}, column {c} is outside the BIFF8 grid"
                    )));
                }
                if number(v).is_none() {
                    references += 1;
                    if !sst.contains(&v.as_str()) {
                        sst.push(v);
                    }
                }
            }
        }
    }
    let total = i32::try_from(references)
        .map_err(|_| craft_cfb::Error::Invalid(format!("{references} shared-string references do not fit the SST record")))?;
    let mut out = Vec::new();
    rec(&mut out, BOF, &bof(0x0005));
    for ifmt in [0u16, 14] {
        let mut xf = vec![0u8; 20];
        if let Some(slot) = xf.get_mut(2..4) {
            slot.copy_from_slice(&ifmt.to_le_bytes());
        }
        rec(&mut out, 0x00E0, &xf);
    }
    let mut patches = Vec::new();
    for (name, _) in sheets {
        patches.push(out.len() + 4);
        let name: String = name.chars().filter(|c| c.len_utf16() == 1).take(255).collect();
        let mut d = vec![0, 0, 0, 0, 0, 0, name.chars().count() as u8];
        chars(&mut d, &name);
        rec(&mut out, 0x0085, &d);
    }
    // cstTotal is the number of LabelSst cells and cstUnique is the number of strings in the table
    // ([MS-XLS] 2.4.265).
    let mut d = Vec::new();
    d.extend_from_slice(&total.to_le_bytes());
    d.extend_from_slice(&(sst.len() as u32).to_le_bytes());
    let mut sst_records = vec![d];
    for s in &sst {
        let text: String = s.chars().filter(|c| c.len_utf16() == 1).take(MAX_STRING).collect();
        let mut d = (text.chars().count() as u16).to_le_bytes().to_vec();
        chars(&mut d, &text);
        sst_records.push(d);
    }
    write_sst(&mut out, &sst_records);
    rec(&mut out, EOF, &[]);
    for ((_, rows), at) in sheets.iter().zip(patches) {
        let off = (out.len() as u32).to_le_bytes();
        if let Some(slot) = out.get_mut(at..at + 4) {
            slot.copy_from_slice(&off);
        }
        rec(&mut out, BOF, &bof(0x0010));
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if v.is_empty() {
                    continue;
                }
                let mut d = Vec::new();
                d.extend_from_slice(&(r as u16).to_le_bytes());
                d.extend_from_slice(&(c as u16).to_le_bytes());
                if let Some((xf, x)) = number(v) {
                    d.extend_from_slice(&xf.to_le_bytes());
                    d.extend_from_slice(&x.to_le_bytes());
                    rec(&mut out, 0x0203, &d);
                } else {
                    let i = sst
                        .iter()
                        .position(|s| s == v)
                        .ok_or_else(|| craft_cfb::Error::Invalid(format!("the cell text {v:?} is not in the shared-string table")))?;
                    d.extend_from_slice(&0u16.to_le_bytes());
                    d.extend_from_slice(&(i as u32).to_le_bytes());
                    rec(&mut out, 0x00FD, &d);
                }
            }
        }
        rec(&mut out, EOF, &[]);
    }
    compound("Workbook", &out)
}

/// The SST record and its Continue records: each string goes whole into a record of at most
/// 8,224 bytes, starting a Continue record when the current one is full.
fn write_sst(out: &mut Vec<u8>, parts: &[Vec<u8>]) {
    let mut kind = 0x00FC;
    let mut cur: Vec<u8> = Vec::new();
    for part in parts {
        if cur.len() + part.len() > 8224 {
            rec(out, kind, &cur);
            kind = 0x003C;
            cur.clear();
        }
        cur.extend_from_slice(part);
    }
    rec(out, kind, &cur);
}

/// `stream` (a Workbook stream starting with the globals' BOF) encrypted with RC4 1.1 and
/// `password`, a FilePass record put after the BOF. `None` for a stream that is not one.
pub fn encrypt_rc4(stream: &[u8], password: &str) -> Option<Vec<u8>> {
    crate::crypt::encrypt(stream, password)
}

/// [`encrypt_rc4`] with RC4 CryptoAPI version 4.2 and a `bits`-bit key, as current Excel writes it.
pub fn encrypt_cryptoapi(stream: &[u8], password: &str, bits: u32) -> Option<Vec<u8>> {
    crate::crypt::encrypt_cryptoapi(stream, password, bits)
}

/// The FilePass record data [`encrypt_cryptoapi`] writes, for tests that damage it.
pub fn cryptoapi_filepass(password: &str, bits: u32) -> Vec<u8> {
    crate::crypt::cryptoapi_filepass(password, bits)
}
