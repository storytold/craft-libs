//! `PresetBrowser::drop_zone`: a preset dragged from the browser and released over the zone reports `DropAt`.

use craft_behavior::{Selection, SelectionChange, SelectionModel};
use craft_ui::contract::{Bind, DropAt, DropPlace, DropZone, ListDrag, SelectMode};
use craft_ui::patterns::presets::{PresetBrowser, PresetDrop, PresetEvent, PresetFns, PresetRow, PresetSource, PresetState, PresetView};
use craft_ui::runtime::drag;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, Sense, UiBuilder, pos2, vec2};
use std::cell::RefCell;
use std::rc::Rc;

const GROUP: u32 = 100;
const ALPHA: u32 = 1;
const BROWSER: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(200.0, 300.0) };
const ZONE: Rect = Rect { min: Pos2::new(260.0, 20.0), max: Pos2::new(460.0, 280.0) };
/// An area over the zone's lower part: a release there is not over the zone.
const COVER: Rect = Rect { min: Pos2::new(260.0, 200.0), max: Pos2::new(460.0, 280.0) };

/// The browser in the left column, a list of one group holding Alpha and Beta, with the zone to its right.
fn rig() -> (Fixture, Rc<RefCell<Vec<PresetEvent<u32>>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let rows = PresetSource::new(
        (),
        PresetFns::new(|_, _| vec![PresetRow::group(GROUP, "Group", 0), PresetRow::item(ALPHA, "Alpha", 1), PresetRow::item(2, "Beta", 1)]),
    );
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        let zone = DropZone::new(ui.layer_id(), ZONE);
        ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
            let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows).view_lock(PresetView::List).drop_zone(zone).show(ui);
            seen.borrow_mut().extend(out.events);
        });
        egui::Area::new(Id::unique("cover")).fixed_pos(COVER.min).show(ui.ctx(), |ui| ui.allocate_exact_size(COVER.size(), Sense::click()));
    });
    fixture.room(vec2(480.0, 320.0)).settle();
    (fixture, events)
}

