//! mbox files (RFC 4155, in the `mboxrd` flavour): messages one after another, each after a
//! `From ` line.

use crate::date::DateTime;
use crate::header;

/// The messages of an mbox file, one at a time, each as the bytes of one message (for
/// [`crate::parse`]).
///
/// A message starts at a line that begins with `From ` and either follows an empty line (or
/// starts the file) or holds a time (`12:08:34`), as the separator lines of every mbox writer
/// do. In the messages `>From ` lines lose one `>` (the `mboxrd` rule, which also reads `mboxo`
/// files), and the empty line that ended a message in the file is not part of it. Bytes before
/// the first separator are a message too, so a lone `.eml` gives one message. A message that is
/// nothing but white space is skipped.
///
/// Linear in the input; a caller that takes only so many messages stops the iterator early.
pub fn mbox_messages(bytes: &[u8]) -> MboxMessages<'_> {
    MboxMessages { bytes, pos: 0, current: None, previous_blank: true, done: false }
}

/// The iterator behind [`mbox_messages`].
#[derive(Clone, Debug)]
pub struct MboxMessages<'a> {
    bytes: &'a [u8],
    pos: usize,
    current: Option<Vec<u8>>,
    previous_blank: bool,
    done: bool,
}

impl Iterator for MboxMessages<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        while !self.done {
            if self.pos >= self.bytes.len() {
                self.done = true;
                if let Some(m) = finish(self.current.take()) {
                    return Some(m);
                }
                break;
            }
            let (line, next) = header::line_at(self.bytes, self.pos);
            let mut out = None;
            if line.starts_with(b"From ") && (self.previous_blank || has_time(line)) {
                out = finish(self.current.take());
                self.current = Some(Vec::new());
            } else {
                let whole = self.bytes.get(self.pos..next).unwrap_or_default();
                // `>From ` was `From ` before it was stored; `>>From ` was `>From `.
                let quoted =
                    line.iter().position(|b| *b != b'>').is_some_and(|i| i > 0 && line.get(i..).is_some_and(|rest| rest.starts_with(b"From ")));
                let unquoted = if quoted { whole.get(1..).unwrap_or_default() } else { whole };
                match &mut self.current {
                    Some(message) => message.extend_from_slice(unquoted),
                    // Before any separator: blank lines are nothing, anything else is a message.
                    None if line.is_empty() => {}
                    None => self.current = Some(unquoted.to_vec()),
                }
            }
            self.previous_blank = line.is_empty();
            self.pos = next;
            if out.is_some() {
                return out;
            }
        }
        None
    }
}

/// A message cut out of an mbox, without the empty line that ended it there.
fn finish(message: Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut bytes = message?;
    if bytes.ends_with(b"\n\r\n") {
        bytes.truncate(bytes.len().saturating_sub(2));
    } else if bytes.ends_with(b"\n\n") {
        bytes.truncate(bytes.len().saturating_sub(1));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) { None } else { Some(bytes) }
}

/// Whether a line holds a clock time (`hh:mm`), as mbox separator lines do.
fn has_time(line: &[u8]) -> bool {
    line.windows(5).any(|w| matches!(w, [a, b, b':', c, d] if a.is_ascii_digit() && b.is_ascii_digit() && c.is_ascii_digit() && d.is_ascii_digit()))
}

