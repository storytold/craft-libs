//! The bytes of an HP-GL file as instructions: two-letter mnemonics and the parameters that follow
//! them, read one at a time as the plot asks for them. Printer escape sequences (PCL, HP RTL, the
//! HP-GL device-control escapes) and PJL lines around and between the instructions are skipped,
//! with the data their escapes carry.

/// The end-of-text character, the default label terminator.
pub const ETX: u8 = 3;
const ESC: u8 = 0x1b;

/// The most digits of a number that count: longer ones are read as their first digits.
const MAX_DIGITS: usize = 18;

/// What the bytes being read hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// HP-GL instructions.
    Hpgl,
    /// PCL or HP RTL: escape sequences and their data, nothing to draw.
    Pcl,
    /// After a Universal Exit Language: PJL lines until a language starts.
    Pjl,
}

/// One item of a PE (Polyline Encoded) instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PeItem {
    /// Select pen.
    Pen(i64),
    /// Rectangle mode: the following coordinates come in fours.
    Rect,
    /// A coordinate pair, after a pen-up flag (`up`) and an absolute flag (`absolute`).
    Point { x: f64, y: f64, up: bool, absolute: bool },
}

/// The state of one PE instruction being read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pe {
    seven_bit: bool,
    fraction_bits: u32,
    up: bool,
    absolute: bool,
}

pub struct Lexer<'a> {
    b: &'a [u8],
    at: usize,
    mode: Mode,
    /// The file holds HP RTL or PCL raster image data.
    pub raster: bool,
}

/// Is `c` a byte that separates numeric parameters (or white space between instructions)?
fn separator(c: u8) -> bool {
    matches!(c, b' ' | b',' | b'\t' | b'\r' | b'\n' | 0)
}