#[trial]
fn a_preset_released_over_the_zone_reports_drop_at_and_nothing_else_does() {
    // (row grabbed, release point, the drop reported)
    type Case = (&'static str, Pos2, Option<(u32, Pos2)>);
    let cases: [Case; 4] = [
        ("Alpha", pos2(360.0, 100.0), Some((ALPHA, pos2(360.0, 100.0)))),
        ("Alpha", pos2(360.0, 240.0), None),
        ("Alpha", pos2(230.0, 100.0), None),
        ("Group", pos2(360.0, 100.0), None),
    ];
    for (row, at, expected) in cases {
        let (mut fixture, events) = rig();
        let from = fixture.by_label(row).expect("the row is shown").center();
        let mut marked = false;
        fixture.drag_at_pausing(from, at - from, |fixture| marked = drag::mark(fixture.ctx()).is_some());
        let drops: Vec<(u32, Pos2)> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                PresetEvent::DropAt { key, at, .. } => Some((*key, *at)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, expected.into_iter().collect::<Vec<_>>(), "{row} released at {at:?}");
        assert_eq!(marked, expected.is_some(), "{row} over {at:?} shows the zone's mark only where it drops");
    }
}

/// Find matches a row's `search_name` beside its label: in both views, a preset shown as "Rot" with the search name
/// "Red" is found by either name, and a preset without a search name is still found by its label.
#[trial]
fn find_matches_a_preset_by_its_search_name_or_its_label() {
    // (query, the presets shown, the presets hidden)
    type Case = (&'static str, &'static [&'static str], &'static [&'static str]);
    let cases: [Case; 3] = [("Red", &["Rot"], &["Blau"]), ("Rot", &["Rot"], &["Blau"]), ("Blau", &["Blau"], &["Rot"])];
    for view in [PresetView::List, PresetView::Grid] {
        for (query, shown, hidden) in cases {
            let rows = PresetSource::new(
                (),
                PresetFns::new(|_, _| {
                    let mut red = PresetRow::item(ALPHA, "Rot", 1);
                    red.search_name = Some(String::from("Red"));
                    vec![PresetRow::group(GROUP, "Group", 0), red, PresetRow::item(2, "Blau", 1)]
                }),
            );
            let mut state = PresetState::<u32>::new();
            state.filter = String::from(query);
            let mut fixture = Fixture::new(move |ui| {
                ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                    let _ = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows).view_lock(view).show(ui);
                });
            });
            fixture.room(vec2(480.0, 320.0)).settle();
            for name in shown {
                assert!(fixture.by_label(name).is_some(), "{view:?}: Find \"{query}\" shows {name}");
            }
            for name in hidden {
                assert!(fixture.by_label(name).is_none(), "{view:?}: Find \"{query}\" hides {name}");
            }
        }
    }
}

/// A browser over `presets` presets in a room as narrow as a docked panel and 300 high, with a footer and
/// `room_height` set when given, and where its New Group button and its last preset are.
fn footer_rig(presets: u32, room_height: Option<f32>) -> (Rect, Option<Rect>) {
    const ROOM: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(250.0, 300.0) };
    let rows = PresetSource::new(
        presets,
        PresetFns::new(|presets: &u32, _| {
            let mut rows = vec![PresetRow::group(GROUP, "Group", 0)];
            rows.extend((1..=*presets).map(|key| PresetRow::item(key, format!("Preset {key}"), 1)));
            rows
        }),
    );
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        ui.scope_builder(UiBuilder::new().max_rect(ROOM), |ui| {
            let mut browser = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                .view_lock(PresetView::List)
                .footer(craft_ui::patterns::presets::PresetFooterOptions::new().new_group(true));
            if let Some(height) = room_height {
                browser = browser.room_height(height);
            }
            let _ = browser.show(ui);
        });
    });
    fixture.room(vec2(480.0, 320.0)).settle();
    let new_group = fixture.by_label("New Group").expect("the footer's New Group button shows").rect;
    let delete = fixture.by_label("Delete").expect("the footer's Delete button shows").rect;
    assert!(delete.right() <= ROOM.right(), "the footer's actions fit the panel's width: {delete:?}");
    let last = fixture.by_label(&format!("Preset {presets}")).map(|target| target.rect);
    (new_group, last)
}

#[trial]
fn the_footer_stays_in_the_room_and_a_room_height_sizes_the_browser_to_its_presets() {
    let (footer, _) = footer_rig(40, None);
    assert!(footer.bottom() <= 300.0, "a long library scrolls above the footer: {footer:?}");
    let (footer, last) = footer_rig(2, Some(200.0));
    let last = last.expect("both presets show");
    assert!(footer.top() >= last.bottom() && footer.top() - last.bottom() < 40.0, "the footer follows two presets: {footer:?} {last:?}");
    let (footer, _) = footer_rig(40, Some(200.0));
    assert!(footer.bottom() <= 240.0, "a long library scrolls within the room height: {footer:?}");
}

type Events = Rc<RefCell<Vec<PresetEvent<u32>>>>;

/// Presets 1 to 4 in `view` under `SelectMode::Multi`, the selection kept by the test and given with `Bind::given`.
fn multi_rig(view: PresetView) -> (Fixture, Rc<RefCell<Selection<u32>>>, Events) {
    let selection = Rc::new(RefCell::new(Selection::<u32>::default()));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let (held, seen) = (selection.clone(), events.clone());
    let rows = PresetSource::new((), PresetFns::new(|_, _| (1..=4).map(|key| PresetRow::item(key, format!("Preset {key}"), 0)).collect()));
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
            let given = held.borrow().clone();
            let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                .view_lock(view)
                .select(SelectMode::Multi)
                .selection(Bind::given(given))
                .show(ui);
            for event in out.events {
                if let PresetEvent::Selection(change) = &event {
                    held.borrow_mut().apply(change.clone());
                }
                seen.borrow_mut().push(event);
            }
        });
    });
    fixture.room(vec2(480.0, 320.0)).settle();
    (fixture, selection, events)
}

