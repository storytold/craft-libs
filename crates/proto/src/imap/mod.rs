// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An IMAP4rev1 client (RFC 3501) with the commands a two-way synchronisation needs: reading
//! mailboxes, and changing flags, moving, copying, removing and filing messages (UID MOVE of
//! RFC 6851; UID EXPUNGE and the `COPYUID` answer of RFC 4315). Special-use mailbox flags
//! (RFC 6154) come through in [`Mailbox::flags`] as the server wrote them. Password sign-in prefers
//! advertised SASL PLAIN, with LOGIN as a fallback when PLAIN is not offered; OAuth uses XOAUTH2.
//!
//! There is no plain EXPUNGE and no CLOSE here, on purpose: both remove every message marked
//! `\Deleted` in the mailbox, also those another program marked.

mod idle;
#[cfg(test)]
mod idle_tests;
mod parse;
mod password;
#[cfg(test)]
mod password_tests;
mod status;
#[cfg(test)]
mod status_tests;
mod utf7;

pub use idle::IdleEvent;
pub use status::{FolderStatus, MAX_STATUS_BATCH};

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Read, Write};

use parse::{Piece, Status, StatusLine, Value};
pub use utf7::{decode_utf7, encode_utf7};

use crate::i18n::{Arg, Text};
use crate::line::Wire;
use crate::said;
use crate::{MAX_LINE, MAX_LITERAL, ProtoError, Secrets, for_trace, tidy, xoauth2};

/// Most responses the server may send to one command.
const MAX_RESPONSES: usize = 250_000;
/// Most text (everything outside literals) the server may send in answer to one command.
const MAX_COMMAND_TEXT: usize = 32 << 20;
/// Most mailboxes one LIST keeps; the ones after are read and dropped. More than a run reads
/// (10,000, #991), since some of those listed are never read (views, folders that can't be
/// opened). What is kept is bounded in bytes too ([`MAX_LIST_BYTES`]).
const MAX_MAILBOXES: usize = 12_000;
/// Most bytes of names and flags one LIST keeps, literals included (which
/// [`MAX_COMMAND_TEXT`] doesn't count): 12,000 mailboxes as most servers list them take a
/// megabyte or two; past this the ones after are read and dropped.
const MAX_LIST_BYTES: usize = 16 << 20;
/// Longest mailbox name kept (as the server wrote it); a mailbox with a longer one is left out,
/// since a name cut short would name another mailbox or none.
const MAX_MAILBOX_NAME: usize = 1_024;
/// Longest flag kept, in bytes.
const MAX_FLAG_LEN: usize = 256;
/// Most distinct messages a FETCH may report, beyond twice the mailbox's `EXISTS`.
const FETCH_SLACK: usize = 1_000;
/// Most literals one response may carry.
const MAX_LITERALS: usize = 32;
/// Most text one response may carry outside its literals.
const MAX_TEXT: usize = 4 * MAX_LINE;
/// Most flags kept for one message or mailbox, and most sent in one command.
const MAX_FLAGS: usize = 64;
/// Most of Gmail's labels kept for one message (`X-GM-LABELS`, #1153); the rest are dropped.
pub const MAX_GMAIL_LABELS: usize = 64;
/// Longest of Gmail's labels kept, in bytes as the server wrote it (cut at a character boundary).
const MAX_GMAIL_LABEL_LEN: usize = 256;
/// Most bytes of Gmail's labels one fetch keeps, all its messages together, each label counted
/// with [`LABEL_OVERHEAD`]: past it, a message's labels are not taken (`None`), as if the server
/// hadn't said them.
const MAX_GMAIL_LABEL_BYTES: usize = 32 << 20;
/// What a label kept costs beyond its bytes (its `String`), counted against
/// [`MAX_GMAIL_LABEL_BYTES`] so that many empty or tiny labels can't get past it.
const LABEL_OVERHEAD: usize = std::mem::size_of::<String>();
/// Text a listing of flags may take for each message the mailbox may report, beyond
/// [`MAX_COMMAND_TEXT`]: a big folder's listing (Gmail's, with labels, runs to a few hundred
/// bytes a message) is never cut short for being big (#1153), and what a hostile server can send
/// stays bounded by the mailbox's size, and by [`MAX_FETCH_TEXT`] whatever size it claims. Only
/// a count: what is kept is bounded on its own ([`MAX_FLAG_BYTES`], [`MAX_GMAIL_LABEL_BYTES`]).
const FETCH_TEXT_PER_MESSAGE: usize = 2 << 10;
/// Most text a listing of flags may take, however many messages the mailbox claims.
const MAX_FETCH_TEXT: usize = 256 << 20;
/// Most bytes of flags one fetch keeps, all its messages together, each message counted with
/// [`ENTRY_OVERHEAD`] and each flag with [`FLAG_OVERHEAD`]: past it the listing fails, so that a
/// server claiming a huge mailbox can't fill memory and no message is taken to have lost its flags.
/// Room for over a million messages with a few flags each.
const MAX_FLAG_BYTES: usize = 256 << 20;
/// What a flag kept costs beyond its bytes (its `String`).
const FLAG_OVERHEAD: usize = std::mem::size_of::<String>();
/// What a message kept in a listing costs beyond its flags and labels: its entry in the map, with
/// room for the map's own nodes.
const ENTRY_OVERHEAD: usize = 2 * std::mem::size_of::<(u32, (Vec<String>, Option<u64>, Option<Vec<String>>))>();
const MAX_CAPABILITIES: usize = 256;
/// Longest string sent in quotes; a longer one goes as a literal.
const MAX_QUOTED: usize = 1_000;
/// Most messages named in one set by [`uid_sets`], and most a `COPYUID` answer may speak of.
const MAX_SET_UIDS: usize = 500;
/// Longest set [`uid_sets`] writes, in characters.
const MAX_SET_TEXT: usize = 800;
/// The most UIDs a search answers with that are taken in.
const MAX_FOUND: usize = 1_000;
/// The most UIDs one of Gmail's searches ([`Imap::uid_search_gmail`]) answers with that are taken
/// in: as many as fit the longest line read, at the longest a UID is written.
pub const MAX_GMAIL_FOUND: usize = 5_000;

/// Budgets shared by the commands of one session. Public constructors use the production caps.
#[derive(Clone, Copy)]
struct Limits {
    responses: usize,
    command_text: usize,
    fetch_text: usize,
    flag_bytes: usize,
    label_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            responses: MAX_RESPONSES,
            command_text: MAX_COMMAND_TEXT,
            fetch_text: MAX_FETCH_TEXT,
            flag_bytes: MAX_FLAG_BYTES,
            label_bytes: MAX_GMAIL_LABEL_BYTES,
        }
    }
}

/// An IMAP session on a stream.
pub struct Imap<S> {
    wire: Wire<S>,
    limits: Limits,
    /// The number in the next command's tag.
    next_tag: u32,
    secrets: Secrets,
    /// What the server said it can do, upper-cased; `None` until it has.
    capabilities: Option<Vec<String>>,
    authenticated: bool,
    /// What the mailbox being opened has said about itself so far.
    selected: Selected,
    /// Every `COPYUID` answer since the last copy or move was sent (`None` for one that couldn't
    /// be read); two are as many as it takes to know there was more than one.
    copied: Vec<Option<Copied>>,
    /// Every `APPENDUID` answer since the last APPEND was sent, as [`Imap::copied`] is kept.
    appended: Vec<Option<Appended>>,
    /// LOGOUT was sent, so BYE is the answer we asked for.
    leaving: bool,
    /// Bytes of text (outside literals) read since the current command was sent.
    text_seen: usize,
    /// Most responses the current command may bring.
    response_cap: usize,
    /// Most text (outside literals) the current command may bring: [`MAX_COMMAND_TEXT`], or more
    /// for a listing of a big mailbox's flags.
    text_cap: usize,
    /// Untagged lines too long to take are read past instead of ending the session, for the
    /// current command alone (a Gmail search, #911); `skipped` says one was.
    skip_long: bool,
    skipped: bool,
    /// The tag of the IDLE command the server is waiting in, if it is ([`Imap::idle_begin`]).
    idle_tag: Option<String>,
    idle_pending: std::collections::VecDeque<IdleEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailbox {
    /// Decoded from modified UTF-7.
    pub name: String,
    /// The name exactly as the server wrote it (use this in commands).
    pub raw: String,
    pub delimiter: Option<char>,
    /// Flags as written, e.g. `\\Noselect`, `\\Sent`, `\\HasChildren`.
    pub flags: Vec<String>,
}

/// The personal IMAP namespace (RFC 2342): `prefix` (as the server spells it, e.g. `"INBOX."`) and
/// hierarchy `delimiter` (`'.'`, `'/'`), if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Namespace {
    pub prefix: String,
    pub delimiter: Option<char>,
}

