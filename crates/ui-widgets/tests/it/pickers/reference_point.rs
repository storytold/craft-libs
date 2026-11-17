//! `ReferencePoint` marks its nine points with the dot the picker has always shown; asked for a presentation, it marks
//! them with the squares of the box's grid, joined to their neighbours, and keeps every part, cell, pick and name. A mark
//! keeps its share of its cell in a room shorter than the picker asks for, so the dots, the squares and the joins between
//! the squares fit a room of any size; the trials below read that of both presentations.

use craft_behavior::Anchor9;
use craft_theme::Size;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::pickers::{PointPresentation, ReferencePoint};
use craft_ui::testing::Fixture;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::trial;
use egui::epaint::RectShape;
use egui::{Key, Modifiers, Pos2, Rect, Shape, Ui};
use std::cell::Cell;
use std::rc::Rc;

/// Three points on each side of the picker's grid.
const GRID: usize = 3;

/// The centre point's place among the nine, which the fixtures pick.
const CENTRE: usize = 4;

/// The slack a painted shape is compared with: the rounding of the layout.
const ROUNDING: f32 = 0.5;

/// The slack a mark's share of its cell is compared with between two rooms: each room's cells are laid out on their own,
/// so the share read off them moves this much at most.
const SHARE_SLACK: f32 = 0.02;

/// The nine points' names, in the order the picker declares them: row by row from the top left.
const NAMES: [&str; GRID * GRID] = ["Top left", "Top", "Top right", "Left", "Center", "Right", "Bottom left", "Bottom", "Bottom right"];

/// The picker a trial shows, holding the point it edits and the mark it shows it with.
struct Picker {
    point: Cell<Anchor9>,
    presentation: PointPresentation,
    compact: bool,
    growth: Option<(f64, f64)>,
}

impl Picker {
    /// The picker as the library shows it, the centre picked.
    fn dots() -> Self {
        Picker { point: Cell::new(Anchor9::Centre), presentation: PointPresentation::Dots, compact: false, growth: None }
    }

    fn squares() -> Self {
        Picker { presentation: PointPresentation::ConnectedSquares, ..Picker::dots() }
    }

    /// The picker the canvas grows both ways about, so every point beside the centre has an arrow.
    fn grown(self) -> Self {
        Picker { growth: Some((1.0, 1.0)), ..self }
    }

    fn compact(self) -> Self {
        Picker { compact: true, ..self }
    }

    /// The picker this holds, over `point`.
    fn build<'b>(&self, point: &'b mut Anchor9) -> ReferencePoint<'b> {
        let mut picker = ReferencePoint::new("Reference point", Bind::bound(point)).presentation(self.presentation);
        if self.compact {
            picker = picker.compact();
        }
        if let Some(growth) = self.growth {
            picker = picker.growth(growth);
        }
        picker
    }

    fn show(&self, ui: &mut Ui) {
        let mut point = self.point.get();
        self.build(&mut point).show(ui);
        self.point.set(point);
    }

    /// The picker at `rect`, the room a fixed slot gives it, rather than the size it asks for.
    fn show_in(&self, ui: &mut Ui, rect: Rect) {
        let mut point = self.point.get();
        self.build(&mut point).show_at(ui, rect);
        self.point.set(point);
    }
}

/// A settled fixture showing `picker`, which the caller keeps to read the point back.
fn settled(picker: &Rc<Picker>) -> Fixture {
    let shown = picker.clone();
    let mut fixture = Fixture::new(move |ui| shown.show(ui));
    fixture.settle();
    fixture
}

/// A settled fixture showing `picker` at `rect`, which the caller keeps to read the point back.
fn settled_in(picker: &Rc<Picker>, rect: Rect) -> Fixture {
    let shown = picker.clone();
    let mut fixture = Fixture::new(move |ui| shown.show_in(ui, rect));
    fixture.settle();
    fixture
}

/// The share of the requested width a narrow fixed slot leaves; its height stays as requested.
const SLOT: f32 = 0.5;

/// The nine points' cells, as the picker reports them to a screen reader: also the names and the count it keeps.
fn cells(fixture: &Fixture) -> Vec<Rect> {
    NAMES.iter().map(|name| fixture.by_label(name).unwrap_or_else(|| panic!("no `{name}` point")).rect).collect()
}

