//! The packages of versions 14 on (R25, R26): a ZIP archive holding the RIFF stream as
//! `content/riffData.cdr` (14 and 15), or as `content/root.dat` with its chunk bodies in the data
//! streams that `content/dataFileList.dat` names (16 on). A data stream is inflated only when a
//! chunk the reader reads, or a list it splits, points into it.

use std::borrow::Cow;
use std::cell::{Cell, OnceCell};

use crate::limits::{Budget, Limits};
use crate::riff::push;
use crate::zip::{self, Archive};
use crate::{Error, Warning};

const RIFF_DATA: &[u8] = b"content/riffData.cdr";
const ROOT: &[u8] = b"content/root.dat";
const DATA_FILE_LIST: &[u8] = b"content/dataFileList.dat";
const DATA_DIR: &[u8] = b"content/data/";

/// Is `bytes` a ZIP archive with a `.cdr` drawing's root stream?
pub(crate) fn is_package(bytes: &[u8]) -> bool {
    zip::is_zip(bytes) && Archive::open(bytes, Limits::default().members).is_ok_and(|a| a.find(RIFF_DATA).or_else(|| a.find(ROOT)).is_some())
}

/// The RIFF stream of the package `bytes`, and its data streams when it has them (16 on).
pub(crate) fn open<'a>(bytes: &'a [u8], budget: &mut Budget) -> Result<(Cow<'a, [u8]>, Option<Streams<'a>>), Error> {
    let zip = Archive::open(bytes, budget.limits.members)?;
    if let Some(i) = zip.find(RIFF_DATA) {
        return Ok((member(&zip, i, budget)?, None));
    }
    let root = zip.find(ROOT).ok_or(Error::NotCdr)?;
    let root = member(&zip, root, budget)?;
    let list = match zip.find(DATA_FILE_LIST) {
        Some(i) => member(&zip, i, budget)?,
        None => Cow::Borrowed(&[][..]),
    };
    // One name for each line; a line feed after the last name ends it.
    let list = list.strip_suffix(b"\n").unwrap_or(&list);
    let mut members = Vec::new();
    if !list.is_empty() {
        for line in list.split(|&b| b == b'\n') {
            if members.len() == budget.limits.members {
                return Err(Error::Limit("package member count"));
            }
            let name = line.strip_suffix(b"\r").unwrap_or(line);
            let found = zip.find(&[DATA_DIR, name].concat());
            // A data stream that is encrypted or packed some other way refuses the file, as the
            // root stream does.
            if let Some(i) = found {
                zip.check(i)?;
            }
            members.push(found);
        }
    }
    let loaded = (0..zip.count()).map(|_| OnceCell::new()).collect();
    let streams = Streams { zip, members, loaded, member_limit: budget.limits.member, damaged: Cell::new(false) };
    Ok((root, Some(streams)))
}

/// Member `i` within the member limit and, when deflated, what is left to inflate.
fn member<'a>(zip: &Archive<'a>, i: usize, budget: &mut Budget) -> Result<Cow<'a, [u8]>, Error> {
    let m = zip.read(i, budget.limits.member, budget.inflate_left())?;
    if let Cow::Owned(v) = &m {
        budget.take_inflated(v.len())?;
    }
    Ok(m)
}

/// What reading one package member gave.
#[derive(Debug)]
enum Slot<'a> {
    Read(Cow<'a, [u8]>),
    Damaged,
    PastLimit,
}

/// The data streams of a version 16+ package, in `dataFileList.dat` order (R26).
#[derive(Debug)]
pub(crate) struct Streams<'a> {
    zip: Archive<'a>,
    /// The member holding each stream; `None` when the archive lacks it.
    members: Vec<Option<usize>>,
    /// Each member's bytes once read, so that streams naming one member read it once.
    loaded: Vec<OnceCell<Slot<'a>>>,
    member_limit: usize,
    /// A stream, or a chunk's bytes in one, could not be found.
    damaged: Cell<bool>,
}

impl<'a> Streams<'a> {
    /// The member holding stream `s`; `None` when there is no such stream.
    pub(crate) fn member_of(&self, s: usize) -> Option<usize> {
        *self.members.get(s)?
    }

    /// The size the archive gives stream `s`; `None` when there is no such stream.
    pub(crate) fn size(&self, s: usize) -> Option<usize> {
        Some(self.zip.size(self.member_of(s)?))
    }

    /// Stream `s`, its member read on first use within the member limit and, when deflated,
    /// within `left` more inflated bytes (which it takes from).
    pub(crate) fn get(&self, s: usize, left: &Cell<usize>) -> Option<&[u8]> {
        let Some(i) = self.member_of(s) else {
            self.damaged.set(true);
            return None;
        };
        let slot = self.loaded.get(i)?.get_or_init(|| {
            let read = self.zip.read(i, self.member_limit, left.get());
            if !matches!(read, Err(Error::Limit(_))) {
                left.set(left.get().saturating_sub(self.zip.inflate_cost(i)));
            }
            match read {
                Ok(b) => Slot::Read(b),
                Err(Error::Limit(_)) => Slot::PastLimit,
                Err(_) => {
                    self.damaged.set(true);
                    Slot::Damaged
                }
            }
        });
        match slot {
            Slot::Read(b) => Some(b.as_ref()),
            Slot::Damaged | Slot::PastLimit => None,
        }
    }

    /// Was stream `s` left out because it is past a limit? Chunks in it are then not damage.
    pub(crate) fn past_limit(&self, s: usize) -> bool {
        let slot = self.member_of(s).and_then(|i| self.loaded.get(i)).and_then(OnceCell::get);
        matches!(slot, Some(Slot::PastLimit))
    }

    /// Mark a chunk whose stream bytes could not be found.
    pub(crate) fn note_damage(&self) {
        self.damaged.set(true);
    }

    /// The warnings for streams that were left out.
    pub(crate) fn warnings(&self, warn: &mut Vec<Warning>) {
        if self.damaged.get() {
            push(warn, Warning::Damaged);
        }
        if self.loaded.iter().any(|s| matches!(s.get(), Some(Slot::PastLimit))) {
            push(warn, Warning::TooDeep);
        }
    }
}