impl Mailbox {
    /// Whether the mailbox has `flag` (`\Noselect`), whatever the letter case.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    pub exists: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Vec<String>,
    /// INTERNALDATE as written by the server, when asked for.
    pub internal_date: Option<String>,
    /// The whole message (BODY[]), when asked for.
    pub body: Option<Vec<u8>>,
    /// Gmail's own id of the message (`X-GM-MSGID`, Google's "Gmail IMAP extensions"), when
    /// asked for and given; `None` from every other server.
    pub gmail_id: Option<u64>,
    /// Gmail's labels of the message (`X-GM-LABELS`, Google's "Gmail IMAP extensions", #1153),
    /// when asked for and given: as the server wrote them, modified UTF-7 decoded, Gmail's own
    /// (`\Inbox`, `\Important`…) with their backslash; at most [`MAX_GMAIL_LABELS`], each cut
    /// short. Text from the server, not yet cleaned for showing. `None` from every other server.
    pub gmail_labels: Option<Vec<String>>,
}

/// What became of the messages a UID COPY or UID MOVE named, when the server says (the `COPYUID`
/// response code of RFC 4315).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    /// `UIDVALIDITY` of the mailbox the messages went to.
    pub uid_validity: u32,
    /// Each message's UID where it was, and its UID where it is now.
    pub uids: Vec<(u32, u32)>,
}

/// Where a message given with APPEND went, when the server says (the `APPENDUID` response code
/// of RFC 4315).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appended {
    /// `UIDVALIDITY` of the mailbox it went to.
    pub uid_validity: u32,
    /// Its UID there.
    pub uid: u32,
}

/// Neither the stream nor anything secret is shown.
impl<S> std::fmt::Debug for Imap<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Imap").field("next_tag", &self.next_tag).field("authenticated", &self.authenticated).finish_non_exhaustive()
    }
}

/// One response from the server.
enum Response {
    /// `+ …`: the server is ready for the rest of the command.
    GoAhead,
    Status {
        tag: Option<String>,
        line: StatusLine,
    },
    /// Untagged data: `* 3 EXISTS`, `* LIST …`, `* 3 FETCH (…)`.
    Data(Vec<Value>),
}

enum Part<'a> {
    Text(Vec<u8>),
    Literal(&'a [u8]),
}

/// A command being put together (without its tag).
struct Command<'a> {
    parts: Vec<Part<'a>>,
    /// What the log shows in place of the command.
    shown: Option<&'static str>,
    /// An argument has a NUL, which no IMAP string can carry.
    unsendable: bool,
}

impl<'a> Command<'a> {
    fn new(verb: &str) -> Self {
        Command { parts: vec![Part::Text(verb.as_bytes().to_vec())], shown: None, unsendable: false }
    }

    fn text(&mut self, bytes: &[u8]) {
        match self.parts.last_mut() {
            Some(Part::Text(text)) => text.extend_from_slice(bytes),
            _ => self.parts.push(Part::Text(bytes.to_vec())),
        }
    }

    /// A bare argument: our own words, or text that was checked.
    fn arg(mut self, word: &str) -> Self {
        self.text(b" ");
        self.text(word.as_bytes());
        self
    }

    /// An argument as a quoted string, or as a literal when quotes can't carry it.
    fn string(mut self, bytes: &'a [u8]) -> Self {
        self.text(b" ");
        if bytes.len() <= MAX_QUOTED && bytes.iter().all(|b| matches!(b, b' '..=b'~')) {
            let mut quoted = Vec::with_capacity(bytes.len() + 2);
            quoted.push(b'"');
            for b in bytes {
                if matches!(b, b'"' | b'\\') {
                    quoted.push(b'\\');
                }
                quoted.push(*b);
            }
            quoted.push(b'"');
            self.text(&quoted);
        } else {
            self.unsendable |= bytes.contains(&0);
            self.parts.push(Part::Literal(bytes));
        }
        self
    }

    /// An argument that always goes as a literal (a message).
    fn literal(mut self, bytes: &'a [u8]) -> Self {
        self.text(b" ");
        self.parts.push(Part::Literal(bytes));
        self
    }

    fn shown_as(mut self, shown: &'static str) -> Self {
        self.shown = Some(shown);
        self
    }

    /// The command for the log: literals by size only.
    fn for_log(&self) -> String {
        if let Some(shown) = self.shown {
            return shown.to_string();
        }
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => String::from_utf8_lossy(text).into_owned(),
                Part::Literal(bytes) => format!("({} bytes)", bytes.len()),
            })
            .collect()
    }
}

/// What one FETCH response says about a message.
#[derive(Default)]
struct Item {
    uid: u32,
    flags: Option<Vec<String>>,
    internal_date: Option<String>,
    body: Option<Vec<u8>>,
    /// The header alone (`BODY[HEADER]`).
    header: Option<Vec<u8>>,
    size: Option<u64>,
    /// Gmail's id of the message (`X-GM-MSGID`).
    gmail_id: Option<u64>,
    /// Gmail's labels of the message (`X-GM-LABELS`).
    gmail_labels: Option<Vec<String>>,
}

impl Item {
    /// From `n FETCH (name value name value …)`, the names in any order.
    fn from(values: Vec<Value>) -> Option<Item> {
        let mut values = values.into_iter();
        values.next()?.number()?;
        if !values.next()?.atom()?.eq_ignore_ascii_case("FETCH") {
            return None;
        }
        let Value::List(pairs) = values.next()? else { return None };
        let mut item = Item::default();
        let mut pairs = pairs.into_iter();
        while let Some(name) = pairs.next() {
            // Something that isn't a name is skipped, in the hope that a name follows.
            let Some(name) = name.atom().map(str::to_ascii_uppercase) else { continue };
            let Some(value) = pairs.next() else { break };
            match (name.as_str(), value) {
                ("UID", value) => item.uid = value.number().unwrap_or(0),
                ("FLAGS", Value::List(flags)) => item.flags = Some(short_flags(flags)),
                ("INTERNALDATE", Value::Str(date)) => item.internal_date = Some(String::from_utf8_lossy(&date).into_owned()),
                ("RFC822.SIZE", value) => item.size = value.atom().and_then(|n| n.parse().ok()),
                ("X-GM-MSGID", value) => item.gmail_id = value.atom().and_then(gmail_id),
                ("X-GM-LABELS", Value::List(labels)) => item.gmail_labels = Some(gmail_labels(labels)),
                ("BODY[]" | "RFC822", Value::Str(body)) => item.body = Some(body),
                ("BODY[HEADER]" | "RFC822.HEADER", Value::Str(header)) => item.header = Some(header),
                _ => {}
            }
        }
        Some(item)
    }
}

/// From `LIST (flags) "delimiter" name`.
fn mailbox_from(values: Vec<Value>) -> Option<Mailbox> {
    let mut values = values.into_iter();
    if !values.next()?.atom()?.eq_ignore_ascii_case("LIST") {
        return None;
    }
    let Value::List(flags) = values.next()? else { return None };
    let delimiter = match values.next()? {
        Value::Str(d) => String::from_utf8_lossy(&d).chars().next(),
        _ => None,
    };
    let raw = values.next()?.into_text()?;
    if raw.len() > MAX_MAILBOX_NAME {
        return None;
    }
    Some(Mailbox { name: decode_utf7(&raw), raw, delimiter, flags: short_flags(flags) })
}

