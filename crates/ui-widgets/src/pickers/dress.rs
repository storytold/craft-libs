//! `RowDress`: what a picker adds to an option's row.

use crate::list::ListRow;

/// What a picker adds to an option's row: the host's content for an option (a detail line, a trailing shortcut text, a
/// thumbnail painter). `Select::decorate` and `ComboBox::decorate` take the closure.
pub struct RowDress<'a, T>(Box<DressFn<'a, T>>);

type DressFn<'a, T> = dyn Fn(&T, ListRow<'a>) -> ListRow<'a> + 'a;

impl<'a, T> RowDress<'a, T> {
    /// The dress that runs `dress` over the row of each option, given the option's value.
    pub fn new(dress: impl Fn(&T, ListRow<'a>) -> ListRow<'a> + 'a) -> Self {
        RowDress(Box::new(dress))
    }

    /// The row of `value` as dressed.
    pub(super) fn apply(&self, value: &T, row: ListRow<'a>) -> ListRow<'a> {
        (self.0)(value, row)
    }
}