/// A primary click on preset `key` with `mods` held.
fn click_preset(fixture: &mut Fixture, key: u32, mods: Modifiers) {
    let row = fixture.by_label(&format!("Preset {key}")).expect("the preset is shown");
    fixture.hold(mods).click(&row);
    fixture.hold(Modifiers::NONE);
    fixture.idle(2);
}

/// A secondary press and release on preset `key`.
fn secondary_click_preset(fixture: &mut Fixture, key: u32) {
    let at = fixture.by_label(&format!("Preset {key}")).expect("the preset is shown").center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![Event::PointerMoved(at)]);
    fixture.frame(vec![button(true)]);
    fixture.frame(vec![button(false)]);
    fixture.idle(2);
}

#[trial]
fn multi_selection_ranges_toggles_and_reports_select_for_each_plain_pick() {
    for view in [PresetView::List, PresetView::Grid] {
        let (mut fixture, selection, events) = multi_rig(view);
        let items = || {
            let mut items = selection.borrow().items.clone();
            items.sort_unstable();
            items
        };
        let selects = || events.borrow().iter().filter(|event| matches!(event, PresetEvent::Select { .. })).count();

        click_preset(&mut fixture, 1, Modifiers::NONE);
        assert_eq!((items(), selects()), (vec![1], 1), "a plain click selects and reports Select in {view:?}");
        fixture.advance(1.0);
        click_preset(&mut fixture, 1, Modifiers::ALT);
        assert_eq!((items(), selects()), (vec![1], 2), "a click on the sole selected preset picks it again in {view:?}");
        assert!(
            events.borrow().iter().any(|event| matches!(event, PresetEvent::Select { key: 1, mods, .. } if mods.alt)),
            "the pick carries its modifiers in {view:?}: {:?}",
            events.borrow()
        );

        events.borrow_mut().clear();
        click_preset(&mut fixture, 3, Modifiers::SHIFT);
        assert_eq!((items(), selects()), (vec![1, 2, 3], 0), "Shift extends from the anchor and reports no Select in {view:?}");

        click_preset(&mut fixture, 2, Modifiers::COMMAND);
        assert_eq!((items(), selects()), (vec![1, 3], 0), "Cmd/Ctrl toggles and reports no Select in {view:?}");

        let before = items();
        // The Control key on every platform: on macOS Cmd+Space is the system's.
        fixture.key(Key::Space, Modifiers::CTRL);
        fixture.idle(2);
        assert_ne!(items(), before, "Ctrl+Space toggles the lead in {view:?}");
        assert_eq!(selects(), 0, "Ctrl+Space is no pick in {view:?}");

        click_preset(&mut fixture, 4, Modifiers::NONE);
        events.borrow_mut().clear();
        secondary_click_preset(&mut fixture, 2);
        assert!(
            events.borrow().contains(&PresetEvent::Selection(SelectionChange::Set(vec![2]))),
            "a secondary press on an unselected preset selects it first in {view:?}: {:?}",
            events.borrow()
        );
        assert_eq!(selects(), 0, "a secondary press is no pick in {view:?}");

        // The secondary press opened the row menu over the rows: Escape closes it before the next pick.
        fixture.key(Key::Escape, Modifiers::NONE);
        fixture.idle(2);
        click_preset(&mut fixture, 3, Modifiers::COMMAND);
        events.borrow_mut().clear();
        secondary_click_preset(&mut fixture, 3);
        assert_eq!(items(), vec![2, 3], "a secondary press on a selected preset leaves the selection whole in {view:?}");
    }
}