/// From `NAMESPACE ((prefix delimiter) ...) other shared` (RFC 2342): the first personal namespace.
fn namespace_from(values: Vec<Value>) -> Option<Namespace> {
    let mut values = values.into_iter();
    if !values.next()?.atom()?.eq_ignore_ascii_case("NAMESPACE") {
        return None;
    }
    let Value::List(personal) = values.next()? else { return None };
    let Value::List(desc) = personal.into_iter().next()? else { return None };
    let mut desc = desc.into_iter();
    let prefix = desc.next()?.into_text()?;
    if prefix.len() > MAX_MAILBOX_NAME {
        return None;
    }
    let delimiter = match desc.next()? {
        Value::Str(d) => String::from_utf8_lossy(&d).chars().next(),
        _ => None,
    };
    Some(Namespace { prefix, delimiter })
}

/// The flags of a list, as many as are kept and each cut at a character boundary.
fn short_flags(flags: Vec<Value>) -> Vec<String> {
    flags
        .into_iter()
        .filter_map(Value::into_text)
        .take(MAX_FLAGS)
        .map(|mut flag| {
            if flag.len() > MAX_FLAG_LEN {
                let mut end = MAX_FLAG_LEN;
                while !flag.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                flag = flag.get(..end).unwrap_or_default().to_owned();
            }
            flag
        })
        .collect()
}

/// Gmail's labels of a message (`X-GM-LABELS`): a list of atoms and strings, modified UTF-7 as
/// mailbox names are; as many as are kept, each cut at a character boundary before it is decoded.
/// Anything in the list that isn't an atom or a string (a list) is skipped.
fn gmail_labels(labels: Vec<Value>) -> Vec<String> {
    labels
        .into_iter()
        // `NIL` in the list is nothing, not a label (a label called so comes quoted).
        .filter(|value| !value.atom().is_some_and(|atom| atom.eq_ignore_ascii_case("NIL")))
        .filter_map(Value::into_text)
        .take(MAX_GMAIL_LABELS)
        .map(|mut label| {
            if label.len() > MAX_GMAIL_LABEL_LEN {
                let mut end = MAX_GMAIL_LABEL_LEN;
                while !label.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                label.truncate(end);
            }
            decode_utf7(&label)
        })
        .collect()
}

/// A message set as ranges of UIDs (`*` as `top`); `None` when it can't be read.
fn set_ranges(set: &str, top: u32) -> Option<Vec<(u32, u32)>> {
    let bound = |text: &str| if text == "*" { Some(top) } else { number(text) };
    set.split(',')
        .map(|part| {
            // A wildcard-only fetch is resolved by the server, not a stale client snapshot.
            if part == "*" || part == "*:*" {
                return Some((1, top));
            }
            let (a, b) = match part.split_once(':') {
                Some((a, b)) => (bound(a)?, bound(b)?),
                None => (bound(part)?, bound(part)?),
            };
            Some((a.min(b), a.max(b)))
        })
        .collect()
}

/// What a FETCH of a set may report: only UIDs in the set, and not more distinct
/// ones than a mailbox of this size can hold.
struct Wanted {
    ranges: Vec<(u32, u32)>,
    cap: usize,
    over: bool,
}

impl Wanted {
    fn new(set: &str, selected: &Selected) -> Option<Wanted> {
        // UIDNEXT is a SELECT-time snapshot; new deliveries can have larger UIDs.
        let ranges = set_ranges(set, u32::MAX)?;
        let cap = usize::try_from(selected.exists).unwrap_or(usize::MAX).saturating_mul(2).saturating_add(FETCH_SLACK);
        Some(Wanted { ranges, cap, over: false })
    }

    /// Most responses a fetch of this mailbox may bring: one for each message it may report, but
    /// never fewer than any other command.
    fn responses(&self, minimum: usize) -> usize {
        self.cap.max(minimum)
    }

    fn wants(&self, uid: u32) -> bool {
        uid != 0 && self.ranges.iter().any(|(from, to)| (*from..=*to).contains(&uid))
    }

    /// Whether one more distinct UID fits after `count`; if not, the command fails.
    fn room(&mut self, count: usize) -> bool {
        if count >= self.cap {
            self.over = true;
        }
        !self.over
    }

    fn check(&self) -> Result<(), ProtoError> {
        if self.over { Err(ProtoError::Protocol(said!("the server reported more messages than the mailbox holds"))) } else { Ok(()) }
    }
}

/// A message set: `1:*`, `5,9,12:20`.
fn is_set(set: &str) -> bool {
    !set.is_empty() && set.len() <= MAX_QUOTED && set.bytes().all(|b| b.is_ascii_digit() || matches!(b, b':' | b',' | b'*'))
}

/// A flag or keyword: `\Seen`, `$Forwarded`.
fn is_flag(flag: &str) -> bool {
    let word = flag.strip_prefix('\\').unwrap_or(flag);
    !word.is_empty()
        && word.len() <= 100
        && word.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'(' | b')' | b'{' | b'%' | b'*' | b'"' | b'\\' | b']'))
}

/// `(\Seen \Flagged)`, once every flag is known to be one.
fn flag_list(flags: &[&str]) -> Result<String, ProtoError> {
    if flags.len() > MAX_FLAGS {
        return Err(ProtoError::Protocol(said!("too many flags for one command")));
    }
    match flags.iter().find(|f| !is_flag(f)) {
        Some(bad) => Err(ProtoError::Protocol(said!("\"{flag}\" can't be used as a flag", flag = tidy(bad)))),
        None => Ok(format!("({})", flags.join(" "))),
    }
}

/// Gmail's id of a message (`X-GM-MSGID`): a 64-bit number, written in digits alone; 0 is none.
fn gmail_id(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|id| *id != 0)
}

/// A number as IMAP writes one: digits and nothing else.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The UIDs a set names, in the order written (`4,7:9` is 4, 7, 8, 9). `None` for a set that
/// doesn't name its messages one by one beyond doubt: `*`, zero, a range written backwards, or
/// more than [`MAX_SET_UIDS`] numbers.
fn uids_of(set: &str) -> Option<Vec<u32>> {
    let mut out: Vec<u32> = Vec::new();
    for part in set.split(',') {
        let (first, last) = match part.split_once(':') {
            Some((first, last)) => (number(first)?, number(last)?),
            None => {
                let one = number(part)?;
                (one, one)
            }
        };
        if first == 0 || first > last || (last - first) as usize >= MAX_SET_UIDS.saturating_sub(out.len()) {
            return None;
        }
        out.extend(first..=last);
    }
    Some(out)
}

/// A set for a command that changes things: it names its messages number by number. `1:*`
/// would take in messages nobody looked at.
fn named(set: &str) -> Result<&str, ProtoError> {
    if set.len() <= MAX_QUOTED && uids_of(set).is_some() {
        Ok(set)
    } else {
        Err(ProtoError::Protocol(said!("\"{set}\" doesn't name the messages to change", set = tidy(set))))
    }
}

/// From the text of a `COPYUID` code: `<uidvalidity> <where they were> <where they are>`.
/// `None` unless it pairs every message beyond doubt (the first set strictly ascending, the
/// second as long and without repeats): a wrong pairing would tie a message here to another one
/// on the server.
fn copied_from(arg: &str) -> Option<Copied> {
    let mut words = arg.split_whitespace();
    let uid_validity = number(words.next()?).filter(|validity| *validity != 0)?;
    let (from, to) = (uids_of(words.next()?)?, uids_of(words.next()?)?);
    let mut distinct = to.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if words.next().is_some() || from.len() != to.len() || distinct.len() != to.len() || !from.is_sorted_by(|a, b| a < b) {
        return None;
    }
    Some(Copied { uid_validity, uids: from.into_iter().zip(to).collect() })
}

