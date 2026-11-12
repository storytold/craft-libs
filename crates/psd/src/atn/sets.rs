//! Whole-file version-16 set streams. The original single-set/legacy API remains compatible.
use super::{
    AtnFile, AtnSetSpan, MAX_ACTIONS, MAX_ITEMS, SpannedAtnArchive, VERSION, codepage, name_encoding_sets, read_set_with_spans, read_version,
    write_set,
};
use crate::{
    error::{PsdError, Result},
    io::{Reader, WriteExt},
};

/// Whole-archive resource ceilings, not Photoshop format maxima. Per-set/action ceilings still apply.
pub const MAX_SETS: usize = MAX_ACTIONS;
/// Maximum action headers across the complete archive, including any metadata sidecar.
pub const MAX_TOTAL_ACTIONS: usize = MAX_ACTIONS;
/// Maximum steps across the complete archive, including any metadata sidecar.
pub const MAX_TOTAL_ITEMS: usize = MAX_ITEMS;

#[derive(Default)]
pub(super) struct Budget {
    actions: usize,
    items: usize,
}

impl Budget {
    pub(super) fn actions(&mut self, count: usize) -> Result<()> {
        if count > MAX_ACTIONS {
            return Err(PsdError::LimitExceeded("too many actions in one set"));
        }
        self.actions = self
            .actions
            .checked_add(count)
            .filter(|sum| *sum <= MAX_TOTAL_ACTIONS)
            .ok_or(PsdError::LimitExceeded("too many actions in actions archive"))?;
        Ok(())
    }

    pub(super) fn items(&mut self, count: usize) -> Result<()> {
        if count > MAX_ITEMS {
            return Err(PsdError::LimitExceeded("too many steps in one action"));
        }
        self.items = self
            .items
            .checked_add(count)
            .filter(|sum| *sum <= MAX_TOTAL_ITEMS)
            .ok_or(PsdError::LimitExceeded("too many steps in actions archive"))?;
        Ok(())
    }
}

/// Read every set in a version-16 file, in order, with one version prefix and strict EOF.
/// A malformed later set fails the whole result. Version12 retains the existing one-set/tail contract.
pub fn parse_sets(data: &[u8]) -> Result<Vec<AtnFile>> {
    parse_sets_with_encoding(data, None)
}

/// [`parse_sets`] with one legacy name encoding for the complete file. Automatic detection also
/// uses the complete stream; its diagnostic is attached once to the first set.
pub fn parse_sets_with_encoding(data: &[u8], encoding: Option<&str>) -> Result<Vec<AtnFile>> {
    parse_archive(data, encoding, false).map(|archive| archive.sets)
}

/// Read a strict complete version16 archive, retaining physical record/field ranges.
/// Legacy version12 conversion is deliberately excluded: its descriptor dialect cannot be
/// spliced unchanged into a version16 library. Existing import APIs still accept it.
/// A header without any real set remains invalid; an empty logical library needs a valid carrier.
pub fn parse_sets_with_spans(data: &[u8]) -> Result<SpannedAtnArchive> {
    parse_archive(data, None, true)
}

fn parse_archive(data: &[u8], encoding: Option<&str>, capture: bool) -> Result<SpannedAtnArchive> {
    let forced = encoding.map(codepage::resolve_label).transpose()?;
    let mut reader = Reader::new(data);
    let version = read_version(&mut reader)?;
    let legacy = version == 12;
    if capture && legacy {
        return Err(PsdError::Unsupported("raw Actions library provenance requires version16".into()));
    }
    let mut files = Vec::new();
    let mut spans = Vec::new();
    let mut budget = Budget::default();
    loop {
        if files.len() >= MAX_SETS {
            return Err(PsdError::LimitExceeded("too many sets in actions archive"));
        }
        let before = reader.remaining();
        let mut span = capture.then(AtnSetSpan::default);
        let mut file = read_set_with_spans(&mut reader, legacy, Some(&mut budget), span.as_mut())?;
        if let Some(span) = span {
            spans.push(span);
        }
        if legacy && !reader.is_empty() {
            file.warnings.push(format!("{} trailing bytes ignored", reader.remaining()));
        }
        if reader.remaining() >= before {
            return Err(PsdError::invalid("actions set made no progress"));
        }
        files.push(file);
        if legacy || reader.is_empty() {
            break;
        }
    }
    let mut warnings = Vec::new();
    let encoding = forced.or_else(|| name_encoding_sets(&files, &mut warnings)).map(|encoding| encoding.name().to_string());
    for file in &mut files {
        file.encoding = encoding.clone();
    }
    if let Some(first) = files.first_mut() {
        first.warnings.extend(warnings);
    }
    Ok(SpannedAtnArchive { sets: files, spans })
}

/// Write all sets under one version16 prefix. Validate aggregate counts before allocating output.
/// At least one set is required; an empty set is valid. Stored names/descriptors stay verbatim.
pub fn write_sets(files: &[AtnFile]) -> Result<Vec<u8>> {
    if files.is_empty() {
        return Err(PsdError::invalid("actions archive must contain at least one set"));
    }
    if files.len() > MAX_SETS {
        return Err(PsdError::LimitExceeded("too many sets in actions archive"));
    }
    let mut budget = Budget::default();
    for file in files {
        budget.actions(file.actions.len())?;
        for action in &file.actions {
            budget.items(action.items.len())?;
        }
    }
    let mut out = Vec::new();
    out.put_u32(VERSION);
    for file in files {
        write_set(&mut out, file)?;
    }
    Ok(out)
}
