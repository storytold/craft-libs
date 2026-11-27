//! Printing sheets the application draws itself, through GDI on Windows. The default Windows
//! route hands the PDF to the system's own renderer; this one hands each sheet to a
//! [`SheetPainter`], which draws it into the page as pixels at the printer's resolution or as a
//! metafile of vector calls. Drivers and systems whose PDF renderer prints blurred get the
//! application's own rendering instead.
//!
//! The job is the same as [`crate::submit`]'s: the driver validates the job's options (paper,
//! two-sided, colour, the driver dialog's settings) before anything is sent, sheets are turned
//! and shrunk onto the job's paper as there, and copies are drawn by the application, collated
//! or not, with a blank back between collated copies of an odd two-sided document.

use crate::{Cancel, Error, Job, rgb};

/// The most pages one drawn job sends, copies included.
pub const MAX_DRAWN_PAGES: usize = 1_000_000;

/// One sheet to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnSheet {
    /// The sheet's page index in the PDF given to [`SheetPainter::open`].
    pub index: usize,
    /// The device page it fills. The sheet covers the whole paper, `(0, 0, page.paper.0,
    /// page.paper.1)` in points from the paper's top-left.
    pub page: rgb::Page,
}

/// Draws the sheets of a drawn job.
pub trait SheetPainter: Send {
    /// The print-ready PDF as it will print: every sheet already at the job's paper size and
    /// orientation. Called once, before anything is sent.
    fn open(&mut self, pdf: &[u8]) -> Result<(), Error>;
    /// Draws one sheet into the started page of `document`.
    fn paint(&mut self, sheet: &DrawnSheet, document: &mut rgb::Document) -> Result<(), Error>;
}

pub(crate) fn gdi_error(e: rgb::PrintError) -> Error {
    Error::Spool(e.to_string())
}

/// The pages of a job of `count` sheets, in the order they are sent: the sheet each page draws,
/// `None` for a blank back. Two-sided sheets go in pairs, so a pair is never split between
/// copies; collated copies of an odd two-sided document get a blank back between them, and so
/// do uncollated copies of the odd last sheet. `None` when there are no sheets or more than
/// [`MAX_DRAWN_PAGES`] pages.
pub(crate) fn page_order(count: usize, copies: u32, collate: bool, two_sided: bool) -> Option<Vec<Option<usize>>> {
    let copies = copies.clamp(1, 999) as usize;
    let per_unit = if two_sided { 2 } else { 1 };
    let units = count.div_ceil(per_unit);
    if count == 0 || units.checked_mul(per_unit)?.checked_mul(copies)? > MAX_DRAWN_PAGES {
        return None;
    }
    let unit = |u: usize| (u * per_unit..(u + 1) * per_unit).map(move |s| (s < count).then_some(s));
    let mut order: Vec<Option<usize>> = Vec::new();
    if collate {
        for _ in 0..copies {
            order.extend((0..units).flat_map(unit));
        }
    } else {
        for u in 0..units {
            for _ in 0..copies {
                order.extend(unit(u));
            }
        }
    }
    // The job's very last blank back adds nothing.
    if order.last() == Some(&None) {
        order.pop();
    }
    Some(order)
}

/// Prints a print-ready PDF with every sheet drawn by `painter` through GDI. Progress counts the
/// pages sent, copies included. `output_file` prints to that file instead of the device (drivers
/// that write files, and tests). Windows only; elsewhere an error says so and nothing is sent.
pub fn submit_drawn(
    pdf: &[u8],
    job: &Job,
    painter: &mut dyn SheetPainter,
    output_file: Option<&str>,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &Cancel,
) -> Result<String, Error> {
    #[cfg(windows)]
    {
        crate::windows::drawn::submit(pdf, job, painter, output_file, progress, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (pdf, job, painter, output_file, progress, cancel);
        Err(Error::Spool("printing sheets drawn by the application needs Windows; use the standard route".into()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[trial]
    fn collated_copies_of_an_odd_two_sided_document_start_on_a_new_sheet() {
        let order = page_order(3, 2, true, true).unwrap();
        assert_eq!(order, vec![Some(0), Some(1), Some(2), None, Some(0), Some(1), Some(2)]);
        // Every copy's first sheet is on a front: an even position.
        let fronts: Vec<usize> = order.iter().enumerate().filter(|(_, s)| **s == Some(0)).map(|(i, _)| i).collect();
        assert!(fronts.iter().all(|i| i % 2 == 0), "{fronts:?}");
    }

    #[trial]
    fn uncollated_copies_repeat_each_sheet_or_each_two_sided_pair() {
        assert_eq!(page_order(2, 3, false, false).unwrap(), vec![Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)]);
        let pairs = page_order(3, 2, false, true).unwrap();
        assert_eq!(pairs, vec![Some(0), Some(1), Some(0), Some(1), Some(2), None, Some(2)]);
        assert!(pairs.iter().enumerate().filter(|(_, s)| **s == Some(2)).all(|(i, _)| i % 2 == 0));
    }

    #[trial]
    fn one_sided_jobs_have_no_blank_pages_and_huge_jobs_are_refused() {
        let order = page_order(5, 3, true, false).unwrap();
        assert_eq!(order.len(), 15);
        assert!(order.iter().all(Option::is_some));
        assert_eq!(page_order(1, 1, true, true).unwrap(), vec![Some(0)]);
        assert!(page_order(0, 1, true, false).is_none());
        assert!(page_order(MAX_DRAWN_PAGES, 2, true, false).is_none());
        assert!(page_order(usize::MAX, 999, false, true).is_none());
    }
}
