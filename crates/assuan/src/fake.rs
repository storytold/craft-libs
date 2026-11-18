//! A fake gpg-agent for the tests: an in-memory stream that answers each line the client writes
//! from a script, and a plain canned stream.

use std::collections::VecDeque;
use std::io::{Read, Result, Write};

/// A scripted agent: each line the client sends is matched with the next step of the script
/// (exactly, or by its start when the step ends in `*`), and the step's answer is then what the
/// client reads. A line that doesn't match is answered with an error and noted in `wrong`.
pub(crate) struct FakeAgent {
    steps: VecDeque<(String, Vec<u8>)>,
    out: VecDeque<u8>,
    partial: Vec<u8>,
    /// Every line the client sent, in order.
    pub(crate) sent: Vec<String>,
    /// The lines that weren't what the script expected.
    pub(crate) wrong: Vec<String>,
    /// Hand reads out a few bytes at a time.
    trickle: usize,
    /// Set by a test's owner-wait hook: whether the client asked for a long wait.
    pub(crate) long_wait: bool,
    /// Each line sent with whether a long wait was on when it was sent.
    pub(crate) waits: Vec<(String, bool)>,
}

impl FakeAgent {
    /// An agent that greets with `OK Pleased to meet you`.
    pub(crate) fn new() -> FakeAgent {
        FakeAgent::greeting(b"OK Pleased to meet you\n")
    }

    pub(crate) fn greeting(greeting: &[u8]) -> FakeAgent {
        FakeAgent {
            steps: VecDeque::new(),
            out: greeting.iter().copied().collect(),
            partial: Vec::new(),
            sent: Vec::new(),
            wrong: Vec::new(),
            trickle: 0,
            long_wait: false,
            waits: Vec::new(),
        }
    }

    /// When the client sends `line`, answer `reply`.
    pub(crate) fn on(mut self, line: &str, reply: impl AsRef<[u8]>) -> FakeAgent {
        self.steps.push_back((line.to_string(), reply.as_ref().to_vec()));
        self
    }

    /// Whether every step was used and every line matched.
    pub(crate) fn done(&self) -> bool {
        self.steps.is_empty() && self.wrong.is_empty()
    }
}

impl Read for FakeAgent {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.trickle += 1;
        let n = buf.len().min(self.out.len()).min([5, 1, 64, 3, 1000, 7][self.trickle % 6]);
        for slot in buf.iter_mut().take(n) {
            *slot = self.out.pop_front().unwrap_or(0);
        }
        Ok(n)
    }
}

impl Write for FakeAgent {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.partial.extend_from_slice(buf);
        while let Some(i) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            let matches = self.steps.front().is_some_and(|(want, _)| match want.strip_suffix('*') {
                Some(start) => line.starts_with(start),
                None => line == *want,
            });
            if (line == "BYE" || line == "RESET") && !matches {
                // Goodbye and RESET need no step of their own (`sent` still shows them).
                self.out.extend(b"OK\n");
            } else if matches {
                let (_, reply) = self.steps.pop_front().unwrap();
                self.out.extend(reply);
            } else {
                self.wrong.push(line.clone());
                self.out.extend(b"ERR 67109139 Unknown IPC command <User defined source 1>\n");
            }
            self.waits.push((line.clone(), self.long_wait));
            self.sent.push(line);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A stream that hands out fixed bytes and keeps what is written.
pub(crate) struct Canned {
    input: Vec<u8>,
    pos: usize,
    pub(crate) written: Vec<u8>,
}

impl Canned {
    pub(crate) fn new(input: impl AsRef<[u8]>) -> Canned {
        Canned { input: input.as_ref().to_vec(), pos: 0, written: Vec::new() }
    }
}

impl Read for Canned {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let left = &self.input[self.pos.min(self.input.len())..];
        let n = left.len().min(buf.len());
        buf[..n].copy_from_slice(&left[..n]);
        self.pos += n;
        Ok(n)
    }
}

impl Write for Canned {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
