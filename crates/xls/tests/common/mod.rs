//! Helpers shared by the integration tests: a hand-laid compound file (independent of the
//! `craft-cfb` writer, and with a layout the hostile-input tests can poke at) and BIFF8 record
//! builders. Every test file is assembled from the [MS-CFB] and [MS-XLS] structures; no
//! Excel-produced file is used.
#![allow(dead_code, reason = "each test binary uses its own part of these helpers")]

use craft_xls::{CellRef, Error, Limits, Value, Workbook};

pub const END: u32 = 0xFFFF_FFFE;
pub const FREE: u32 = 0xFFFF_FFFF;

pub fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn dir_entry(name: &str, kind: u8, child: u32, start: u32, size: u32) -> [u8; 128] {
    let mut e = [0u8; 128];
    let units: Vec<u16> = name.encode_utf16().collect();
    for (i, u) in units.iter().enumerate() {
        e[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
    }
    if !name.is_empty() {
        e[64..66].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
    }
    e[66] = kind;
    e[67] = 1; // black
    put32(&mut e, 68, FREE);
    put32(&mut e, 72, FREE);
    put32(&mut e, 76, child);
    put32(&mut e, 116, start);
    put32(&mut e, 120, size);
    e
}

/// A version 3 compound file (512-byte sectors) whose root holds one stream. Streams under
/// 4096 bytes go to the mini stream, like real files.
pub fn compound(name: &str, data: &[u8]) -> Vec<u8> {
    let mini = data.len() < 4096;
    let pad = |n: usize, to: usize| n.div_ceil(to) * to;
    // Sector payloads after the FAT: directory, mini FAT (if any), then data.
    let mut payload: Vec<Vec<u8>> = vec![];
    let mut chains: Vec<(usize, usize)> = vec![]; // (first payload index, sector count)
    let (dir_at, minifat_at, data_at);
    let mut dir = vec![];
    let mut minifat = vec![];
    let stream_sectors;
    if mini {
        let m = data.len().div_ceil(64).max(1);
        for i in 0..m {
            minifat.extend_from_slice(&(if i + 1 == m { END } else { i as u32 + 1 }).to_le_bytes());
        }
        minifat.resize(512, 0xFF);
        let mut container = data.to_vec();
        container.resize(pad(data.len().max(1), 64), 0);
        stream_sectors = container.len().div_ceil(512);
        container.resize(stream_sectors * 512, 0);
        dir_at = 0;
        minifat_at = Some(1);
        data_at = 2;
        payload.push(vec![]);
        payload.push(minifat);
        payload.extend(container.chunks(512).map(<[u8]>::to_vec));
        chains.push((data_at, stream_sectors));
        dir.extend_from_slice(&dir_entry("Root Entry", 5, 1, 0, (stream_sectors * 512) as u32));
        dir.extend_from_slice(&dir_entry(name, 2, FREE, 0, data.len() as u32));
    } else {
        let mut body = data.to_vec();
        stream_sectors = body.len().div_ceil(512);
        body.resize(stream_sectors * 512, 0);
        dir_at = 0;
        minifat_at = None;
        data_at = 1;
        payload.push(vec![]);
        payload.extend(body.chunks(512).map(<[u8]>::to_vec));
        chains.push((data_at, stream_sectors));
        dir.extend_from_slice(&dir_entry("Root Entry", 5, 1, END, 0));
        dir.extend_from_slice(&dir_entry(name, 2, FREE, 0, data.len() as u32));
    }
    dir.extend_from_slice(&dir_entry("", 0, FREE, FREE, 0));
    dir.extend_from_slice(&dir_entry("", 0, FREE, FREE, 0));
    // FAT sectors come first; their count depends on the total.
    let mut fat_n = 1;
    while (fat_n + payload.len()).div_ceil(128) > fat_n {
        fat_n += 1;
    }
    let sec = |i: usize| (fat_n + i) as u32;
    // Fix the directory's start sectors now that the layout is known.
    let first_data = sec(data_at);
    if mini {
        put32(&mut dir, 116, first_data); // root: mini stream container
    } else {
        put32(&mut dir, 128 + 116, first_data);
    }
    payload[dir_at] = dir;
    let total = fat_n + payload.len();
    let mut fat = vec![FREE; fat_n * 128];
    for f in fat.iter_mut().take(fat_n) {
        *f = 0xFFFF_FFFD;
    }
    fat[sec(dir_at) as usize] = END;
    if let Some(m) = minifat_at {
        fat[sec(m) as usize] = END;
    }
    for (first, n) in chains {
        for k in 0..n {
            fat[sec(first + k) as usize] = if k + 1 == n { END } else { sec(first + k + 1) };
        }
    }
    let mut out = vec![0u8; 512];
    out[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    out[24..26].copy_from_slice(&0x3Eu16.to_le_bytes());
    out[26..28].copy_from_slice(&3u16.to_le_bytes());
    out[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    out[30..32].copy_from_slice(&9u16.to_le_bytes());
    out[32..34].copy_from_slice(&6u16.to_le_bytes());
    put32(&mut out, 44, fat_n as u32);
    put32(&mut out, 48, sec(dir_at));
    put32(&mut out, 56, 4096);
    put32(&mut out, 60, minifat_at.map_or(END, sec));
    put32(&mut out, 64, u32::from(minifat_at.is_some()));
    put32(&mut out, 68, END);
    for i in 0..109 {
        put32(&mut out, 76 + i * 4, if i < fat_n { i as u32 } else { FREE });
    }
    for f in &fat {
        out.extend_from_slice(&f.to_le_bytes());
    }
    for p in payload {
        let mut p = p;
        p.resize(512, 0);
        out.extend_from_slice(&p);
    }
    assert_eq!(out.len(), 512 * (1 + total));
    out
}

pub fn rec(id: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = id.to_le_bytes().to_vec();
    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

pub fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Cell header: row, column, XF index.
pub fn cell(r: u16, c: u16, xf: u16) -> Vec<u8> {
    u16s(&[r, c, xf])
}

pub fn bof(dt: u16) -> Vec<u8> {
    rec(0x0809, &cat(&[&u16s(&[0x0600, dt, 0x0DBB, 0x07CC]), &[0; 8]]))
}

pub fn eof() -> Vec<u8> {
    rec(0x000A, &[])
}

/// XLUnicodeString with compressed characters.
pub fn xlstr(s: &str) -> Vec<u8> {
    cat(&[&(s.len() as u16).to_le_bytes(), &[0], s.as_bytes()])
}

/// A FORMULA record with a numeric cached result.
pub fn formula(r: u16, c: u16, cached: [u8; 8], rgce: &[u8]) -> Vec<u8> {
    let mut p = cell(r, c, 0);
    p.extend_from_slice(&cached);
    p.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // flags, chn
    p.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    p.extend_from_slice(rgce);
    rec(0x0006, &p)
}

pub fn num(n: f64) -> [u8; 8] {
    n.to_le_bytes()
}

pub fn special(kind: u8, v: u8) -> [u8; 8] {
    [kind, 0, v, 0, 0, 0, 0xFF, 0xFF]
}

/// PtgRef (value class) with relative row and column.
pub fn ptg_ref(r: u16, c: u16) -> Vec<u8> {
    cat(&[&[0x44], &u16s(&[r, c | 0xC000])])
}

pub fn ptg_int(n: u16) -> Vec<u8> {
    cat(&[&[0x1E], &n.to_le_bytes()])
}

pub fn exp(r: u16, c: u16) -> Vec<u8> {
    cat(&[&[0x01], &u16s(&[r, c])])
}

/// What a read skipped, as text.
#[derive(Debug)]
pub struct Report {
    pub warnings: Vec<String>,
}

pub fn read_xls(bytes: &[u8]) -> Result<(Workbook, Report), Error> {
    read_limited(bytes, Limits::default())
}

pub fn read_limited(bytes: &[u8], limits: Limits) -> Result<(Workbook, Report), Error> {
    let read = craft_xls::read_with(bytes, limits)?;
    Ok((read.workbook, Report { warnings: read.warnings.iter().map(ToString::to_string).collect() }))
}

/// `B3` → its position.
pub fn at(a1: &str) -> CellRef {
    let letters = a1.bytes().take_while(u8::is_ascii_alphabetic).count();
    let col = a1[..letters].bytes().fold(0u32, |n, b| n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1) - 1;
    CellRef::new(a1[letters..].parse::<u32>().unwrap() - 1, col)
}

pub fn text(s: &str) -> Value {
    Value::Text(s.into())
}

/// The value at `a1` of sheet `sheet` (empty for a cell that is not stored).
pub fn v(wb: &Workbook, sheet: usize, a1: &str) -> Value {
    wb.sheets[sheet].cells.get(&at(a1)).map(|c| c.value.clone()).unwrap_or_default()
}

/// The formula text at `a1` of the first sheet.
pub fn ftext(wb: &Workbook, a1: &str) -> Option<String> {
    wb.sheets[0].cells.get(&at(a1)).and_then(|c| c.formula.as_ref()).map(|f| f.text.clone())
}

/// The number format of the cell at `a1` of the first sheet.
pub fn fmt(wb: &Workbook, a1: &str) -> String {
    let cell = &wb.sheets[0].cells[&at(a1)];
    cell.format.map_or_else(|| "General".to_string(), |i| wb.formats[usize::from(i)].number_format.clone())
}
