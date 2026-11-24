//! Optional file path indexing; geometry is decoded only by the existing readers.
use super::{Budget, Diagnostic, Limits, Property, Scan, SourcePath, StopReason};
use crate::{Cursor, Error, Result};

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(Error::Cancelled) }
}

/// An immutable framing fact already observed during the required Xcf parse.
/// Missing end means the tolerant initial scan did not observe a terminator.
#[derive(Clone, Copy)]
pub(crate) struct Table {
    pub start: usize,
    pub end: Option<usize>,
    /// Slot bytes observed without overlapping a forward target are excluded,
    /// even when no complete table terminator could be established.
    pub minimum_end: usize,
    /// Earliest target bounds the pointer envelope when its zero is missing.
    pub target_start: Option<usize>,
}

pub(crate) fn offsets(
    mut cursor: Cursor<'_>,
    source: Option<Table>,
    limits: super::super::Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<usize>> {
    check(keep_going)?;
    let Some(source) = source else { return Ok(Scan::complete(Vec::new(), 0)) };
    cursor.pos = source.start;
    let retained = budget.retained_left;
    let mut records = Vec::new();
    loop {
        if let Err(error) = check(keep_going) {
            budget.retained_left = retained;
            return Err(error);
        }
        let offset = cursor.pos;
        let result = pointer(&mut cursor, source.end.unwrap_or(source.minimum_end), source.target_start, limits.records, records.len(), budget);
        match result {
            Ok(None) => return Ok(Scan::complete(records, cursor.pos)),
            Ok(Some(pointer)) => {
                if let Err(reason) = budget.retain(&mut records, pointer) {
                    return Ok(Scan::stopped(records, offset, reason));
                }
            }
            Err(reason) => return Ok(Scan::stopped(records, offset, reason)),
        }
    }
}

// A known terminator excludes the entire table, including its final zero.
// Without it, admit only forward targets from slots actually reached and report
// the eventual malformed/limited suffix; never infer an unobserved table end.
fn pointer(
    cursor: &mut Cursor<'_>,
    table_end: usize,
    target_start: Option<usize>,
    limit: usize,
    index: usize,
    budget: &mut Budget,
) -> std::result::Result<Option<usize>, StopReason> {
    let width = if cursor.wide { 8 } else { 4 };
    budget.work(width)?;
    if cursor.pos.checked_add(width).is_none_or(|end| target_start.is_some_and(|start| end > start)) {
        return Err(StopReason::Malformed);
    }
    let pointer = cursor.ptr().map_err(|_| StopReason::Malformed)?;
    if pointer == 0 {
        return Ok(None);
    }
    if pointer < table_end.max(cursor.pos) || pointer >= cursor.b.len() {
        return Err(StopReason::Malformed);
    }
    if index >= limit.min(crate::MAX_CHANNELS) {
        return Err(StopReason::RecordLimit);
    }
    Ok(Some(pointer))
}

// standalone_with_points returns at most one path. Track its actual decode
// debit separately from output growth rather than estimating geometry capacity.
fn merge_standalone<'a>(
    output: &mut Vec<SourcePath<'a>>,
    scan: Scan<SourcePath<'a>>,
    staged_charge: usize,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<(usize, Option<Diagnostic>)> {
    let Scan { mut records, stopped, end_offset } = scan;
    if output.is_empty() {
        *output = records;
    } else if let Some(path) = records.pop() {
        check(keep_going)?;
        let offset = path.offset;
        if let Err(reason) = budget.retain(output, path) {
            // The path (properties, strokes and knots) and its staging vector
            // are all dropped; earlier output and attempted work stay charged.
            drop(records);
            budget.retained_left += staged_charge;
            return Ok((offset, Some(Diagnostic { offset, ordinal: output.len(), reason })));
        }
        drop(records);
        // The private reader admitted one slot into an initially empty Vec via
        // Budget::retain. Only that exact container debit is now discarded.
        budget.retained_left += 2 * std::mem::size_of::<SourcePath<'_>>();
    }
    Ok((end_offset, stopped.map(|diagnostic| Diagnostic { ordinal: output.len(), ..diagnostic })))
}

pub(crate) fn decode<'a>(
    cursor: Cursor<'a>,
    source: Option<Table>,
    properties: Scan<Property<'a>>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
    property_charge: usize,
) -> Result<Scan<SourcePath<'a>>> {
    let mut property_charge = property_charge;
    let result = decode_inner(cursor, source, properties, limits, budget, keep_going, &mut property_charge);
    // The raw image-property vector has now been dropped. Its source payloads
    // remain borrowed, so only returned path storage stays charged.
    budget.retained_left += property_charge;
    result
}

fn decode_inner<'a>(
    cursor: Cursor<'a>,
    source: Option<Table>,
    properties: Scan<Property<'a>>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
    property_charge: &mut usize,
) -> Result<Scan<SourcePath<'a>>> {
    check(keep_going)?;
    let mut output = Vec::new();
    let mut end_offset = properties.end_offset;
    for property in properties.records {
        check(keep_going)?;
        if !matches!(property.kind, 23 | 25) {
            continue;
        }
        let remaining = Limits { paths: limits.paths.saturating_sub(output.len()), ..limits };
        let Some(offset) = property.offset.checked_add(8) else { return Ok(Scan::stopped(output, property.offset, StopReason::Malformed)) };
        let scan = super::read_list_into(property.payload, offset, property.kind == 25, remaining, budget, keep_going, output)?;
        let stopped = scan.stopped;
        output = scan.records;
        if let Some(diagnostic) = stopped {
            return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
        }
    }
    // The consuming property iterator is gone before standalone decoding begins.
    budget.retained_left += *property_charge;
    *property_charge = 0;
    if let Some(diagnostic) = properties.stopped {
        return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
    }
    let Some(source) = source else { return Ok(Scan::complete(output, end_offset)) };
    let mut table = cursor.at(source.start);
    let table_limit = limits.paths.saturating_sub(output.len());
    let mut points_left = limits.points;
    let mut strokes_left = limits.strokes;
    let mut source_index = 0;
    loop {
        check(keep_going)?;
        let slot = table.pos;
        let offset = match pointer(&mut table, source.end.unwrap_or(source.minimum_end), source.target_start, table_limit, source_index, budget) {
            Ok(Some(offset)) => offset,
            Ok(None) => return Ok(Scan::complete(output, end_offset.max(table.pos))),
            Err(reason) => return Ok(Scan::stopped(output, slot, reason)),
        };
        // Decode the admitted target before spending work on any later slot,
        // so a tight work allowance preserves the completed source prefix.
        let remaining = Limits { paths: limits.paths.saturating_sub(output.len()), points: points_left, strokes: strokes_left, ..limits };
        let retained = budget.retained_left;
        let scan = super::standalone_with_points(cursor.b, offset, source_index, remaining, budget, keep_going, &mut points_left)?;
        let staged_charge = retained - budget.retained_left;
        strokes_left = strokes_left.saturating_sub(scan.records.first().map_or(0, |path| path.strokes.len()));
        let (end, stopped) = merge_standalone(&mut output, scan, staged_charge, budget, keep_going)?;
        end_offset = end_offset.max(end);
        if let Some(diagnostic) = stopped {
            return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
        }
        source_index += 1; // pointer() caps this at MAX_CHANNELS before admission.
    }
}

#[cfg(test)]
mod tests;
