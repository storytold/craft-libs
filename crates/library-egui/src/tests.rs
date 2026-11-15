//! The pane against a made-up app whose Library holds pages, with a tab of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::*;

/// A 16×9 PNG.
fn png() -> String {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(16, 9, image::Rgba([40, 90, 200, 255])).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    craft_library::base64(bytes.get_ref())
}

#[derive(Default)]
struct Pages {
    folders: Vec<String>,
    acted: Vec<(String, Value)>,
    asked: Vec<String>,
    picker: bool,
    /// Show every word as «word», and remember the words asked for.
    translate: bool,
    words: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// The app takes dropped pages.
    drags: bool,
    /// The app has an insert of its own in the context menu.
    inserts: bool,
    /// There is nowhere to put a page.
    nowhere: bool,
}

impl Host for Pages {
    fn act(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.acted.push((id.to_string(), params.clone()));
        if id == "library.addFolder" {
            let path = params["path"].as_str().unwrap_or_default().to_string();
            self.folders.push(path.clone());
            return Ok(json!({"path": path, "added": true}));
        }
        Ok(json!({}))
    }
    fn ask(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.asked.push(id.to_string());
        let pages = json!([{"index": 0, "title": "Cover", "hidden": false}, {"index": 1, "title": "Terms", "hidden": true}]);
        match id {
            "library.list" if params.get("file").is_some() => Ok(json!([{"file": "/lib/contract.doc", "title": "Contract", "pages": pages}])),
            "library.list" => Ok(json!([{"file": "/lib/contract.doc", "folder": "/lib", "title": "contract"}])),
            "library.search" => Ok(json!([{"file": "/lib/contract.doc", "page": 1, "title": "Terms", "match": "title"}])),
            "library.thumbnail" => Ok(json!({"png": png(), "width": 16, "height": 9})),
            "library.shapes" => Ok(json!({"shapes": [{"name": "Signature block"}, {"name": "Seal"}, {"name": "Inner", "parent": "Seal"}]})),
            _ => Err(format!("no {id}")),
        }
    }
    fn folders(&self) -> Vec<String> {
        self.folders.clone()
    }
    fn pick_folder(&mut self) -> Option<Option<String>> {
        self.picker.then(|| Some("/picked".to_string()))
    }
    fn can_place(&self) -> bool {
        !self.nowhere
    }
    fn report(&mut self, _message: String) {}
    fn tab(&mut self, ui: &mut Ui, id: &str) {
        ui.label(format!("the app's own {id} tab"));
    }
    fn text<'a>(&self, english: &'a str) -> Cow<'a, str> {
        self.words.borrow_mut().insert(english.to_string());
        if self.translate { Cow::Owned(format!("«{english}»")) } else { Cow::Borrowed(english) }
    }
    fn menu_inserts(&self) -> Vec<MenuInsert> {
        if self.inserts { vec![MenuInsert { label: PLACE_IN_MARGIN, params: json!({"placement": "margin"}) }] } else { vec![] }
    }
    fn drag_hint(&self) -> Option<&'static str> {
        self.drags.then_some(DRAG_HINT)
    }
}

const PLACE_IN_MARGIN: &str = "Place in margin";
const DRAG_HINT: &str = "Drag onto a page to put it there";

const LABELS: Labels = Labels {
    nouns: Nouns { one: "page", many: "pages" },
    tabs: &[(ITEMS, "Pages"), ("stamps", "Stamps")],
    item: "Page",
    insert_item: "Insert page",
    insert_shapes: None,
    insert_element: Some("Insert element"),
    search_hint: "Search pages",
    add_folder_tip: "Show the pages of a folder of documents",
    empty: "Add a folder of documents to see their pages here.",
    not_in_browser: "Not in the browser.",
    aspect: 1.4,
    one_per_file: false,
};

