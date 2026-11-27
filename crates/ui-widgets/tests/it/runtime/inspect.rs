//! Inspected regions: a region records its own components, hit areas and focus stops, names a node by its label, a
//! reader shown before it gets the record of the same pass, and the inspector numbers every stop.

use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, CommandSet, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::InspectOverlay;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Rect};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

/// What the latest real pass saw.
struct Seen {
    early: Option<Arc<Recorded>>,
    recorded: Option<Arc<Recorded>>,
    region: Rect,
    other: Rect,
}

#[trial]
fn a_region_records_its_own_elements_and_numbers_every_stop() {
    let key = Id::NULL.with("trial.inspect");
    let seen = Rc::new(RefCell::new(Seen { early: None, recorded: None, region: Rect::NOTHING, other: Rect::NOTHING }));
    let out = Rc::clone(&seen);
    let (mut snap, mut size) = (true, 1u8);
    let mut fixture = Fixture::new(move |ui| {
        let early = inspect::recorded(ui.ctx(), key);
        let (shown, recorded) = inspect::record(ui, key, |ui| {
            Stack::new().show(ui, |ui, _| {
                Button::new("Apply").show(ui);
                Checkbox::new(Bind::bound(&mut snap)).label("Snap to grid").show(ui);
                RadioGroup::new("Size", Bind::bound(&mut size)).show(ui, |ui, _| {
                    Radio::new(1u8).label("Small").show(ui);
                    Radio::new(2u8).label("Medium").show(ui);
                    Radio::new(3u8).label("Large").show(ui);
                });
            })
        });
        let other = Stack::new().show(ui, |ui, _| {
            Button::new("Elsewhere").show(ui);
        });
        if let Some(recorded) = &recorded {
            InspectOverlay::new(Arc::clone(recorded)).show_at(ui, shown.output.rect);
        }
        if recorded.is_some() {
            *out.borrow_mut() = Seen { early, recorded, region: shown.output.rect, other: other.output.rect };
        }
    });
    // AccessKit off, as in an app no screen reader reads: the record does not depend on it.
    fixture.ctx().disable_accesskit();
    // One frame: the reader shown before the region finds its record within it.
    fixture.frame(Vec::new());
    let seen = seen.borrow();
    let recorded = seen.recorded.as_deref().expect("the region ran");
    assert_eq!(seen.early.as_deref(), Some(recorded), "a reader shown before the region gets this pass's record");

    let elements = &recorded.elements;
    let inside = seen.region.expand(0.5);
    for element in elements.iter().filter(|element| element.rect.is_positive()) {
        assert!(inside.contains_rect(element.rect), "{} at {:?} lies in the region {:?}", element.component, element.rect, seen.region);
        assert!(!element.rect.intersects(seen.other.shrink(0.5)), "{} meets the stack outside the region", element.component);
    }
    let ids: HashSet<Id> = elements.iter().map(|element| element.id).collect();
    assert!(recorded.hits.iter().all(|hit| ids.contains(&hit.owner)), "the region's hit areas are its own components'");
    let components = |name: &str| elements.iter().filter(|element| element.part.is_none() && element.component == name).count();
    assert_eq!(components("Button"), 1, "the button outside the region is not recorded");
    let checkbox =
        elements.iter().find(|element| element.part.is_none() && element.component == "Checkbox").and_then(|element| element.facts.as_ref());
    assert_eq!(checkbox.and_then(|facts| facts.name.as_deref()), Some("Snap to grid"), "a node is named by the label that labels it: {checkbox:?}");

    // Every radio has its own number, and the numbers run 1..n in show order.
    let stops: Vec<(usize, &str)> = elements.iter().filter_map(|element| Some((element.stop?, element.component))).collect();
    let numbers: Vec<usize> = stops.iter().map(|(number, _)| *number).collect();
    assert_eq!(numbers, (1..=stops.len()).collect::<Vec<_>>(), "{stops:?}");
    assert_eq!(stops.iter().filter(|(_, component)| *component == "Radio").count(), 3, "{stops:?}");
    assert!(stops.iter().any(|(_, component)| *component == "Button") && stops.iter().any(|(_, component)| *component == "Checkbox"), "{stops:?}");

    // The drawing numbers each stop.
    let painted: HashSet<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    for number in 1..=stops.len() {
        assert!(painted.contains(&number.to_string()), "badge {number} is painted");
    }
}

/// The inspected region's rect and its record, as the latest real pass showed them.
type Drawn = (Rect, Arc<Recorded>);

