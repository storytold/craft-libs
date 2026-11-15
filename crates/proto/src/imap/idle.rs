// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! IDLE (RFC 2177): the server says when a mailbox changes, instead of being asked. The session
//! is in one of two states, and this module only moves it between them: a mailbox is selected
//! (EXAMINE), [`Imap::idle_begin`] puts the server to waiting, [`Imap::idle_poll`] hears what it
//! says (a line a second at most, so that the caller can look at the clock and at whether it was
//! told to stop), and [`Imap::idle_end`] says DONE and reads on to the command's end.
//!
//! The server's word is bounded as everywhere: a line over [`crate::MAX_LINE`] and a line that
//! announces a literal end the session, and the caller counts what it hears.

use std::io::{Read, Write};

use super::Imap;
use super::parse;
use crate::i18n::{Arg, said};
use crate::{ProtoError, for_trace};

/// Most lines [`Imap::idle_end`] reads on the way to the command's end.
const MAX_ENDING: usize = 256;
/// Most quiet reads [`Imap::idle_end`] waits through for one line.
const MAX_QUIET: usize = 30;

/// What the server said while it was waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleEvent {
    /// `* n EXISTS`: the mailbox now has `n` messages.
    Exists(u32),
    /// `* n EXPUNGE`: the message with that sequence number is gone.
    Expunge(u32),
    /// `* n RECENT`: `n` messages are new since the mailbox was opened.
    Recent(u32),
    /// Anything else the server said unasked (a flag change, a status line): not news of mail.
    Other,
    /// The server ended the command itself (`tag OK`): IDLE has to be begun again.
    Ended,
}

/// From the text after `* `: `3 EXISTS`.
fn event_from(rest: &[u8], secrets: &crate::Secrets) -> Result<IdleEvent, ProtoError> {
    let text = String::from_utf8_lossy(rest);
    let mut words = text.split_whitespace();
    let (first, second) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let count = first.parse::<u32>().ok();
    Ok(match (count, second.to_ascii_uppercase().as_str()) {
        (Some(n), "EXISTS") => IdleEvent::Exists(n),
        (Some(n), "EXPUNGE") => IdleEvent::Expunge(n),
        (Some(n), "RECENT") => IdleEvent::Recent(n),
        _ if first.eq_ignore_ascii_case("BYE") => {
            return Err(ProtoError::Io(said!("the server ended the session: {reason}", reason = crate::tidy(&secrets.hide(&text)))));
        }
        _ => IdleEvent::Other,
    })
}

impl<S: Read + Write> Imap<S> {
    /// Send IDLE and wait for the server's go-ahead. The session has to have a mailbox selected;
    /// a server that refuses (NO, BAD) is an error. While idle, the only commands that may follow
    /// are [`Imap::idle_poll`] and [`Imap::idle_end`].
    pub fn idle_begin(&mut self) -> Result<(), ProtoError> {
        if self.idle_tag.is_some() {
            return Err(ProtoError::Protocol(said!("the server is already idling")));
        }
        self.idle_pending.clear();
        let tag = self.tag();
        self.text_seen = 0;
        log::trace!("C: {tag} IDLE");
        self.wire.write(format!("{tag} IDLE\r\n").as_bytes())?;
        self.wire.flush()?;
        self.idle_tag = Some(tag.clone());
        // Untagged lines may come before the `+`; the tagged answer is a refusal.
        for _ in 0..MAX_ENDING {
            match self.read_response()? {
                super::Response::GoAhead => return Ok(()),
                super::Response::Status { tag: Some(answered), line } => {
                    self.idle_tag = None;
                    if answered != tag {
                        return Err(ProtoError::Protocol(said!("the server answered a command that wasn't sent")));
                    }
                    return Err(ProtoError::Rejected(self.refusal(&line)));
                }
                super::Response::Status { tag: None, line } => {
                    if line.status == parse::Status::Bye {
                        self.idle_tag = None;
                        return Err(ProtoError::Io(said!("the server ended the session: {reason}", reason = Arg::said(self.refusal(&line)))));
                    }
                    self.note_status(&line);
                }
                super::Response::Data(values) => {
                    self.note_data(&values);
                    let event = match (values.first().and_then(parse::Value::number), values.get(1).and_then(parse::Value::atom)) {
                        (Some(n), Some(word)) if word.eq_ignore_ascii_case("EXISTS") => IdleEvent::Exists(n),
                        (Some(n), Some(word)) if word.eq_ignore_ascii_case("EXPUNGE") => IdleEvent::Expunge(n),
                        (Some(n), Some(word)) if word.eq_ignore_ascii_case("RECENT") => IdleEvent::Recent(n),
                        _ => IdleEvent::Other,
                    };
                    self.idle_pending.push_back(event);
                }
            }
        }
        self.idle_tag = None;
        Err(ProtoError::Protocol(said!("the server sent too many responses to one command")))
    }