fn harness(host: Pages) -> Harness<'static, (State, Pages)> {
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 900.0)).build_ui_state(
        |ui, s: &mut (State, Pages)| {
            let pal = Palette::from_visuals(ui.visuals());
            show(ui, &mut s.0, &mut s.1, &LABELS, &pal);
        },
        (State::default(), host),
    );
    h.run_steps(4);
    h
}

#[test]
fn an_app_with_pages_lists_reads_and_inserts_them() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], ..Default::default() });
    assert!(h.state().0.ready());
    assert!(h.query_by_label("Page 1: Cover").is_some() && h.query_by_label("Page 2: Terms").is_some());
    assert!(h.state().1.asked.iter().any(|a| a == "library.thumbnail"), "thumbnails drawn: {:?}", h.state().1.asked);
    h.get_by_label("Page 2: Terms").click();
    h.run_steps(2);
    assert_eq!(h.state().1.acted.last(), Some(&("library.insert".to_string(), json!({"file": "/lib/contract.doc", "page": 1}))));
    // Search speaks the app's nouns too.
    h.state_mut().0.query = "terms".into();
    for _ in 0..30 {
        h.step();
    }
    assert_eq!(h.state().0.hits(), Some(1));
    assert!(h.query_by_label("Page 1: Cover").is_none());
}

#[test]
fn insert_element_lists_top_level_names_and_inserts_one() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], ..Default::default() });
    let at = h.get_by_label("Page 1: Cover").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert!(h.query_by_label("Insert shapes on current slide").is_none(), "an app without that insert doesn't offer it");
    h.get_by_label("Insert element").click();
    h.run_steps(2);
    assert!(h.query_by_label("Inner").is_none(), "grouped shapes aren't offered on their own here");
    h.get_by_label("Seal").click();
    h.run_steps(2);
    assert_eq!(h.state().1.acted.last(), Some(&("library.insert".to_string(), json!({"file": "/lib/contract.doc", "page": 0, "names": ["Seal"]}))));
}

#[test]
fn the_apps_own_tab_is_drawn_by_the_app() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], ..Default::default() });
    h.get_by_label("Stamps").click();
    h.run_steps(2);
    assert_eq!(h.state().0.tab, "stamps");
    assert!(h.query_by_label("the app's own stamps tab").is_some());
}

#[test]
fn without_folders_the_pane_says_what_to_do_and_adds_one() {
    let mut h = harness(Pages::default());
    assert!(h.query_by_label_contains("Add a folder of documents").is_some());
    // No picker: a path field.
    h.get_by_label("Add Folder…").click();
    h.run_steps(2);
    assert!(h.state().0.adding);
    // With a picker: the picked folder is added and shown.
    let mut h = harness(Pages { picker: true, ..Default::default() });
    h.get_by_label("Add Folder…").click();
    h.run_steps(2);
    assert_eq!(h.state().1.folders, vec!["/picked".to_string()]);
    assert_eq!(h.state().0.folder.as_deref(), Some("/picked"));
}

#[test]
fn thumbnails_decode_and_hostile_ones_fail_quietly() {
    assert!(decode(&unbase64(&png()).unwrap()).is_some_and(|i| i.size == [16, 9]));
    assert!(decode(b"not a png").is_none());
    assert_eq!(unbase64("Zm9vYmFy"), Some(b"foobar".to_vec()));
    assert_eq!(unbase64("Zg=="), Some(b"f".to_vec()));
    assert!(unbase64("**").is_none());
}

/// The words of [`LABELS`], which an app's catalog holds beside [`words::ALL`].
fn label_words() -> Vec<&'static str> {
    let mut w: Vec<&str> = LABELS.tabs.iter().map(|(_, l)| *l).collect();
    w.extend([LABELS.item, LABELS.insert_item, LABELS.search_hint, LABELS.add_folder_tip, LABELS.empty, LABELS.not_in_browser]);
    w.extend(LABELS.insert_shapes.into_iter().chain(LABELS.insert_element));
    w.extend([PLACE_IN_MARGIN, DRAG_HINT]);
    w
}