#[trial]
fn titled_groups_join_modifier_selection_only_when_enabled() {
    for view in [PresetView::List, PresetView::Grid] {
        for enabled in [false, true] {
            let rows = PresetSource::new(
                (),
                PresetFns::new(|_, _| vec![PresetRow::group(GROUP, "Group", 0), PresetRow::item(ALPHA, "Alpha", 1), PresetRow::item(2, "Beta", 1)]),
            );
            let mut state = PresetState::new();
            let selection = Rc::new(RefCell::new(Selection::default()));
            let events = Rc::new(RefCell::new(Vec::new()));
            let (held, seen) = (selection.clone(), events.clone());
            let mut fixture = Fixture::new(move |ui| {
                let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                    .view_lock(view)
                    .select(SelectMode::Multi)
                    .select_groups(enabled)
                    .selection(Bind::bound(&mut *held.borrow_mut()))
                    .show(ui);
                seen.borrow_mut().extend(out.events);
            });
            fixture.room(vec2(480.0, 320.0)).settle();
            let alpha = fixture.by_label("Alpha").expect("the first preset is shown");
            fixture.click(&alpha).settle();
            assert_eq!(selection.borrow().items, [ALPHA]);
            events.borrow_mut().clear();
            let group = fixture.by_label("Group").expect("the group header is shown");
            fixture.hold(Modifiers::COMMAND).click(&group).hold(Modifiers::NONE).settle();
            let expected = if enabled { vec![ALPHA, GROUP] } else { vec![ALPHA] };
            assert_eq!(selection.borrow().items, expected, "{view:?}, group selection enabled: {enabled}");
            assert!(!events.borrow().iter().any(|event| matches!(event, PresetEvent::Select { .. })));
            if enabled {
                assert_eq!(fixture.tree().selected_of(group.node), Some(true));
                assert!(fixture.by_label("Beta").is_some(), "the group remains open in {view:?}");
                fixture.advance(1.0);
                fixture.hold(Modifiers::COMMAND).click(&group).hold(Modifiers::NONE).settle();
                assert_eq!(selection.borrow().items, [ALPHA], "Cmd/Ctrl toggles the group off in {view:?}");
                fixture.access(AccessActionKind::Click, &group).settle();
                assert_eq!(selection.borrow().items, [GROUP], "accessibility Click selects the group in {view:?}");
                let beta = fixture.by_label("Beta").expect("accessibility Click keeps the group open");
                fixture.hold(Modifiers::SHIFT).click(&beta).hold(Modifiers::NONE).settle();
                assert_eq!(selection.borrow().items, [GROUP, ALPHA, 2], "Shift range includes the group in {view:?}");
                fixture.access(AccessActionKind::Click, &group).settle();
                fixture.key(Key::Enter, Modifiers::NONE).settle();
                assert!(fixture.by_label("Beta").is_none(), "Enter closes the selected group in {view:?}");
            }
        }
    }
}

#[trial]
fn a_payload_dropped_onto_a_preset_reports_dropped_and_wins_over_an_enclosing_drop_action() {
    // (view, row grabbed, row released over, the preset reported)
    let cases: [(PresetView, &str, &str, Option<u32>); 3] = [
        (PresetView::List, "Preset 1", "Preset 2", Some(2)),
        (PresetView::List, "Preset 1", "Group", None),
        (PresetView::Grid, "Preset 1", "Preset 2", Some(2)),
    ];
    for (view, grabbed, over, expected) in cases {
        let events: Events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        let rows = PresetSource::new(
            (),
            PresetFns::new(|_, _| {
                vec![
                    PresetRow::item(1, "Preset 1", 0),
                    PresetRow::item(2, "Preset 2", 0),
                    PresetRow::group(GROUP, "Group", 0),
                    PresetRow::item(3, "Preset 3", 1),
                ]
            }),
        );
        let mut state = PresetState::<u32>::new();
        let mut fixture = Fixture::new(move |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                    .view_lock(view)
                    .drags(true)
                    .takes_drop(PresetDrop::new::<ListDrag<u32>>(DropPlace::Onto, |drag, at| matches!(at, DropAt::Onto(key) if *key != drag.grabbed)))
                    .show(ui);
                seen.borrow_mut().extend(out.events);
            });
        });
        fixture.room(vec2(480.0, 320.0)).settle();
        let from = fixture.by_label(grabbed).expect("the grabbed row is shown").center();
        let to = fixture.by_label(over).expect("the row released over is shown").center();
        fixture.hold(Modifiers::ALT).drag_at(from, to - from);
        fixture.hold(Modifiers::NONE);
        let dropped: Vec<(DropAt<u32>, bool)> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                PresetEvent::Dropped { at, mods, .. } => Some((at.clone(), mods.alt)),
                _ => None,
            })
            .collect();
        let expected: Vec<(DropAt<u32>, bool)> = expected.into_iter().map(|key| (DropAt::Onto(key), true)).collect();
        assert_eq!(dropped, expected, "{grabbed} released over {over} in {view:?}");
    }
}