/// Each Tab-order badge shows whole inside the drawing, in both directions: a badge on a stop at the region's edge
/// moves in until it fits, and on an axis where it already fits it stays on its stop's leading top corner.
#[trial]
fn every_badge_shows_whole_inside_the_region() {
    use craft_ui::runtime::env::Direction;
    use egui::{Shape, vec2};

    for direction in [Direction::Ltr, Direction::Rtl] {
        let key = Id::NULL.with("trial.inspect.badges");
        let seen: Rc<RefCell<Option<Drawn>>> = Rc::new(RefCell::new(None));
        let out = Rc::clone(&seen);
        let mut snap = true;
        let mut fixture = Fixture::new(move |ui| {
            let (shown, recorded) = inspect::record(ui, key, |ui| {
                Stack::new().show(ui, |ui, _| {
                    Button::new("Apply").show(ui);
                    Checkbox::new(Bind::bound(&mut snap)).label("Snap to grid").show(ui);
                })
            });
            if let Some(recorded) = recorded {
                InspectOverlay::new(Arc::clone(&recorded)).show_at(ui, shown.output.rect);
                *out.borrow_mut() = Some((shown.output.rect, recorded));
            }
        });
        fixture.room(vec2(400.0, 300.0)).direction(direction);
        fixture.ctx().disable_accesskit();
        fixture.settle();
        let shapes = fixture.frame_shapes();
        let seen = seen.borrow();
        let (region, recorded) = seen.as_ref().expect("the region ran");
        let mut stops: Vec<(usize, usize)> =
            recorded.elements.iter().enumerate().filter_map(|(index, element)| Some((element.stop?, index))).collect();
        stops.sort_unstable();
        assert!(stops.len() >= 2, "{direction:?}: the region has stops: {stops:?}");
        for (number, index) in stops {
            let text = fixture.texts().iter().find(|text| text.text == number.to_string()).cloned().expect("the badge's number is painted");
            // The badge's disc and ring, centred on its number; a circle's stroke lies outside its radius.
            let reach = shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Circle(circle) if circle.center.distance(text.rect.center()) < 1.0 => Some(circle.radius + circle.stroke.width),
                    _ => None,
                })
                .fold(0.0_f32, f32::max);
            assert!(reach > 0.0, "{direction:?}: badge {number} has a disc round its number");
            let badge = Rect::from_center_size(text.rect.center(), vec2(2.0 * reach, 2.0 * reach));
            assert!(region.expand(0.05).contains_rect(badge), "{direction:?}: badge {number} at {badge:?} shows whole in the region {region:?}");
            assert!(region.expand(0.5).contains_rect(text.rect), "{direction:?}: badge {number}'s number at {:?} shows whole", text.rect);
            // Where the stop's corner leaves room for the whole badge, the badge stays on it.
            let stop = recorded.stop_rect(index).expect("the stop is recorded");
            let corner = if direction == Direction::Rtl { stop.right_top() } else { stop.left_top() };
            let room = region.shrink(reach);
            if (room.min.x..=room.max.x).contains(&corner.x) {
                assert!((badge.center().x - corner.x).abs() < 1.0, "{direction:?}: badge {number} stays on its stop across");
            }
            if (room.min.y..=room.max.y).contains(&corner.y) {
                assert!((badge.center().y - corner.y).abs() < 1.0, "{direction:?}: badge {number} stays on its stop down");
            }
        }
    }
}

/// The commands the command trial's buttons are bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "inspectTrial")]
#[non_exhaustive]
enum Bound {
    #[command(label = "Apply", modal = "always")]
    Apply,
    #[command(label = "Revert", modal = "always")]
    Revert,
}

#[trial]
fn an_element_records_the_command_it_is_bound_to_with_accesskit_off() {
    let key = Id::NULL.with("trial.inspect.command");
    let seen: Rc<RefCell<Option<Arc<Recorded>>>> = Rc::new(RefCell::new(None));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (_, recorded) = inspect::record(ui, key, |ui| {
            Stack::new().show(ui, |ui, _| {
                Button::command(Bound::Apply).show(ui);
                Button::command(Bound::Revert).show(ui);
                Button::new("Plain").show(ui);
            })
        });
        if recorded.is_some() {
            *out.borrow_mut() = recorded;
        }
    });
    // AccessKit off: no node is built, and the record still carries each control's command.
    fixture.ctx().disable_accesskit();
    fixture.frame(Vec::new());
    let seen = seen.borrow();
    let recorded = seen.as_deref().expect("the region ran");
    let buttons: Vec<&inspect::Element> =
        recorded.elements.iter().filter(|element| element.part.is_none() && element.component == "Button").collect();
    assert_eq!(buttons.len(), 3, "the three buttons are recorded: {buttons:?}");
    let commands: Vec<Option<CommandRef>> = buttons.iter().map(|element| element.command.clone()).collect();
    assert_eq!(
        commands,
        vec![Some(CommandRef::from(Bound::Apply)), Some(CommandRef::from(Bound::Revert)), None],
        "each bound button records its own command, the plain one none"
    );
    // The rect a badge is placed at is the bound control's own.
    for (index, element) in recorded.elements.iter().enumerate().filter(|(_, element)| element.command.is_some()) {
        let rect = recorded.stop_rect(index).expect("the element is recorded");
        assert!(rect.is_positive() && element.rect.expand(0.5).contains_rect(rect), "{:?} lies on its control {:?}", rect, element.rect);
    }
}