    /// The next thing the server says while idle, or `None` after one read if the line is
    /// incomplete or no bytes arrived (see [`crate::MAX_LINE`] for the bound on a line).
    pub fn idle_poll(&mut self) -> Result<Option<IdleEvent>, ProtoError> {
        let Some(tag) = self.idle_tag.clone() else {
            return Err(ProtoError::Protocol(said!("the server isn't idling")));
        };
        if let Some(event) = self.idle_pending.pop_front() {
            return Ok(Some(event));
        }
        let Some(line) = self.wire.poll_line()? else { return Ok(None) };
        if log::log_enabled!(log::Level::Trace) {
            log::trace!("S: {}", for_trace(&self.secrets.hide(&String::from_utf8_lossy(&line))));
        }
        if let Some(rest) = line.strip_prefix(b"* ") {
            // A literal in what is said while idle is nothing a server has a reason to send.
            if parse::literal_marker(rest).is_some() {
                return Err(ProtoError::Protocol(said!("the server sent data it has no reason to send while idle")));
            }
            return event_from(rest, &self.secrets).map(Some);
        }
        let text = String::from_utf8_lossy(&line);
        if text.strip_prefix(tag.as_str()).is_some_and(|rest| rest.starts_with(' ')) {
            // The server ended IDLE by itself (a timeout of its own, say).
            self.idle_tag = None;
            let status = text.split_ascii_whitespace().nth(1).and_then(parse::Status::parse);
            return match status {
                Some(parse::Status::Ok) => Ok(Some(IdleEvent::Ended)),
                Some(parse::Status::No | parse::Status::Bad) => Err(ProtoError::Rejected(said!("the server didn't end IDLE properly"))),
                _ => Err(ProtoError::Protocol(said!("the server returned an invalid IDLE completion"))),
            };
        }
        Err(ProtoError::Protocol(said!("the server said something that is no news while idle")))
    }

    /// Say DONE and read what the server still has to say, up to the end of the command. What was
    /// said on the way is given back. After this the session is as it was before
    /// [`Imap::idle_begin`].
    pub fn idle_end(&mut self) -> Result<Vec<IdleEvent>, ProtoError> {
        let Some(tag) = self.idle_tag.take() else {
            return Err(ProtoError::Protocol(said!("the server isn't idling")));
        };
        log::trace!("C: DONE");
        self.wire.write(b"DONE\r\n")?;
        self.wire.flush()?;
        let mut events: Vec<_> = self.idle_pending.drain(..).collect();
        for _ in 0..MAX_ENDING {
            // The server may take a moment to answer: a few polls of the stream's own wait.
            let mut polled = None;
            for _ in 0..MAX_QUIET {
                polled = self.wire.poll_line()?;
                if polled.is_some() {
                    break;
                }
            }
            let Some(line) = polled else { return Err(ProtoError::Io(said!("the server took too long to answer"))) };
            if let Some(rest) = line.strip_prefix(b"* ") {
                if parse::literal_marker(rest).is_some() {
                    return Err(ProtoError::Protocol(said!("the server sent data it has no reason to send while idle")));
                }
                events.push(event_from(rest, &self.secrets)?);
                continue;
            }
            let text = String::from_utf8_lossy(&line);
            let mut words = text.splitn(3, ' ');
            let (answered, status) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
            if answered != tag {
                return Err(ProtoError::Protocol(said!("the server answered a command that wasn't sent")));
            }
            return match parse::Status::parse(status) {
                Some(parse::Status::Ok) => Ok(events),
                _ => Err(ProtoError::Rejected(said!("the server didn't end IDLE properly"))),
            };
        }
        Err(ProtoError::Protocol(said!("the server sent too many responses to one command")))
    }

    /// Whether the server said it can do IDLE (upper-cased capability list as last said).
    pub fn offers_idle(&self) -> bool {
        self.offers("IDLE")
    }
}
