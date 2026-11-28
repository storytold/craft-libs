// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! STATUS (RFC 3501 §6.3.10): what a mailbox holds, asked without opening it, for a program to
//! tell whether it has changed since it was last read (#276). Many mailboxes are asked in one
//! go: the commands are written together and the answers read together (pipelining, RFC 3501
//! §5.5), so a hundred mailboxes cost about one round trip, not a hundred.

use std::io::{Read, Write};

use super::parse::{Status, Value};
use super::{Command, Imap, MAX_QUOTED, Part, Response};
use crate::i18n::said;
use crate::{ProtoError, for_trace};

/// Most mailboxes asked in one go: their commands are a few kilobytes at most, so the server
/// can't be left waiting to be read while we write.
pub const MAX_STATUS_BATCH: usize = 50;

/// What STATUS says of a mailbox: all four counts, or nothing at all (a server that leaves one
/// out gets its mailbox read, never passed over on a guess).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderStatus {
    /// How many messages there are.
    pub messages: u32,
    /// The UID the next message to arrive gets.
    pub uid_next: u32,
    pub uid_validity: u32,
    /// How many are not marked `\Seen`.
    pub unseen: u32,
}

/// From `STATUS name (MESSAGES 3 UIDNEXT 9 …)`; `None` when it isn't one, or a count is missing.
fn status_from(values: Vec<Value>) -> Option<FolderStatus> {
    let mut values = values.into_iter();
    if !values.next()?.atom()?.eq_ignore_ascii_case("STATUS") {
        return None;
    }
    values.next()?;
    let Value::List(pairs) = values.next()? else { return None };
    let (mut messages, mut uid_next, mut uid_validity, mut unseen) = (None, None, None, None);
    let mut pairs = pairs.into_iter();
    while let (Some(name), Some(value)) = (pairs.next(), pairs.next()) {
        let Some(number) = value.number() else { continue };
        match name.atom().map(str::to_ascii_uppercase).as_deref() {
            Some("MESSAGES") => messages = Some(number),
            Some("UIDNEXT") => uid_next = Some(number),
            Some("UIDVALIDITY") => uid_validity = Some(number),
            Some("UNSEEN") => unseen = Some(number),
            _ => {}
        }
    }
    Some(FolderStatus { messages: messages?, uid_next: uid_next?, uid_validity: uid_validity?, unseen: unseen? })
}

fn command(raw_name: &str) -> Command<'_> {
    Command::new("STATUS").string(raw_name.as_bytes()).arg("(MESSAGES UIDNEXT UIDVALIDITY UNSEEN)")
}

impl<S: Read + Write> Imap<S> {
    /// STATUS of one mailbox; `None` when the server refuses or leaves a count out.
    pub fn status(&mut self, raw_name: &str) -> Result<Option<FolderStatus>, ProtoError> {
        let mut found = None;
        let line = self.exchange(command(raw_name), &mut |values| {
            if found.is_none() {
                found = status_from(values);
            }
        })?;
        Ok(found.filter(|_| line.status == Status::Ok))
    }

    /// STATUS of each of `raw_names`, in the same order, `None` for one the server refuses or
    /// leaves a count out of. At most [`MAX_STATUS_BATCH`] are asked in one go; a name that
    /// can't go in a quoted string is asked alone.
    ///
    /// An error is the connection's: the answers read before it are lost with it.
    pub fn status_many(&mut self, raw_names: &[&str]) -> Result<Vec<Option<FolderStatus>>, ProtoError> {
        let mut out = Vec::with_capacity(raw_names.len());
        for batch in raw_names.chunks(MAX_STATUS_BATCH) {
            let plain = batch.iter().all(|name| name.len() <= MAX_QUOTED && name.bytes().all(|b| matches!(b, b' '..=b'~')));
            if plain && batch.len() > 1 {
                out.extend(self.status_batch(batch)?);
            } else {
                for name in batch {
                    out.push(self.status(name)?);
                }
            }
        }
        Ok(out)
    }

    /// One batch, written together: the answers come in the order of the commands.
    fn status_batch(&mut self, batch: &[&str]) -> Result<Vec<Option<FolderStatus>>, ProtoError> {
        let mut tags = Vec::with_capacity(batch.len());
        let mut out = Vec::with_capacity(batch.len().saturating_mul(64));
        for name in batch {
            let command = command(name);
            if command.unsendable {
                return Err(ProtoError::Protocol(said!("the text has a character that can't be sent")));
            }
            let tag = self.tag();
            if log::log_enabled!(log::Level::Trace) {
                log::trace!("C: {tag} {}", for_trace(&self.secrets.hide(&command.for_log())));
            }
            out.extend_from_slice(tag.as_bytes());
            out.push(b' ');
            for part in &command.parts {
                if let Part::Text(text) = part {
                    out.extend_from_slice(text);
                }
            }
            out.extend_from_slice(b"\r\n");
            tags.push(tag);
        }
        self.wire.write(&out)?;
        self.wire.flush()?;
        self.text_seen = 0;
        let mut found: Vec<Option<FolderStatus>> = vec![None; batch.len()];
        let mut answered = 0usize;
        // Each command brings one STATUS and its tagged answer; the rest is slack for what a
        // server says unasked.
        let cap = batch.len().saturating_mul(2).saturating_add(64);
        for _ in 0..cap {
            if answered >= tags.len() {
                break;
            }
            match self.read_response()? {
                Response::GoAhead => return Err(ProtoError::Protocol(said!("the server asked for more than the command had"))),
                Response::Status { tag: Some(tag), line } => {
                    if tags.get(answered) != Some(&tag) || !matches!(line.status, Status::Ok | Status::No | Status::Bad) {
                        return Err(ProtoError::Protocol(said!("the server answered a command that wasn't sent")));
                    }
                    self.note_status(&line);
                    if line.status != Status::Ok
                        && let Some(slot) = found.get_mut(answered)
                    {
                        *slot = None;
                    }
                    answered += 1;
                }
                Response::Status { tag: None, line } => {
                    if line.status == Status::Bye && !self.leaving {
                        return Err(ProtoError::Io(said!("the server ended the session")));
                    }
                    self.note_status(&line);
                }
                Response::Data(values) => {
                    if self.text_seen > self.limits.command_text {
                        return Err(ProtoError::Protocol(said!("the server sent too much in answer to one command")));
                    }
                    self.note_data(&values);
                    if let Some(slot) = found.get_mut(answered)
                        && slot.is_none()
                    {
                        *slot = status_from(values);
                    }
                }
            }
        }
        if answered < tags.len() {
            return Err(ProtoError::Protocol(said!("the server sent too many responses to one command")));
        }
        Ok(found)
    }
}
