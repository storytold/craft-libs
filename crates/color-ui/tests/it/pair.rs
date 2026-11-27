//! `SwatchPair` wells take the theme's well shape (`shape.wells`): a round well is round to the pointer too. Focus
//! arrives at the active well, so a panel opening with the pair first activates no other well, and only focus moving
//! onto a well activates it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_color_ui::ui::{PairWell, SwatchPair, SwatchPairOutput, Well};
use craft_theme::{Srgba, ThemeId};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, Ui, Vec2, vec2};

/// Where a click lands on the primary well in theme `theme`: at the well's middle and just inside the corner of its
/// square. Each is the well the click reached, if any.
fn clicks(theme: &str) -> [Option<Well>; 2] {
    let clicked = Rc::new(Cell::new(None));
    let seen = clicked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let wells = [PairWell::new("Foreground colour", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Background colour", Srgba::rgb(1.0, 1.0, 1.0))];
        let shown = SwatchPair::<()>::new("Colours", wells, Bind::given(Well::Primary)).popover(false).show(ui);
        if shown.clicked.is_some() {
            seen.set(shown.clicked);
        }
    });
    fixture.theme_id(ThemeId::parse(theme).expect("a theme id")).room(vec2(200.0, 200.0));
    fixture.settle();
    let well = fixture.by_role(Role::RADIO_BUTTON).first().expect("the primary well").rect;
    [well.center(), well.min + Vec2::splat(1.5)].map(|at| {
        clicked.set(None);
        fixture.click_at(at);
        fixture.settle();
        clicked.get()
    })
}

/// What the two pairs of the drag trial reported.
#[derive(Default)]
struct Log {
    first: Option<(Well, u8)>,
    second: Option<(Well, u8)>,
    over_second: Option<Well>,
}