/// From the text of an `APPENDUID` code: `<uidvalidity> <uid>`, for one message. `None` for
/// anything else (a set of UIDs speaks of more than the one message given).
fn appended_from(arg: &str) -> Option<Appended> {
    let mut words = arg.split_whitespace();
    let uid_validity = number(words.next()?).filter(|validity| *validity != 0)?;
    let uid = number(words.next()?).filter(|uid| *uid != 0)?;
    words.next().is_none().then_some(Appended { uid_validity, uid })
}

/// Whether `text` is an IMAP `date-time` without its quotes (RFC 3501): `01-Jul-2003 09:52:37
/// +0000`, the day padded with a space or a nought.
fn is_date_time(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| b.get(range).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    b.len() == 26
        && (digits(0..2) || (at(0, b' ') && digits(1..2)))
        && at(2, b'-')
        && b.get(3..6).is_some_and(|m| m.iter().all(u8::is_ascii_alphabetic))
        && at(6, b'-')
        && digits(7..11)
        && at(11, b' ')
        && digits(12..14)
        && at(14, b':')
        && digits(15..17)
        && at(17, b':')
        && digits(18..20)
        && at(20, b' ')
        && (at(21, b'+') || at(21, b'-'))
        && digits(22..26)
}

/// Whether [`Imap::uid_search_header`] can look for `text`: printable ASCII, at most
/// `MAX_QUOTED` bytes. What it can't be asked for, it refuses unsent.
pub fn searchable(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_QUOTED && text.bytes().all(|b| matches!(b, b' '..=b'~'))
}

/// UIDs as message sets for commands (`4,7:9,12`), each with the UIDs it names: sorted, each UID
/// once, consecutive numbers as ranges, and no set longer than one command should carry. A
/// range only ever spans numbers that were asked for.
pub fn uid_sets(uids: &[u32]) -> Vec<(String, Vec<u32>)> {
    let mut sorted: Vec<u32> = uids.iter().copied().filter(|uid| *uid != 0).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut sets = Vec::new();
    let (mut text, mut uids_named) = (String::new(), Vec::new());
    let mut at = 0;
    // Every round takes at least one UID off the list.
    while let Some(&first) = sorted.get(at) {
        let (mut last, mut next) = (first, at + 1);
        while uids_named.len() + (next - at) < MAX_SET_UIDS && sorted.get(next).is_some_and(|uid| last.checked_add(1) == Some(*uid)) {
            last = last.saturating_add(1);
            next += 1;
        }
        let range = if last == first { first.to_string() } else { format!("{first}:{last}") };
        if !text.is_empty() && text.len() + 1 + range.len() > MAX_SET_TEXT {
            sets.push((std::mem::take(&mut text), std::mem::take(&mut uids_named)));
        }
        if !text.is_empty() {
            text.push(',');
        }
        text.push_str(&range);
        uids_named.extend_from_slice(sorted.get(at..next).unwrap_or(&[]));
        at = next;
        if uids_named.len() >= MAX_SET_UIDS {
            sets.push((std::mem::take(&mut text), std::mem::take(&mut uids_named)));
        }
    }
    if !text.is_empty() {
        sets.push((text, uids_named));
    }
    sets
}

fn ignore(_: Vec<Value>) {}

impl<S: Read + Write> Imap<S> {
    /// Read the greeting (`* OK` or `* PREAUTH`; `* BYE` is an error).
    pub fn connect(stream: S) -> Result<Self, ProtoError> {
        let mut imap = Self::resume(stream);
        match imap.read_response()? {
            Response::Status { tag: None, line } => match line.status {
                Status::Ok | Status::Preauth => {
                    imap.authenticated = line.status == Status::Preauth;
                    imap.note_status(&line);
                    Ok(imap)
                }
                Status::Bye => Err(ProtoError::Rejected(imap.refusal(&line))),
                Status::No | Status::Bad => {
                    Err(ProtoError::Protocol(said!("the server greeted with a refusal: {reason}", reason = Arg::said(imap.refusal(&line)))))
                }
            },
            _ => Err(ProtoError::Protocol(said!("the server didn't greet like an IMAP server"))),
        }
    }

    /// Continue on a stream that already greeted (after the caller wrapped it in TLS).
    pub fn resume(stream: S) -> Self {
        Self::resume_with_limits(stream, Limits::default())
    }

    fn resume_with_limits(stream: S, limits: Limits) -> Self {
        Imap {
            wire: Wire::new(stream),
            limits,
            next_tag: 1,
            secrets: Secrets::default(),
            capabilities: None,
            authenticated: false,
            selected: Selected::default(),
            copied: Vec::new(),
            appended: Vec::new(),
            leaving: false,
            text_seen: 0,
            response_cap: limits.responses,
            text_cap: limits.command_text,
            skip_long: false,
            skipped: false,
            idle_tag: None,
            idle_pending: std::collections::VecDeque::new(),
        }
    }

    /// Whether the session is signed in: after [`login`](Self::login) or
    /// [`authenticate_xoauth2`](Self::authenticate_xoauth2), or from the start when the server greeted with PREAUTH.
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    /// The stream the session runs on, for its owner to set up between commands (how long a
    /// socket waits, say). Reading from it or writing to it would take the session apart.
    pub fn get_mut(&mut self) -> &mut S {
        self.wire.get_mut()
    }

    /// Whether the server has said it can do `capability` (upper-cased); `false` while it hasn't said anything.
    fn offers(&self, capability: &str) -> bool {
        self.capabilities.as_ref().is_some_and(|caps| caps.iter().any(|c| c == capability))
    }

    /// Upper-cased capability atoms (`IMAP4REV1`, `STARTTLS`, `SPECIAL-USE`…).
    pub fn capabilities(&mut self) -> Result<Vec<String>, ProtoError> {
        self.capabilities = None;
        self.run(Command::new("CAPABILITY"), &mut ignore)?;
        Ok(self.capabilities.clone().unwrap_or_default())
    }

    /// Send STARTTLS and hand the stream back for the caller to wrap; call `resume` afterwards.
    pub fn starttls(mut self) -> Result<S, ProtoError> {
        self.run(Command::new("STARTTLS"), &mut ignore)?;
        // Anything after the go-ahead was slipped in before encryption started.
        if self.wire.has_pending() {
            return Err(ProtoError::Protocol(said!("the server sent data before the secure connection started")));
        }
        Ok(self.wire.into_inner())
    }

    /// LOGIN with properly quoted (or literal) strings. NO → `ProtoError::Auth`.
    pub fn login(&mut self, user: &str, password: &str) -> Result<(), ProtoError> {
        if user.contains('\0') || password.contains('\0') {
            return Err(ProtoError::Auth(said!("the user name or password has a character that can't be sent")));
        }
        // RFC 3501 §6.2.3: a server that says LOGINDISABLED must not be sent a password.
        if self.offers("LOGINDISABLED") {
            return Err(ProtoError::Auth(said!("the server doesn't take a password on this connection")));
        }
        self.secrets.add(password);
        let command = Command::new("LOGIN").string(user.as_bytes()).string(password.as_bytes()).shown_as("LOGIN ***");
        let line = self.exchange(command, &mut ignore)?;
        match line.status {
            Status::Ok => {
                self.authenticated = true;
                Ok(())
            }
            Status::No => Err(ProtoError::Auth(self.refusal(&line))),
            _ => Err(ProtoError::Rejected(self.refusal(&line))),
        }
    }

