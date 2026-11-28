use super::*;
use proptest::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Reads `data` in pieces of the sizes in `sizes`, in turn.
struct Pieces<'a> {
    data: &'a [u8],
    sizes: Vec<usize>,
    turn: usize,
}

impl Read for Pieces<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let size = self.sizes.get(self.turn % self.sizes.len().max(1)).copied().unwrap_or(1).max(1);
        self.turn += 1;
        let n = size.min(out.len()).min(self.data.len());
        let (head, tail) = self.data.split_at(n);
        out.get_mut(..n).unwrap_or_default().copy_from_slice(head);
        self.data = tail;
        Ok(n)
    }
}

/// Counts the bytes `inner` gives.
struct Counting<R> {
    inner: R,
    read: Rc<Cell<usize>>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(out)?;
        self.read.set(self.read.get() + n);
        Ok(n)
    }
}

/// Every token of `src` from the slice lexer, with the offset after it.
fn from_slice(src: &[u8]) -> Vec<(Token, usize)> {
    let mut l = ai_lex::Lexer::new(src);
    std::iter::from_fn(|| l.next_token().map(|t| (Token::from(t), l.offset()))).collect()
}

/// Every token of `src` read in pieces of `sizes`, holding `chunk` bytes more at a time.
fn from_stream(src: &[u8], sizes: Vec<usize>, chunk: usize) -> Vec<(Token, usize)> {
    let mut l = Lexer::new(Pieces { data: src, sizes, turn: 0 });
    l.chunk = chunk.max(1);
    let mut out = vec![];
    loop {
        match l.next_token() {
            Ok(Some(t)) => out.push((t, l.offset())),
            Ok(None) => return out,
            Err(e) => panic!("{e}"),
        }
    }
}

fn assert_agree(src: &[u8]) {
    let want = from_slice(src);
    for (sizes, chunk) in [(vec![1], 1), (vec![1, 7, 2], 3), (vec![5], 64), (vec![1 << 20], CHUNK), (vec![3, 1000, 1], 17)] {
        assert_eq!(from_stream(src, sizes.clone(), chunk), want, "sizes {sizes:?}, chunk {chunk}, input {:?}", String::from_utf8_lossy(src));
    }
}

/// The inputs of the slice lexer's own tests.
const CASES: &[&[u8]] = &[
    b"%AI5_BeginLayer\r\n1 (a\\)b) Ln\r%_/X : ;\n0 %_BS\n<4142>\n",
    b"1 % a note (\n2 %_Bs\n3 %_ 4\n",
    b"%_%44434241\n%_% not hex\n%_%\n1\n",
    b"[1 0 0 1 0 0] XI\r\n%%BeginData: 7\r\nXI\n(%)A%%EndData\r\nXH\r\n",
    b"%_[1 0 0 1 0 0] XI\r%_%%BeginData: 7\rXI\n(U)A%%EndData\r%_XH\ru\r",
    b"%%BeginData: 9 Hex Bytes\r\n%00FF\r\n%%EndData\r\n1\r\n",
    b"%_XI\r%_%%BeginData: 4\r%_XI\r(\x00\x00\x00%_%%EndData\r%_XH\r1\r",
    b"%AI26_BeginPlacedObjectPreview\r\n2 1 (\x00\xff\r\n%AI26_EndPlacedObjectPreview\r\nN\r\n",
    b"%%BeginDocument: a.eps\n(\x00 frobnicate\n%%EndDocument\nN\n",
    b"%AI26_BeginPlacedObjectPreview\r1 (",
    b"%_%%BeginDocument: a.eps\r%_1 0 0 setrgbcolor\r%_%%EndDocument\r%_~\r",
    b"%_%%BeginDocument: a.eps\r%_%%EndDocument\rN\r",
    b"%%BeginDocument: a.eps\n%%BeginDocument: b.eps\n%%EndDocument\n~\n",
    b"%%BeginDocument: a.eps\n%%BeginData: 99999999999\n",
    b"%%BeginDocument: outer.eps\r\n1 0 0 setrgbcolor\r\n%%BeginDocument: inner.eps\r(%%EndDocument) pop\r%%EndDocument\r\n\
      %%BeginData: 14 Binary Bytes\n%%EndDocument\n%%BeginBinary: 14\n%%EndDocument\n%%BeginData: 1 Hex Lines\n%%EndDocument\n%%EndDocument\r\n~\r\n",
    b"(unterminated \\",
    b"<12",
    b"%%BeginData: 99999999999\r\nXI",
    b"%%BeginData: 5\rXI\r",
    b"/",
    b"1e999 nan",
    b"(\\101\\r\\\r\n\\7x)",
    b"%",
    b"%_",
];