/// Art dragged from another list (a layer row) onto the browser, in each view: released on a preset it reports
/// `Dropped` onto that preset, and released on the notice of an empty browser the drop slot around it takes it.
#[trial]
fn a_payload_from_another_list_drops_on_a_preset_or_on_an_empty_browser_in_each_view() {
    use craft_ui::contract::{ShowCollection as _, ShowContainer as _};
    use craft_ui::list::{List, ListRow};
    use craft_ui::sections::DropAction;
    #[derive(Clone, Debug, PartialEq)]
    struct Art(u32);
    const LAYERS: Rect = Rect { min: Pos2::new(260.0, 0.0), max: Pos2::new(460.0, 120.0) };
    for view in [PresetView::List, PresetView::Grid] {
        for filled in [true, false] {
            let events: Events = Rc::new(RefCell::new(Vec::new()));
            let slot = Rc::new(RefCell::new(None::<Art>));
            let (seen, taken) = (events.clone(), slot.clone());
            let rows = PresetSource::new(
                filled,
                PresetFns::new(
                    |filled: &bool, _| if *filled { vec![PresetRow::item(1, "Leaf", 0), PresetRow::item(2, "Dot", 0)] } else { Vec::new() },
                ),
            );
            let mut state = PresetState::<u32>::new();
            let mut fixture = Fixture::new(move |ui| {
                ui.scope_builder(UiBuilder::new().max_rect(LAYERS), |ui| {
                    let keys = [7_u32];
                    let source = craft_behavior::keyed(&keys, |key: &u32| *key);
                    List::<u32>::new("Layers")
                        .drag_payload(|drag: &ListDrag<u32>| Some(Art(drag.grabbed)))
                        .show_items(ui, &source, &mut |_, _| ListRow::new("Art"));
                });
                ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                    let out = DropAction::<Art>::new(|_| true).show(ui, |ui, _| {
                        let out = PresetBrowser::new("Symbols", Bind::bound(&mut state), &rows)
                            .view_lock(view)
                            .takes_drop(PresetDrop::new::<Art>(DropPlace::Onto, |_, _| true))
                            .empty("No symbols")
                            .show(ui);
                        seen.borrow_mut().extend(out.events);
                    });
                    if let Some(art) = out.output.dropped {
                        *taken.borrow_mut() = Some(art);
                    }
                });
            });
            fixture.room(vec2(480.0, 320.0)).settle();
            let from = fixture.by_label("Art").expect("the layer row").center();
            let to = fixture.by_label(if filled { "Dot" } else { "No symbols" }).expect("the preset or the notice").center();
            fixture.drag_at(from, to - from);
            let onto: Vec<DropAt<u32>> = events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    PresetEvent::Dropped { at, .. } => Some(at.clone()),
                    _ => None,
                })
                .collect();
            if filled {
                assert_eq!(onto, vec![DropAt::Onto(2)], "{view:?}: the drop on a preset is the browser's");
            } else {
                assert_eq!(*slot.borrow(), Some(Art(7)), "{view:?}: the slot around an empty browser takes the drop");
            }
        }
    }
}

