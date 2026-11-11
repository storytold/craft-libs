//! The preset library's rules, the open-group state and the nesting rule, the panel menu and the file entries.

use craft_ui::patterns::presets::{GroupPath, PresetChange, PresetId, PresetKey, PresetLibrary, children_end};
use craft_ui::testing::trial;

type Key = PresetKey<PresetId>;

/// Root: Loose; A: Red, Green, folder Inner (Blue); B: Cyan; nesting 1.
fn sample() -> (PresetLibrary<u32>, [PresetId; 5]) {
    let mut lib = PresetLibrary::new().with_nesting(1);
    let root = GroupPath::root();
    let loose = lib.add(&root, "Loose", 0).unwrap();
    let a = lib.new_group(&root, "A").unwrap();
    let red = lib.add(&a, "Red", 1).unwrap();
    let green = lib.add(&a, "Green", 2).unwrap();
    let inner = lib.new_group(&a, "Inner").unwrap();
    let blue = lib.add(&inner, "Blue", 3).unwrap();
    let b = lib.new_group(&root, "B").unwrap();
    let cyan = lib.add(&b, "Cyan", 4).unwrap();
    (lib, [loose, red, green, blue, cyan])
}

/// The display position `target` has once `moved` and the rows inside them are taken out: the `to` a drop before it has.
fn before(lib: &PresetLibrary<u32>, moved: &[Key], target: &Key) -> usize {
    let rows = lib.rows();
    let mut gone = vec![false; rows.len()];
    for key in moved {
        let at = rows.iter().position(|row| row.key == *key).unwrap();
        gone[at..children_end(&rows, at)].fill(true);
    }
    rows.iter().zip(&gone).filter(|(_, gone)| !**gone).position(|(row, _)| row.key == *target).unwrap()
}

fn group_labels(lib: &PresetLibrary<u32>) -> Vec<String> {
    lib.rows().iter().filter(|row| row.is_group()).map(|row| row.label.clone()).collect()
}

/// A group dropped before another group becomes its sibling and takes its presets along; a drop past the end lands after
/// the last row, inside that row's group.
#[trial]
fn reordering_a_group_changes_its_parent_and_keeps_its_presets() {
    let (mut lib, [_loose, _red, _green, blue, _cyan]) = sample();
    let inner = PresetKey::group(["A", "Inner"]);
    let to = before(&lib, std::slice::from_ref(&inner), &PresetKey::group(["B"]));
    let changes = lib.reorder(std::slice::from_ref(&inner), to).unwrap();
    assert!(matches!(&changes[0], PresetChange::GroupMoved { from, to, .. } if from.names() == ["A", "Inner"] && to.names() == ["Inner"]));
    assert_eq!(lib.group_of(blue), Some(GroupPath::new(["Inner"])));
    assert_eq!(group_labels(&lib), ["A", "Inner", "B"]);

    let a = PresetKey::group(["A"]);
    let end = lib.rows().len();
    lib.reorder(std::slice::from_ref(&a), end).unwrap();
    assert!(lib.group(&GroupPath::new(["B", "A"])).is_some(), "after Cyan, which is in B");
}