/// The mark a point shows: the shape drawn inside its cell that is smaller than the cell and as tall as it is wide.
fn mark(shapes: &[Shape], cell: Rect) -> Option<&RectShape> {
    shapes.iter().find_map(|shape| match shape {
        Shape::Rect(rect)
            if cell.contains_rect(rect.rect) && rect.rect.width() < cell.width() && (rect.rect.width() - rect.rect.height()).abs() <= ROUNDING =>
        {
            Some(rect)
        }
        _ => None,
    })
}

/// The mark of the point in `cell`, whatever its size: the shape as tall as it is wide centred on the cell.
fn centred_mark(shapes: &[Shape], cell: Rect) -> Option<&RectShape> {
    // The centre cell shares its centre with the picker's background. Choose the smallest centred square, not the
    // first painted one, without requiring containment: an oversized mark must still reach the containment assertion.
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if same(rect.rect.center(), cell.center()) && (rect.rect.width() - rect.rect.height()).abs() <= ROUNDING => Some(rect),
            _ => None,
        })
        .min_by(|a, b| a.rect.width().total_cmp(&b.rect.width()))
}

/// The mark of each of the nine points, in the order of [`NAMES`].
fn marks<'a>(shapes: &'a [Shape], cells: &[Rect]) -> Vec<&'a RectShape> {
    cells.iter().map(|cell| mark(shapes, *cell).unwrap_or_else(|| panic!("no mark in the cell at {:?}", cell.center()))).collect()
}

/// The lines a frame painted, as the two points each joins.
fn lines(shapes: &[Shape]) -> Vec<[Pos2; 2]> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, .. } => Some(*points),
            Shape::Path(path) if path.points.len() == 2 => Some([path.points[0], path.points[1]]),
            _ => None,
        })
        .collect()
}

/// The lines the squares presentation draws: the point of each cell joined to the point on its right and to the one
/// below it, from one square's edge to the other's.
fn links(marks: &[&RectShape]) -> Vec<[Pos2; 2]> {
    let mut links = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        let (column, row) = (at % GRID, at / GRID);
        if column + 1 < GRID {
            links.push([mark.rect.right_center(), marks[at + 1].rect.left_center()]);
        }
        if row + 1 < GRID {
            links.push([mark.rect.center_bottom(), marks[at + GRID].rect.center_top()]);
        }
    }
    links
}

fn same(first: Pos2, second: Pos2) -> bool {
    (first - second).length() <= ROUNDING
}

/// Whether `painted` holds a line joining the two points `wanted` joins, either way round.
fn joins(painted: &[[Pos2; 2]], wanted: [Pos2; 2]) -> bool {
    painted.iter().any(|line| (same(line[0], wanted[0]) && same(line[1], wanted[1])) || (same(line[0], wanted[1]) && same(line[1], wanted[0])))
}

/// The share of its cell a mark takes, as the two rooms are compared on it: the relation the one allocation keeps,
/// without a size or a ratio of the look written down here.
fn share(mark: &RectShape, cell: Rect) -> f32 {
    mark.rect.width() / cell.width()
}

#[trial]
fn the_default_presentation_marks_the_nine_points_with_dots_and_joins_none_of_them() {
    let picker = Rc::new(Picker::dots());
    let mut fixture = settled(&picker);
    let cells = cells(&fixture);
    assert_eq!(cells.len(), GRID * GRID, "a cell for each point of the box");
    let shapes = fixture.frame_shapes();
    let marks = marks(&shapes, &cells);
    let painted = lines(&shapes);
    assert!(painted.is_empty(), "the default presentation joins nothing: {painted:?}");
    for (at, mark) in marks.iter().enumerate() {
        assert!(cells[at].contains_rect(mark.rect), "the dot of the point at {at} stays in its cell");
        assert!(same(mark.rect.center(), cells[at].center()), "the dot of the point at {at} sits at the centre of its cell");
        assert!(mark.fill.a() > 0, "the dot of the point at {at} is filled");
        assert!(mark.stroke.is_empty(), "the dot of the point at {at} is not outlined");
    }
    assert!(marks[CENTRE].rect.width() > marks[0].rect.width(), "the pick's dot is the wider one");
}