    /// AUTHENTICATE XOAUTH2: an OAuth access token in place of a password. NO → `ProtoError::Auth`.
    ///
    /// The token goes out with the command (RFC 4959) only to a server that listed both
    /// `SASL-IR` and `AUTH=XOAUTH2`. Any other server, one that listed neither or hasn't said
    /// what it can do, is tried all the same, but asked first: it gets the token after its
    /// go-ahead, so one that doesn't know the mechanism refuses without having seen it.
    pub fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Result<(), ProtoError> {
        let payload = xoauth2(user, token)?;
        self.secrets.add(token);
        self.secrets.add(&payload);
        let inline = self.offers("SASL-IR") && self.offers("AUTH=XOAUTH2");
        let tag = self.tag();
        log::trace!("C: {tag} AUTHENTICATE XOAUTH2{}", if inline { " ***" } else { "" });
        let mut out = format!("{tag} AUTHENTICATE XOAUTH2");
        if inline {
            out.push(' ');
            out.push_str(&payload);
        }
        out.push_str("\r\n");
        self.wire.write(out.as_bytes())?;
        self.wire.flush()?;
        // What each `+` from the server is answered with: the token (unless it went with the
        // command), an empty line for the reasons a refusal starts with (JSON in base64), and
        // for a question after that, the end of the exchange.
        let answers = [(payload.as_str(), "***"), ("", ""), ("*", "*")];
        let mut asked = usize::from(inline);
        let line = loop {
            if let Some(line) = self.read_until(&tag, true, &mut ignore)? {
                break line;
            }
            let Some((answer, shown)) = answers.get(asked) else {
                return Err(ProtoError::Protocol(said!("the server asked for more than a sign-in has")));
            };
            asked += 1;
            log::trace!("C: {shown}");
            self.wire.write(format!("{answer}\r\n").as_bytes())?;
            self.wire.flush()?;
        };
        match line.status {
            // The exchange was called off, whatever the server makes of that.
            _ if asked == answers.len() => Err(ProtoError::Auth(said!("the server asked for more than a user name and access token"))),
            // Nobody is signed in by a token that was never sent.
            Status::Ok if asked == 0 => Err(ProtoError::Protocol(said!("the server ended the command early"))),
            Status::Ok => {
                self.authenticated = true;
                Ok(())
            }
            Status::No => Err(ProtoError::Auth(self.refusal(&line))),
            _ => Err(ProtoError::Rejected(self.refusal(&line))),
        }
    }

    /// LIST "" "*".
    pub fn list(&mut self) -> Result<Vec<Mailbox>, ProtoError> {
        self.list_counted().map(|(list, _)| list)
    }

    /// [`list`](Self::list), and how many more mailboxes the server listed that were read and
    /// dropped because the list was full (`MAX_MAILBOXES`, `MAX_LIST_BYTES`): 0 when it
    /// was kept whole.
    pub fn list_counted(&mut self) -> Result<(Vec<Mailbox>, usize), ProtoError> {
        let mut out: Vec<Mailbox> = Vec::new();
        let mut seen = HashSet::new();
        let (mut kept, mut dropped) = (0usize, 0usize);
        self.run(Command::new("LIST").arg("\"\"").arg("\"*\""), &mut |values| {
            let full = out.len() >= MAX_MAILBOXES || kept >= MAX_LIST_BYTES;
            // A mailbox the server names twice is listed once; past either cap, nothing is kept.
            let Some(mailbox) = mailbox_from(values) else { return };
            if full {
                dropped = dropped.saturating_add(1);
            } else if seen.insert(mailbox.raw.clone()) {
                // The name twice (as written, decoded) and once more in `seen`, and the flags.
                let size = mailbox.raw.len().saturating_mul(2).saturating_add(mailbox.name.len());
                kept = mailbox.flags.iter().fold(kept.saturating_add(size), |kept, flag| kept.saturating_add(flag.len()));
                out.push(mailbox);
            }
        })?;
        Ok((out, dropped))
    }

    /// NAMESPACE (RFC 2342): the server's first personal namespace, or `None` when the server
    /// offers no `NAMESPACE` capability or names no personal namespace.
    pub fn namespace(&mut self) -> Result<Option<Namespace>, ProtoError> {
        if !self.offers("NAMESPACE") {
            return Ok(None);
        }
        let mut found = None;
        self.run(Command::new("NAMESPACE"), &mut |values| {
            if found.is_none() {
                found = namespace_from(values);
            }
        })?;
        Ok(found)
    }

    /// EXAMINE (read-only) or SELECT.
    pub fn select(&mut self, raw_name: &str, read_only: bool) -> Result<Selected, ProtoError> {
        self.selected = Selected::default();
        let command = Command::new(if read_only { "EXAMINE" } else { "SELECT" }).string(raw_name.as_bytes());
        self.run(command, &mut ignore)?;
        Ok(self.selected.clone())
    }

    /// UID FETCH `<set>` (UID FLAGS): every message in the set with its flags. `set` like "1:*" or "5,9,12:20".
    ///
    /// The messages come back in UID order, each once.
    pub fn uid_fetch_flags(&mut self, set: &str) -> Result<Vec<Fetched>, ProtoError> {
        self.fetch_flags(set, false, false)
    }

    /// [`uid_fetch_flags`](Self::uid_fetch_flags), with Gmail's id and labels of each message as
    /// well (`UID FETCH <set> (UID FLAGS X-GM-MSGID X-GM-LABELS)`, in [`Fetched::gmail_id`] and
    /// [`Fetched::gmail_labels`]): for Gmail's server alone, which documents them among its IMAP
    /// extensions. A message it gives no id, or one that isn't a number, has none; one it gives
    /// no labels has `None` there, and one past `MAX_GMAIL_LABEL_BYTES` of labels in the fetch
    /// too.
    pub fn uid_fetch_flags_gmail(&mut self, set: &str) -> Result<Vec<Fetched>, ProtoError> {
        self.fetch_flags(set, true, true)
    }

    /// [`uid_fetch_flags`](Self::uid_fetch_flags), with Gmail's labels of each message as well but
    /// not its id (`UID FETCH <set> (UID FLAGS X-GM-LABELS)`, in [`Fetched::gmail_labels`]): for a
    /// Gmail folder whose ids aren't wanted (every folder but the Inbox), so that its listing is
    /// no longer than it has to be.
    pub fn uid_fetch_flags_labels(&mut self, set: &str) -> Result<Vec<Fetched>, ProtoError> {
        self.fetch_flags(set, false, true)
    }

    fn fetch_flags(&mut self, set: &str, ids: bool, labels_wanted: bool) -> Result<Vec<Fetched>, ProtoError> {
        type FetchedFlags = (Vec<String>, Option<u64>, Option<Vec<String>>);
        let limits = self.limits;
        let mut found: BTreeMap<u32, FetchedFlags> = BTreeMap::new();
        // The bytes of labels and of flags kept so far, all messages together.
        let mut label_bytes = 0usize;
        let mut flag_bytes = 0usize;
        let items = match (ids, labels_wanted) {
            (true, true) => "(UID FLAGS X-GM-MSGID X-GM-LABELS)",
            (true, false) => "(UID FLAGS X-GM-MSGID)",
            (false, true) => "(UID FLAGS X-GM-LABELS)",
            (false, false) => "(UID FLAGS)",
        };
        let command = Self::fetch(set, items)?;
        let mut wanted = Wanted::new(set, &self.selected).ok_or_else(|| ProtoError::Protocol(said!("not a set of messages")))?;
        self.response_cap = wanted.responses(self.limits.responses);
        let claimed = usize::try_from(self.selected.exists).unwrap_or(usize::MAX);
        self.text_cap = claimed.saturating_mul(FETCH_TEXT_PER_MESSAGE).clamp(limits.command_text, limits.fetch_text);
        let answered = self.run(command, &mut |values| {
            // A response with a UID shows the message is there, even one that forgot the flags.
            if flag_bytes <= limits.flag_bytes
                && let Some(item) = Item::from(values)
                && wanted.wants(item.uid)
                && (found.contains_key(&item.uid) || wanted.room(found.len()))
            {
                // Each word counted as it comes, a repeat included: past the budget nothing more
                // is kept and the listing fails below.
                let size = item.flags.iter().flatten().map(|flag| flag.len().saturating_add(FLAG_OVERHEAD)).sum::<usize>();
                let entry = if found.contains_key(&item.uid) { 0 } else { ENTRY_OVERHEAD };
                flag_bytes = flag_bytes.saturating_add(size).saturating_add(entry);
                if flag_bytes > limits.flag_bytes {
                    return;
                }
                let (flags, id, labels) = found.entry(item.uid).or_default();
                // When the server speaks of a message twice, its later word counts.
                if let Some(later) = item.flags {
                    *flags = later;
                }
                if ids && item.gmail_id.is_some() {
                    *id = item.gmail_id;
                }
                if labels_wanted && let Some(later) = item.gmail_labels {
                    let size = later.iter().map(|label| label.len().saturating_add(LABEL_OVERHEAD)).sum::<usize>();
                    if label_bytes.saturating_add(size) <= limits.label_bytes {
                        label_bytes = label_bytes.saturating_add(size);
                        *labels = Some(later);
                    }
                }
            }
        });
        self.response_cap = limits.responses;
        self.text_cap = limits.command_text;
        answered?;
        if flag_bytes > limits.flag_bytes {
            return Err(ProtoError::Protocol(said!("more flags than one listing keeps")));
        }
        wanted.check()?;
        Ok(found
            .into_iter()
            .map(|(uid, (flags, gmail_id, gmail_labels))| Fetched { uid, flags, internal_date: None, body: None, gmail_id, gmail_labels })
            .collect())
    }

