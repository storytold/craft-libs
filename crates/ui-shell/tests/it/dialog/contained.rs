//! A dialog opened in an embedded shell is contained in that shell's region.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Scroll, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::WindowKey;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::shell::AppShell;
use craft_ui::testing::contract::clicked;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What the two regions of the fixture report.
struct Seen {
    /// Clicks on the button outside both regions, and on the button under each region's dialog.
    outside: Cell<u32>,
    under: [Cell<u32>; 2],
    open: [Cell<bool>; 2],
    regions: RefCell<[Rect; 2]>,
}

/// One region: an embedded shell with a button in its body and a dialog over it while `open`.
fn region(ui: &mut Ui, at: usize, seen: &Seen) {
    let name = ["A", "B"][at];
    let mut open = seen.open[at].get();
    let out = AppShell::embedded(WindowKey::new(format!("region-{name}")))
        .body(|ui, _, _| {
            if clicked(&Button::new(format!("Under {name}")).show(ui)) {
                seen.under[at].set(seen.under[at].get() + 1);
            }
            Dialog::new(format!("Dialog {name}"))
                .opened(Bind::bound(&mut open))
                .buttons([FooterButton::new(format!("Done {name}"), DialogRole::Accept)])
                .show(ui, |_, _| {});
        })
        .show(ui, &mut ());
    seen.open[at].set(open);
    seen.regions.borrow_mut()[at] = out.response.rect;
}

fn fixture(seen: Rc<Seen>) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        if clicked(&Button::new("Outside").show(ui)) {
            seen.outside.set(seen.outside.get() + 1);
        }
        ui.columns(2, |columns| {
            region(&mut columns[0], 0, &seen);
            region(&mut columns[1], 1, &seen);
        });
    });
    fixture.settle();
    fixture
}

fn both_open() -> Rc<Seen> {
    let seen =
        Rc::new(Seen { outside: Cell::default(), under: Default::default(), open: Default::default(), regions: RefCell::new([Rect::NOTHING; 2]) });
    seen.open[0].set(true);
    seen.open[1].set(true);
    seen
}

/// Each dialog sits inside its own region, a press outside the regions reaches the widget there, and the region's
/// scrim takes the press meant for what lies under its dialog.
#[trial]
fn a_dialog_in_an_embedded_region_is_placed_in_it_and_blocks_only_it() {
    let seen = both_open();
    let mut fixture = fixture(seen.clone());
    let regions = *seen.regions.borrow();
    let dialogs = fixture.by_role(Role::DIALOG);
    assert_eq!(dialogs.len(), 2, "both dialogs are shown");
    for (at, region) in regions.iter().enumerate() {
        let inside = dialogs.iter().filter(|dialog| region.contains_rect(dialog.rect)).count();
        assert_eq!(inside, 1, "region {at} holds exactly one dialog: {region:?} vs {dialogs:?}");
    }
    let outside = fixture.by_label("Outside").expect("the outside button is shown");
    fixture.click(&outside);
    fixture.settle();
    assert_eq!(seen.outside.get(), 1, "a press outside the regions reaches the widget there");
    let under = fixture.by_label("Under A").expect("the button under the first dialog is shown");
    fixture.click(&under);
    fixture.settle();
    assert_eq!(seen.under[0].get(), 0, "the region's own scrim takes the press meant for what lies under its dialog");
}

/// An embedded shell scrolled out of sight with its dialog open shows nothing and does not fail: its region has no
/// room, so neither its body nor its dialog runs in that pass.
#[trial]
fn an_embedded_shell_out_of_sight_shows_nothing() {
    let mut fixture = Fixture::new(|ui| {
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            let _ = Button::new("Top").show(ui);
            let view = ui.clip_rect();
            let below = Rect::from_min_size(egui::pos2(view.min.x, view.max.y + view.height()), egui::vec2(view.width(), view.height()));
            let mut open = true;
            AppShell::embedded(WindowKey::new("out-of-sight"))
                .body(|ui, _, _| {
                    let _ = Button::new("Hidden").show(ui);
                    Dialog::new("Out of sight").opened(Bind::bound(&mut open)).show(ui, |_, _| {});
                })
                .show_at(ui, below, &mut ());
        });
    });
    fixture.settle();
    assert!(fixture.by_label("Top").is_some(), "the visible part is shown");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "the dialog of a shell out of sight is not shown");
    assert!(fixture.by_label("Hidden").is_none(), "the body of a shell out of sight does not run");
}