#[trial]
fn the_squares_presentation_outlines_the_points_and_joins_them_into_the_boxs_grid() {
    let picker = Rc::new(Picker::squares());
    let mut fixture = settled(&picker);
    let cells = cells(&fixture);
    let shapes = fixture.frame_shapes();
    let marks = marks(&shapes, &cells);
    for (at, mark) in marks.iter().enumerate() {
        assert!(cells[at].contains_rect(mark.rect), "the square of the point at {at} stays in its cell");
        assert!(same(mark.rect.center(), cells[at].center()), "the square of the point at {at} sits at the centre of its cell");
        let picked = at == CENTRE;
        assert_eq!(mark.fill.a() > 0, picked, "the pick's square is filled and the rest are not, at {at}");
        assert_eq!(!mark.stroke.is_empty(), !picked, "the rest are outlined and the pick is not, at {at}");
    }
    let painted = lines(&shapes);
    let wanted = links(&marks);
    for line in &wanted {
        assert!(joins(&painted, *line), "each neighbouring pair is joined from mark edge to mark edge: {line:?}");
    }
    assert!(painted.iter().all(|line| joins(&wanted, *line)), "the grid joins neighbours only, not arbitrary points");
}

#[trial]
fn the_presentation_changes_the_mark_and_leaves_every_cell_and_the_size_alone() {
    let dots = Rc::new(Picker::dots());
    let squares = Rc::new(Picker::squares());
    let (dot_fixture, square_fixture) = (settled(&dots), settled(&squares));
    assert_eq!(cells(&dot_fixture), cells(&square_fixture), "the nine parts and the cells they sense keep their places");
    assert_eq!(dot_fixture.shown(), square_fixture.shown(), "and the picker takes the size it took before");
    let compact_dots = Rc::new(Picker::dots().compact());
    let compact_squares = Rc::new(Picker::squares().compact());
    let (small_dots, small_squares) = (settled(&compact_dots), settled(&compact_squares));
    assert_eq!(small_dots.shown(), small_squares.shown(), "the compact picker keeps its icon size too");
    assert!(small_squares.shown().width() < square_fixture.shown().width(), "and is the smaller picker");
    assert_eq!(cells(&small_dots), cells(&small_squares), "both compact presentations retain identical named cells");
}

#[trial]
fn the_growth_arrows_keep_their_shafts_beside_the_pick_in_either_presentation() {
    let dots = Rc::new(Picker::dots().grown());
    let squares = Rc::new(Picker::squares().grown());
    let (mut dot_fixture, mut square_fixture) = (settled(&dots), settled(&squares));
    let cells = cells(&dot_fixture);
    assert_eq!(cells, self::cells(&square_fixture));
    let dot_lines = lines(&dot_fixture.frame_shapes());
    let square_lines = lines(&square_fixture.frame_shapes());
    for (at, cell) in cells.iter().enumerate().filter(|(at, _)| *at != CENTRE) {
        let shaft = dot_lines
            .iter()
            .find(|line| cell.contains(line[0]) && cell.contains(line[1]))
            .unwrap_or_else(|| panic!("a growth shaft in neighbouring cell {at}"));
        assert!(joins(&square_lines, *shaft), "changing presentation keeps the growth shaft at the same geometry in cell {at}");
    }
}

