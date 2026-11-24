//! Content transfer encodings (RFC 2045): base64 and quoted-printable.

/// Longest encoded line, without its line break.
const LINE: usize = 76;

/// Decode base64, skipping anything that isn't base64 (line breaks, spaces, junk). Padding ends
/// a group, so several padded chunks one after another decode as they should.
pub fn base64_decode(input: &[u8]) -> Vec<u8> {
    fn flush(out: &mut Vec<u8>, acc: u32, n: u8) {
        match n {
            2 => out.push((acc >> 4) as u8),
            3 => {
                out.push((acc >> 10) as u8);
                out.push((acc >> 2) as u8);
            }
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let (mut acc, mut n) = (0u32, 0u8);
    for &c in input {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                flush(&mut out, acc, n);
                (acc, n) = (0, 0);
                continue;
            }
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        n += 1;
        if n == 4 {
            out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]);
            (acc, n) = (0, 0);
        }
    }
    flush(&mut out, acc, n);
    out
}

/// Base64 on one line (RFC 4648 section 4: `A-Za-z0-9+/`, padded with `=`).
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol = |n: u32| char::from(ALPHABET.get((n & 63) as usize).copied().unwrap_or(b'A'));
    let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
    for group in data.chunks(3) {
        let byte = |i: usize| u32::from(group.get(i).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        out.push(symbol(n >> 18));
        out.push(symbol(n >> 12));
        out.push(if group.len() > 1 { symbol(n >> 6) } else { '=' });
        out.push(if group.len() > 2 { symbol(n) } else { '=' });
    }
    out
}

/// Base64 in lines of 76 characters, CRLF between them (none after the last).
pub fn base64_lines(data: &[u8]) -> Vec<u8> {
    let encoded = base64(data).into_bytes();
    let mut out = Vec::with_capacity(encoded.len() + encoded.len() / LINE * 2);
    for (i, line) in encoded.chunks(LINE).enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(line);
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}

