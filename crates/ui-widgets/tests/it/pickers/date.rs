//! Date values and the calendar maths: parsing at a precision, and moves across month and year edges.

use craft_behavior::NavMove;
use craft_i18n::{DateFormat, Weekday};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::pickers::{CalendarPopover, CalendarView, DatePicker, DatePrecision, PickedDate};
use craft_ui::runtime::env::Env;
use craft_ui::testing::Fixture;
use egui::{Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const YEAR: i32 = 2026;
const JANUARY: u8 = 1;
const FEBRUARY: u8 = 2;
const AUGUST: u8 = 8;
const SEPTEMBER: u8 = 9;
const OCTOBER: u8 = 10;
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

/// The page titles of August, September and October in the fixture's calendar.
fn record_titles(ui: &mut Ui, titles: &RefCell<Vec<String>>) {
    let dates = Env::of(ui).dates();
    *titles.borrow_mut() = [AUGUST, SEPTEMBER, OCTOBER].iter().map(|month| dates.local_year_month(YEAR, *month)).collect();
}

/// An open calendar on an August value: Next pages to September and the page stays there while the value does; an
/// October value set from outside moves it to October.
fn follows(fixture: &mut Fixture, value: &Cell<Option<PickedDate>>, titles: &RefCell<Vec<String>>, what: &str) {
    let shows = |fixture: &Fixture, month: usize| fixture.by_label(&titles.borrow()[month]).is_some();
    assert!(shows(fixture, 0), "{what} opens on the value's month");
    let next = fixture.by_label("Next").expect("the calendar's next button is shown");
    fixture.click(&next).idle(4);
    assert!(shows(fixture, 1) && !shows(fixture, 0), "{what} pages to the next month");
    fixture.idle(4);
    assert!(shows(fixture, 1), "{what} keeps the arrows' month while the value stays");
    value.set(Some(PickedDate::day(YEAR, OCTOBER, 3)));
    fixture.idle(4);
    assert!(shows(fixture, 2) && !shows(fixture, 1), "{what} moves to the month of a value set from outside");
}

/// A value set from outside while the calendar is open moves its page; the arrows' month stays otherwise.
#[craft_ui::testing::trial]
fn the_calendar_popover_follows_a_value_set_while_it_is_open() {
    let value = Rc::new(Cell::new(Some(PickedDate::day(YEAR, AUGUST, 14))));
    let titles = Rc::new(RefCell::new(Vec::new()));
    let (shown, named) = (value.clone(), titles.clone());
    let mut fixture = Fixture::new(move |ui| {
        record_titles(ui, &named);
        let opener = Button::new("Due").show(ui);
        let mut open = true;
        let mut now = shown.get();
        CalendarPopover::new("Due date", Bind::bound(&mut open), Bind::bound(&mut now))
            .precisions(&[DatePrecision::Day])
            .opened_by(&opener)
            .show(ui, |_, _| {});
        shown.set(now);
    });
    fixture.idle(10);
    follows(&mut fixture, &value, &titles, "the calendar popover");

    let value = Rc::new(Cell::new(Some(PickedDate::day(YEAR, AUGUST, 14))));
    let (shown, named) = (value.clone(), titles.clone());
    let mut fixture = Fixture::new(move |ui| {
        record_titles(ui, &named);
        let mut now = shown.get();
        DatePicker::new("From", Bind::bound(&mut now)).precisions(&[DatePrecision::Day]).show(ui);
        shown.set(now);
    });
    fixture.idle(10);
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
    follows(&mut fixture, &value, &titles, "the date picker's calendar");
}

/// A date format keeps the weeks starting where the app puts them, whatever the locale says.
#[craft_ui::testing::trial]
fn a_date_format_starts_its_weeks_where_the_app_says() {
    use craft_i18n::Weekday;
    let europe = DateFormat::from_tag("de-DE");
    assert_eq!(europe.first_weekday(), Weekday::Monday, "the locale's own week");
    let sunday = europe.with_first_weekday(Weekday::Sunday);
    assert_eq!(sunday.first_weekday(), Weekday::Sunday, "the app's week");
    let week = CalendarView::open(Some(PickedDate::day(YEAR, FEBRUARY, 1)), None, &[DatePrecision::Day], &sunday);
    assert_eq!(
        craft_ui::pickers::month_grid(YEAR, FEBRUARY, sunday.first_weekday()).iter().position(Option::is_some),
        Some(0),
        "{week:?}: February 2026 starts a Sunday-first week"
    );
}

/// A calendar page starts its weeks on the day it is given: February 2026 begins on a Sunday, so its 1st is in the first
/// column of a Sunday-first week and the last of a Monday-first one.
#[craft_ui::testing::trial]
fn a_calendar_page_starts_its_weeks_on_the_day_it_is_given() {
    use craft_ui::pickers::Calendar;
    let first_cell = |day: Weekday| {
        let view = CalendarView::open(
            Some(PickedDate::day(YEAR, FEBRUARY, 1)),
            None,
            &[DatePrecision::Day],
            &DateFormat::from_tag("en-US").with_first_weekday(day),
        );
        let mut fixture = Fixture::new(move |ui| {
            Calendar::new("Due date", view, None).first_weekday(day).show(ui);
        });
        fixture.settle();
        fixture.texts().iter().find(|run| run.text == "1").expect("the 1st is painted").rect.center().x
    };
    let (sunday, monday) = (first_cell(Weekday::Sunday), first_cell(Weekday::Monday));
    assert!(sunday < monday, "the 1st is in the first column of a Sunday-first week ({sunday}) and the last of a Monday-first one ({monday})");
}

/// Today is centred under the page, Clear is offered only while the field holds a date, and the weekday heads are the
/// two-letter names starting with Sunday's.
#[craft_ui::testing::trial]
fn today_is_centred_and_clear_waits_for_a_value() {
    let mut picked: Option<PickedDate> = None;
    let mut open = true;
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Pick").show(ui);
        CalendarPopover::new("Due date", Bind::bound(&mut open), Bind::bound(&mut picked))
            .opened_by(&opener)
            .precisions(&[DatePrecision::Day])
            .first_weekday(Weekday::Sunday)
            .today(PickedDate::day(2026, 2, 10))
            .show(ui, |_, _| {});
    });
    fixture.room(egui::vec2(600.0, 500.0)).settle();
    let texts = fixture.texts();
    let at = |word: &str| texts.iter().find(|run| run.text == word).unwrap_or_else(|| panic!("{word} is painted")).rect.center().x;
    assert!(texts.iter().all(|run| run.text != "Clear"), "no value, no Clear");
    let middle = (at("Su") + at("Sa")) / 2.0;
    assert!((at("Today") - middle).abs() < 6.0, "Today is centred: {} against {middle}", at("Today"));
}
