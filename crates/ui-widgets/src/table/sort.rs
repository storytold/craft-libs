//! Ordering rows by the table's sort state.

use crate::contract::ColumnSort;
use craft_behavior::ColumnKey;
use std::cmp::Ordering;

/// Sorts `rows` in place by `sort`: `compare` orders two rows by one column, ascending, and a descending sort
/// reverses its result. Rows that compare equal keep their order in both directions. `None` leaves the rows as they
/// are.
pub fn sort_rows<T>(rows: &mut [T], sort: Option<&ColumnSort>, compare: impl Fn(&T, &T, &ColumnKey) -> Ordering) {
    let Some(sort) = sort else { return };
    rows.sort_by(|a, b| {
        let order = compare(a, b, &sort.key);
        if sort.descending { order.reverse() } else { order }
    });
}
