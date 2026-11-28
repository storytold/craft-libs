//! The guide layout calculation and the units the guide dialogs measure in.

use craft_behavior::Axis;
use craft_ui::canvas::Guide;
use craft_ui::patterns::form_dialogs::{GuideGroup, GuideLayoutError, GuideLayoutOptions, GuideLength, GuideRect, GuideTarget, GuideUnits};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::trial;

const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 800.0;
/// Edges of 8 columns: both sides of each.
const EIGHT_COLUMN_EDGES: usize = 16;
/// The most columns the engine takes, one more than allowed.
const TOO_MANY_COLUMNS: u32 = 1000;
const TARGETS_OVER_LIMIT: usize = 8;

fn pixel_units() -> GuideUnits {
    GuideUnits::new(UnitKind::Pixels, WIDTH, HEIGHT, 72.0)
}

fn document() -> GuideRect {
    GuideRect::new(0.0, 0.0, WIDTH, HEIGHT)
}

fn just_margin(margin: [f64; 4]) -> GuideLayoutOptions {
    let mut options = GuideLayoutOptions::default();
    options.columns.enabled = false;
    options.margin = margin.map(GuideLength::pixels);
    options
}

fn made(options: &GuideLayoutOptions, targets: &[GuideRect], existing: &[Guide]) -> Result<Vec<Guide>, GuideLayoutError> {
    options.lines(targets, existing, &pixel_units(), 1, None)
}

fn positions(guides: &[Guide], along: Axis) -> Vec<f64> {
    guides.iter().filter(|guide| guide.along == along).map(|guide| guide.position).collect()
}

/// Eight columns with a 20 px gutter over 1000 px make 16 vertical lines; a fixed width with Center Columns centres
/// them; margins add their four lines; a layout wider than the room, margins over the size, nothing turned on and
/// too many lines are errors.
#[trial]
fn layout_lines_follow_the_columns_rows_and_margins() {
    let eight = made(&GuideLayoutOptions::default(), &[document()], &[]).expect("the factory layout fits");
    let across = positions(&eight, Axis::Y);
    assert_eq!(across.len(), EIGHT_COLUMN_EDGES);
    assert_eq!(&across[..4], [0.0, 107.5, 127.5, 235.0]);
    assert!(positions(&eight, Axis::X).is_empty(), "rows off add none");

    let mut fixed = GuideLayoutOptions::default();
    fixed.columns = GuideGroup::new(true, 3, Some(GuideLength::pixels(100.0)), GuideLength::pixels(20.0));
    fixed.center_columns = true;
    assert_eq!(positions(&made(&fixed, &[document()], &[]).unwrap(), Axis::Y), [330.0, 430.0, 450.0, 550.0, 570.0, 670.0]);
    fixed.center_columns = false;
    assert_eq!(positions(&made(&fixed, &[document()], &[]).unwrap(), Axis::Y)[..2], [0.0, 100.0]);

    let margins = made(&just_margin([10.0, 20.0, 30.0, 40.0]), &[document()], &[]).unwrap();
    assert_eq!((positions(&margins, Axis::Y), positions(&margins, Axis::X)), (vec![20.0, 960.0], vec![10.0, 770.0]));
    let mut percent = just_margin([0.0; 4]);
    percent.margin[1] = GuideLength::percent(10.0);
    assert_eq!(positions(&made(&percent, &[document()], &[]).unwrap(), Axis::Y), [100.0, 1000.0], "percent is of the target's width");
    let mut off = just_margin([10.0; 4]);
    off.margin_enabled = false;
    assert_eq!(made(&off, &[document()], &[]), Err(GuideLayoutError::Nothing), "Margin off adds nothing");

    let mut wide = GuideLayoutOptions::default();
    wide.columns.size = Some(GuideLength::pixels(200.0));
    assert_eq!(made(&wide, &[document()], &[]), Err(GuideLayoutError::DoesNotFit));
    assert_eq!(made(&just_margin([0.0, 600.0, 0.0, 600.0]), &[document()], &[]), Err(GuideLayoutError::MarginsTooLarge));
    assert_eq!(made(&just_margin([0.0; 4]), &[document()], &[]), Err(GuideLayoutError::Nothing));
    let mut selected = GuideLayoutOptions::default();
    selected.target = GuideTarget::SelectedArtboards;
    assert_eq!(made(&selected, &[], &[]), Err(GuideLayoutError::NoArtboards));
    let mut many = GuideLayoutOptions::default();
    many.columns.number = TOO_MANY_COLUMNS;
    many.columns.gutter = GuideLength::pixels(0.0);
    assert_eq!(made(&many, &[document(); TARGETS_OVER_LIMIT], &[]), Err(GuideLayoutError::TooMany));
}
