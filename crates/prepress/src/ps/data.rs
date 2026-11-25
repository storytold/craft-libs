//! Image data in a PostScript program: compressed, then ASCII85 or bytes as they are.

use std::borrow::Cow;

use super::PsLevel;

/// Encoded image data and the decode filter that reads it.
pub(super) struct Payload<'a> {
    bytes: Cow<'a, [u8]>,
    filter: &'static str,
}

impl<'a> Payload<'a> {
    /// JPEG data as it is.
    pub fn dct(jpeg: &'a [u8]) -> Self {
        Self { bytes: Cow::Borrowed(jpeg), filter: "/DCTDecode" }
    }
}

/// `samples` compressed for `level`: Flate at LanguageLevel 3, RunLength at 2.
pub(super) fn samples(level: PsLevel, samples: &[u8]) -> Payload<'static> {
    match level {
        PsLevel::Level3 => Payload { bytes: Cow::Owned(craft_ps::write::deflate(samples)), filter: "/FlateDecode" },
        PsLevel::Level2 => Payload { bytes: Cow::Owned(run_length(samples)), filter: "/RunLengthDecode" },
    }
}

/// An `image` reading `payload` from the program itself. `dict` is the image dictionary
/// without its `/DataSource` and closing `>>`.
///
/// - ASCII: an `ASCII85Decode` filter on `currentfile`, flushed after the image so the program
///   resumes after the data's `~>`.
/// - Binary, LanguageLevel 3: a `SubFileDecode` of the payload's byte count, so the program
///   resumes exactly after it.
/// - Binary, LanguageLevel 2: the decode filter straight on `currentfile`; RunLength and DCT data
///   end themselves. The bytes sit inside `%%BeginData` / `%%EndData`.
pub(super) fn write_image(out: &mut Vec<u8>, level: PsLevel, binary: bool, dict: &str, payload: &Payload<'_>) {
    let (bytes, filter) = (&payload.bytes[..], payload.filter);
    if !binary {
        let mut s = String::new();
        craft_ps::write::push_inline_image(&mut s, "CraftSrc", "", dict, filter, "image", &craft_eps::ascii85::encode(bytes));
        s.push('\n');
        out.extend_from_slice(s.as_bytes());
        return;
    }
    let program = match level {
        PsLevel::Level3 => format!(
            "{{ /CraftSrc currentfile {} () /SubFileDecode filter def\n{dict}\n/DataSource CraftSrc {filter} filter >> image\nCraftSrc flushfile }} exec\n",
            bytes.len()
        ),
        PsLevel::Level2 => format!("{{ {dict}\n/DataSource currentfile {filter} filter >> image }} exec\n"),
    };
    let total = program.len() + bytes.len() + 1;
    out.extend_from_slice(format!("%%BeginData: {total} Binary Bytes\n").as_bytes());
    out.extend_from_slice(program.as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\n%%EndData\n");
}

/// `data` for `RunLengthDecode`: runs of 2 to 128 equal bytes as `257 − n, byte`, other bytes
/// in literal runs of up to 128 as `n − 1, bytes`, then the end-of-data byte 128.
pub(super) fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 128 + 2);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        let run = data[i..].iter().take(128).take_while(|v| **v == b).count();
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(b);
            i += run;
            continue;
        }
        // A literal run ends before the next pair of equal bytes.
        let start = i;
        while i < data.len() && i - start < 128 && !(i + 1 < data.len() && data[i] == data[i + 1]) {
            i += 1;
        }
        if i == start {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&data[start..i]);
    }
    out.push(128);
    out
}

/// Whether a JPEG has an Adobe `APP14` segment and four components: Adobe applications store
/// such CMYK data inverted, so it is read with `Decode [1 0 1 0 1 0 1 0]`.
pub(super) fn adobe_inverted(jpeg: &[u8]) -> bool {
    let mut adobe = false;
    let mut at = 2;
    while at + 4 <= jpeg.len() {
        if jpeg[at] != 0xFF {
            return false;
        }
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        let segment = jpeg.get(at + 4..at + 2 + len).unwrap_or_default();
        match marker {
            0xEE if segment.starts_with(b"Adobe") => adobe = true,
            // Start of frame: the component count follows the precision and size.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => return adobe && segment.get(5) == Some(&4),
            0xDA => return false,
            _ => {}
        }
        at += 2 + len;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RunLengthDecode as the PostScript Language Reference defines it.
    fn decode(data: &[u8]) -> Vec<u8> {
        let (mut out, mut i) = (Vec::new(), 0);
        while let Some(&n) = data.get(i) {
            match n {
                128 => break,
                0..=127 => {
                    out.extend_from_slice(&data[i + 1..i + 2 + usize::from(n)]);
                    i += 2 + usize::from(n);
                }
                _ => {
                    out.extend(std::iter::repeat_n(data[i + 1], 257 - usize::from(n)));
                    i += 2;
                }
            }
        }
        out
    }

    #[test]
    fn run_length_data_decodes_to_the_samples_and_shrinks_flat_areas() {
        let mut samples: Vec<u8> = (0..=255).collect();
        samples.extend([7; 300]);
        samples.extend([1, 2, 2, 3]);
        let encoded = run_length(&samples);
        assert_eq!(decode(&encoded), samples);
        assert!(run_length(&[9; 1000]).len() < 30);
        assert_eq!(decode(&run_length(&[])), Vec::<u8>::new());
    }
}
