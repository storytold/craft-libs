//! Optional file path indexing; geometry is decoded only by the existing readers.
use super::{Budget, Diagnostic, Limits, Property, Scan, SourcePath, StopReason};
use crate::{Cursor, Error, Result};

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(Error::Cancelled) }
}

pub(crate) fn offsets(
    mut cursor: Cursor<'_>,
    start: Option<usize>,
    limits: super::super::Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<usize>> {
    check(keep_going)?;
    let Some(start) = start else { return Ok(Scan::complete(Vec::new(), 0)) };
    cursor.pos = start;
    let retained = budget.retained_left;
    let mut records = Vec::new();
    loop {
        if let Err(error) = check(keep_going) {
            budget.retained_left = retained;
            return Err(error);
        }
        let offset = cursor.pos;
        let result = pointer(&mut cursor, limits.records, records.len(), budget);
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

// Validate only the slots reached by this optional scan. A disclosed prefix is
// not a claim that the unread remainder of the table is well framed.
fn pointer(cursor: &mut Cursor<'_>, limit: usize, index: usize, budget: &mut Budget) -> std::result::Result<Option<usize>, StopReason> {
    budget.work(if cursor.wide { 8 } else { 4 })?;
    let pointer = cursor.ptr().map_err(|_| StopReason::Malformed)?;
    if pointer == 0 {
        return Ok(None);
    }
    if pointer < cursor.pos || pointer >= cursor.b.len() {
        return Err(StopReason::Malformed);
    }
    if index >= limit.min(crate::MAX_CHANNELS) {
        return Err(StopReason::RecordLimit);
    }
    Ok(Some(pointer))
}

fn merge<'a>(
    output: &mut Vec<SourcePath<'a>>,
    scan: Scan<SourcePath<'a>>,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<(usize, Option<Diagnostic>)> {
    let Scan { records, stopped, end_offset } = scan;
    if output.is_empty() {
        *output = records;
    } else {
        // Both containers may be live: admit geometric output growth before allocating.
        for path in records {
            check(keep_going)?;
            let offset = path.offset;
            if let Err(reason) = budget.retain(output, path) {
                return Ok((offset, Some(Diagnostic { offset, ordinal: output.len(), reason })));
            }
        }
    }
    Ok((end_offset, stopped.map(|diagnostic| Diagnostic { ordinal: output.len(), ..diagnostic })))
}

pub(crate) fn decode<'a>(
    cursor: Cursor<'a>,
    start: Option<usize>,
    properties: Scan<Property<'a>>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
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
        let scan = if property.kind == 23 {
            super::legacy(property.payload, offset, remaining, budget, keep_going)?
        } else {
            super::vectors(property.payload, offset, remaining, budget, keep_going)?
        };
        let (_, stopped) = merge(&mut output, scan, budget, keep_going)?;
        if let Some(diagnostic) = stopped {
            return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
        }
    }
    if let Some(diagnostic) = properties.stopped {
        return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
    }
    let Some(start) = start else { return Ok(Scan::complete(output, end_offset)) };
    let mut table = cursor.at(start);
    let table_limit = limits.paths.saturating_sub(output.len());
    let mut points_left = limits.points;
    let mut strokes_left = limits.strokes;
    let mut source_index = 0;
    loop {
        check(keep_going)?;
        let slot = table.pos;
        let offset = match pointer(&mut table, table_limit, source_index, budget) {
            Ok(Some(offset)) => offset,
            Ok(None) => return Ok(Scan::complete(output, end_offset.max(table.pos))),
            Err(reason) => return Ok(Scan::stopped(output, slot, reason)),
        };
        // Decode the admitted target before spending work on any later slot,
        // so a tight work allowance preserves the completed source prefix.
        let remaining = Limits { paths: limits.paths.saturating_sub(output.len()), points: points_left, strokes: strokes_left, ..limits };
        let scan = super::standalone_with_points(cursor.b, offset, source_index, remaining, budget, keep_going, &mut points_left)?;
        strokes_left = strokes_left.saturating_sub(scan.records.first().map_or(0, |path| path.strokes.len()));
        let (end, stopped) = merge(&mut output, scan, budget, keep_going)?;
        end_offset = end_offset.max(end);
        if let Some(diagnostic) = stopped {
            return Ok(Scan::stopped(output, diagnostic.offset, diagnostic.reason));
        }
        source_index += 1; // pointer() caps this at MAX_CHANNELS before admission.
    }
}

#[cfg(test)]
mod tests;
