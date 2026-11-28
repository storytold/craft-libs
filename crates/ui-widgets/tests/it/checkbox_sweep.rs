//! Dragging a checkbox sets the related controls the pointer crosses.

use std::cell::RefCell;
use std::rc::Rc;

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Area, Id, LayerId, Order, Pos2, Rect, emath::TSTransform};

#[trial]
fn a_checkbox_sweep_keeps_its_group_and_skips_disabled_boxes() {
    let values = Rc::new(RefCell::new([true, false, true, true, true]));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut values = shown.borrow_mut();
        Stack::new().show(ui, |ui, _| {
            for (index, label) in ["Above", "Already clear", "Disabled", "Start"].into_iter().enumerate() {
                Checkbox::new(Bind::bound(&mut values[index])).label(label).enabled(index != 2).show(ui);
            }
        });
        Stack::new().show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut values[4])).label("Other group").show(ui);
        });
    });
    fixture.settle();
    let above = fixture.by_label("Above").expect("above");
    let start = fixture.by_label("Start").expect("start");
    let other = fixture.by_label("Other group").expect("other group");
    fixture.press(&start);
    fixture.frame(vec![pointer::moved(above.center())]);
    fixture.release(&other);
    fixture.idle(2);
    assert_eq!(*values.borrow(), [false, false, true, false, true]);
}

#[trial]
fn a_checkbox_sweep_includes_the_release_position() {
    let values = Rc::new(RefCell::new([false, false, false]));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut values = shown.borrow_mut();
        for (value, label) in values.iter_mut().zip(["First", "Middle", "Last"]) {
            Checkbox::new(Bind::bound(value)).label(label).show(ui);
        }
    });
    fixture.settle();
    let first = fixture.by_label("First").expect("first");
    let last = fixture.by_label("Last").expect("last");
    fixture.press(&last);
    fixture.release(&first);
    fixture.idle(2);
    assert_eq!(*values.borrow(), [true, true, true]);
}

#[trial]
fn a_completed_checkbox_sweep_replays_before_the_next_press() {
    let values = Rc::new(RefCell::new([false; 3]));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut values = shown.borrow_mut();
        for (value, label) in values.iter_mut().zip(["Earlier sibling", "Sweep start", "Next checkbox"]) {
            Checkbox::new(Bind::bound(value)).label(label).show(ui);
        }
    });
    fixture.settle();
    let earlier = fixture.by_label("Earlier sibling").expect("earlier").center();
    let start = fixture.by_label("Sweep start").expect("start").center();
    let next = fixture.by_label("Next checkbox").expect("next").center();

    fixture.frame(vec![pointer::moved(start)]);
    fixture.frame(vec![pointer::press(start), pointer::moved(earlier), pointer::release(earlier)]);
    assert_eq!(*values.borrow(), [false; 3], "the first frame only delivers the press");
    fixture.frame(vec![pointer::moved(next), pointer::press(next)]);
    assert_eq!(*values.borrow(), [true, true, false], "the completed sweep edits the earlier sibling even as the next press starts");
    fixture.frame(vec![pointer::release(next)]);
    fixture.idle(2);
    assert_eq!(*values.borrow(), [true, true, true], "the next click still toggles its own checkbox");
}

#[trial]
fn a_completed_checkbox_sweep_allows_an_immediate_click_on_its_owner() {
    for batched_click in [false, true] {
        let values = Rc::new(RefCell::new([false; 2]));
        let shown = values.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut values = shown.borrow_mut();
            for (value, label) in values.iter_mut().zip(["Crossed checkbox", "Sweep owner"]) {
                Checkbox::new(Bind::bound(value)).label(label).show(ui);
            }
        });
        fixture.settle();
        let crossed = fixture.by_label("Crossed checkbox").expect("crossed").center();
        let owner = fixture.by_label("Sweep owner").expect("owner").center();

        fixture.frame(vec![pointer::moved(owner)]);
        fixture.frame(vec![pointer::press(owner), pointer::moved(crossed), pointer::release(crossed)]);
        let mut click = vec![pointer::moved(owner), pointer::press(owner)];
        if batched_click {
            click.push(pointer::release(owner));
        }
        fixture.frame(click);
        if !batched_click {
            fixture.frame(vec![pointer::release(owner)]);
        }
        fixture.idle(2);
        assert_eq!(*values.borrow(), [true, false], "the next click toggles the same owner (batched: {batched_click})");
    }
}

#[trial]
fn a_checkbox_sweep_uses_an_exposed_sliver_but_skips_covered_and_clipped_boxes() {
    let values = Rc::new(RefCell::new([false; 5]));
    let shown = values.clone();
    let covers = Rc::new(RefCell::new(Vec::<Rect>::new()));
    let overlays = covers.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut values = shown.borrow_mut();
        let clip = ui.clip_rect();
        for (i, (value, label)) in values.iter_mut().zip(["Start", "Sliver", "Covered", "Clipped", "End"]).enumerate() {
            if i == 3 {
                ui.set_clip_rect(Rect::from_min_max(clip.min, clip.min));
            }
            Checkbox::new(Bind::bound(value)).label(label).show(ui);
            ui.set_clip_rect(clip);
        }
        for (i, rect) in overlays.borrow().iter().enumerate() {
            let id = Id::unique(("checkbox-sweep-cover", i));
            let pos = Pos2::new(rect.min.x.floor(), rect.min.y.floor());
            ui.ctx().set_transform_layer(LayerId::new(Order::Foreground, id), TSTransform::from_translation(rect.min - pos));
            Area::new(id).order(Order::Foreground).fixed_pos(pos).default_size(rect.size()).show(ui.ctx(), |ui| {
                ui.set_min_size(rect.size());
            });
        }
    });
    fixture.settle();
    let start = fixture.by_label("Start").expect("start");
    let sliver = fixture.by_label("Sliver").expect("sliver").rect;
    let covered = fixture.by_label("Covered").expect("covered").rect;
    let end = fixture.by_label("End").expect("end");
    let edge = sliver.top() + 2.2;
    let x = start.center().x;
    *covers.borrow_mut() = vec![
        Rect::from_min_max(Pos2::new(x - 12.0, sliver.top() - 1.0), Pos2::new(x + 12.0, edge)),
        Rect::from_min_max(Pos2::new(x - 12.0, edge + 0.15), Pos2::new(x + 12.0, sliver.bottom() + 1.0)),
        Rect::from_min_max(Pos2::new(x - 12.0, covered.top() - 1.0), Pos2::new(x + 12.0, covered.bottom() + 1.0)),
    ];
    fixture.settle();
    fixture.press(&start);
    fixture.frame(vec![pointer::moved(Pos2::new(x, end.center().y))]);
    fixture.release(&end);
    fixture.idle(2);
    assert_eq!(*values.borrow(), [true, true, false, false, true]);
}