/// Delete and Backspace on the selected preset report the same `Delete` the row menu and the footer give, in both views.
#[trial]
fn delete_and_backspace_on_a_selected_preset_report_delete() {
    for view in [PresetView::List, PresetView::Grid] {
        for key in [Key::Delete, Key::Backspace] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let seen = events.clone();
            let rows = PresetSource::new((), PresetFns::new(|_, _| vec![PresetRow::item(ALPHA, "Alpha", 0), PresetRow::item(2, "Beta", 0)]));
            let mut state = PresetState::<u32>::new();
            let mut fixture = Fixture::new(move |ui| {
                ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                    let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows).view_lock(view).show(ui);
                    seen.borrow_mut().extend(out.events);
                });
            });
            fixture.room(vec2(480.0, 320.0)).settle();
            let row = fixture.by_label("Beta").expect("the browser shows Beta");
            fixture.click(&row);
            fixture.idle(2);
            events.borrow_mut().clear();
            fixture.key(key, Modifiers::NONE);
            fixture.idle(2);
            let deletes: Vec<_> = events.borrow().iter().filter(|event| matches!(event, PresetEvent::Delete(_))).cloned().collect();
            assert_eq!(deletes, vec![PresetEvent::Delete(2)], "{key:?} in {view:?} deletes the selected preset");
        }
    }
}

/// What a host's view of the browser reads: its rows and its state, which the host's revision stands for.
#[derive(craft_ui::contract::MemoProps)]
struct Host<'a> {
    revision: u64,
    #[memo_props(covered_by = "revision", reason = "the rows are fixed in this trial")]
    rows: &'a PresetSource<'a, u32>,
    #[memo_props(covered_by = "revision", reason = "nothing writes the state in this trial")]
    state: &'a mut PresetState<u32>,
    #[memo_props(hash = "a fieldless enum")]
    view: PresetView,
}

fn host(props: &mut Host<'_>, ui: &mut egui::Ui) {
    let _ = PresetBrowser::new("Patterns", Bind::bound(props.state), props.rows).view_lock(props.view).show(ui);
}

/// The browser reads the pointer and the modifiers only for its rows' own events, and its tile grid reads the
/// modifiers, not the pointer frame: a pointer move beside it replays its view in both views (#2812). Before, every
/// pointer move in the window ran it.
#[trial]
fn a_pointer_move_beside_the_browser_replays_it() {
    use craft_ui::contract::MemoView;
    use craft_ui::testing::{MemoRun, last_frame_memo_runs, pointer};
    for view in [PresetView::List, PresetView::Grid] {
        let rows = PresetSource::new(
            (),
            PresetFns::new(|_, _| vec![PresetRow::group(GROUP, "Group", 0), PresetRow::item(ALPHA, "Alpha", 1), PresetRow::item(2, "Beta", 1)]),
        );
        let mut state = PresetState::<u32>::new();
        let mut fixture = Fixture::new(move |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                MemoView::new("browser", Host { revision: 0, rows: &rows, state: &mut state, view }, host).show(ui);
            });
        });
        fixture.room(vec2(480.0, 320.0)).settle().idle(2);
        let runs = |fixture: &Fixture| {
            let runs = last_frame_memo_runs(fixture.ctx());
            let count = |wanted: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("Host") && *run == wanted).count();
            (count(MemoRun::Ran), count(MemoRun::Replayed))
        };
        // The pointer enters the window beside the browser; in an app it is already there.
        fixture.frame(vec![pointer::moved(pos2(360.0, 90.0))]);
        for step in 0..3 {
            fixture.frame(vec![pointer::moved(pos2(360.0, 100.0 + step as f32 * 10.0))]);
            assert_eq!(runs(&fixture), (0, 1), "{view:?}: pointer move {step} beside the browser replays its view");
        }
    }
}

/// A preset dropped in the left half of another tile of the grid lands before that tile, in a grid with group headers
/// as in one without: the reorder's place counts the rows as the host's rows are.
#[trial]
fn a_grid_drop_in_the_left_half_of_a_tile_lands_before_it_under_a_group_header() {
    use craft_ui::patterns::presets::PresetReorder;
    for grouped in [false, true] {
        let events: Events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        let rows = PresetSource::new(
            grouped,
            PresetFns::new(|grouped: &bool, _| {
                let mut rows = if *grouped { vec![PresetRow::group(GROUP, "Group", 0)] } else { Vec::new() };
                rows.extend((1..=3).map(|key| PresetRow::item(key, format!("Preset {key}"), usize::from(*grouped))));
                rows
            }),
        );
        let mut state = PresetState::<u32>::new();
        let mut fixture = Fixture::new(move |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                    .view_lock(PresetView::Grid)
                    .reorder(PresetReorder::items())
                    .show(ui);
                seen.borrow_mut().extend(out.events);
            });
        });
        fixture.room(vec2(480.0, 320.0)).settle();
        let first = fixture.by_label("Preset 1").expect("the first preset");
        let from = fixture.by_label("Preset 3").expect("the last preset").center();
        let to = first.center() - vec2(first.rect.width() / 4.0, 0.0);
        fixture.drag_at(from, to - from).settle();
        let places: Vec<usize> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                PresetEvent::Reorder(edit) => edit.value().map(|mv| mv.to),
                _ => None,
            })
            .collect();
        assert_eq!(places, [usize::from(grouped)], "Preset 3 takes Preset 1's row (grouped: {grouped})");
    }
}