/// Pointer-only parts have no access node or Tab stop, but consumers still need their original
/// geometry, not the larger minimum-size hit box, to relate a value to its drawn position.
#[trial]
fn pointer_only_thumb_bounds_match_paint_and_value_travel() {
    use craft_ui::runtime::env::RootEnv;
    use craft_ui::testing::{Frames, pointer};
    use craft_ui::value_drag::Slider;
    use egui::{Pos2, Vec2};

    const FIELD: Rect = Rect { min: Pos2::new(30.0, 40.0), max: Pos2::new(250.0, 70.0) };
    let value = Rc::new(RefCell::new(0.0));
    let recorded = Rc::new(RefCell::new(None::<Arc<Recorded>>));
    let (shown_value, shown_record) = (Rc::clone(&value), Rc::clone(&recorded));
    let mut show = move |ui: &mut egui::Ui| {
        let (_, record) = inspect::record(ui, Id::NULL.with("thumb-bounds"), |ui| {
            Slider::new(Bind::bound(&mut *shown_value.borrow_mut()), 0.0..=100.0).step(1.0).label("Position").show_at(ui, FIELD);
        });
        if let Some(record) = record {
            *shown_record.borrow_mut() = Some(record);
        }
    };
    let mut frames = Frames::new();
    // Built-in mouse targets are 8 pt, smaller than the 12 pt knob. Use a genuine host
    // size override so the sensing core actually expands this part to a 32 pt target.
    let mut root = RootEnv::new();
    root.sizes = Arc::new(craft_theme::AppSizes::parse(br#"{ "all": { "size.hit.target": 32 } }"#).expect("valid enlarged hit target"));
    frames.root(root);
    frames.ctx.disable_accesskit();
    frames.idle(3, &mut show);
    let thumb = || {
        let record = recorded.borrow();
        let record = record.as_ref().expect("the region ran");
        let thumb =
            record.hits.iter().find(|hit| hit.part.is_some_and(|part| part.key == Slider::THUMB)).expect("a pointer-only thumb is recorded").clone();
        assert!(
            record.elements.iter().all(|element| element.id != thumb.owner || element.part != thumb.part),
            "the pointer-only thumb does not need a fabricated access node or Tab stop"
        );
        thumb
    };
    let start = thumb();
    assert!(start.rect.contains_rect(start.bounds), "the expanded hit target contains its original geometry");
    assert!(start.rect.width() > start.bounds.width(), "minimum hit size is not the drawn thumb width");
    assert!((start.bounds.left() - FIELD.left()).abs() < 0.01, "minimum thumb is flush with the track start");
    assert_eq!(start.bounds.height(), FIELD.height(), "the thumb's original sensing shape spans the track height");
    let shapes = frames.frame_shapes(&mut show);
    // Along::thumb_rect deliberately keeps the track's full height; Slider::paint draws the
    // smaller square knob at the same centre. The raw sensing width is exactly the painted
    // knob width, which determines value travel. Do not mistake the sensing height or the
    // larger shadow/expanded hit box for that width.
    assert!(
        shapes.iter().any(|shape| match shape {
            egui::Shape::Rect(rect) =>
                rect.rect.x_range() == start.bounds.x_range()
                    && rect.rect.center().y == start.bounds.center().y
                    && rect.rect.height() == rect.rect.width()
                    && rect.rect.height() < start.bounds.height(),
            _ => false,
        }),
        "the raw thumb width and centre identify the actual painted knob: raw={:?}, hit={:?}, shapes={:?}",
        start.bounds,
        start.rect,
        shapes
            .iter()
            .map(|shape| match shape {
                egui::Shape::Rect(rect) => format!("Rect box={:?} stroke={:?} kind={:?}", rect.rect, rect.stroke, rect.stroke_kind),
                other => format!("{:?} bounds={:?}", std::mem::discriminant(other), other.visual_bounding_rect()),
            })
            .collect::<Vec<_>>()
    );

    *value.borrow_mut() = 100.0;
    frames.idle(3, &mut show);
    let end = thumb();
    assert!((end.bounds.right() - FIELD.right()).abs() < 0.01, "maximum thumb is flush with the track end");
    assert_eq!(start.bounds.size(), end.bounds.size(), "only the thumb's position changes");
    let travel = end.bounds.center().x - start.bounds.center().x;
    for fraction in [0.0f32, 0.65, 1.0] {
        let at = start.bounds.center() + Vec2::new(travel * fraction, 0.0);
        pointer::click(&mut frames, at, &mut show);
        frames.idle(3, &mut show);
        assert!((*value.borrow() - f64::from(fraction) * 100.0).abs() < 1e-4, "the raw thumb centres delimit the real value travel at {fraction}");
        assert!((thumb().bounds.center().x - at.x).abs() < 0.01, "the clicked value is drawn at the same position");
    }
}

/// A kept inspected slider moves both the original thumb and its expanded hit area with the memo room.
#[trial]
fn a_replayed_inspection_moves_both_thumb_rectangles() {
    use craft_ui::contract::{MemoProps, MemoView};
    use craft_ui::runtime::env::RootEnv;
    use craft_ui::runtime::inspect::HitArea;
    use craft_ui::testing::layout::in_region;
    use craft_ui::testing::{Frames, MemoRun, audit_replays, in_audit_run, last_frame_memo_runs};
    use craft_ui::value_drag::Slider;
    use egui::{pos2, vec2};
    use std::cell::Cell;

    #[derive(MemoProps)]
    struct InspectedThumb {}

    thread_local! {
        static AUDITED_HIT: Cell<bool> = const { Cell::new(false) };
    }

    fn key() -> Id {
        Id::NULL.with("trial.inspect.replayed-thumb")
    }

    fn thumb(record: &Recorded) -> &HitArea {
        record.hits.iter().find(|hit| hit.part.is_some_and(|part| part.key == Slider::THUMB)).expect("a pointer-only thumb")
    }

    fn show(_: &mut InspectedThumb, ui: &mut egui::Ui) {
        let (_, record) = inspect::record(ui, key(), |ui| {
            Slider::new(Bind::given(0.0), 0.0..=100.0).label("Position").show(ui);
        });
        if in_audit_run(ui.ctx())
            && let Some(record) = record
        {
            let hit = thumb(&record);
            AUDITED_HIT.set(hit.rect.width() > hit.bounds.width());
        }
    }

    let mut frames = Frames::new();
    let mut root = RootEnv::new();
    root.sizes = Arc::new(craft_theme::AppSizes::parse(br#"{ "all": { "size.hit.target": 32 } }"#).expect("valid enlarged hit target"));
    frames.root(root);
    frames.ctx.disable_accesskit();
    audit_replays(&frames.ctx, false);

    let draw = |frames: &mut Frames, y: f32| {
        let room = Rect::from_min_size(pos2(30.0, y), vec2(250.0, 80.0));
        let mut hit = None;
        frames.frame(vec![], |ui| {
            in_region(ui, room, |ui| MemoView::new("thumb", InspectedThumb {}, show).show(ui));
            hit = inspect::recorded(ui.ctx(), key()).map(|record| thumb(&record).clone());
        });
        hit.expect("the inspector recorded the thumb")
    };
    for _ in 0..3 {
        draw(&mut frames, 40.0);
    }
    let original = draw(&mut frames, 40.0);
    assert!(original.rect.width() > original.bounds.width(), "the hit area is larger than the original thumb");
    let moved = draw(&mut frames, 72.0);
    let runs = last_frame_memo_runs(&frames.ctx);
    assert!(runs.iter().any(|(name, _, run)| name.ends_with("InspectedThumb") && *run == MemoRun::Replayed), "the view replayed: {runs:?}");
    let shift = vec2(0.0, 32.0);
    assert_eq!(moved.owner, original.owner);
    assert_eq!(moved.part, original.part);
    assert_eq!(moved.bounds, original.bounds.translate(shift), "original geometry follows the view");
    assert_eq!(moved.rect, original.rect.translate(shift), "expanded hit geometry follows the view");

    // The audit compares that kept journal with a real run at the new place, including both hit rectangles.
    AUDITED_HIT.set(false);
    audit_replays(&frames.ctx, true);
    assert_eq!(draw(&mut frames, 72.0), moved, "the replay agrees with the real inspection at the new place");
    assert!(AUDITED_HIT.get(), "the audit compared a real record with an expanded thumb hit");
}
