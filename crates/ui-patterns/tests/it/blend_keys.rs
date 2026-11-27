//! The separator rule of every blend mode list.

use craft_ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut, blend_entries};
use craft_ui::runtime::env::Label;
use craft_ui::testing::trial;

/// The positions of the separators in the entries `modes` give.
fn separators(modes: &[BlendKeyEntry<&'static str>]) -> Vec<usize> {
    let entries = blend_entries(modes, |mode| Label::from(mode.key));
    entries.iter().enumerate().filter(|(_, entry)| entry.is_separator()).map(|(at, _)| at).collect()
}

#[trial]
fn separators_head_photoshops_groups_and_never_lead() {
    use BlendShortcut as S;
    let lettered = |key, shortcut| BlendKeyEntry::new(key, shortcut);

    // A list that starts with a group head has no leading separator.
    assert_eq!(separators(&[lettered("darken", S::Darken), lettered("multiply", S::Multiply), lettered("lighten", S::Lighten)]), vec![2]);
    // Pass Through and Normal are in the first group.
    assert_eq!(separators(&[lettered("passThrough", S::PassThrough), lettered("normal", S::Normal), lettered("darken", S::Darken)]), vec![2]);
    // Behind and Clear stay in the first group.
    assert_eq!(
        separators(&[lettered("normal", S::Normal), lettered("behind", S::Behind), lettered("clear", S::Clear), lettered("darken", S::Darken)]),
        vec![3]
    );
    // An unlettered mode never heads a group, first or not.
    assert_eq!(
        separators(&[
            lettered("multiply", S::Multiply),
            BlendKeyEntry::unlettered("darkerColor"),
            lettered("lighten", S::Lighten),
            lettered("overlay", S::Overlay)
        ]),
        vec![2, 4]
    );
    assert!(separators(&[BlendKeyEntry::unlettered("darkerColor"), lettered("multiply", S::Multiply)]).is_empty());
}
