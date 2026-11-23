//! Borrow compressed input instead of accumulating a second copy across empty members.
use std::io::{self, BufRead, Read};

use super::{CHUNK, Input};

pub(super) struct Reader<'a> {
    input: Input<'a>,
    decoder: crabz2::BlockDecoder,
    window: usize,
    block_window: usize,
    output: Vec<u8>,
    position: usize,
    done: bool,
}

impl<'a> Reader<'a> {
    pub(super) fn new(input: Input<'a>) -> Self {
        Self { input, decoder: crabz2::BlockDecoder::new(), window: 0, block_window: CHUNK, output: Vec::new(), position: 0, done: false }
    }

    /// Restart only the uncommitted block with a larger borrowed window. Commit/rebase
    /// after every attempt, including empty-member EOF and truncation boundaries, so
    /// neither input storage nor cancellation latency grows with the member chain.
    fn refill(&mut self) -> io::Result<bool> {
        while !self.done {
            self.input.state.check().map_err(io::Error::other)?;
            self.output.clear();
            self.position = 0;
            // BZh's grade gives the nominal block size. It is only a window
            // hint: some compressed blocks need more bytes, so keep the retry.
            if let Some(&grade @ b'1'..=b'9') = self.input.remaining.strip_prefix(b"BZh").and_then(|header| header.first()) {
                self.block_window = usize::from(grade - b'0') * 100_000;
            }
            let available = self.window.max(self.block_window).min(self.input.remaining.len());
            let bytes = self.input.remaining.get(..available).ok_or_else(|| io::Error::other("invalid bzip2 input window"))?;
            let step = self.decoder.next_block(bytes, &mut self.output);
            let consumed = self.decoder.consumed();
            if consumed > available {
                return Err(io::Error::other("invalid bzip2 committed input range"));
            }
            // rebase's contract requires n <= consumed(); the exact committed count
            // above satisfies it even when the next partial member needs more bytes.
            self.decoder.rebase(consumed);
            self.input.consume(consumed);
            self.window = available.saturating_sub(consumed);
            match step {
                Ok(crabz2::Step::Block) if !self.output.is_empty() => return Ok(true),
                Ok(crabz2::Step::Block) => {}
                Ok(crabz2::Step::Eof) if self.input.remaining.is_empty() => self.done = true,
                Ok(crabz2::Step::Eof) => {}
                Err(crabz2::Error::Truncated) if self.window < self.input.remaining.len() => {}
                Err(error) => return Err(error.into()),
            }
            // Doubling uncommitted input avoids repeatedly decoding a large block
            // in tiny increments; every byte still borrows the admitted source slice.
            self.window = self.window.saturating_add(available.max(CHUNK)).min(self.input.remaining.len());
        }
        Ok(false)
    }
}

impl Read for Reader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || (self.position >= self.output.len() && !self.refill()?) {
            return Ok(0);
        }
        let count = output.len().min(self.output.len().saturating_sub(self.position));
        let end = self.position.saturating_add(count);
        let bytes = self.output.get(self.position..end).ok_or_else(|| io::Error::other("invalid bzip2 output range"))?;
        output.get_mut(..count).ok_or_else(|| io::Error::other("invalid bzip2 destination range"))?.copy_from_slice(bytes);
        self.position = end;
        Ok(count)
    }
}