/// Add one message to an mbox file.
///
/// `message` (CRLF or LF line ends, kept as they are so that attachments survive byte for byte)
/// goes after a `From sender date` line and before an empty line. Lines of it that begin with
/// `From `, or with `>From `, get one more `>` (the `mboxrd` rule), which [`mbox_messages`]
/// takes off again.
///
/// `sender` loses its white space and control characters, so that it can't end the separator
/// line, and is `MAILER-DAEMON` when nothing is left. `date` is written as it is given (mbox
/// files have it in UTC); one that doesn't exist is written as the start of 1970.
pub fn write_mbox_message(out: &mut Vec<u8>, sender: &str, date: &DateTime, message: &[u8]) {
    let sender: String = sender.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    let sender = if sender.is_empty() { "MAILER-DAEMON" } else { sender.as_str() };
    let stamp = date.asctime().unwrap_or_else(|| "Thu Jan  1 00:00:00 1970".to_string());
    out.extend_from_slice(format!("From {sender} {stamp}\r\n").as_bytes());
    let mut pos = 0;
    while pos < message.len() {
        let (line, next) = header::line_at(message, pos);
        let text = line.iter().position(|b| *b != b'>').and_then(|i| line.get(i..)).unwrap_or_default();
        if text.starts_with(b"From ") {
            out.push(b'>');
        }
        out.extend_from_slice(message.get(pos..next).unwrap_or_default());
        pos = next;
    }
    if !out.ends_with(b"\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> Vec<String> {
        mbox_messages(bytes).map(|m| String::from_utf8_lossy(&m).into_owned()).collect()
    }

    const DATE: DateTime = DateTime { year: 2003, month: 7, day: 1, hour: 8, minute: 52, second: 37, offset_minutes: 0 };

    #[test]
    fn reads_a_unix_mbox() {
        let mbox = b"From ada@example.org Tue Jul  1 08:52:37 2003\n\
Subject: One\n\
Status: RO\n\
\n\
First body\n\
>From the start, quoted\n\
>>From twice\n\
> From not quoted\n\
\n\
From bob@example.org Wed Jul  2 09:00:00 2003\n\
Subject: Two\n\
\n\
Second body\n";
        assert_eq!(
            read(mbox),
            ["Subject: One\nStatus: RO\n\nFirst body\nFrom the start, quoted\n>From twice\n> From not quoted\n", "Subject: Two\n\nSecond body\n"]
        );
    }

    #[test]
    fn separators_need_a_blank_line_or_a_time() {
        // No blank line between the messages, but the separator is dated.
        let tight =
            b"From a@example.org Tue Jul  1 08:52:37 2003\nSubject: One\n\nx\nFrom b@example.org Wed Jul  2 09:00:00 2003\nSubject: Two\n\ny\n";
        assert_eq!(read(tight), ["Subject: One\n\nx\n", "Subject: Two\n\ny\n"]);
        // A body line that starts with "From " in the middle of a paragraph is text.
        let prose = b"From a@example.org Tue Jul  1 08:52:37 2003\nSubject: One\n\nHello\nFrom now on we meet on Fridays\n";
        assert_eq!(read(prose), ["Subject: One\n\nHello\nFrom now on we meet on Fridays\n"]);
    }

    #[test]
    fn empty_and_odd_files() {
        assert!(read(b"").is_empty());
        assert!(read(b"\n\n\r\n").is_empty());
        assert!(read(b"From a@example.org Tue Jul  1 08:52:37 2003\n").is_empty());
        assert!(read(b"From \n\nFrom \n\nFrom ").is_empty());
        // A `From ` line in the middle of a paragraph, without a time, is content.
        assert_eq!(read(b"From \nFrom \n\nFrom ").len(), 1);
        // A lone message without a separator.
        assert_eq!(read(b"\nSubject: Lone\n\nbody\n"), ["Subject: Lone\n\nbody\n"]);
        // The iterator stays finished.
        let mut messages = mbox_messages(b"Subject: x\n\ny\n");
        assert!(messages.next().is_some());
        assert!(messages.next().is_none() && messages.next().is_none());
    }

    #[test]
    fn writes_separators_and_quotes_from_lines() {
        let mut out = Vec::new();
        write_mbox_message(&mut out, "ada@example.org", &DATE, b"Subject: Hi\r\n\r\nFrom here\r\n>From there\r\nplain\r\n");
        assert_eq!(
            String::from_utf8_lossy(&out),
            "From ada@example.org Tue Jul  1 08:52:37 2003\r\nSubject: Hi\r\n\r\n>From here\r\n>>From there\r\nplain\r\n\r\n"
        );
        let mut out = Vec::new();
        write_mbox_message(&mut out, " \r\n", &DateTime::default(), b"x");
        assert_eq!(String::from_utf8_lossy(&out), "From MAILER-DAEMON Thu Jan  1 00:00:00 1970\r\nx\r\n\r\n");
    }

    #[test]
    fn a_written_mbox_reads_back() {
        let messages: [&[u8]; 4] = [
            b"Subject: One\r\n\r\nFrom the top\r\n\r\nFrom after a blank line\r\n>From quoted already\r\n",
            "Subject: Two\r\n\r\nGrüße\r\n\r\nFrom München\r\n".as_bytes(),
            b"Subject: bare lf\n\nFrom here\n>>From there\n",
            b"Subject: no body\r\n\r\n",
        ];
        let mut out = Vec::new();
        for message in messages {
            write_mbox_message(&mut out, "ada@example.org", &DATE, message);
        }
        let back: Vec<Vec<u8>> = mbox_messages(&out).collect();
        assert_eq!(back, messages);
    }

    #[test]
    fn hostile_senders_cannot_break_the_separator() {
        let mut out = Vec::new();
        write_mbox_message(&mut out, "a@example.org\r\nFrom evil@example.org Tue Jul  1 08:52:37 2003", &DATE, b"Subject: x\r\n\r\ny\r\n");
        assert_eq!(mbox_messages(&out).count(), 1);
        assert_eq!(out.iter().filter(|b| **b == b'\n').count(), 5);
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_read(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600)) {
            let total: usize = mbox_messages(&data).map(|m| m.len()).sum();
            proptest::prop_assert!(total <= data.len());
        }

        #[test]
        fn from_soup_reads(s in "(From |>From |>|\n|\r\n|a|12:34| |: ){0,60}") {
            let _ = mbox_messages(s.as_bytes()).count();
        }

        #[test]
        fn any_message_written_reads_back(s in "(From |>From |>|\r\n|a|12:34| |: ){0,40}") {
            // A message is not white space alone, and ends its last line.
            let message = format!("X: 1\r\n\r\n{s}\r\n");
            let mut out = Vec::new();
            write_mbox_message(&mut out, "a@example.org", &DATE, message.as_bytes());
            write_mbox_message(&mut out, "b@example.org", &DATE, message.as_bytes());
            let back: Vec<Vec<u8>> = mbox_messages(&out).collect();
            proptest::prop_assert_eq!(back, vec![message.clone().into_bytes(), message.into_bytes()]);
        }
    }
}