/// A pair whose wells drag `first` and `first + 1`; it accepts every payload when `takes_all`, else only 2.
fn draggable(ui: &mut Ui, name: &'static str, first: u8, back: Mixed<Srgba>, takes_all: bool) -> SwatchPairOutput<u8> {
    let wells = [PairWell::new("Fore", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Back", back)];
    SwatchPair::<()>::new(name, wells, Bind::given(Well::Primary))
        .popover(false)
        .drags([Some(first), Some(first + 1)])
        .accepts(move |_, payload| takes_all || *payload == 2)
        .show(ui)
}

/// A point on the primary well that the secondary one does not cover, and the reverse.
fn in_primary(well: Rect) -> egui::Pos2 {
    well.min + well.size() * vec2(0.15, 0.5)
}

fn in_secondary(well: Rect) -> egui::Pos2 {
    well.min + well.size() * vec2(0.85, 0.5)
}

/// A well drags its payload past the slop and the well under the pointer takes it when `accepts` allows; a mixed well
/// drags nothing and takes nothing.
#[trial]
fn a_pair_well_drags_its_payload_and_takes_a_drop_a_mixed_well_does_neither() {
    let log = Rc::new(RefCell::new(Log::default()));
    let seen = log.clone();
    let mut fixture = Fixture::new(move |ui| {
        let first = draggable(ui, "First", 1, Mixed::Same(Srgba::WHITE), false);
        let second = draggable(ui, "Second", 3, Mixed::Mixed, true);
        let mut log = seen.borrow_mut();
        if first.dropped.is_some() {
            log.first = first.dropped;
        }
        if second.dropped.is_some() {
            log.second = second.dropped;
        }
        log.over_second = second.over;
    });
    fixture.room(vec2(240.0, 400.0));
    fixture.settle();
    let wells = fixture.by_role(Role::RADIO_BUTTON);
    let [a_primary, a_secondary, b_primary, b_secondary] = [0, 1, 2, 3].map(|i| wells[i].rect);
    let drag = |fixture: &mut Fixture, from: egui::Pos2, to: egui::Pos2| {
        *log.borrow_mut() = Log::default();
        fixture.drag_at(from, to - from);
    };

    drag(&mut fixture, in_primary(a_primary), in_secondary(a_secondary));
    assert_eq!(log.borrow().first, None, "payload 1 is refused by the pair that accepts only 2");

    drag(&mut fixture, in_secondary(a_secondary), in_primary(a_primary));
    assert_eq!(log.borrow().first, Some((Well::Primary, 2)), "the secondary well's payload lands on the primary one");

    drag(&mut fixture, in_primary(a_primary), in_primary(b_primary));
    assert_eq!(log.borrow().second, Some((Well::Primary, 1)), "the primary well's payload lands on the other pair's well");

    drag(&mut fixture, in_primary(a_primary), in_secondary(b_secondary));
    assert_eq!(log.borrow().second, None, "a mixed well takes no drop");

    *log.borrow_mut() = Log::default();
    let over = Cell::new(None);
    fixture.drag_at_pausing(in_primary(a_primary), in_primary(b_primary) - in_primary(a_primary), |_| over.set(log.borrow().over_second));
    assert_eq!(over.get(), Some(Well::Primary), "the well a payload is over is reported while it is held");

    drag(&mut fixture, in_secondary(b_secondary), in_primary(a_primary));
    assert_eq!(log.borrow().first, None, "a mixed well drags nothing");
    assert_eq!(log.borrow().over_second, None, "a mixed well starts no drag");
}

/// Adwaita's wells are round: a click in the corner of a well's square misses it. Pro's are square: it hits.
#[trial]
fn the_well_corner_follows_the_theme_well_shape() {
    assert_eq!(clicks("craft.adwaita"), [Some(Well::Primary), None], "round wells");
    assert_eq!(clicks("craft.pro"), [Some(Well::Primary), Some(Well::Primary)], "square wells");
}

/// A rest of the pointer on a well shows the well's label, and one on the Swap or Default button shows that button's
/// label: a host that names a well "Fill (X)" shows its key there.
#[trial]
fn a_rest_on_a_well_or_a_button_shows_its_label() {
    let mut fixture = Fixture::new(|ui| {
        let wells = [PairWell::new("Fill (X)", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Stroke (X)", Srgba::rgb(1.0, 1.0, 1.0))];
        SwatchPair::<()>::new("Fill and Stroke", wells, Bind::given(Well::Primary))
            .swap_button("Swap (Shift+X)")
            .default_button("Default (D)")
            .popover(false)
            .show(ui);
    });
    fixture.room(vec2(200.0, 200.0)).settle();
    let wells = fixture.by_role(Role::RADIO_BUTTON);
    let swap = fixture.by_label("Swap (Shift+X)").expect("the Swap button").rect.center();
    let default = fixture.by_label("Default (D)").expect("the Default button").rect.center();
    let spots =
        [(in_primary(wells[0].rect), "Fill (X)"), (in_secondary(wells[1].rect), "Stroke (X)"), (swap, "Swap (Shift+X)"), (default, "Default (D)")];
    for (at, tip) in spots {
        fixture.frame(vec![moved(egui::pos2(199.0, 199.0))]);
        fixture.settle();
        assert!(!fixture.texts().iter().any(|run| run.text == tip), "no {tip:?} before the rest");
        fixture.frame(vec![moved(at)]);
        fixture.advance(DELAY);
        fixture.settle();
        assert!(fixture.texts().iter().any(|run| run.text == tip), "a rest there shows {tip:?}");
    }
}

/// A popover that opens with the pair as its first stop puts the focus on the active well, as Tab into the pair does:
/// the arrival makes no other well active (a Color panel opened over the Stroke chip leaves Stroke active).
#[trial]
fn a_popover_opening_on_the_pair_focuses_the_active_well_and_activates_none() {
    use craft_ui::buttons::Button;
    use craft_ui::popover::Popover;
    let activated = Rc::new(Cell::new(false));
    let seen = activated.clone();
    let mut open = false;
    let mut fixture = Fixture::new(move |ui| {
        let button = Button::new("Colours").show(ui).response;
        if button.clicked() {
            open = true;
        }
        let _ = Popover::new("Colours", Bind::bound(&mut open)).opened_by(&button).show(ui, |ui, _| {
            let wells = [PairWell::new("Fill", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Stroke", Srgba::rgb(0.0, 0.0, 0.0))];
            let shown = SwatchPair::<()>::new("Fill and stroke", wells, Bind::given(Well::Secondary)).show(ui);
            seen.set(seen.get() || !shown.choice.events.is_empty());
        });
    });
    fixture.room(vec2(400.0, 300.0)).settle();
    let opener = fixture.by_label("Colours").expect("the opener");
    fixture.click(&opener).settle();
    let tree = fixture.tree();
    let focused = craft_ui::testing::access::focused_key(tree);
    assert_eq!(tree.role_of(focused), Some(Role::RADIO_BUTTON), "a well takes the focus");
    assert_eq!(tree.toggled_of(focused), Some(Mixed::Same(true)), "the focus arrives at the active well");
    assert!(!activated.get(), "the arrival activates no well");
}

/// A well that keeps the focus while the host makes the other one active (a Stroke chip clicked elsewhere) leaves the
/// host's choice; an arrow that moves the focus onto a well makes that well active.
#[trial]
fn the_host_activates_the_other_well_while_one_keeps_the_focus_and_an_arrow_activates() {
    let active = Rc::new(Cell::new(Well::Primary));
    let held = active.clone();
    let mut fixture = Fixture::new(move |ui| {
        let wells = [PairWell::new("Fill", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Stroke", Srgba::rgb(0.0, 0.0, 0.0))];
        let mut well = held.get();
        let _ = SwatchPair::<()>::new("Fill and stroke", wells, Bind::bound(&mut well)).show(ui);
        held.set(well);
    });
    fixture.room(vec2(400.0, 300.0)).settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    let tree = fixture.tree();
    assert_eq!(tree.toggled_of(craft_ui::testing::access::focused_key(tree)), Some(Mixed::Same(true)), "Tab arrives at the active Fill well");
    active.set(Well::Secondary);
    fixture.settle();
    assert_eq!(active.get(), Well::Secondary, "the focused Fill well leaves the host's Stroke active");
    fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    assert_eq!(active.get(), Well::Secondary, "an arrow that keeps the focus on Fill changes nothing");
    fixture.key(Key::ArrowRight, Modifiers::NONE).key(Key::ArrowLeft, Modifiers::NONE).settle();
    assert_eq!(active.get(), Well::Primary, "the arrow onto Fill makes it active");
}