/// The host's rows of a `PresetRowMenu::Host` row menu are plain menu items, and a checked one a check box: a screen
/// reader hears no unchecked check box on a command row.
#[trial]
fn host_row_menu_rows_are_menu_items_and_only_a_checked_row_a_check_box() {
    use craft_ui::contract::{Entry, Role};
    use craft_ui::patterns::presets::PresetRowMenu;
    let rows = PresetSource::new((), PresetFns::new(|_, _| vec![PresetRow::item(ALPHA, "Alpha", 0)]));
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
            let _ = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows)
                .view_lock(PresetView::List)
                .row_menu(PresetRowMenu::Host)
                .row_items(|_| vec![Entry::new(String::from("options"), "Options…"), Entry::new(String::from("lock"), "Locked").checked(true)])
                .show(ui);
        });
    });
    fixture.room(vec2(480.0, 320.0)).settle();
    let alpha = fixture.by_label("Alpha").expect("the preset").center();
    let button = |pressed| Event::PointerButton { pos: alpha, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![Event::PointerMoved(alpha)]);
    fixture.frame(vec![button(true)]);
    fixture.frame(vec![button(false)]);
    fixture.idle(2);
    let tree = fixture.tree();
    let names = |role| fixture.by_role(role).iter().filter_map(|row| tree.name_of(row.node).map(str::to_owned)).collect::<Vec<_>>();
    assert_eq!(names(Role::MENU_ITEM), ["Options…"], "a plain host row is a menu item");
    assert_eq!(names(Role::MENU_ITEM_CHECK_BOX), ["Locked"], "a checked host row is a check box");
}

/// Host tools can share the header's reading start without changing the default order or view actions.
#[trial]
fn host_toolbar_tools_can_lead_the_view_controls_without_changing_the_default() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;

    for at_start in [false, true] {
        let state = Rc::new(RefCell::new(PresetState::<u32>::new()));
        let shown = Rc::clone(&state);
        let rows = PresetSource::new((), PresetFns::new(|_, _| vec![PresetRow::item(ALPHA, "Alpha", 0)]));
        let mut fixture = Fixture::new(move |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
                let _ = PresetBrowser::new("Swatches", Bind::bound(&mut *shown.borrow_mut()), &rows)
                    .toolbar(true)
                    .search(false)
                    .tools_at_start(at_start)
                    .tools(|ui: &mut egui::Ui| {
                        Button::new("Host Tool").show(ui);
                    })
                    .show(ui);
            });
        });
        fixture.room(vec2(480.0, 320.0)).settle();
        let host = fixture.by_label("Host Tool").expect("host control").center();
        let grid = fixture.by_label("Grid View").expect("grid action").center();
        let list = fixture.by_label("List View").expect("list action").center();
        assert!((host.y - grid.y).abs() < 1.0, "both groups share the toolbar row");
        assert_eq!(host.x < grid.x, at_start, "the default stays trailing; the opt-in leads");
        assert!(grid.x < list.x, "the view choices retain their order");
        let target = fixture.by_label("List View").expect("list action");
        fixture.click(&target).settle();
        assert_eq!(state.borrow().view, PresetView::List);
        let target = fixture.by_label("Grid View").expect("grid action");
        fixture.click(&target).settle();
        assert_eq!(state.borrow().view, PresetView::Grid);
    }
}