/// A room shorter than the picker asks for keeps all nine cells, and every mark shrinks with the room by the one share
/// the picker allocates: each point's mark keeps the share of its cell it has at the size the picker asks for, the pick's
/// dot twice the others' width kept apart from theirs, and a square its own. That is what keeps the marks from overrunning
/// their cells. The squares stay joined to their neighbours across the shorter gap, so the presentation keeps its grid in
/// the short room too.
#[trial]
fn every_presentation_keeps_its_marks_and_joins_inside_a_room_shorter_than_the_picker_asks_for() {
    for (name, picker) in [
        ("dots", Picker::dots()),
        ("squares", Picker::squares()),
        ("compact dots", Picker::dots().compact()),
        ("compact squares", Picker::squares().compact()),
    ] {
        let picker = Rc::new(picker);
        let mut natural = settled(&picker);
        let asked = natural.shown();
        let natural_cells = cells(&natural);
        let natural_shapes = natural.frame_shapes();
        let natural_marks = marks(&natural_shapes, &natural_cells);
        let forced = Rect::from_min_size(asked.min, egui::vec2(asked.width() * SLOT, asked.height()));
        let mut fixture = settled_in(&picker, forced);
        let cells = cells(&fixture);
        assert_eq!(cells.len(), GRID * GRID, "the fixed room keeps the nine points, {name}");
        assert!(
            cells[0].width() < asked.width() / GRID as f32,
            "the room is smaller than the one the picker asks for: a cell of {:?} in a room of {forced:?}",
            cells[0].size()
        );
        let shapes = fixture.frame_shapes();
        let marks: Vec<&RectShape> = cells
            .iter()
            .enumerate()
            .map(|(at, cell)| centred_mark(&shapes, *cell).unwrap_or_else(|| panic!("no {name} mark at the centre of the cell at {at}")))
            .collect();
        for (at, (cell, mark)) in cells.iter().zip(&marks).enumerate() {
            assert!(cell.contains_rect(mark.rect), "the {name} mark at {at} stays in its cell: {:?} in {cell:?}", mark.rect);
            assert!(mark.rect.width() < natural_marks[at].rect.width(), "the {name} mark at {at} shrinks with the room");
            let (short, natural) = (share(mark, *cell), share(natural_marks[at], natural_cells[at]));
            assert!(
                (short - natural).abs() <= SHARE_SLACK,
                "the {name} mark at {at} takes {short} of its cell in the short room and {natural} at the size the picker asks for, so one share sizes every mark"
            );
        }
        if picker.presentation == PointPresentation::ConnectedSquares {
            let painted = lines(&shapes);
            let wanted = links(&marks);
            for line in &wanted {
                assert!(joins(&painted, *line), "the short room keeps the join between neighbouring marks: {line:?}");
            }
            assert!(painted.iter().all(|line| joins(&wanted, *line)), "narrow-room joins still connect only neighbours");
        }
    }
}

#[trial]
fn a_press_picks_its_point_and_space_picks_the_focused_one_in_either_presentation() {
    for (name, picker) in [("dots", Picker::dots()), ("squares", Picker::squares())] {
        let picker = Rc::new(picker);
        let mut fixture = settled(&picker);
        let corner = fixture.by_label("Bottom right").unwrap_or_else(|| panic!("the bottom right {name}"));
        fixture.click(&corner);
        fixture.settle();
        assert_eq!(picker.point.get(), Anchor9::BottomRight, "a press on the {name} of a point picks it");
        let corner = fixture.by_label("Top left").unwrap_or_else(|| panic!("the top left {name}"));
        fixture.access(AccessActionKind::Focus, &corner);
        fixture.settle();
        fixture.key(Key::Space, Modifiers::NONE);
        fixture.settle();
        assert_eq!(picker.point.get(), Anchor9::TopLeft, "Space picks the {name} the focus is on");
    }
}

#[trial]
fn compact_density_keeps_every_mark_in_its_cell_and_compact_squares_joined() {
    let dots = Rc::new(Picker::dots().compact());
    let squares = Rc::new(Picker::squares().compact());
    let shown_dots = dots.clone();
    let shown_squares = squares.clone();
    let mut dot_fixture = Fixture::new(move |ui| shown_dots.show(ui));
    let mut square_fixture = Fixture::new(move |ui| shown_squares.show(ui));
    dot_fixture.density(Size::Compact).settle();
    square_fixture.density(Size::Compact).settle();
    let dot_cells = cells(&dot_fixture);
    let square_cells = cells(&square_fixture);
    assert_eq!(dot_fixture.shown(), square_fixture.shown());
    assert_eq!(dot_cells, square_cells, "the smallest-density presentations still share every cell");
    let dot_shapes = dot_fixture.frame_shapes();
    let square_shapes = square_fixture.frame_shapes();
    let mut previous: Option<f32> = None;
    for (name, shapes) in [("dots", &dot_shapes), ("squares", &square_shapes)] {
        for (at, cell) in dot_cells.iter().enumerate() {
            let mark = centred_mark(shapes, *cell).unwrap_or_else(|| panic!("a compact {name} mark in cell {at}"));
            assert!(cell.contains_rect(mark.rect), "the compact-density {name} mark fits its actual cell");
            assert!(mark.rect.is_positive());
            if let Some(side) = previous {
                assert!((mark.rect.width() - side).abs() <= ROUNDING, "compact mode keeps the marks the same size in either presentation");
            }
            previous = Some(mark.rect.width());
        }
    }
    let square_marks = marks(&square_shapes, &square_cells);
    let painted = lines(&square_shapes);
    for line in links(&square_marks) {
        assert!(joins(&painted, line), "compact-density squares remain joined at their actual mark edges");
    }
}