    /// UID FETCH `<set>` (UID RFC822.SIZE): the size in bytes of every message in the set, in UID order.
    pub fn uid_fetch_sizes(&mut self, set: &str) -> Result<Vec<(u32, u64)>, ProtoError> {
        let mut out: Vec<(u32, u64)> = Vec::new();
        let command = Self::fetch(set, "(UID RFC822.SIZE)")?;
        let mut wanted = Wanted::new(set, &self.selected).ok_or_else(|| ProtoError::Protocol(said!("not a set of messages")))?;
        self.response_cap = wanted.responses(self.limits.responses);
        let answered = self.run(command, &mut |values| {
            if let Some(Item { uid, size: Some(size), .. }) = Item::from(values)
                && wanted.wants(uid)
                && wanted.room(out.len())
            {
                out.push((uid, size));
            }
        });
        self.response_cap = self.limits.responses;
        answered?;
        wanted.check()?;
        out.sort_by_key(|(uid, _)| *uid);
        out.dedup_by_key(|(uid, _)| *uid);
        Ok(out)
    }

    /// UID FETCH `<uid>` (UID FLAGS INTERNALDATE BODY.PEEK[]); `None` when the UID is gone.
    pub fn uid_fetch_message(&mut self, uid: u32) -> Result<Option<Fetched>, ProtoError> {
        self.fetch_one(uid, "(UID FLAGS INTERNALDATE BODY.PEEK[])", false)
    }

    /// UID FETCH `<uid>` (UID FLAGS INTERNALDATE `BODY.PEEK[HEADER]`): the header alone of a message
    /// that is not taken whole (one that is too large), in [`Fetched::body`], up to and with the
    /// empty line that ends it. `None` when the UID is gone.
    pub fn uid_fetch_header(&mut self, uid: u32) -> Result<Option<Fetched>, ProtoError> {
        self.fetch_one(uid, "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER])", true)
    }

    /// One message with its flags and date, and either all of it or (`header_only`) its header.
    fn fetch_one(&mut self, uid: u32, items: &str, header_only: bool) -> Result<Option<Fetched>, ProtoError> {
        if uid == 0 {
            return Ok(None);
        }
        let mut found: Option<Fetched> = None;
        self.run(Self::fetch(&uid.to_string(), items)?, &mut |values| {
            // Other messages may be mentioned in passing; this one may come in more than one response.
            if let Some(item) = Item::from(values)
                && item.uid == uid
            {
                let message = found.get_or_insert_with(|| Fetched { uid, ..Default::default() });
                if let Some(flags) = item.flags {
                    message.flags = flags;
                }
                message.internal_date = item.internal_date.or(message.internal_date.take());
                // Only what was asked for: a whole message in answer to a question about its
                // header is not taken in.
                let asked_for = if header_only { item.header } else { item.body };
                message.body = asked_for.or(message.body.take());
            }
        })?;
        Ok(found.filter(|message| message.body.is_some()))
    }

    /// UID STORE `<uid>` +FLAGS.SILENT / -FLAGS.SILENT (flags).
    pub fn uid_store(&mut self, uid: u32, add: bool, flags: &[&str]) -> Result<(), ProtoError> {
        let flags = flag_list(flags)?;
        if uid == 0 {
            return Err(ProtoError::Protocol(said!("a message that isn't on the server has no flags there")));
        }
        self.store(&uid.to_string(), add, &flags)
    }

    /// UID STORE `<set>` +FLAGS.SILENT / -FLAGS.SILENT (flags): the same change for every message
    /// the set names (number by number, as [`uid_sets`] writes it; `1:*` is refused). A UID that
    /// is no longer there is passed over by the server without a word.
    pub fn uid_store_set(&mut self, set: &str, add: bool, flags: &[&str]) -> Result<(), ProtoError> {
        let flags = flag_list(flags)?;
        self.store(named(set)?, add, &flags)
    }

