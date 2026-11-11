//! `Env`: the root, scopes and the interface scale.

use craft_behavior::MotionMode;
use craft_theme::ControlSize;
use craft_ui::runtime::env::{Env, EnvValue, with_env};
use craft_ui::testing::memory::scoped;
use craft_ui::testing::{Frames, trial};

#[derive(Clone, Debug, PartialEq, EnvValue)]
#[env(root = Units("pt"))]
struct Units(&'static str);

/// 2.3 merge table: enabled ANDs and disables the child `Ui`, read-only ORs, motion takes the lowest mode, the
/// innermost value wins, and `Env::of` follows the `Ui` tree.
#[trial]
fn scopes_merge_by_the_table_and_follow_the_ui_tree() {
    let mut frames = Frames::new();
    frames.frame(vec![], |ui| {
        let root = Env::of(ui);
        assert!(root.enabled && !root.read_only && ui.is_enabled());
        assert_eq!(root.size, ControlSize::Md);
        assert_eq!(root.get::<Units>(), Units("pt"));
        with_env(
            ui,
            |o| {
                o.size = Some(ControlSize::Sm);
                o.enabled = Some(false);
                o.read_only = Some(true);
                o.motion = Some(MotionMode::Reduced);
                o.values.set(Units("mm"));
            },
            |ui| {
                with_env(
                    ui,
                    |o| {
                        o.enabled = Some(true);
                        o.read_only = Some(false);
                        o.motion = Some(MotionMode::Full);
                        o.values.set(Units("in"));
                    },
                    |ui| {
                        let inner = Env::of(ui);
                        assert!(!inner.enabled, "nothing below can enable again");
                        assert!(!ui.is_enabled(), "a disabling scope disables its child Ui, and an inner scope cannot enable it");
                        assert!(inner.read_only, "nothing below can make it writable");
                        assert_eq!(inner.motion.mode, MotionMode::Reduced, "the lowest mode wins");
                        assert_eq!(inner.size, ControlSize::Sm, "the outer scope's size is inherited");
                        assert_eq!(inner.get::<Units>(), Units("in"), "the innermost app value wins");
                        scoped(ui, |child| assert_eq!(Env::of(child).get::<Units>(), Units("in"), "a child Ui sees its parent's entry"));
                    },
                );
                assert_eq!(Env::of(ui).get::<Units>(), Units("mm"), "leaving a scope restores the outer entry");
                assert!(!ui.is_enabled(), "the scope's own Ui stays disabled after an inner scope");
            },
        );
        assert_eq!(Env::of(ui).get::<Units>(), Units("pt"));
        assert!(ui.is_enabled(), "the parent Ui is untouched by the scope");
    });
}