#[test]
fn every_word_shown_comes_through_the_host() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], translate: true, drags: true, inserts: true, ..Default::default() });
    // The pane's words and the app's, translated; the English isn't shown.
    for shown in ["«Add Folder…»", "«Remove»", "«Refresh»", "«Pages»", "«Stamps»"] {
        assert!(h.query_by_label(shown).is_some(), "{shown}");
    }
    assert!(h.query_by_label("Add Folder…").is_none());
    assert!(h.query_by_label("«Library folder»").is_some(), "the drop-down's accessible name");
    // A template is translated whole, then filled in.
    assert!(h.query_by_label("««Page» 1: Cover»").is_some());
    let at = h.get_by_label("««Page» 1: Cover»").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert!(h.query_by_label("«Insert page»").is_some() && h.query_by_label("«Insert element»").is_some());
    assert!(h.query_by_label("«Place in margin»").is_some(), "the app's own insert");
    assert!(h.state().1.words.borrow().contains(DRAG_HINT), "the drag hint in the tooltip");
    h.state_mut().0.query = "nothing like it".into();
    h.state_mut().1.asked.clear();
    for _ in 0..30 {
        h.step();
    }
    // Every word asked for is one an app's catalog can hold.
    let known: Vec<&str> = words::ALL.iter().copied().chain(label_words()).collect();
    for w in h.state().1.words.borrow().iter() {
        assert!(known.contains(&w.as_str()), "{w:?} is in neither words::ALL nor the Labels");
    }
}

#[test]
fn templates_fill_in_once_and_keep_other_braces() {
    assert_eq!(fill("Reading {read} of {total} files…", &[("read", "3"), ("total", "10")]), "Reading 3 of 10 files…");
    // A value holding a placeholder is shown as it is, and unknown braces stay.
    assert_eq!(fill("{title} {x} {", &[("title", "{title}")]), "{title} {x} {");
    assert_eq!(fill("{name} (×{count})", &[("name", "Logo"), ("count", "2")]), "Logo (×2)");
}

/// `n` one-picture files in `/lib`, `pic0.png`…; their pictures are titled `p0`… or not at all.
struct Pics {
    n: usize,
    titled: bool,
    /// The app takes dropped pictures.
    drags: bool,
}

impl Host for Pics {
    fn act(&mut self, _id: &str, _params: Value) -> Result<Value, String> {
        Ok(json!({}))
    }
    fn ask(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let titled = self.titled;
        let file = |i: usize| {
            let title = if titled { format!("p{i}") } else { String::new() };
            json!({"file": format!("/lib/pic{i}.png"), "folder": "/lib", "title": format!("pic{i}"), "pages": [{"index": 0, "title": title, "hidden": false}]})
        };
        match id {
            "library.list" => match params["file"].as_str() {
                Some(f) => Ok(json!([(0..self.n).map(file).find(|v| v["file"] == f).unwrap_or(json!({}))])),
                None => Ok(Value::Array((0..self.n).map(file).collect())),
            },
            "library.thumbnail" => Ok(json!({"png": png(), "width": 16, "height": 9})),
            _ => Ok(json!([])),
        }
    }
    fn folders(&self) -> Vec<String> {
        vec!["/lib".into()]
    }
    fn pick_folder(&mut self) -> Option<Option<String>> {
        None
    }
    fn can_place(&self) -> bool {
        true
    }
    fn report(&mut self, _message: String) {}
    fn drag_hint(&self) -> Option<&'static str> {
        self.drags.then_some(DRAG_HINT)
    }
}

fn pics(n: usize, titled: bool, labels: Labels, scrolled: bool) -> Harness<'static, State> {
    pics_with(Pics { n, titled, drags: false }, labels, scrolled)
}

