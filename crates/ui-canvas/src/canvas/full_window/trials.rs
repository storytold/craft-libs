//! Trials of the full-window dialog shell's keys.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::FullWindowDialog;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::input::key_press;
use crate::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};
use std::cell::Cell;

/// A full-window dialog has no overlay layer; its footer letters follow its own focus scope.
#[trial]
fn a_footer_letter_activates_its_button_in_a_full_window_dialog() {
    let picked = Cell::new(None);
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap) = (true, false);
        let shown = FullWindowDialog::new("Preferences")
            .opened(Bind::bound(&mut open))
            .initial_focus(Initial::First)
            .buttons([FooterButton::new("Reset", DialogRole::Destructive).letter('r'), FooterButton::new("Close", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label("Snap").show(ui);
            });
        if shown.output.picked.is_some() {
            picked.set(shown.output.picked);
        }
    };
    frames.idle(30, &mut ui);
    frames.frame(vec![key_press(Key::R, Modifiers::NONE)], &mut ui);
    frames.idle(2, &mut ui);
    assert_eq!(picked.get(), Some(0), "the letter picks its button");
}

/// The sample workspace on a dialog host, run once per frame.
mod hosting {
    use super::super::HostedWorkspace;
    use super::super::stories::{BrushChanged, BrushWorkspace, Flat};
    use crate::contract::Role;
    use crate::dialog::{DialogHost, DialogResult};
    use crate::testing::access::AccessTree;
    use crate::testing::input::key_press;
    use crate::testing::pointer::click;
    use crate::testing::{Frames, trial};
    use egui::{Key, Modifiers, Pos2, Rect, Ui};
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    type Hosted = HostedWorkspace<BrushWorkspace>;

    fn workspace() -> Hosted {
        HostedWorkspace::new(BrushWorkspace { brush: 10.0, ..BrushWorkspace::default() }, Arc::new(Flat))
    }

    /// The bounds of the first node with `role`, and `name` when given.
    fn bounds(tree: &AccessTree, role: Role, name: Option<&str>) -> Option<Rect> {
        let key = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(role) && name.is_none_or(|name| tree.name_of(*key) == Some(name)));
        key.and_then(|key| tree.bounds_of(key))
    }

    /// The centre of the first node with `role`, and `name` when given.
    fn centre(tree: &AccessTree, role: Role, name: Option<&str>) -> Pos2 {
        bounds(tree, role, name).unwrap_or_else(|| panic!("no {role:?} named {name:?}")).center()
    }

    /// The bounds of the workspace dialog in the last frame's tree.
    fn dialog_bounds(frames: &mut Frames) -> Option<Rect> {
        bounds(&frames.tree(), Role::DIALOG, Some("Liquify"))
    }

    #[trial]
    fn a_hosted_workspace_opens_full_window_and_returns_its_result() {
        let host = RefCell::new(DialogHost::new());
        let screen = Cell::new(Rect::NOTHING);
        let mut frames = Frames::new();
        let mut ui = |ui: &mut Ui| {
            screen.set(ui.ctx().content_rect());
            host.borrow_mut().run(ui);
        };
        host.borrow_mut().open(workspace());
        frames.idle(30, &mut ui);
        let tree = frames.tree();
        assert_eq!(bounds(&tree, Role::DIALOG, Some("Liquify")), Some(screen.get()), "the workspace fills the window");

        let slider = centre(&tree, Role::SLIDER, None);
        click(&mut frames, slider, &mut ui);
        let events = host.borrow_mut().get_mut::<Hosted>().expect("still open").take_events();
        let Some(&BrushChanged(size)) = events.last() else { panic!("a click on the slider reports the brush change") };
        assert_ne!(size, 10.0);
        assert!(host.borrow_mut().get_mut::<Hosted>().is_some_and(|hosted| hosted.take_events().is_empty()), "taken events are gone");

        let ok = centre(&tree, Role::BUTTON, Some("OK"));
        click(&mut frames, ok, &mut ui);
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Ok(size)), "OK answers the brush size");
        assert!(dialog_bounds(&mut frames).is_none(), "the workspace closed");

        for _ in 0..2 {
            host.borrow_mut().open(workspace());
            frames.idle(10, &mut ui);
            assert!(dialog_bounds(&mut frames).is_some(), "a reopened workspace shows");
            frames.frame(vec![key_press(Key::Escape, Modifiers::NONE)], &mut ui);
            frames.idle(2, &mut ui);
            assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Cancel), "Escape cancels");
        }

        host.borrow_mut().open(workspace());
        frames.idle(10, &mut ui);
        let cancel = centre(&frames.tree(), Role::BUTTON, Some("Cancel"));
        click(&mut frames, cancel, &mut ui);
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Cancel), "Cancel cancels");
        assert!(host.borrow().stack().is_empty(), "the cancelled workspace leaves the stack");
    }

    #[trial]
    fn view_commands_fire_inside_a_hosted_workspace() {
        let host = RefCell::new(DialogHost::new());
        let mut frames = Frames::new();
        let mut ui = |ui: &mut Ui| host.borrow_mut().run(ui);
        host.borrow_mut().open(workspace());
        frames.idle(30, &mut ui);
        let zoom = || host.borrow().get::<Hosted>().map(|hosted| hosted.workspace().view.zoom_percent()).expect("open");
        let fitted = zoom();
        frames.frame(vec![key_press(Key::Equals, Modifiers::COMMAND)], &mut ui);
        frames.idle(2, &mut ui);
        assert!(zoom() > fitted, "Cmd+= zooms the workspace's view in: {fitted} -> {}", zoom());
    }
}
