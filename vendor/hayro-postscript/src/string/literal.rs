// Keep in sync with `hayro-syntax/src/object/string.rs` (`read_literal`).

use alloc::vec::Vec;

use crate::reader::Reader;

pub(crate) fn decode_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut r = Reader::new(data);

    while let Some(byte) = r.read_byte() {
        match byte {
            b'\\' => {
                let next = r.read_byte()?;

                if let Some(first) = octal_digit(next) {
                    // Craft patch (octal overflow, craft-libs #2321): read one
                    // to three octal digits and keep the low eight bits of
                    // the value. "The number ddd may consist of one, two, or
                    // three octal digits; high-order overflow shall be
                    // ignored" (ISO 32000-2 §7.3.4.2; PostScript Language
                    // Reference, third edition, §3.2.2), so `\737` is 0xDF.
                    // Previously an escape above `\377` was dropped.
                    let mut value = first;
                    for _ in 0..2 {
                        let Some(digit) = r.peek_byte().and_then(octal_digit) else {
                            break;
                        };
                        r.forward();
                        // At most 0o777 after three digits, so no overflow.
                        value = value * 8 + digit;
                    }

                    // Truncation to the low byte is the overflow rule above.
                    out.push((value & 0xFF) as u8);
                } else {
                    match next {
                        b'n' => out.push(0xA),
                        b'r' => out.push(0xD),
                        b't' => out.push(0x9),
                        b'b' => out.push(0x8),
                        b'f' => out.push(0xC),
                        b'(' => out.push(b'('),
                        b')' => out.push(b')'),
                        b'\\' => out.push(b'\\'),
                        b'\n' | b'\r' => {
                            // "If the \ is followed immediately by a newline
                            // (CR, LF, or CR-LF pair), the scanner ignores
                            // both the initial \ and the newline; this breaks
                            // a string into multiple lines without including
                            // the newline character as part of the string."
                            r.skip_eol();
                        }
                        _ => out.push(next),
                    }
                }
            }
            b'(' | b')' => out.push(byte),
            // "But if a newline appears without a preceding \, the result is
            // equivalent to \n."
            b'\n' | b'\r' => {
                out.push(b'\n');
                r.skip_eol();
            }
            other => out.push(other),
        }
    }

    Some(())
}

fn octal_digit(byte: u8) -> Option<u16> {
    match byte {
        b'0'..=b'7' => Some(u16::from(byte - b'0')),
        _ => None,
    }
}