fn pics_with(host: Pics, labels: Labels, scrolled: bool) -> Harness<'static, State> {
    let Pics { n, titled, drags } = host;
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 480.0)).build_ui_state(
        move |ui, st: &mut State| {
            let pal = Palette::from_visuals(ui.visuals());
            let mut host = Pics { n, titled, drags };
            // Some apps put their panes in a scroll area of their own.
            if scrolled {
                egui::ScrollArea::vertical().show(ui, |ui| show(ui, st, &mut host, &labels, &pal));
            } else {
                show(ui, st, &mut host, &labels, &pal);
            }
        },
        State::default(),
    );
    h.run_steps(6);
    h
}

#[test]
fn files_of_one_item_each_share_one_grid() {
    let flat = Labels { one_per_file: true, item: "Picture", ..LABELS };
    let h = pics(4, false, flat, false);
    // Named after the file, with no heading of its own, two or more to a row.
    let first = h.get_by_label("Picture: pic0").rect();
    let second = h.get_by_label("Picture: pic1").rect();
    assert_eq!(first.top(), second.top(), "side by side");
    assert!(h.query_by_label("pic0").is_none(), "no heading per file");
    // Grouped by file, each file has its heading and its one item a row to itself.
    let h = pics(4, false, LABELS, false);
    assert!(h.query_by_label("pic0").is_some());
    let cells: Vec<_> = h.query_all_by_label("Page 1").map(|n| n.rect().top()).collect();
    assert!(cells.len() == 4 && cells[0] < cells[1], "{cells:?}");
}

#[test]
fn the_controls_stay_in_view_when_the_grid_scrolls() {
    // The wheel scrolls the grid whether or not its items can be dragged out.
    for drags in [false, true] {
        let mut h = pics_with(Pics { n: 40, titled: true, drags }, LABELS, true);
        let controls = h.get_by_label("Library folder").rect().top();
        let item = || "Page 1: p0";
        let before = h.get_by_label(item()).rect().top();
        let over = h.get_by_label(item()).rect().center();
        h.event(egui::Event::PointerMoved(over));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(8);
        assert!(h.get_by_label(item()).rect().top() < before - 100.0, "the grid scrolled (drags: {drags})");
        assert_eq!(h.get_by_label("Library folder").rect().top(), controls, "the folder and search controls didn't");
    }
}

/// Press the primary button at `from`, move to `to` in a few steps, and hold it there.
fn press_and_move<S>(h: &mut Harness<'_, S>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.step();
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.step();
    }
}

fn release<S>(h: &mut Harness<'_, S>, at: egui::Pos2) {
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.step();
    h.step();
}

fn carried<S>(h: &Harness<'_, S>) -> Option<Dragged> {
    egui::DragAndDrop::payload::<Dragged>(&h.ctx).map(|d| (*d).clone())
}

#[test]
fn an_app_that_takes_drops_drags_items_out_and_a_click_still_inserts() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], drags: true, ..Default::default() });
    // A click inserts, as without dragging.
    h.get_by_label("Page 2: Terms").click();
    h.run_steps(2);
    assert_eq!(h.state().1.acted.last(), Some(&("library.insert".to_string(), json!({"file": "/lib/contract.doc", "page": 1}))));
    let acted = h.state().1.acted.len();
    // A drag carries the item, keyed as `library.insert` takes it.
    let at = h.get_by_label("Page 2: Terms").rect().center();
    press_and_move(&mut h, at, at + egui::vec2(-150.0, 300.0));
    let item = Dragged { file: "/lib/contract.doc".into(), item: 1 };
    assert_eq!(carried(&h), Some(item.clone()));
    assert_eq!(item.params(LABELS.nouns), json!({"file": "/lib/contract.doc", "page": 1}));
    // Let go where nothing takes it: nothing is inserted, and the drag is over.
    release(&mut h, at + egui::vec2(-150.0, 300.0));
    assert_eq!(h.state().1.acted.len(), acted, "the pane inserts nothing itself");
    assert_eq!(carried(&h), None);
    // A drag the app takes where it is let go.
    press_and_move(&mut h, at, at + egui::vec2(-150.0, 300.0));
    let dropped = egui::DragAndDrop::take_payload::<Dragged>(&h.ctx).map(|d| (*d).clone());
    assert_eq!(dropped, Some(item));
    release(&mut h, at + egui::vec2(-150.0, 300.0));
    // Escape gives one up.
    press_and_move(&mut h, at, at + egui::vec2(0.0, 200.0));
    assert!(carried(&h).is_some());
    h.key_press(egui::Key::Escape);
    h.step();
    assert_eq!(carried(&h), None);
    release(&mut h, at + egui::vec2(0.0, 200.0));
    assert_eq!(h.state().1.acted.len(), acted);
}

