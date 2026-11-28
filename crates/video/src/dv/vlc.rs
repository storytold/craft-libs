//! The variable-length codes of the AC coefficients (ITU-R BT.1618-1 §2.4, Table 25; SMPTE ST 314-2005 §5.4, the same
//! table). A code stands for (run, amplitude): `run` coefficients quantised to 0, then one of magnitude `amplitude`
//! (0 too), with a sign bit after it when the amplitude is not 0. A pair the table has no code for is sent as
//! (run − 1, 0) and (0, amplitude). The block ends with EOB (0110).
//!
//! Both documents print two pairs of codes a digit short: (7, 1) and (8, 1) as `1110000s` / `1110001s`, and (1, 16)
//! and (1, 17) as `11111011110s` / `11111011111s`. As printed, `1110001s` would be a prefix of (9, 1)'s `11100010s`.
//! Table 24 gives all four codes a length of 8 and 12 bits, which makes them `11100000s`, `11100001s`,
//! `111110111110s` and `111110111111s`, the codes left over between their neighbours.

use std::sync::OnceLock;

/// What a code decodes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Code {
    /// No code starts with these bits (an escape for a run or an amplitude the table does not use).
    Invalid,
    /// End of block.
    Eob,
    /// `run` zero coefficients, then `amplitude` (signed; 0 makes it one more zero).
    Pair { run: u8, amplitude: i16 },
}

/// One entry of the lookup on 16 bits: the code the bits start with, and its length (with the sign bit).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    pub(crate) length: u8,
    pub(crate) code: Code,
}

/// The longest code: (0, amplitude) by escape, 7 + 8 bits and the sign.
pub(crate) const LONGEST: u32 = 16;

/// The codes of Table 25 that are not escapes: (run, amplitude, code without its sign bit).
const CODES: &[(u8, u8, &str)] = &[
    (0, 1, "00"),
    (0, 2, "010"),
    (1, 1, "0111"),
    (0, 3, "1000"),
    (0, 4, "1001"),
    (2, 1, "10100"),
    (1, 2, "10101"),
    (0, 5, "10110"),
    (0, 6, "10111"),
    (3, 1, "110000"),
    (4, 1, "110001"),
    (0, 7, "110010"),
    (0, 8, "110011"),
    (5, 1, "1101000"),
    (6, 1, "1101001"),
    (2, 2, "1101010"),
    (1, 3, "1101011"),
    (1, 4, "1101100"),
    (0, 9, "1101101"),
    (0, 10, "1101110"),
    (0, 11, "1101111"),
    (7, 1, "11100000"),
    (8, 1, "11100001"),
    (9, 1, "11100010"),
    (10, 1, "11100011"),
    (3, 2, "11100100"),
    (4, 2, "11100101"),
    (2, 3, "11100110"),
    (1, 5, "11100111"),
    (1, 6, "11101000"),
    (1, 7, "11101001"),
    (0, 12, "11101010"),
    (0, 13, "11101011"),
    (0, 14, "11101100"),
    (0, 15, "11101101"),
    (0, 16, "11101110"),
    (0, 17, "11101111"),
    (11, 1, "111100000"),
    (12, 1, "111100001"),
    (13, 1, "111100010"),
    (14, 1, "111100011"),
    (5, 2, "111100100"),
    (6, 2, "111100101"),
    (3, 3, "111100110"),
    (4, 3, "111100111"),
    (2, 4, "111101000"),
    (2, 5, "111101001"),
    (1, 8, "111101010"),
    (0, 18, "111101011"),
    (0, 19, "111101100"),
    (0, 20, "111101101"),
    (0, 21, "111101110"),
    (0, 22, "111101111"),
    (5, 3, "1111100000"),
    (3, 4, "1111100001"),
    (3, 5, "1111100010"),
    (2, 6, "1111100011"),
    (1, 9, "1111100100"),
    (1, 10, "1111100101"),
    (1, 11, "1111100110"),
    (0, 0, "11111001110"),
    (1, 0, "11111001111"),
    (6, 3, "11111010000"),
    (4, 4, "11111010001"),
    (3, 6, "11111010010"),
    (1, 12, "11111010011"),
    (1, 13, "11111010100"),
    (1, 14, "11111010101"),
    (2, 0, "111110101100"),
    (3, 0, "111110101101"),
    (4, 0, "111110101110"),
    (5, 0, "111110101111"),
    (7, 2, "111110110000"),
    (8, 2, "111110110001"),
    (9, 2, "111110110010"),
    (10, 2, "111110110011"),
    (7, 3, "111110110100"),
    (8, 3, "111110110101"),
    (4, 5, "111110110110"),
    (3, 7, "111110110111"),
    (2, 7, "111110111000"),
    (2, 8, "111110111001"),
    (2, 9, "111110111010"),
    (2, 10, "111110111011"),
    (2, 11, "111110111100"),
    (1, 15, "111110111101"),
    (1, 16, "111110111110"),
    (1, 17, "111110111111"),
];

/// The lookup on the next 16 bits (most significant first).
pub(crate) fn table() -> &'static [Entry] {
    static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();
    TABLE.get_or_init(build)
}

