//! `Tracked` dirty flag and `Hooks::flush`.

use craft_command::{Cause, Change, Hooks, Tracked};

fn one_change(changes: Vec<Change<u32>>) -> Change<u32> {
    let mut iter = changes.into_iter();
    let change = iter.next().expect("one change");
    assert!(iter.next().is_none(), "flush emits one change");
    change
}

#[test]
fn write_without_mark_bumped_flush_emits_one_untracked() {
    let hooks = Hooks::<()>::new();
    let bus = hooks.change_bus::<u32>();
    let first = bus.subscribe();
    let second = bus.subscribe();
    let mut tracked = Tracked::new(1u32);
    assert!(!tracked.is_dirty());

    *tracked.write() = 9;
    assert!(tracked.is_dirty());
    assert_eq!(*tracked, 9);
    hooks.flush(&mut tracked, 3);

    let expected = Change { revision: 3, cause: Cause::Untracked, merged: false, snapshot: 9, prev: None };
    assert_eq!(one_change(bus.drain(first)), expected);
    assert_eq!(one_change(bus.drain(second)), expected);
    assert!(!tracked.is_dirty());
}

#[test]
fn flush_again_emits_nothing() {
    let hooks = Hooks::<()>::new();
    let bus = hooks.change_bus::<u32>();
    let sub = bus.subscribe();
    let mut tracked = Tracked::new(1u32);
    *tracked.write() = 4;
    hooks.flush(&mut tracked, 1);
    assert_eq!(bus.drain(sub).len(), 1);

    hooks.flush(&mut tracked, 2);
    assert!(bus.drain(sub).is_empty(), "a second flush of a clean value emits nothing");
}

#[test]
fn mark_bumped_then_flush_emits_nothing() {
    let hooks = Hooks::<()>::new();
    let bus = hooks.change_bus::<u32>();
    let sub = bus.subscribe();
    let mut tracked = Tracked::new(1u32);
    *tracked.write() = 5;
    assert!(tracked.is_dirty());
    tracked.mark_bumped();
    assert!(!tracked.is_dirty());
    hooks.flush(&mut tracked, 2);
    assert!(bus.drain(sub).is_empty(), "mark_bumped clears the dirty flag before flush");
}