/// Decode quoted-printable: `=XX` is a byte, `=` at the end of a line joins it to the next, and
/// white space at the end of a line is padding. A `=` that is neither stays as it is.
pub fn qp_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while let Some(&c) = input.get(i) {
        match c {
            b'=' => {
                if let (Some(h), Some(l)) = (input.get(i + 1).copied().and_then(hex), input.get(i + 2).copied().and_then(hex)) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                let mut j = i + 1;
                while matches!(input.get(j), Some(b' ' | b'\t')) {
                    j += 1;
                }
                match input.get(j) {
                    Some(b'\r') if input.get(j + 1) == Some(&b'\n') => i = j + 2,
                    Some(b'\r' | b'\n') => i = j + 1,
                    None => i = j,
                    Some(_) => {
                        out.push(b'=');
                        i += 1;
                    }
                }
            }
            b' ' | b'\t' => {
                let mut j = i;
                while matches!(input.get(j), Some(b' ' | b'\t')) {
                    j += 1;
                }
                if !matches!(input.get(j), Some(b'\r' | b'\n') | None) {
                    out.extend_from_slice(input.get(i..j).unwrap_or_default());
                }
                i = j;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Encode text as quoted-printable. `\n` is the line break and is written as CRLF; lines are at
/// most 76 characters. White space before a line break, a `From ` at the start of a line (which
/// mbox files would have to quote) and everything outside printable ASCII are encoded.
pub fn qp_encode(input: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(input.len() + input.len() / 8);
    let mut col = 0;
    for (i, &c) in input.iter().enumerate() {
        if c == b'\n' {
            out.extend_from_slice(b"\r\n");
            col = 0;
            continue;
        }
        let next = input.get(i + 1);
        let literal = match c {
            b'=' => false,
            b' ' | b'\t' => !matches!(next, Some(b'\n') | None),
            b'F' if col == 0 => input.get(i..i + 5) != Some(b"From "),
            b'.' if col == 0 => !matches!(next, Some(b'\n') | None),
            33..=126 => true,
            _ => false,
        };
        let width = if literal { 1 } else { 3 };
        // Keep room for the `=` of a soft line break.
        if col + width > LINE - 1 {
            out.extend_from_slice(b"=\r\n");
            col = 0;
        }
        if literal {
            out.push(c);
        } else {
            out.extend_from_slice(&[
                b'=',
                HEX.get(usize::from(c >> 4)).copied().unwrap_or(b'0'),
                HEX.get(usize::from(c & 15)).copied().unwrap_or(b'0'),
            ]);
        }
        col += width;
    }
    out
}

/// Undo a content transfer encoding named as `Content-Transfer-Encoding` names it (any letter
/// case): `base64` and `quoted-printable`; `7bit`, `8bit`, `binary` and anything unknown are the
/// bytes themselves.
pub fn decode(body: &[u8], encoding: &str) -> Vec<u8> {
    let encoding = encoding.trim();
    if encoding.eq_ignore_ascii_case("base64") {
        base64_decode(body)
    } else if encoding.eq_ignore_ascii_case("quoted-printable") {
        qp_decode(body)
    } else {
        body.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_rfc_4648_vectors() {
        for (plain, coded) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded.as_bytes()), plain.as_bytes());
        }
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn transfer_encodings_by_name() {
        assert_eq!(decode(b"SGk=", " Base64 "), b"Hi");
        assert_eq!(decode(b"a=3Db", "Quoted-Printable"), b"a=b");
        for identity in ["7bit", "8bit", "binary", "", "x-unknown"] {
            assert_eq!(decode(b"a=3Db", identity), b"a=3Db");
        }
    }

    #[test]
    fn base64_plain_and_padded() {
        assert_eq!(base64_decode(b"SGVsbG8="), b"Hello");
        assert_eq!(base64_decode(b"SGVsbG8"), b"Hello");
        assert_eq!(base64_decode(b"SGVsbA=="), b"Hell");
        assert_eq!(base64_decode(b"SGVs"), b"Hel");
        assert_eq!(base64_decode(b""), b"");
        assert_eq!(base64_decode(b"="), b"");
        assert_eq!(base64_decode(b"S"), b"");
    }

    #[test]
    fn base64_skips_junk_and_line_breaks() {
        assert_eq!(base64_decode(b"SGVs\r\nbG8g\r\nd29y bGQ=\r\n"), b"Hello world");
        assert_eq!(base64_decode(b"  S*G(V)s!b~G8\x00\xFF= "), b"Hello");
        // Two padded chunks, as some senders write.
        assert_eq!(base64_decode(b"SGk=SGk="), b"HiHi");
    }

    #[test]
    fn base64_lines_wrap_at_76() {
        let data: Vec<u8> = (0..200u8).collect();
        let lines = base64_lines(&data);
        let text = String::from_utf8(lines.clone()).unwrap();
        assert!(text.split("\r\n").all(|l| l.len() <= 76 && !l.is_empty()));
        assert_eq!(text.split("\r\n").next().map(str::len), Some(76));
        assert!(!text.ends_with('\n'));
        assert_eq!(base64_decode(&lines), data);
        assert_eq!(base64_lines(b""), b"");
    }

    #[test]
    fn quoted_printable_decodes() {
        assert_eq!(qp_decode(b"caf=E9 =3D ok"), b"caf\xE9 = ok");
        assert_eq!(qp_decode(b"lower=e9"), b"lower\xE9");
        assert_eq!(qp_decode(b"soft=\r\nbreak=\nand= \t\r\nmore"), b"softbreakandmore");
        assert_eq!(qp_decode(b"keep\r\nhard\nbreaks"), b"keep\r\nhard\nbreaks");
        assert_eq!(qp_decode(b"pad   \r\nded\t\nend  "), b"pad\r\nded\nend");
        assert_eq!(qp_decode(b"a  b"), b"a  b");
    }

    #[test]
    fn quoted_printable_tolerates_stray_equals() {
        assert_eq!(qp_decode(b"a=b"), b"a=b");
        assert_eq!(qp_decode(b"a=ZZ"), b"a=ZZ");
        assert_eq!(qp_decode(b"a=4"), b"a=4");
        assert_eq!(qp_decode(b"x=="), b"x=");
        assert_eq!(qp_decode(b"end="), b"end");
        assert_eq!(qp_decode(b"=\r"), b"");
    }

    #[test]
    fn quoted_printable_encodes() {
        assert_eq!(qp_encode("café = ok\nnext".as_bytes()), b"caf=C3=A9 =3D ok\r\nnext");
        assert_eq!(qp_encode(b"trailing \nand\t\nend "), b"trailing=20\r\nand=09\r\nend=20");
        assert_eq!(qp_encode(b"From me\nFrom you\n Fromage"), b"=46rom me\r\n=46rom you\r\n Fromage");
        assert_eq!(qp_encode(b".\n.x\n."), b"=2E\r\n.x\r\n=2E");
        assert_eq!(qp_encode(b"a\rb\x00"), b"a=0Db=00");
        assert_eq!(qp_encode(b""), b"");
    }

    #[test]
    fn quoted_printable_lines_stay_within_76() {
        let text = format!("{}\n{}", "x".repeat(300), "é".repeat(100));
        let encoded = qp_encode(text.as_bytes());
        let s = String::from_utf8(encoded.clone()).unwrap();
        for line in s.split("\r\n") {
            assert!(line.len() <= 76, "{}", line.len());
        }
        assert_eq!(qp_decode(&encoded), text.replace('\n', "\r\n").as_bytes());
    }

    #[test]
    fn long_runs_decode_quickly() {
        let spaces = [b' '; 300_000];
        assert_eq!(qp_decode(&spaces), b"");
        let mut padded = spaces.to_vec();
        padded.push(b'x');
        assert_eq!(qp_decode(&padded).len(), 300_001);
        // Every `=` but the last is literal; the last one is a soft break at the end.
        let equals = [b'='; 300_000];
        assert_eq!(qp_decode(&equals).len(), 299_999);
        let mixed = b"= ".repeat(150_000);
        let _ = qp_decode(&mixed);
    }

    proptest::proptest! {
        #[test]
        fn base64_round_trips(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            proptest::prop_assert_eq!(base64_decode(&base64_lines(&data)), data);
        }

        #[test]
        fn quoted_printable_round_trips(text in proptest::prelude::any::<String>()) {
            let text = text.replace('\r', "");
            let encoded = qp_encode(text.as_bytes());
            proptest::prop_assert!(encoded.split(|b| *b == b'\n').all(|l| l.len() <= 77));
            proptest::prop_assert!(encoded.iter().all(|b| matches!(*b, b'\r' | b'\n' | b'\t' | 32..=126)));
            proptest::prop_assert_eq!(qp_decode(&encoded), text.replace('\n', "\r\n").into_bytes());
        }

        #[test]
        fn decoders_take_any_bytes(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            let _ = base64_decode(&data);
            let _ = qp_decode(&data);
        }
    }
}