#[test]
fn the_slice_lexers_cases_give_the_same_tokens() {
    for src in CASES {
        assert_agree(src);
    }
}

#[test]
fn an_alpha_images_operands_are_found_after_older_bytes_are_dropped() {
    // Filler past several reads, so that what is before the operand line has been dropped.
    let mut src = b"1 ".repeat(5000);
    // 4 x 4 pixels and an alpha count: the 16 alpha bytes follow the 3 counted samples, and hold a
    // marker that `XH` follows. Without the operand line the samples would end at that marker.
    src.extend_from_slice(b"\r\n[1 0 0 1 0 0] 0 0 4 4 4 4 8 3 1 0 0 XI\r\n%%BeginData: 7\r\nXI\nabc%%EndData\r\nXH\r\n.%%EndData\r\nXH\r\nu\r\n");
    let toks = from_slice(&src);
    assert!(toks.iter().any(|(t, _)| t.tok == Tok::Data(b"abc%%EndData\r\nXH\r\n.".to_vec())), "{:?}", toks.last());
    assert_agree(&src);
}

#[test]
fn large_samples_give_the_same_token() {
    // Past the default read size, both with the count right and with an alpha channel after it.
    let samples: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
    for operands in [&b"1 XI"[..], b"[1 0 0 1 0 0] 0 0 400 500 400 500 8 1 1 0 0 XI"] {
        let mut src = operands.to_vec();
        src.extend_from_slice(format!("\r\n%%BeginData: {}\r\nXI\n", samples.len() + 4).as_bytes());
        src.extend_from_slice(&samples);
        src.extend_from_slice(b"\r\n%%EndData\r\nXH\r\n1 Ln\r\n");
        let want = from_slice(&src);
        assert!(want.iter().any(|(t, _)| matches!(&t.tok, Tok::Data(d) if d.len() >= samples.len())));
        assert_eq!(from_stream(&src, vec![4096, 1, 70_000], CHUNK), want);
        assert_eq!(from_stream(&src, vec![1 << 20], CHUNK), want);
    }
}