    /// UID SEARCH UNDELETED HEADER `<field>` `<text>`: the messages of the open mailbox, other than
    /// those marked as deleted, whose header field `field` has `text` in it (the server matches a
    /// part of the field, whatever its case), by UID in UID order, at most `MAX_FOUND` of them.
    /// `field` is a field's name, and `text` one that [`searchable`] lets through: anything else
    /// is refused unsent.
    pub fn uid_search_header(&mut self, field: &str, text: &str) -> Result<Vec<u32>, ProtoError> {
        if field.is_empty() || !field.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(ProtoError::Protocol(said!("\"{field}\" is not the name of a header field", field = tidy(field))));
        }
        if !searchable(text) {
            return Err(ProtoError::Protocol(said!("the text can't be looked for on the server")));
        }
        let mut found = BTreeSet::new();
        let command = Command::new("UID SEARCH UNDELETED HEADER").arg(field).string(text.as_bytes());
        self.run(command, &mut |values| {
            if values.first().and_then(Value::atom).is_some_and(|word| word.eq_ignore_ascii_case("SEARCH")) {
                let uids = values.iter().skip(1).filter_map(Value::atom).filter_map(|n| n.parse::<u32>().ok()).filter(|uid| *uid != 0);
                for uid in uids {
                    if found.len() >= MAX_FOUND {
                        break;
                    }
                    found.insert(uid);
                }
            }
        })?;
        Ok(found.into_iter().collect())
    }

    /// UID SEARCH UID `<set>` UNDELETED X-GM-RAW "`<query>`": the messages of the open mailbox, among
    /// those `set` names and not marked as deleted, that Gmail's own search finds for `query`
    /// (`category:social`), by UID in UID order. `X-GM-RAW` is Gmail's (Google's "Gmail IMAP extensions"): for Gmail's server alone.
    ///
    /// The query goes in quotes, without the CR, LF and NUL that could end the command early; one
    /// that is empty then, or isn't printable ASCII within `MAX_QUOTED` bytes, is refused unsent.
    /// Of the answer, only UIDs inside `set` are taken, at most [`MAX_GMAIL_FOUND`] of them: a
    /// caller that wants them all asks for sets of fewer messages than that. An answer longer than
    /// [`MAX_LINE`] is read past and refused ([`ProtoError::Rejected`]): the
    /// session goes on.
    pub fn uid_search_gmail(&mut self, set: &str, query: &str) -> Result<Vec<u32>, ProtoError> {
        if !is_set(set) {
            return Err(ProtoError::Protocol(said!("\"{set}\" is not a set of messages", set = tidy(set))));
        }
        let ranges = set_ranges(set, u32::MAX).ok_or_else(|| ProtoError::Protocol(said!("not a set of messages")))?;
        let query: String = query.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect();
        if !searchable(&query) {
            return Err(ProtoError::Protocol(said!("the text can't be looked for on the server")));
        }
        let mut found = BTreeSet::new();
        // Messages marked as deleted aren't shown, so they aren't asked about either.
        let command = Command::new("UID SEARCH UID").arg(set).arg("UNDELETED X-GM-RAW").string(query.as_bytes());
        (self.skip_long, self.skipped) = (true, false);
        let answer = self.run(command, &mut |values| {
            if values.first().and_then(Value::atom).is_some_and(|word| word.eq_ignore_ascii_case("SEARCH")) {
                let uids = values.iter().skip(1).filter_map(Value::number).filter(|uid| *uid != 0);
                for uid in uids.filter(|uid| ranges.iter().any(|(from, to)| (*from..=*to).contains(uid))) {
                    if found.len() >= MAX_GMAIL_FOUND {
                        break;
                    }
                    found.insert(uid);
                }
            }
        });
        let skipped = std::mem::take(&mut self.skipped);
        self.skip_long = false;
        answer?;
        // An answer too long to read is no answer: the session goes on without it.
        if skipped {
            return Err(ProtoError::Rejected(said!("the server's answer was longer than {kb} KB", kb = crate::MAX_LINE / 1024)));
        }
        Ok(found.into_iter().collect())
    }

    fn store(&mut self, set: &str, add: bool, flags: &str) -> Result<(), ProtoError> {
        // No flags: nothing to ask of the server.
        if flags == "()" {
            return Ok(());
        }
        let command = Command::new("UID STORE").arg(set).arg(if add { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" }).arg(flags);
        self.run(command, &mut ignore).map(|_| ())
    }

    /// UID MOVE `<set>` `<mailbox>` (RFC 6851, for a server that offers `MOVE`): the messages leave
    /// the open mailbox for the named one in one step. Answers where they are now when the
    /// server says so beyond doubt (`COPYUID`), `None` otherwise.
    pub fn uid_move(&mut self, set: &str, raw_name: &str) -> Result<Option<Copied>, ProtoError> {
        self.transfer("UID MOVE", set, raw_name)
    }

    /// UID COPY `<set>` `<mailbox>`: copies of the messages are added to the named mailbox. Answers
    /// like [`uid_move`](Self::uid_move).
    pub fn uid_copy(&mut self, set: &str, raw_name: &str) -> Result<Option<Copied>, ProtoError> {
        self.transfer("UID COPY", set, raw_name)
    }

    fn transfer(&mut self, verb: &str, set: &str, raw_name: &str) -> Result<Option<Copied>, ProtoError> {
        let command = Command::new(verb).arg(named(set)?).string(raw_name.as_bytes());
        self.copied.clear();
        let done = self.run(command, &mut ignore);
        // One answer that could be read, or nothing is known: two that might disagree are none.
        let copied = match std::mem::take(&mut self.copied).as_mut_slice() {
            [only] => only.take(),
            _ => None,
        };
        done.map(|_| copied)
    }

    /// UID EXPUNGE `<set>` (RFC 4315): the messages of the set that are marked `\Deleted` go for
    /// good, and no others. Refused unsent unless the server has said `UIDPLUS`: a server
    /// without it only knows the EXPUNGE that takes every marked message of the mailbox.
    pub fn uid_expunge(&mut self, set: &str) -> Result<(), ProtoError> {
        let set = named(set)?;
        if !self.offers("UIDPLUS") {
            return Err(ProtoError::Protocol(said!("the server can't remove single messages (it doesn't offer UIDPLUS)")));
        }
        self.run(Command::new("UID EXPUNGE").arg(set), &mut ignore).map(|_| ())
    }

    /// CREATE `<mailbox>`. `raw_name` is the name as the server wants it: in modified UTF-7
    /// ([`encode_utf7`]), its levels joined with the server's own delimiter.
    pub fn create(&mut self, raw_name: &str) -> Result<(), ProtoError> {
        self.run(Command::new("CREATE").string(raw_name.as_bytes()), &mut ignore).map(|_| ())
    }

    /// SUBSCRIBE `<mailbox>`: other mail programs list subscribed mailboxes only.
    pub fn subscribe(&mut self, raw_name: &str) -> Result<(), ProtoError> {
        self.run(Command::new("SUBSCRIBE").string(raw_name.as_bytes()), &mut ignore).map(|_| ())
    }

    /// APPEND with a literal (handles the `+` continuation).
    pub fn append(&mut self, raw_name: &str, flags: &[&str], message: &[u8]) -> Result<(), ProtoError> {
        self.append_at(raw_name, flags, None, message).map(|_| ())
    }

    /// [`append`](Self::append), with the time the server is to keep as the message's arrival
    /// (its `INTERNALDATE`, an IMAP `date-time` such as `01-Jul-2003 09:52:37 +0000`) when one is
    /// given. Answers where the message went when the server says so beyond doubt (`APPENDUID`),
    /// `None` otherwise.
    pub fn append_at(&mut self, raw_name: &str, flags: &[&str], date: Option<&str>, message: &[u8]) -> Result<Option<Appended>, ProtoError> {
        let mut command = Command::new("APPEND").string(raw_name.as_bytes());
        if !flags.is_empty() {
            command = command.arg(&flag_list(flags)?);
        }
        if let Some(date) = date {
            if !is_date_time(date) {
                return Err(ProtoError::Protocol(said!("\"{date}\" is not a date and time IMAP can carry", date = tidy(date))));
            }
            command = command.arg(&format!("\"{date}\""));
        }
        self.appended.clear();
        let done = self.run(command.literal(message), &mut ignore);
        // One answer that could be read, or nothing is known.
        let appended = match std::mem::take(&mut self.appended).as_mut_slice() {
            [only] => only.take(),
            _ => None,
        };
        done.map(|_| appended)
    }

    /// LOGOUT. A server that hangs up without answering has logged us out all the same.
    pub fn logout(mut self) -> Result<(), ProtoError> {
        self.leaving = true;
        match self.run(Command::new("LOGOUT"), &mut ignore) {
            Ok(_) | Err(ProtoError::Io(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn fetch(set: &str, items: &str) -> Result<Command<'static>, ProtoError> {
        if !is_set(set) {
            return Err(ProtoError::Protocol(said!("\"{set}\" is not a set of messages", set = tidy(set))));
        }
        Ok(Command::new("UID FETCH").arg(set).arg(items))
    }

    /// How many commands have been sent on this connection (the tags used so far).
    pub fn commands(&self) -> u32 {
        self.next_tag.wrapping_sub(1)
    }

    /// The tag of the next command.
    fn tag(&mut self) -> String {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        tag
    }

    /// Run a command that must end in OK; untagged data goes to `on_data` as it arrives.
    fn run(&mut self, command: Command<'_>, on_data: &mut dyn FnMut(Vec<Value>)) -> Result<StatusLine, ProtoError> {
        let line = self.exchange(command, on_data)?;
        match line.status {
            Status::Ok => Ok(line),
            _ => Err(ProtoError::Rejected(self.refusal(&line))),
        }
    }

    /// Send a command and read up to its tagged answer: OK, NO or BAD.
    fn exchange(&mut self, command: Command<'_>, on_data: &mut dyn FnMut(Vec<Value>)) -> Result<StatusLine, ProtoError> {
        if command.unsendable {
            return Err(ProtoError::Protocol(said!("the text has a character that can't be sent")));
        }
        let tag = self.tag();
        self.text_seen = 0;
        if log::log_enabled!(log::Level::Trace) {
            log::trace!("C: {tag} {}", for_trace(&self.secrets.hide(&command.for_log())));
        }
        let mut out = Vec::with_capacity(128);
        out.extend_from_slice(tag.as_bytes());
        out.push(b' ');
        for part in &command.parts {
            match part {
                Part::Text(text) => out.extend_from_slice(text),
                Part::Literal(bytes) => {
                    // A literal is announced, and only sent once the server says `+`.
                    out.extend_from_slice(format!("{{{}}}\r\n", bytes.len()).as_bytes());
                    self.wire.write(&out)?;
                    self.wire.flush()?;
                    out.clear();
                    match self.read_until(&tag, true, on_data)? {
                        None => {}
                        // The command can't have succeeded without the rest of it.
                        Some(line) if line.status == Status::Ok => return Err(ProtoError::Protocol(said!("the server ended the command early"))),
                        Some(refused) => return Ok(refused),
                    }
                    self.wire.write(bytes)?;
                }
            }
        }
        out.extend_from_slice(b"\r\n");
        self.wire.write(&out)?;
        self.wire.flush()?;
        match self.read_until(&tag, false, on_data)? {
            Some(line) => Ok(line),
            None => Err(ProtoError::Protocol(said!("the server asked for more than the command had"))),
        }
    }

    /// Read responses up to the command's tagged answer (`Some`) or, when a literal is waiting to
    /// be sent, the server's go-ahead (`None`).
    fn read_until(&mut self, tag: &str, literal_waiting: bool, on_data: &mut dyn FnMut(Vec<Value>)) -> Result<Option<StatusLine>, ProtoError> {
        for _ in 0..self.response_cap {
            let response = self.read_response()?;
            if self.text_seen > self.text_cap {
                return Err(ProtoError::Protocol(said!("the server sent too much in answer to one command")));
            }
            match response {
                Response::GoAhead if literal_waiting => return Ok(None),
                Response::GoAhead => return Err(ProtoError::Protocol(said!("the server asked for more than the command had"))),
                Response::Status { tag: Some(answered), line } => {
                    if answered != tag || !matches!(line.status, Status::Ok | Status::No | Status::Bad) {
                        return Err(ProtoError::Protocol(said!("the server answered a command that wasn't sent")));
                    }
                    self.note_status(&line);
                    return Ok(Some(line));
                }
                Response::Status { tag: None, line } => {
                    if line.status == Status::Bye && !self.leaving {
                        return Err(ProtoError::Io(said!("the server ended the session: {reason}", reason = Arg::said(self.refusal(&line)))));
                    }
                    self.note_status(&line);
                }
                Response::Data(values) => {
                    self.note_data(&values);
                    on_data(values);
                }
            }
        }
        Err(ProtoError::Protocol(said!("the server sent too many responses to one command")))
    }

    /// A line of the server's text, counted against what one command may take.
    fn read_text(&mut self) -> Result<Vec<u8>, ProtoError> {
        loop {
            let line = if self.skip_long { self.wire.read_line_or_skip()? } else { Some(self.wire.read_line()?) };
            let Some(line) = line else {
                self.skipped = true;
                self.text_seen = self.text_seen.saturating_add(crate::MAX_LINE);
                if self.text_seen > self.text_cap {
                    return Err(ProtoError::Protocol(said!("the server sent too much in answer to one command")));
                }
                continue;
            };
            self.text_seen = self.text_seen.saturating_add(line.len()).saturating_add(2);
            return Ok(line);
        }
    }

    fn read_response(&mut self) -> Result<Response, ProtoError> {
        let line = self.read_text()?;
        if log::log_enabled!(log::Level::Trace) {
            log::trace!("S: {}", for_trace(&self.secrets.hide(&String::from_utf8_lossy(&line))));
        }
        if line.first() == Some(&b'+') {
            return Ok(Response::GoAhead);
        }
        let Some(rest) = line.strip_prefix(b"* ") else {
            // Tagged: `A0001 OK done`.
            let text = String::from_utf8_lossy(&line);
            let mut words = text.splitn(3, ' ');
            let (tag, word, rest) = (words.next().unwrap_or(""), words.next().unwrap_or(""), words.next().unwrap_or(""));
            return match Status::parse(word) {
                Some(status) if !tag.is_empty() && tag != "*" => {
                    Ok(Response::Status { tag: Some(tag.to_string()), line: StatusLine::new(status, rest) })
                }
                _ => Err(ProtoError::Protocol(said!("\"{line}\" is not an IMAP response", line = tidy(&self.secrets.hide(&text))))),
            };
        };
        // `* OK …`, `* BYE …`: the rest is text for people, where `{5}` is no literal.
        let word_end = rest.iter().position(|b| *b == b' ').unwrap_or(rest.len());
        let word = String::from_utf8_lossy(rest.get(..word_end).unwrap_or(&[]));
        if let Some(status) = Status::parse(&word) {
            let text = String::from_utf8_lossy(rest.get(word_end..).unwrap_or(&[]));
            return Ok(Response::Status { tag: None, line: StatusLine::new(status, &text) });
        }
        // Data: a line, and for as long as a line ends in `{n}`, n bytes and another line.
        let mut pieces = Vec::new();
        let mut text = rest.to_vec();
        let (mut text_bytes, mut literal_bytes) = (text.len(), 0usize);
        for _ in 0..=MAX_LITERALS {
            let Some((_, claimed)) = parse::literal_marker(&text) else {
                pieces.push(Piece::Text(text));
                return Ok(Response::Data(parse::values(pieces)?));
            };
            let size = usize::try_from(claimed).ok().filter(|n| literal_bytes.saturating_add(*n) <= MAX_LITERAL).ok_or_else(|| {
                ProtoError::Protocol(said!(
                    "the server sent {claimed} bytes in one piece, more than the {mb} MB MailCraft accepts",
                    claimed,
                    mb = MAX_LITERAL >> 20
                ))
            })?;
            let literal = self.wire.read_block(size)?;
            literal_bytes = literal_bytes.saturating_add(size);
            log::trace!("S: ({size} bytes)");
            pieces.push(Piece::Text(text));
            pieces.push(Piece::Literal(literal));
            text = self.read_text()?;
            text_bytes = text_bytes.saturating_add(text.len());
            if text_bytes > MAX_TEXT {
                return Err(ProtoError::Protocol(said!("the server's response is too long")));
            }
        }
        Err(ProtoError::Protocol(said!("the server's response has too many parts")))
    }

    /// Keep what a status line says in its response code.
    fn note_status(&mut self, line: &StatusLine) {
        match line.code.as_str() {
            "UIDVALIDITY" => self.selected.uid_validity = line.arg.parse().unwrap_or(0),
            "UIDNEXT" => self.selected.uid_next = line.arg.parse().unwrap_or(0),
            "COPYUID" if self.copied.len() < 2 => self.copied.push(copied_from(&line.arg)),
            "APPENDUID" if self.appended.len() < 2 => self.appended.push(appended_from(&line.arg)),
            "CAPABILITY" => self.capabilities = Some(line.arg.split_whitespace().take(MAX_CAPABILITIES).map(str::to_ascii_uppercase).collect()),
            _ => {}
        }
    }

    /// Keep what untagged data says about the session, whichever command it arrives during.
    fn note_data(&mut self, values: &[Value]) {
        match (values.first().and_then(Value::atom), values.get(1).and_then(Value::atom)) {
            (Some(count), Some(word)) if word.eq_ignore_ascii_case("EXISTS") => self.selected.exists = count.parse().unwrap_or(0),
            (Some(word), _) if word.eq_ignore_ascii_case("CAPABILITY") => {
                self.capabilities = Some(values.iter().skip(1).filter_map(Value::atom).take(MAX_CAPABILITIES).map(str::to_ascii_uppercase).collect());
            }
            _ => {}
        }
    }

    /// The server's own words for why it said no, fit to show.
    /// What the server said for itself, as it said it; MailCraft's words when it said nothing.
    fn refusal(&self, line: &StatusLine) -> Text {
        // Hidden before it is shortened: a secret cut in half would no longer be recognised.
        let text = tidy(&self.secrets.hide(&line.text));
        if text.is_empty() { said!("the command was refused") } else { Text::verbatim(text) }
    }
}

#[cfg(test)]
mod gmail_labels_tests;
#[cfg(test)]
mod gmail_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod tests;