/// `bits` (a string of 0 and 1) as a number.
fn value(bits: &str) -> u32 {
    bits.bytes().fold(0, |v, b| (v << 1) | u32::from(b == b'1'))
}

fn build() -> Vec<Entry> {
    let invalid = Entry { length: 0, code: Code::Invalid };
    let mut table = vec![invalid; 1 << LONGEST];
    let mut put = |code: u32, length: u32, decoded: Code| {
        let Some(shift) = LONGEST.checked_sub(length) else {
            return;
        };
        let first = (code << shift) as usize;
        let count = 1usize << shift;
        if let Some(slots) = table.get_mut(first..first + count) {
            for slot in slots {
                *slot = Entry { length: length as u8, code: decoded };
            }
        }
    };
    put(0b0110, 4, Code::Eob);
    for &(run, amplitude, bits) in CODES {
        let (code, length) = (value(bits), bits.len() as u32);
        if amplitude == 0 {
            put(code, length, Code::Pair { run, amplitude: 0 });
        } else {
            let amplitude = i16::from(amplitude);
            put(code << 1, length + 1, Code::Pair { run, amplitude });
            put((code << 1) | 1, length + 1, Code::Pair { run, amplitude: -amplitude });
        }
    }
    // (R, 0): 1111110 and R in 6 bits, R = 6 to 61.
    for run in 6..=61u32 {
        put((0b111_1110 << 6) | run, 13, Code::Pair { run: run as u8, amplitude: 0 });
    }
    // (0, A): 1111111, A in 8 bits and the sign, A = 23 to 255.
    for amplitude in 23..=255u32 {
        let code = (0b111_1111 << 9) | (amplitude << 1);
        let amplitude = amplitude as i16;
        put(code, 16, Code::Pair { run: 0, amplitude });
        put(code | 1, 16, Code::Pair { run: 0, amplitude: -amplitude });
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table 24: the length of the codes (without the sign bit) by run (rows) and amplitude (columns).
    const LENGTHS: &[&[u32]] = &[
        &[11, 2, 3, 4, 4, 5, 5, 6, 6, 7, 7, 7, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9],
        &[11, 4, 5, 7, 7, 8, 8, 8, 9, 10, 10, 10, 11, 11, 11, 12, 12, 12],
        &[12, 5, 7, 8, 9, 9, 10, 12, 12, 12, 12, 12],
        &[12, 6, 8, 9, 10, 10, 11, 12],
        &[12, 6, 8, 9, 11, 12],
        &[12, 7, 9, 10],
        &[13, 7, 9, 11],
        &[13, 8, 12, 12],
        &[13, 8, 12, 12],
        &[13, 8, 12],
        &[13, 8, 12],
        &[13, 9],
        &[13, 9],
        &[13, 9],
        &[13, 9],
    ];

    #[test]
    fn every_code_of_table_24_has_its_length() {
        for (run, row) in LENGTHS.iter().enumerate() {
            for (amplitude, &length) in row.iter().enumerate() {
                let found = CODES.iter().find(|&&(r, a, _)| usize::from(r) == run && usize::from(a) == amplitude);
                match found {
                    Some(&(_, _, bits)) => {
                        assert_eq!(bits.len() as u32, length, "({run}, {amplitude})")
                    }
                    // (6, 0) and (7, 0) are escapes of 13 bits.
                    None => assert!(amplitude == 0 && run >= 6 && length == 13, "({run}, {amplitude})"),
                }
            }
        }
    }

    #[test]
    fn no_code_is_the_start_of_another() {
        let mut codes: Vec<String> = CODES
            .iter()
            .flat_map(|&(_, amplitude, bits)| if amplitude == 0 { vec![bits.to_owned()] } else { vec![format!("{bits}0"), format!("{bits}1")] })
            .chain(std::iter::once("0110".to_owned()))
            .chain((6..=61).map(|run| format!("1111110{run:06b}")))
            .chain((23..=255).flat_map(|a| [format!("1111111{a:08b}0"), format!("1111111{a:08b}1")]))
            .collect();
        codes.sort();
        for pair in codes.windows(2) {
            assert!(!pair[1].starts_with(&pair[0]), "{} starts {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn the_lookup_decodes_codes_and_escapes() {
        let table = table();
        let at = |bits: &str| {
            let padded = format!("{bits:0<16}");
            table[value(&padded) as usize]
        };
        let entry = at("0110");
        assert_eq!((entry.length, entry.code), (4, Code::Eob));
        let entry = at("001");
        assert_eq!((entry.length, entry.code), (3, Code::Pair { run: 0, amplitude: -1 }));
        let entry = at("111001000");
        assert_eq!((entry.length, entry.code), (9, Code::Pair { run: 3, amplitude: 2 }));
        let entry = at("1111110100000");
        assert_eq!((entry.length, entry.code), (13, Code::Pair { run: 32, amplitude: 0 }));
        let entry = at("1111111111111111");
        assert_eq!((entry.length, entry.code), (16, Code::Pair { run: 0, amplitude: -255 }));
        // Escapes for a run under 6 or an amplitude under 23 are not used.
        assert_eq!(at("1111110000101").code, Code::Invalid);
        assert_eq!(at("1111111000101100").code, Code::Invalid);
    }
}