/// Pieces of editing data, the sections' markers among them, joined in any order.
fn piece() -> impl Strategy<Value = Vec<u8>> {
    let fixed: &[&[u8]] = &[
        b"1",
        b"-3.5",
        b".5",
        b"1e-3",
        b"1e999",
        b"+",
        b"Ln",
        b"XI",
        b"XH",
        b"[",
        b"]",
        b"{",
        b"}",
        b")",
        b">",
        b"u",
        b"~",
        b"/X",
        b"/",
        b"(a\\)b)",
        b"(nest (x) y)",
        b"(\\101\\r\\\r\n)",
        b"(",
        b"<4142>",
        b"<4 1 G>",
        b"<",
        b" ",
        b"\t",
        b"\r",
        b"\n",
        b"\r\n",
        b"\0",
        b"%_",
        b"%_%_",
        b"% comment (",
        b"%AI5_BeginLayer",
        b"%%Note:x",
        b"%_%4443",
        b"%_% not hex",
        b"%AI5_BeginPlace",
        b"%%BeginData: 7\r\n",
        b"%%BeginData: 0\n",
        b"%%BeginData: 3 Hex Bytes\r\n",
        b"%%BeginData: 12\r",
        b"XI\n",
        b"XI\r\n",
        b"%_XI\r",
        b"%%EndData",
        b"%%EndData\r\nXH",
        b"%_%%EndData\r%_XH",
        b"%AI26_BeginPlacedObjectPreview\r\n",
        b"%AI26_EndPlacedObjectPreview",
        b"%%BeginDocument: a.eps\n",
        b"%%EndDocument\n",
        b"%_%%EndDocument\r",
        b"%%BeginData: 5 Binary Bytes\n",
        b"%%BeginBinary: 3\n",
        b"%%BeginData: 1 Hex Lines\n",
        b"%%BeginData: 2 Lines\n",
        b"[1 0 0 1 0 0] 0 0 2 2 2 2 8 3 1 XI\r\n",
        b"%_[1 0 0 1 0 0] 0 0 1 3 1 3 8 1 1 XI\r",
    ];
    prop_oneof![
        8 => proptest::sample::select(fixed).prop_map(<[u8]>::to_vec),
        2 => proptest::collection::vec(any::<u8>(), 0..24),
        1 => (0usize..1500).prop_map(|n| b"1 ".repeat(n)),
        2 => (0usize..40, proptest::collection::vec(any::<u8>(), 0..40), any::<bool>(), any::<bool>()).prop_map(|(n, samples, hidden, end)| {
            let mut s = format!("{}%%BeginData: {n}\r\n{}XI\n", if hidden { "%_" } else { "" }, if hidden { "%_" } else { "" }).into_bytes();
            s.extend_from_slice(&samples);
            if end {
                s.extend_from_slice(if hidden { b"%_%%EndData\r%_XH\r" } else { b"%%EndData\r\nXH\r\n" });
            }
            s
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn tokens_agree_with_the_slice_lexer_for_any_read_sizes(
        pieces in proptest::collection::vec(piece(), 0..24),
        sizes in proptest::collection::vec(1usize..40, 1..6),
        chunk in 1usize..80,
    ) {
        let src = pieces.concat();
        let want = from_slice(&src);
        prop_assert_eq!(from_stream(&src, sizes, chunk), want);
    }
}

#[test]
fn a_token_past_the_limit_is_refused_without_reading_it() {
    const LIMIT: usize = 4096;
    let starts: [(&[u8], u8); 7] = [
        (b"%%BeginData: 99999999999\r\nXI\n", b'x'),
        (b"(", b'x'),
        (b"<", b'1'),
        (b"%AI26_BeginPlacedObjectPreview\r\n", b'x'),
        (b"%%BeginDocument: a.eps\n", b'x'),
        (b"%%BeginDocument: a.eps\n%%BeginData: 99999999999\n", b'\n'),
        (b"%", b'x'),
    ];
    for (start, fill) in starts {
        // A stream that never ends: only the limit stops the token.
        let read = Rc::new(Cell::new(0));
        let reader = Counting { inner: io::Cursor::new(start).chain(io::repeat(fill)), read: read.clone() };
        let mut l = Lexer::with_limit(reader, LIMIT);
        assert!(matches!(l.next_token(), Err(Error::TooLarge { limit: LIMIT })), "{:?}", String::from_utf8_lossy(start));
        assert!(read.get() <= LIMIT, "{} bytes read for {:?}", read.get(), String::from_utf8_lossy(start));
        assert!(l.buf.capacity() <= LIMIT + CHUNK, "{:?}", String::from_utf8_lossy(start));
        // A refused token stays refused.
        assert!(matches!(l.next_token(), Ok(None)));
    }
    // A word too: the limit counts from where the token starts, after what came before it.
    let read = Rc::new(Cell::new(0));
    let reader = Counting { inner: io::Cursor::new(b"1 2 ".repeat(3000)).chain(io::repeat(b'a')), read: read.clone() };
    let toks: Vec<_> = Lexer::with_limit(reader, LIMIT).collect();
    assert_eq!(toks.len(), 6001);
    assert!(matches!(toks.last(), Some(Err(Error::TooLarge { .. }))));
    assert!(read.get() <= 12_000 + LIMIT);
}

#[test]
fn samples_within_the_limit_are_read() {
    let mut src = b"%%BeginData: 1003\r\nXI\n".to_vec();
    src.extend(std::iter::repeat_n(b'x', 1000));
    src.extend_from_slice(b"%%EndData\r\nXH\r\n");
    // The marker that `XH` follows ends the samples without reading on to the end of the window
    // a marker is looked for in (twice the count past it), which the limit would refuse.
    src.extend_from_slice(&b"1 ".repeat(1500));
    for sizes in [vec![1 << 20], vec![1], vec![7, 300]] {
        let reader = Pieces { data: &src, sizes: sizes.clone(), turn: 0 };
        let toks: Vec<_> = Lexer::with_limit(reader, 1100).take(3).map(|t| t.map(|t| t.tok)).collect::<Result<_, _>>().unwrap();
        assert_eq!(toks, [Tok::Data(vec![b'x'; 1000]), Tok::Word(b"XH".to_vec()), Tok::Num(1.0)], "reads of {sizes:?}");
    }
    // The same count with a lower limit is refused.
    assert!(matches!(Lexer::with_limit(&src[..], 900).next_token(), Err(Error::TooLarge { limit: 900 })));
}

#[test]
fn bytes_read_ahead_between_tokens_count_against_the_limit() {
    // The raw read takes in the whole token ahead of it, which is still too large: a string, a
    // word, a hexadecimal string and a comment line.
    let mut one = [0u8; 1];
    for (open, close) in [(&b"("[..], &b")"[..]), (b"", b" "), (b"<", b">"), (b"%", b"\n")] {
        let mut src = [b"\n", open].concat();
        src.extend(std::iter::repeat_n(b'a', 100));
        src.extend_from_slice(close);
        src.extend_from_slice(b"1");
        let mut l = Lexer::with_limit(&src[..], 16);
        assert_eq!(l.raw().read(&mut one).unwrap(), 1);
        assert!(matches!(l.next_token(), Err(Error::TooLarge { limit: 16 })), "{:?}", String::from_utf8_lossy(&src));
    }
    // Tokens within the limit after a raw read are read as usual.
    let mut l = Lexer::with_limit(&b"\n1 Ln 22 (ab)"[..], 4);
    assert_eq!(l.raw().read(&mut one).unwrap(), 1);
    let toks: Vec<_> = l.map(|t| t.map(|t| t.tok)).collect::<Result<_, _>>().unwrap();
    assert_eq!(toks, [Tok::Num(1.0), Tok::Word(b"Ln".to_vec()), Tok::Num(22.0), Tok::Str(b"ab".to_vec())]);
}

#[test]
fn a_failing_reader_ends_with_its_error() {
    struct Broken(usize);
    impl Read for Broken {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                return Err(io::Error::other("unplugged"));
            }
            let n = self.0.min(out.len()).min(1);
            out.get_mut(..n).unwrap_or_default().fill(b'7');
            self.0 -= n;
            Ok(n)
        }
    }
    let mut l = Lexer::new(Broken(3));
    assert!(matches!(l.next_token(), Err(Error::Io(e)) if e.to_string() == "unplugged"));
}

#[test]
fn ascii85_data_is_read_between_tokens() {
    let decoded: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let mut src = b"/Binary : /ASCII85Decode ,\r\n".to_vec();
    src.extend_from_slice(crate::ascii85::encode(&decoded).as_bytes());
    src.extend_from_slice(b"%_; 1 Ln\r\n");
    for sizes in [vec![1], vec![13, 1, 4], vec![1 << 20]] {
        let mut l = Lexer::new(Pieces { data: &src, sizes, turn: 0 });
        l.chunk = 7;
        for _ in 0..4 {
            assert!(matches!(l.next_token(), Ok(Some(_))));
        }
        let mut out = vec![];
        crate::ascii85::Decoder::new(l.raw(), decoded.len()).read_to_end(&mut out).unwrap_or_default();
        assert_eq!(out, decoded);
        let rest: Vec<_> = l.map(|t| t.map(|t| t.tok)).collect::<Result<_, _>>().unwrap_or_default();
        assert_eq!(rest, [Tok::Word(b";".to_vec()), Tok::Num(1.0), Tok::Word(b"Ln".to_vec())]);
    }
    // The slice lexer's way with the same data: skip past the end.
    let mut l = Lexer::new(&b"/Binary : /ASCII85Decode ,\r\n%_(abc[%\r\n%x~>\r\n%_; 1"[..]);
    for _ in 0..4 {
        assert!(matches!(l.next_token(), Ok(Some(_))));
    }
    assert_eq!(l.skip_past(b"~>").unwrap_or_default(), b"\r\n%_(abc[%\r\n%x~>");
    assert_eq!(l.next_token().ok().flatten().map(|t| t.tok), Some(Tok::Word(b";".to_vec())));
}
