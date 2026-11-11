//! Date values and the calendar maths: parsing at a precision, and moves across month and year edges.

use craft_behavior::NavMove;
use craft_i18n::DateFormat;
use craft_ui::pickers::{CalendarView, DatePrecision, PickedDate};

const YEAR: i32 = 2026;
const JANUARY: u8 = 1;
const FEBRUARY: u8 = 2;
/// February 2026 has 28 days and starts on a Sunday.
const FEBRUARY_DAYS: u8 = 28;
const LAST_JANUARY_DAY: u8 = 31;
const WEEK: i32 = 7;

/// A value reads as its year, month or day, past a trailing time, and a date that does not exist is none.
#[craft_ui::testing::trial]
fn picked_dates_parse_at_their_precision() {
    assert_eq!(PickedDate::parse("2026"), Some(PickedDate::year(YEAR)));
    assert_eq!(PickedDate::parse("2026-08"), Some(PickedDate::month(YEAR, 8)));
    assert_eq!(PickedDate::parse("2026-08-14"), Some(PickedDate::day(YEAR, 8, 14)));
    assert_eq!(PickedDate::parse("2026-08-14T10:30"), Some(PickedDate::day(YEAR, 8, 14)));
    assert_eq!(PickedDate::parse("2026-02-30"), None);
    assert_eq!(PickedDate::parse("2026-8"), None);
    assert_eq!(PickedDate::year(7).iso(), "0007");
}

/// A step past the month's first or last row goes on into the neighbouring month, a day or a week from the cell; paging
/// keeps the place in the month at most the new month's last day.
#[craft_ui::testing::trial]
fn the_calendar_crosses_months_past_the_grids_ends_and_pages() {
    let dates = DateFormat::from_tag("en-US");
    let january = PickedDate::day(YEAR, JANUARY, 1);
    let view = CalendarView::open(Some(january), None, &[DatePrecision::Day], &dates);
    assert_eq!((view.year, view.month, view.active), (YEAR, JANUARY, 1));
    let before = view.crossed(NavMove::Back, 1, &dates);
    assert_eq!((before.year, before.month, before.active), (YEAR - 1, 12, i32::from(LAST_JANUARY_DAY)), "the day before the 1st");
    let above = view.crossed(NavMove::Up, 3, &dates);
    assert_eq!((above.year, above.month, above.active), (YEAR - 1, 12, 27), "a week before January 3rd");
    let below = view.crossed(NavMove::Down, i32::from(LAST_JANUARY_DAY), &dates);
    assert_eq!((below.month, below.active), (FEBRUARY, WEEK), "a week after January 31st");

    let last = PickedDate::day(YEAR, JANUARY, LAST_JANUARY_DAY);
    let end = CalendarView::open(Some(last), None, &[DatePrecision::Day], &dates);
    let next = end.crossed(NavMove::PageDown, end.active, &dates);
    assert_eq!((next.month, i32::from(FEBRUARY_DAYS)), (FEBRUARY, next.active));
    assert_eq!(next.crossed(NavMove::PageUp, next.active, &dates).month, JANUARY);
    assert_eq!(view.crossed(NavMove::Start, 1, &dates), view, "Home is the grid's");
}
