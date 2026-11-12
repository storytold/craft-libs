//! A minimal BIFF8 writer for tests and fixtures (not a general `.xls` writer).

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
/// Characters kept from one string: 4,000 UTF-16 units and the 3-byte header fit one record.
const MAX_STRING: usize = 4000;

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

/// A compound file whose root holds one stream.
pub fn compound(stream: &str, data: &[u8]) -> Vec<u8> {
    craft_cfb::write_streams(craft_cfb::Version::V3, &[(stream, data)]).unwrap_or_default()
}

/// A minimal BIFF8 workbook: one worksheet per `(name, rows)`. Cells that read as numbers are
/// Number records, a cell `date:<serial>` is a Number with the date format `m/d/yyyy` (built-in
/// format 14), an empty cell is left out, and everything else goes through the shared-string
/// table (as UTF-16 when it isn't Latin-1).
///
/// shortcut: characters outside the Basic Multilingual Plane are dropped, sheet names are cut at
/// 255 characters and strings at 4,000 (so each fits one record); lift when a fixture needs more.
pub fn workbook(sheets: &[(&str, Vec<Vec<String>>)]) -> Vec<u8> {
    let mut sst: Vec<&str> = Vec::new();
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
    for c in sheets.iter().flat_map(|(_, rows)| rows.iter().flatten()) {
        if c.parse::<f64>().is_err() && !c.starts_with("date:") && !sst.contains(&c.as_str()) {
            sst.push(c);
        }
    }
    let mut d = Vec::new();
    d.extend_from_slice(&(sst.len() as u32).to_le_bytes());
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
                if let Some(x) = v.strip_prefix("date:").and_then(|x| x.parse::<f64>().ok()) {
                    d.extend_from_slice(&1u16.to_le_bytes());
                    d.extend_from_slice(&x.to_le_bytes());
                    rec(&mut out, 0x0203, &d);
                } else if let Ok(x) = v.parse::<f64>() {
                    d.extend_from_slice(&0u16.to_le_bytes());
                    d.extend_from_slice(&x.to_le_bytes());
                    rec(&mut out, 0x0203, &d);
                } else {
                    d.extend_from_slice(&0u16.to_le_bytes());
                    let i = sst.iter().position(|s| s == v).unwrap_or(0) as u32;
                    d.extend_from_slice(&i.to_le_bytes());
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
