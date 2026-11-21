//! What the report step builds for a component in each pass: its AccessKit data only for a reader (a node, a merged
//! role, a role picked from alternatives), and its tooltip text only for a tooltip the pointer or keyboard wants or
//! one still shown.

use craft_behavior::{IconId, keyed};
use craft_ui::buttons::IconButton;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::tabs::{Tab, TabStrip};
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, last_pass_access_builds, last_pass_tip_texts, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Ui, Vec2, pos2, vec2};
use std::cell::RefCell;

/// The rows of each scene.
const ROWS: usize = 40;
/// A screen every row of every scene fits in, the menu's and the tab strip's included.
const ROOM: Vec2 = vec2(8000.0, 1400.0);

/// The `Access` values the last idle pass of `show` built with AccessKit on, and with it off.
fn access_builds(show: impl Fn(&mut Ui)) -> (u32, u32) {
    let mut frames = Frames::new();
    frames.screen(ROOM);
    frames.idle(3, &show);
    let on = last_pass_access_builds(&frames.ctx);
    frames.ctx.disable_accesskit();
    frames.idle(3, &show);
    (on, last_pass_access_builds(&frames.ctx))
}

/// Count relation: with AccessKit on, every row's node reads its access data; with it off, a pass builds none, for
/// component rows (Field rows, an open menu's items) and for a collection's members (a tab strip's tabs) alike.
#[trial]
fn reports_build_access_data_only_for_a_reader() {
    let labels: Vec<String> = (0..ROWS).map(|row| format!("Row {row}")).collect();
    let fields = |ui: &mut Ui| {
        for label in &labels {
            Field::new(label.clone()).show(ui, |ui, _| NumberField::new(label.clone(), Bind::given(Some(1.0))).show(ui));
        }
    };
    let menu = |ui: &mut Ui| {
        let mut open = true;
        let _ = Menu::new("Items").open(Bind::bound(&mut open)).id_salt("menu").show(ui, |ui, _| {
            for label in &labels {
                let _ = MenuItem::new(label.clone()).id_salt(label.as_str()).show(ui);
            }
        });
    };
    let keys: Vec<usize> = (0..ROWS).collect();
    let tabs = |ui: &mut Ui| {
        let source = keyed(&keys, |key: &usize| *key);
        let _ = TabStrip::new("Documents").shown(0).show_items(ui, &source, &mut |key, _| Tab::new(labels[*key].clone()));
    };
    for (scene, (on, off)) in [("Field rows", access_builds(fields)), ("menu items", access_builds(menu)), ("tabs", access_builds(tabs))] {
        assert!(on >= ROWS as u32, "with AccessKit on, each of the {scene} reads its access data ({on} built)");
        assert_eq!(off, 0, "with AccessKit off, none of the {scene} builds access data");
    }
}

/// Count relation: icon buttons whose label is their tooltip build no tooltip text while the pointer is away. The one
/// the pointer rests on builds its own once a pass, before and while its tooltip shows it; once the pointer leaves, the
/// fading tooltip builds it once a pass, and nothing builds it after the fade.
#[trial]
fn tooltip_texts_are_built_only_for_a_wanted_tooltip() {
    let labels: Vec<String> = (0..ROWS).map(|row| format!("Tool {row}")).collect();
    let rects = RefCell::new(Vec::new());
    let show = |ui: &mut Ui| {
        let mut shown = rects.borrow_mut();
        shown.clear();
        for label in &labels {
            shown.push(IconButton::new(IconId::new("play")).label(label.clone()).show(ui).response.rect);
        }
    };
    let painted = |frames: &Frames| frames.texts().iter().any(|run| run.text == labels[ROWS / 2]);
    let mut frames = Frames::new();
    frames.screen(ROOM);
    frames.idle(3, &show);
    assert_eq!(last_pass_tip_texts(&frames.ctx), 0, "no tooltip is wanted, so no text is built");
    let rested: Rect = rects.borrow()[ROWS / 2];
    frames.frame(vec![moved(rested.center())], &show);
    frames.idle(2, &show);
    assert_eq!(last_pass_tip_texts(&frames.ctx), 1, "only the button under the pointer builds its text");
    assert!(!painted(&frames), "the tooltip waits for its delay");
    frames.advance(DELAY);
    frames.idle(3, &show);
    assert!(painted(&frames), "the tooltip shows the button's label");
    assert_eq!(last_pass_tip_texts(&frames.ctx), 1, "the shown tooltip's text is built once a pass");
    frames.frame(vec![moved(pos2(ROOM.x - 10.0, ROOM.y - 10.0))], &show);
    assert!(painted(&frames), "the tooltip fades out after the pointer leaves");
    assert_eq!(last_pass_tip_texts(&frames.ctx), 1, "the fading tooltip's text is built once a pass");
    frames.advance(1.0);
    frames.idle(3, &show);
    assert!(!painted(&frames), "the faded tooltip is gone");
    assert_eq!(last_pass_tip_texts(&frames.ctx), 0, "nothing builds a text after the fade");
}