impl<'a> Lexer<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0, mode: Mode::Hpgl, raster: false }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }

    /// Whether everything has been read.
    pub fn done(&self) -> bool {
        self.at >= self.b.len()
    }

    /// How many bytes have been read.
    pub fn pos(&self) -> usize {
        self.at
    }

    /// The next instruction's mnemonic, in upper case; `strict`: only white space, semicolons and
    /// escape sequences may come before it (anything else gives `None`, as the end does).
    pub fn op(&mut self, strict: bool) -> Option<[u8; 2]> {
        loop {
            let c = self.peek()?;
            if c == ESC {
                self.escape();
                continue;
            }
            match self.mode {
                Mode::Pcl => {
                    self.at += 1;
                    continue;
                }
                Mode::Pjl => {
                    if separator(c) {
                        self.at += 1;
                    } else if c == b'@' {
                        self.pjl_line();
                    } else {
                        self.mode = Mode::Hpgl;
                    }
                    continue;
                }
                Mode::Hpgl => {}
            }
            let next = self.b.get(self.at + 1).copied();
            if c.is_ascii_alphabetic()
                && let Some(n) = next.filter(u8::is_ascii_alphabetic)
            {
                self.at += 2;
                return Some([c.to_ascii_uppercase(), n.to_ascii_uppercase()]);
            }
            if strict && !(separator(c) || c == b';' || c < 0x20) {
                return None;
            }
            self.at += 1;
        }
    }

    /// One `@PJL` line: an `ENTER LANGUAGE` command picks what follows.
    fn pjl_line(&mut self) {
        let rest = &self.b[self.at..];
        let end = rest.iter().position(|&c| c == b'\n').map_or(rest.len(), |n| n + 1);
        // A command names its language within its first 256 bytes; longer lines aren't copied.
        let line = String::from_utf8_lossy(&rest[..end.min(256)]).to_ascii_uppercase();
        // COMMENT/ECHO text may mention LANGUAGE without changing the printer language.
        if let Some((command, language)) = line.split_once("LANGUAGE")
            && command.split_ascii_whitespace().eq(["@PJL", "ENTER"])
        {
            let lang = language.trim_start_matches([' ', '\t', '=']).trim();
            self.mode = if lang.starts_with("HPGL") { Mode::Hpgl } else { Mode::Pcl };
        }
        self.at += end;
    }

    /// An escape sequence at the current byte, with the data it carries.
    fn escape(&mut self) {
        self.at += 1;
        let Some(c) = self.peek() else { return };
        self.at += 1;
        match c {
            // Universal Exit Language (`ESC%-12345X`) and the language switches (`ESC%#B` to HP-GL,
            // `ESC%#A` to PCL or HP RTL).
            b'%' => {
                let (_, end) = self.value();
                match end {
                    Some(b'X') => self.mode = Mode::Pjl,
                    Some(b'B') => self.mode = Mode::Hpgl,
                    Some(b'A') => self.mode = Mode::Pcl,
                    _ => {}
                }
            }
            // Reset: PCL follows, until a language switch.
            b'E' => self.mode = Mode::Pcl,
            // HP-GL device control: `ESC.` and a letter, with parameters up to a colon.
            b'.' => {
                let Some(kind) = self.peek() else { return };
                self.at += 1;
                if !b"()YZBEJKLORUX".contains(&kind) {
                    let rest = &self.b[self.at..self.b.len().min(self.at + 80)];
                    if let Some(n) = rest.iter().position(|&c| c == b':')
                        && rest[..n].iter().all(|c| c.is_ascii_digit() || matches!(c, b';' | b' ' | b','))
                    {
                        self.at += n + 1;
                    }
                }
            }
            // Parameterized PCL: a group character, then values each ending in a letter (upper case
            // ends the sequence); `W`, `*b…V` (raster data by plane) and `&p…X` (transparent print
            // data) are followed by that many bytes of data. Without a group character (`ESC(8U`,
            // `ESC)0N`), one value and the letter ending it.
            0x21..=0x2f => {
                let Some(group) = self.peek().filter(|g| (0x60..=0x7e).contains(g)) else {
                    self.value();
                    return;
                };
                self.at += 1;
                loop {
                    let (v, end) = self.value();
                    let Some(end) = end else { return };
                    let upper = end.to_ascii_uppercase();
                    if upper == b'W' || (c, group, upper) == (b'*', b'b', b'V') || (c, group, upper) == (b'&', b'p', b'X') {
                        let n = v.max(0.0).min((self.b.len() - self.at) as f64) as usize;
                        if (c, group) == (b'*', b'b') && n > 0 {
                            self.raster = true;
                        }
                        self.at += n;
                    }
                    if (c, group, end) == (b'*', b'r', b'A') {
                        self.raster = true;
                    }
                    if !end.is_ascii_lowercase() {
                        return;
                    }
                }
            }
            // A two-character escape.
            _ => {}
        }
    }

    /// A PCL value (`[+-]digits[.digits]`, any part missing) and the character that ends it; `None`
    /// when that is not a letter or `@` (which is then left unread).
    fn value(&mut self) -> (f64, Option<u8>) {
        let v = self.number_here().unwrap_or(0.0);
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() || c == b'@' => {
                self.at += 1;
                (v, Some(c))
            }
            _ => (v, None),
        }
    }

    /// A number starting at the current byte (`[+-]digits[.digits]`), `None` when it has no digit
    /// (its sign and point are read all the same).
    fn number_here(&mut self) -> Option<f64> {
        let mut neg = false;
        if let Some(c @ (b'+' | b'-')) = self.peek() {
            neg = c == b'-';
            self.at += 1;
        }
        let (mut v, mut digits, mut scale) = (0.0f64, 0usize, 1.0f64);
        let mut point = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                if digits < MAX_DIGITS {
                    if point {
                        scale /= 10.0;
                        v += f64::from(c - b'0') * scale;
                    } else {
                        v = v * 10.0 + f64::from(c - b'0');
                    }
                } else if !point {
                    // Past the digits that count, an integer part keeps its size.
                    v *= 10.0;
                }
                digits += 1;
            } else if c == b'.' && !point {
                point = true;
            } else {
                break;
            }
            self.at += 1;
        }
        let v = v.min(f64::MAX);
        (digits > 0).then_some(if neg { -v } else { v })
    }

    /// The next numeric parameter of the instruction being read, or `None` at its end (a semicolon
    /// ending it is read).
    pub fn number(&mut self) -> Option<f64> {
        loop {
            let c = self.peek()?;
            if separator(c) {
                self.at += 1;
            } else if c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.') {
                if let Some(v) = self.number_here() {
                    return Some(v);
                }
            } else {
                if c == b';' {
                    self.at += 1;
                }
                return None;
            }
        }
    }

    /// Up to `out.len()` numeric parameters into `out` (the rest are read and dropped) → how many.
    pub fn numbers(&mut self, out: &mut [f64]) -> usize {
        let mut n = 0;
        while let Some(v) = self.number() {
            if let Some(slot) = out.get_mut(n) {
                *slot = v;
            }
            n += 1;
        }
        n.min(out.len())
    }

    /// Read and drop the parameters of the instruction being read: numbers and quoted strings.
    pub fn params(&mut self) {
        loop {
            while self.number().is_some() {}
            if self.peek() != Some(b'"') {
                return;
            }
            self.quoted();
        }
    }

    /// Read and drop numeric parameters → whether a semicolon ended them (an instruction the reader
    /// doesn't know, written out in full).
    pub fn closed_params(&mut self) -> bool {
        while self.number().is_some() {}
        self.at > 0 && self.b.get(self.at - 1) == Some(&b';')
    }

    /// A comment (CO): its quoted string or nothing, then the terminator or the end → whether it
    /// is one.
    pub fn comment(&mut self) -> bool {
        while self.peek().is_some_and(separator) {
            self.at += 1;
        }
        match self.peek() {
            Some(b'"') => self.quoted(),
            Some(b';') => {
                self.at += 1;
                true
            }
            None => true,
            Some(_) => false,
        }
    }

    /// The text of a label, up to and including `terminator` (or the end).
    pub fn label(&mut self, terminator: u8) {
        let rest = &self.b[self.at..];
        self.at += rest.iter().position(|&c| c == terminator).map_or(rest.len(), |n| n + 1);
    }

    /// A quoted string (CO): the parameters up to its closing quote, then the terminator → whether
    /// there was one.
    pub fn quoted(&mut self) -> bool {
        while self.peek().is_some_and(separator) {
            self.at += 1;
        }
        let found = self.peek() == Some(b'"');
        if found {
            self.at += 1;
            let rest = &self.b[self.at..];
            self.at += rest.iter().position(|&c| c == b'"').map_or(rest.len(), |n| n + 1);
        }
        if self.peek() == Some(b';') {
            self.at += 1;
        }
        found
    }

    /// The character right after the mnemonic (DT, SM), `None` when it is a semicolon or the end
    /// (the semicolon is read).
    pub fn char(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.at += 1;
        (c != b';').then_some(c)
    }

    /// The next item of a PE instruction, `None` at its end (its semicolon is read).
    pub fn pe(&mut self, st: &mut Pe) -> Option<PeItem> {
        loop {
            let raw = self.peek()?;
            let c = raw & 0x7f;
            // Spaces, deletes, control characters and bytes 128 to 160 and 255 are ignored.
            if raw <= 32 || raw == 127 || (128..=160).contains(&raw) || raw == 255 {
                self.at += 1;
                continue;
            }
            match c {
                b';' => {
                    self.at += 1;
                    return None;
                }
                // Flags keep their meaning with the eighth bit set; digits are 63 and up (191 and up).
                b':' | b'<' | b'>' | b'=' | b'7' | b'9' if raw < 63 || (0xb7..191).contains(&raw) => {
                    self.at += 1;
                    match c {
                        b':' => return self.pe_value(st).map(PeItem::Pen).or_else(|| self.end_pe()),
                        b'<' => st.up = true,
                        b'>' => match self.pe_value(st) {
                            Some(n) => st.fraction_bits = n.clamp(0, 52) as u32,
                            None => return self.end_pe(),
                        },
                        b'=' => st.absolute = true,
                        b'7' => st.seven_bit = true,
                        _ => return Some(PeItem::Rect),
                    }
                }
                _ => {
                    let scale = 0.5f64.powi(st.fraction_bits as i32);
                    let (Some(x), Some(y)) = (self.pe_value(st), self.pe_value(st)) else { return self.end_pe() };
                    let item = PeItem::Point { x: x as f64 * scale, y: y as f64 * scale, up: st.up, absolute: st.absolute };
                    (st.up, st.absolute) = (false, false);
                    return Some(item);
                }
            }
        }
    }

    /// The rest of a PE instruction that can't be decoded: see [`Lexer::skip_pe`].
    fn end_pe(&mut self) -> Option<PeItem> {
        self.skip_pe();
        None
    }

    /// The data of a PE instruction, up to and including its semicolon, unread: its data is never
    /// read as instructions.
    pub fn skip_pe(&mut self) {
        let rest = &self.b[self.at..];
        self.at += rest.iter().position(|&c| c == b';').map_or(rest.len(), |n| n + 1);
    }

    /// One encoded PE number, `None` when the instruction ends inside it (or a byte that is no
    /// digit comes first, which is read).
    fn pe_value(&mut self, st: &Pe) -> Option<i64> {
        let (base, low, high): (u64, std::ops::RangeInclusive<u8>, std::ops::RangeInclusive<u8>) =
            if st.seven_bit { (32, 63..=94, 95..=126) } else { (64, 63..=126, 191..=254) };
        let (mut v, mut place) = (0u64, 1u64);
        loop {
            let raw = self.peek()?;
            let c = if st.seven_bit { raw & 0x7f } else { raw };
            if raw <= 32 || raw == 127 || (128..=160).contains(&raw) || raw == 255 {
                self.at += 1;
                continue;
            }
            if c == b';' {
                return None;
            }
            self.at += 1;
            let (digit, last) = if low.contains(&c) {
                (u64::from(c - 63), false)
            } else if high.contains(&c) {
                (u64::from(c - high.start()), true)
            } else {
                return None;
            };
            v = v.saturating_add(digit.saturating_mul(place));
            place = place.saturating_mul(base);
            if last {
                // The low bit is the sign.
                let n = i64::try_from(v >> 1).unwrap_or(i64::MAX);
                return Some(if v & 1 == 1 { -n } else { n });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pjl_comments_do_not_select_a_language() {
        for command in ["COMMENT LANGUAGE=PCL", "ECHO ENTER LANGUAGE=PCL", "SET LANGUAGE=PCL"] {
            let bytes = format!("\x1b%-12345X@PJL {command}\n@PJL ENTER LANGUAGE=HPGL2\nIN;");
            assert_eq!(Lexer::new(bytes.as_bytes()).op(true), Some(*b"IN"));
        }
        assert_eq!(Lexer::new(b"\x1b%-12345X@pjl\tenter\tlanguage=HPGL2\r\nIN;").op(true), Some(*b"IN"));
    }

    #[test]
    fn pjl_language_separator_accepts_horizontal_tabs() {
        assert_eq!(Lexer::new(b"\x1b%-12345X@PJL ENTER LANGUAGE\t=\tHPGL2\nIN;").op(true), Some(*b"IN"));
    }

    #[test]
    fn framing_labels_and_bounded_numbers() {
        let mut lex = Lexer::new(b"\x1b%-12345X@PJL ENTER LANGUAGE=HPGL2\nIN;LBPU999;\x03PA1,2,3,4;SP2;");
        assert_eq!(lex.op(true), Some(*b"IN"));
        lex.params();
        assert_eq!(lex.op(true), Some(*b"LB"));
        lex.label(ETX);
        assert_eq!(lex.op(true), Some(*b"PA"));
        let mut values = [0.0; 2];
        assert_eq!(lex.numbers(&mut values), 2);
        assert_eq!(values, [1.0, 2.0]);
        assert_eq!(lex.op(true), Some(*b"SP"));
        assert_eq!(lex.number(), Some(2.0));
    }

    #[test]
    fn encoded_flags_and_truncated_pair_recover_at_semicolon() {
        let mut lex = Lexer::new(b"PE<=\xc1\xc3\xc5;PU;");
        assert_eq!(lex.op(true), Some(*b"PE"));
        let mut state = Pe::default();
        assert_eq!(lex.pe(&mut state), Some(PeItem::Point { x: 1.0, y: 2.0, up: true, absolute: true }));
        assert_eq!(lex.pe(&mut state), None);
        assert_eq!(lex.op(true), Some(*b"PU"));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn short_encoded_payloads_terminate_with_finite_coordinates() {
        for value in 0..=u16::MAX {
            let [a, b] = value.to_be_bytes();
            let bytes = [a, b, b';'];
            let mut lex = Lexer::new(&bytes);
            let mut state = Pe::default();
            let mut ended = false;
            for _ in 0..=bytes.len() {
                let before = lex.pos();
                let Some(item) = lex.pe(&mut state) else {
                    ended = true;
                    break;
                };
                assert!(lex.pos() > before);
                assert!(lex.pos() <= bytes.len());
                if let PeItem::Point { x, y, .. } = item {
                    assert!(x.is_finite() && y.is_finite());
                }
            }
            assert!(ended);
        }
    }

    #[test]
    fn pcl_payloads_are_never_interpreted_as_plot_commands() {
        for escape in [b"\x1b*b6W".as_slice(), b"\x1b*b6V", b"\x1b&p6X"] {
            let mut bytes = escape.to_vec();
            bytes.extend(b"PD9,9;\x1b%0BIN;");
            let mut lex = Lexer::new(&bytes);
            assert_eq!(lex.op(true), Some(*b"IN"));
            assert_eq!(lex.raster, escape.starts_with(b"\x1b*b"));
        }
        for prefix in [
            b"\x1bEgarbage\x1b%0B".as_slice(),
            b"\x1b(8U",
            b"\x1b.r12,3:",
            b"\x1b.Y",
            b"\x1b*z1a2B",
            b"\x1b0",
            b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL\nignored\x1b%0B",
        ] {
            let mut bytes = prefix.to_vec();
            bytes.extend(b"IN;");
            assert_eq!(Lexer::new(&bytes).op(true), Some(*b"IN"));
        }
        let mut raster = Lexer::new(b"\x1b*r1AIN;");
        assert_eq!(raster.op(true), Some(*b"IN"));
        assert!(raster.raster);
        for bytes in [b"\x1b".as_slice(), b"\x1b.", b"\x1b*b", b"\x1b*b999999999999Wshort"] {
            let mut lex = Lexer::new(bytes);
            assert_eq!(lex.op(false), None);
            assert!(lex.done());
        }
    }

    #[test]
    fn malformed_parameters_comments_and_label_delimiters_preserve_boundaries() {
        let mut lex = Lexer::new(b" +-.12,999999999999999999999999999999;PU;");
        assert_eq!(lex.number(), Some(-(0.1 + 0.02)));
        assert!(lex.number().is_some_and(f64::is_finite));
        assert_eq!(lex.number(), None);
        assert_eq!(lex.op(true), Some(*b"PU"));
        for (bytes, closed) in [(b"1,2;".as_slice(), true), (b"1,2PU", false)] {
            assert_eq!(Lexer::new(bytes).closed_params(), closed);
        }
        for (bytes, valid) in [(b" \"PD9,9;\";".as_slice(), true), (b";", true), (b"", true), (b"unquoted", false)] {
            assert_eq!(Lexer::new(bytes).comment(), valid);
        }
        let mut lex = Lexer::new(b"1,2\"label\";SP3;");
        lex.params();
        assert_eq!(lex.op(true), Some(*b"SP"));
        assert!(!Lexer::new(b"bad").quoted());
        assert_eq!(Lexer::new(b";").char(), None);
        assert_eq!(Lexer::new(b"@").char(), Some(b'@'));
        assert_eq!(Lexer::new(b"1IN;").op(true), None);
        assert_eq!(Lexer::new(b"1IN;").op(false), Some(*b"IN"));
        let mut lex = Lexer::new(b"unterminated");
        lex.label(ETX);
        assert!(lex.done());
    }
}
