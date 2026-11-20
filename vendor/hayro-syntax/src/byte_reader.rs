//! A byte reader.

use alloc::vec::Vec;
use core::cell::Cell;
use core::ops::Range;

/// Direct arrays and dictionaries may nest at most 64 levels, including in decoded streams.
pub(crate) const MAX_CONTAINER_DEPTH: usize = 64;

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// The underlying data of the reader.
    pub data: &'a [u8],
    /// The current byte-offset.
    pub offset: usize,
    /// PdfCraft patch: set when a read needed a byte past the end of `data`. A decision made from
    /// such a read could change if more data followed (see `content::UntypedIter`).
    past_end: Cell<bool>,
    // PdfCraft patch: bound direct-object nesting while skipping arrays and dictionaries
    // and while parsing dictionaries (`Dict::skip`, `Array::skip`, `parse_dict_with`).
    skip_depth: usize,
    // PdfCraft #5320: distinguish depth refusal from malformed input.
    depth_refused: bool,
    control: Option<crate::cancel::Control>,
    next_check: Cell<usize>,
}

impl<'a> Reader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self::new_with(data, 0)
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self { data, offset, past_end: Cell::new(false), skip_depth: 0, depth_refused: false, control: None, next_check: Cell::new(0) }
    }

    pub(crate) fn set_control(&mut self, control: Option<crate::cancel::Control>) {
        if self.control.is_none() {
            self.control = control;
        }
    }

    pub(crate) fn control(&self) -> Option<&crate::cancel::Control> {
        self.control.as_ref()
    }

    fn checkpoint(&self) -> Option<()> {
        let next = self.next_check.get();
        if self.control.is_some() && (self.offset >= next || self.offset < next.saturating_sub(64 * 1024)) {
            crate::cancel::check(self.control.as_ref())?;
            self.next_check.set(self.offset.saturating_add(64 * 1024));
        }
        Some(())
    }

    /// PdfCraft patch: whether a read needed bytes past the end of the data since the flag was
    /// last cleared.
    #[inline]
    pub(crate) fn looked_past_end(&self) -> bool {
        self.past_end.get()
    }

    /// PdfCraft patch: clears the flag set by [`Reader::looked_past_end`].
    #[inline]
    pub(crate) fn clear_looked_past_end(&self) {
        self.past_end.set(false);
    }

    pub(crate) fn enter_container(&mut self) -> Option<()> {
        if self.skip_depth >= MAX_CONTAINER_DEPTH {
            self.depth_refused = true;
            return None;
        }
        self.skip_depth += 1;
        Some(())
    }

    pub(crate) fn leave_container(&mut self) {
        self.skip_depth = self.skip_depth.saturating_sub(1);
    }

    /// PdfCraft patch (29): whether `enter_container` refused a level since the last call, and
    /// clears it.
    pub(crate) fn take_depth_refused(&mut self) -> bool {
        core::mem::take(&mut self.depth_refused)
    }

    /// PdfCraft patch (29): marks a refused level again (see `take_depth_refused`).
    pub(crate) fn set_depth_refused(&mut self, refused: bool) {
        self.depth_refused = refused;
    }

    /// PdfCraft patch (29): whether the reader is inside exactly one array or dictionary, the
    /// outermost one of the object it reads.
    pub(crate) fn in_outermost_container(&self) -> bool {
        self.skip_depth == 1
    }

    /// PdfCraft patch (29): steps over the direct array or dictionary at the offset without
    /// parsing it, in one pass and without recursion, however deeply it nests: each `]` must
    /// close an array and each `>>` a dictionary, and strings and comments are stepped over so
    /// the delimiters in them don't count. Nothing in it is read, so nothing can be reached
    /// through it. Returns `None`, with the offset where it was, when no array or dictionary
    /// starts here, a closing delimiter doesn't match its opener, a hexadecimal string holds
    /// something other than hexadecimal digits and whitespace, or the data ends before it
    /// closes.
    pub(crate) fn skip_unparsed_container(&mut self) -> Option<()> {
        let start = self.offset;
        let skipped = self.skip_unparsed_container_inner();
        if skipped.is_none() {
            self.offset = start;
        }
        skipped
    }

    fn skip_unparsed_container_inner(&mut self) -> Option<()> {
        if self.peek_tag(b"<<").is_none() && self.peek_tag(b"[").is_none() {
            return None;
        }
        // What each open level is, one bit per level (set for a dictionary): at most one word
        // for every 64 bytes of the data, since every level takes at least one byte.
        let mut kinds: Vec<u64> = Vec::new();
        let mut depth = 0usize;
        loop {
            let closes_dict = match self.read_byte()? {
                b'[' => {
                    open_level(&mut kinds, &mut depth, false)?;
                    continue;
                }
                b'<' if self.peek_byte() == Some(b'<') => {
                    self.forward();
                    open_level(&mut kinds, &mut depth, true)?;
                    continue;
                }
                // A hexadecimal string: it ends at the first `>` and holds only what
                // `String::skip` accepts, hexadecimal digits and whitespace.
                b'<' => {
                    loop {
                        let byte = self.read_byte()?;
                        if byte == b'>' {
                            break;
                        }
                        if !byte.is_ascii_hexdigit() && !crate::trivia::is_white_space_character(byte) {
                            return None;
                        }
                    }
                    continue;
                }
                b']' => false,
                b'>' if self.peek_byte() == Some(b'>') => {
                    self.forward();
                    true
                }
                b')' | b'>' => return None,
                // A literal string, with its balanced parentheses and escapes.
                b'(' => {
                    let mut open = 1usize;
                    while open > 0 {
                        match self.read_byte()? {
                            b'\\' => {
                                self.read_byte()?;
                            }
                            b'(' => open = open.checked_add(1)?,
                            b')' => open -= 1,
                            _ => {}
                        }
                    }
                    continue;
                }
                // A comment runs to the end of its line.
                b'%' => {
                    while !matches!(self.read_byte()?, b'\n' | b'\r') {}
                    continue;
                }
                _ => continue,
            };
            depth = depth.checked_sub(1)?;
            let is_dict = kinds.get(depth / 64).is_some_and(|w| (w >> (depth % 64)) & 1 == 1);
            if is_dict != closes_dict {
                return None;
            }
            if depth == 0 {
                return Some(());
            }
        }
    }

    /// Returns `true` if the reader has reached the end of the data.
    #[inline]
    pub fn at_end(&self) -> bool {
        if self.checkpoint().is_none() {
            return true;
        }
        let at_end = self.offset >= self.data.len();
        if at_end {
            // PdfCraft patch: a parser that stops at the end decides from it (see `past_end`).
            self.past_end.set(true);
        }

        at_end
    }

    /// PdfCraft patch: carries over whether `other`, a clone of this reader, looked past the end.
    #[inline]
    pub(crate) fn absorb_past_end(&self, other: &Reader<'_>) {
        if other.past_end.get() {
            self.past_end.set(true);
        }
    }

    /// Moves the reader offset to the end of the data.
    #[inline]
    pub fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    /// Moves the reader to the specified offset.
    #[inline]
    pub fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    /// Returns the remaining data from the current offset to the end.
    #[inline]
    pub fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Returns the total length of the underlying data.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the underlying data is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns a slice of the data for the specified range.
    #[inline]
    pub fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range)
    }

    /// Returns the current offset of the reader.
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Reads the specified number of bytes and advances the offset.
    #[inline]
    pub fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;

        Some(v)
    }

    /// Reads a single byte and advances the offset.
    #[inline]
    pub fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;

        Some(v)
    }

    /// Skips the specified number of bytes by advancing the offset.
    #[inline]
    pub fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    /// Peeks the specified number of bytes.
    #[inline]
    pub fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.checkpoint()?;
        let bytes = self
            .offset
            .checked_add(len)
            .and_then(|end| self.data.get(self.offset..end));
        if bytes.is_none() {
            self.past_end.set(true);
        }

        bytes
    }

    /// Peeks a single byte.
    #[inline]
    pub fn peek_byte(&self) -> Option<u8> {
        self.checkpoint()?;
        let byte = self.data.get(self.offset).copied();
        if byte.is_none() {
            self.past_end.set(true);
        }

        byte
    }

    /// Eat the next byte if it satisfies the condition.
    #[inline]
    pub fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    /// Advances the offset by one byte.
    #[inline]
    pub fn forward(&mut self) {
        self.offset += 1;
    }

    /// Advances the offset by one byte if the current byte satisfies the predicate.
    #[inline]
    pub fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    /// Advances the offset while bytes satisfy the predicate, at least one time.
    #[inline]
    pub fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    /// Advances the offset if the next bytes match the specified tag.
    #[inline]
    pub fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();

        Some(())
    }

    /// Advances the offset while the given byte satisfies the predicate.
    #[inline]
    pub fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    /// Checks if the next bytes match the specified tag.
    #[inline]
    pub fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        let mut cloned = self.clone();

        for b in tag.iter().copied() {
            if cloned.peek_byte() == Some(b) {
                cloned.forward();
            } else {
                self.past_end.set(cloned.past_end.get());
                return None;
            }
        }

        Some(())
    }

    /// Read a u16 integer (in big endian order).
    #[inline]
    pub fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a u32 integer (in big endian order).
    #[inline]
    pub fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 integer (in big endian order).
    #[inline]
    pub fn read_u64(&mut self) -> Option<u64> {
        let bytes = self.read_bytes(8)?;

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

/// PdfCraft patch (29): opens one more level in `Reader::skip_unparsed_container`, a dictionary
/// when `dict`, recording its kind in `kinds` (one bit per level).
fn open_level(kinds: &mut Vec<u64>, depth: &mut usize, dict: bool) -> Option<()> {
    let word = *depth / 64;
    if word == kinds.len() {
        kinds.push(0);
    }
    let bits = kinds.get_mut(word)?;
    let bit = 1u64 << (*depth % 64);
    if dict {
        *bits |= bit;
    } else {
        *bits &= !bit;
    }
    *depth = depth.checked_add(1)?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::Reader;
    use crate::object::{Array, Dict, FromBytes};
    use crate::reader::ReaderExt;
    use alloc::format;
    use alloc::string::String;

    #[test]
    fn a_rewound_reader_keeps_checking_cancellation() {
        let data = alloc::vec![0; 4 * 64 * 1024];
        let (control, polls) = crate::cancel::stop_at(3);
        let mut reader = Reader::new(&data);
        reader.set_control(Some(control));
        reader.jump(2 * 64 * 1024);
        assert_eq!(reader.read_byte(), Some(0));
        reader.jump(0);
        assert_eq!(reader.read_byte(), Some(0));
        reader.jump(64 * 1024);
        assert!(reader.read_byte().is_none());
        assert_eq!(polls.load(core::sync::atomic::Ordering::Relaxed), 3);
        assert_eq!(reader.offset(), 64 * 1024);
    }

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let reader = Reader::new(b"abc");
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }

    #[test]
    fn direct_container_limit_is_sixty_four_and_unwinds_after_failure() {
        for depth in [1, 63, 64, 65, 128, 129] {
            let array = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
            let dictionary = format!("{}0{}", "<< /A ".repeat(depth), " >>".repeat(depth));
            for in_content in [false, true] {
                let mut reader = Reader::new(array.as_bytes());
                assert_eq!(reader.skip::<Array<'_>>(in_content).is_some(), depth <= 64, "array {depth}");
                assert_eq!(reader.skip_depth, 0, "a rejected array must unwind its depth");

                let mut reader = Reader::new(dictionary.as_bytes());
                assert!(reader.skip::<Dict<'_>>(in_content).is_some(), "outer dictionary {depth} survives");
                assert_eq!(reader.skip_depth, 0, "a recovered dictionary must unwind its depth");
                // #5320 changes refusal of the whole dictionary into omission of its whole entry.
                // The cap still prevents traversing any part of that overdeep value.
                let parsed = Dict::from_bytes(dictionary.as_bytes()).expect("the outer dictionary survives");
                assert_eq!(parsed.contains_key("A"), depth <= 64, "dictionary {depth}: whole or absent");
            }
        }
    }

    #[test]
    fn mixed_arrays_and_dictionaries_share_the_same_limit() {
        for depth in [64, 65] {
            let mut data = String::from("0");
            for level in 0..depth {
                data = if level % 2 == 0 { format!("<< /A {data} >>") } else { format!("[{data}]") };
            }
            let mut reader = Reader::new(data.as_bytes());
            // At 65 the outside container is a dictionary, so its entire /A is omitted.
            // At 64 the outside container is an array and the whole value fits.
            assert!(reader.skip::<crate::object::Object<'_>>(true).is_some());
            if depth > 64 {
                let parsed = Dict::from_bytes(data.as_bytes()).expect("outer dictionary survives");
                assert!(!parsed.contains_key("A"), "the mixed value is absent, not truncated at the cap");
            }
            assert_eq!(reader.skip_depth, 0);
        }
    }
}