#[test]
fn an_app_without_drops_keeps_its_items_in_the_pane() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], ..Default::default() });
    let at = h.get_by_label("Page 2: Terms").rect().center();
    press_and_move(&mut h, at, at + egui::vec2(-150.0, 300.0));
    assert_eq!(carried(&h), None);
    release(&mut h, at + egui::vec2(-150.0, 300.0));
    assert!(h.state().1.acted.is_empty());
}

#[test]
fn a_right_drag_doesnt_carry_the_item() {
    let mut h = harness(Pages { folders: vec!["/lib".into()], drags: true, ..Default::default() });
    let at = h.get_by_label("Page 1: Cover").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(at + egui::vec2(0.0, 30.0 * k as f32)));
        h.step();
    }
    assert_eq!(carried(&h), None);
}

#[test]
fn a_finger_still_scrolls_the_grid_over_items_that_drag() {
    let mut h = pics_with(Pics { n: 40, titled: true, drags: true }, LABELS, true);
    let item = || "Page 1: p0";
    let before = h.get_by_label(item()).rect().top();
    let from = h.get_by_label(item()).rect().center();
    let touch = |phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(0), phase, pos, force: None };
    // A touch screen sends the touch and the pointer events it stands for.
    h.event(touch(egui::TouchPhase::Start, from));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    let mut to = from;
    for _ in 0..6 {
        to += egui::vec2(0.0, -40.0);
        h.event(touch(egui::TouchPhase::Move, to));
        h.event(egui::Event::PointerMoved(to));
        h.step();
    }
    assert_eq!(carried(&h), None, "a finger doesn't carry the item");
    h.event(touch(egui::TouchPhase::End, to));
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerGone);
    h.run_steps(4);
    assert!(h.get_by_label(item()).rect().top() < before - 100.0, "the grid followed the finger");
}

#[test]
fn the_apps_own_inserts_follow_the_panes_in_the_menu() {
    let open_menu = |h: &mut Harness<'static, (State, Pages)>| {
        let at = h.get_by_label("Page 1: Cover").rect().center();
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(2);
    };
    let mut h = harness(Pages { folders: vec!["/lib".into()], inserts: true, ..Default::default() });
    open_menu(&mut h);
    let ours = h.get_by_label("Insert page").rect().top();
    assert!(h.get_by_label(PLACE_IN_MARGIN).rect().top() > ours, "after the pane's own");
    h.get_by_label(PLACE_IN_MARGIN).click();
    h.run_steps(2);
    let placed = json!({"file": "/lib/contract.doc", "page": 0, "placement": "margin"});
    assert_eq!(h.state().1.acted.last(), Some(&("library.insert".to_string(), placed)));
    // With nowhere to put the item, it is offered but can't be chosen.
    let mut h = harness(Pages { folders: vec!["/lib".into()], inserts: true, nowhere: true, ..Default::default() });
    open_menu(&mut h);
    assert!(h.get_by_label(PLACE_IN_MARGIN).accesskit_node().is_disabled());
    h.get_by_label(PLACE_IN_MARGIN).click();
    h.run_steps(2);
    assert!(h.state().1.acted.is_empty());
    // Without inserts of its own, the menu has only the pane's.
    let mut h = harness(Pages { folders: vec!["/lib".into()], ..Default::default() });
    open_menu(&mut h);
    assert!(h.query_by_label("Insert page").is_some() && h.query_by_label(PLACE_IN_MARGIN).is_none());
}
