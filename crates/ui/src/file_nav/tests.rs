use super::*;

const TOP: &[FileNavItem<'static, &str>] =
    &[FileNavItem { key: "home", label: "Home" }, FileNavItem { key: "new", label: "New" }, FileNavItem { key: "open", label: "Open" }];
const REST: &[FileNavItem<'static, &str>] = &[FileNavItem { key: "info", label: "Info" }, FileNavItem { key: "print", label: "Print" }];
const FOOT: &[FileNavItem<'static, &str>] = &[
    FileNavItem { key: "options", label: "Options" },
    FileNavItem { key: "community", label: "A community with a name far too long for the column" },
    FileNavItem { key: "about", label: "About" },
];

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx
}

fn frame(ctx: &egui::Context, band: f32, height: f32, events: Vec<egui::Event>) -> (Option<FileNavAction<&'static str>>, egui::FullOutput) {
    frame_with(ctx, band, height, &[], events)
}

fn style() -> FileNavStyle {
    FileNavStyle::on_accent(Color32::from_rgb(0x3B, 0x5B, 0xDB))
}

/// One frame of the column with the given entries disabled.
fn frame_with(
    ctx: &egui::Context,
    band: f32,
    height: f32,
    disabled: &[&'static str],
    events: Vec<egui::Event>,
) -> (Option<FileNavAction<&'static str>>, egui::FullOutput) {
    let style = style();
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, height))), events, ..Default::default() };
    let mut action = None;
    let mut full = ctx.run_ui(input, |ui| {
        let nothing = |_: Option<&str>, _: &Painter, _: Rect, _: Color32| {};
        action = FileNav::new("Back", &[TOP, REST], Some("open"), &style, &nothing).foot(FOOT).disabled(disabled).title_band(band).show(ui).action;
    });
    full.textures_delta.clear();
    (action, full)
}

fn node<'a>(full: &'a egui::FullOutput, label: &str) -> Option<&'a egui::accesskit::Node> {
    let update = full.platform_output.accesskit_update.as_ref().expect("accesskit is on");
    update.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(label))
}

fn centre(full: &egui::FullOutput, label: &str) -> egui::Pos2 {
    let b = node(full, label).expect("node with the label").bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

fn click(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

fn press(label: &str) -> Option<FileNavAction<&'static str>> {
    press_with(&[], label)
}

fn press_with(disabled: &[&'static str], label: &str) -> Option<FileNavAction<&'static str>> {
    let ctx = context();
    let p = centre(&frame_with(&ctx, 0.0, 700.0, disabled, vec![]).1, label);
    frame_with(&ctx, 0.0, 700.0, disabled, vec![egui::Event::PointerMoved(p), click(p, true)]);
    frame_with(&ctx, 0.0, 700.0, disabled, vec![click(p, false)]).0
}

#[test]
fn back_pages_and_the_foot_report_what_was_chosen() {
    assert_eq!(press("Back"), Some(FileNavAction::Back));
    assert_eq!(press("New"), Some(FileNavAction::Choose("new")));
    assert_eq!(press("Print"), Some(FileNavAction::Choose("print")), "a page of the second group");
    assert_eq!(press("Options"), Some(FileNavAction::Choose("options")), "an entry of the foot");
    assert_eq!(frame(&context(), 0.0, 700.0, vec![]).0, None, "nothing without input");
}

#[test]
fn the_page_that_is_showing_is_the_only_one_announced_as_chosen() {
    let (_, full) = frame(&context(), 0.0, 700.0, vec![]);
    let toggled = |label: &str| node(&full, label).expect("entry").toggled();
    assert_eq!(toggled("Open"), Some(egui::accesskit::Toggled::True));
    assert_eq!(toggled("Home"), Some(egui::accesskit::Toggled::False));
    assert_eq!(toggled("Back"), None, "Back is an action, not a page");
}

#[test]
fn the_order_is_back_then_the_groups_then_the_foot_at_the_bottom() {
    let (_, full) = frame(&context(), 0.0, 700.0, vec![]);
    let y = |label: &str| centre(&full, label).y;
    assert!(y("Back") < y("Home") && y("Open") < y("Info") && y("Print") < y("Options") && y("Options") < y("About"));
    assert!(y("Info") - y("Open") > y("Open") - y("New"), "a rule separates the groups");
    assert!(y("About") > 700.0 - 60.0, "the foot sits at the bottom of the column");
}

#[test]
fn a_title_band_keeps_back_clear_of_the_window_buttons_and_drags_the_window() {
    let ctx = context();
    let without = centre(&frame(&ctx, 0.0, 700.0, vec![]).1, "Back").y;
    let with = centre(&frame(&ctx, 28.0, 700.0, vec![]).1, "Back").y;
    assert!(with > 28.0 && with > without, "Back sits under the band, at {with}");

    let ctx = context();
    let (from, to) = (pos2(100.0, 14.0), pos2(140.0, 20.0));
    frame(&ctx, 28.0, 700.0, vec![]);
    // A band that only drags starts the drag on the press itself, or on the first move after it.
    let pressed = frame(&ctx, 28.0, 700.0, vec![egui::Event::PointerMoved(from), click(from, true)]).0;
    let moved = frame(&ctx, 28.0, 700.0, vec![egui::Event::PointerMoved(to)]).0;
    assert_eq!(pressed.or(moved), Some(FileNavAction::DragWindow));
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    vec![event(true), event(false)]
}

/// Tab into the column, press the keys one frame each, then Enter: what the focused row does.
fn walk_then_enter(band: f32, keys: &[egui::Key]) -> Option<FileNavAction<&'static str>> {
    walk_with(band, &[], keys)
}

/// Tab from the top of a column that fits: the labels of the rows that get the focus, in turn.
fn tab_stops(presses: usize) -> Vec<String> {
    let ctx = context();
    let frame = |events| frame(&ctx, 0.0, 700.0, events);
    frame(vec![]);
    let mut stops = Vec::new();
    for _ in 0..presses {
        let (_, full) = frame(key(egui::Key::Tab));
        let update = full.platform_output.accesskit_update.as_ref().expect("accesskit is on");
        if let Some(label) = update.nodes.iter().find(|(id, _)| *id == update.focus).and_then(|(_, n)| n.label()) {
            stops.push(label.to_string());
        }
    }
    stops
}

#[test]
fn tab_meets_the_foot_in_the_order_it_is_drawn() {
    let stops = tab_stops(10);
    let at = |label: &str| stops.iter().position(|s| s == label).unwrap_or(usize::MAX);
    let community = "A community with a name far too long for the column";
    assert!(at("Print") < at("Options") && at("Options") < at(community) && at(community) < at("About"), "{stops:?}");
}

fn walk_with(band: f32, disabled: &[&'static str], keys: &[egui::Key]) -> Option<FileNavAction<&'static str>> {
    let ctx = context();
    let frame = |events| frame_with(&ctx, band, 700.0, disabled, events);
    frame(vec![]);
    frame(key(egui::Key::Tab));
    // The column keeps the arrow keys from the frame after a row has the focus.
    frame(vec![]);
    for k in keys {
        frame(key(*k));
    }
    frame(key(egui::Key::Enter)).0
}

#[test]
fn the_arrow_keys_walk_the_entries_and_enter_chooses() {
    use egui::Key::{ArrowDown as Down, ArrowUp as Up, End, Home};
    assert_eq!(walk_then_enter(0.0, &[]), Some(FileNavAction::Back), "Tab reaches Back first");
    assert_eq!(walk_then_enter(28.0, &[]), Some(FileNavAction::Back), "the title band is no stop on the way");
    assert_eq!(walk_then_enter(0.0, &[Down, Down]), Some(FileNavAction::Choose("new")));
    assert_eq!(walk_then_enter(0.0, &[Down, Down, Down, Down]), Some(FileNavAction::Choose("info")), "across the rule into the next group");
    assert_eq!(walk_then_enter(0.0, &[Down, Down, Down, Down, Down, Down]), Some(FileNavAction::Choose("options")), "and on into the foot");
    assert_eq!(walk_then_enter(0.0, &[Up]), Some(FileNavAction::Choose("about")), "up from the first wraps to the last");
    assert_eq!(walk_then_enter(0.0, &[Up, Down]), Some(FileNavAction::Back), "and down from the last wraps to the first");
    assert_eq!(walk_then_enter(0.0, &[End]), Some(FileNavAction::Choose("about")));
    assert_eq!(walk_then_enter(0.0, &[End, Up, Home]), Some(FileNavAction::Back));
}

#[test]
fn a_label_longer_than_the_column_stays_inside_it() {
    let (_, full) = frame(&context(), 0.0, 700.0, vec![]);
    let b = node(&full, "A community with a name far too long for the column").expect("the long entry").bounds().expect("bounds");
    assert!(b.x1 <= 200.0, "the row is as wide as the column, not as the label: {}", b.x1);
}

#[test]
fn a_column_too_short_for_its_entries_scrolls_instead_of_overlapping() {
    for height in [1.0, 40.0, 200.0] {
        assert_eq!(frame(&context(), 28.0, height, vec![]).0, None);
    }
    // 300 is too short for Back, five pages and three foot entries: the foot follows the pages.
    let (_, full) = frame(&context(), 0.0, 300.0, vec![]);
    let y = |label: &str| centre(&full, label).y;
    assert!(y("Options") > y("Print") + 20.0, "the foot is under the last page, not over it");
    assert!(y("About") > 300.0, "and the column scrolls to reach the rest");
}

#[test]
fn the_neutral_style_marks_the_page_with_the_accent() {
    let v = egui::Visuals::light();
    let (neutral, filled) = (FileNavStyle::from_visuals(&v), FileNavStyle::on_accent(v.selection.bg_fill));
    assert_eq!(neutral.mark, v.selection.bg_fill);
    assert_eq!(filled.fill, v.selection.bg_fill);
    assert_ne!(neutral.fill, filled.fill);
    assert_eq!(neutral.width, filled.width, "only the colours differ");
}

#[test]
fn a_disabled_entry_is_announced_as_unavailable_and_never_chosen() {
    let disabled = &["info", "options"];
    assert_eq!(press_with(disabled, "Info"), None, "a page");
    assert_eq!(press_with(disabled, "Options"), None, "an entry of the foot");
    assert_eq!(press_with(disabled, "Print"), Some(FileNavAction::Choose("print")), "its neighbour still answers");
    assert_eq!(press_with(disabled, "Back"), Some(FileNavAction::Back));

    let (_, full) = frame_with(&context(), 0.0, 700.0, disabled, vec![]);
    let unavailable = |label: &str| node(&full, label).expect("entry").is_disabled();
    assert!(unavailable("Info") && unavailable("Options"));
    assert!(!unavailable("Print") && !unavailable("Back") && !unavailable("About"));
    let info = node(&full, "Info").expect("Info");
    assert!(!info.supports_action(egui::accesskit::Action::Click) && !info.supports_action(egui::accesskit::Action::Focus));
    assert_eq!(info.toggled(), Some(egui::accesskit::Toggled::False), "still a page, only not one to choose now");
}

#[test]
fn the_keys_pass_over_disabled_entries() {
    use egui::Key::{ArrowDown as Down, ArrowUp as Up, End, Home};
    let disabled = &["new", "info", "about"];
    assert_eq!(walk_with(0.0, disabled, &[]), Some(FileNavAction::Back));
    assert_eq!(walk_with(0.0, disabled, &[Down, Down]), Some(FileNavAction::Choose("open")), "New is passed over");
    assert_eq!(walk_with(0.0, disabled, &[Down, Down, Down]), Some(FileNavAction::Choose("print")), "and Info across the rule");
    assert_eq!(walk_with(0.0, disabled, &[End]), Some(FileNavAction::Choose("community")), "End reaches the last entry that answers");
    assert_eq!(walk_with(0.0, disabled, &[Up]), Some(FileNavAction::Choose("community")), "and so does Up from the first");
    assert_eq!(walk_with(0.0, disabled, &[End, Down]), Some(FileNavAction::Back), "Down from it wraps to Back");
    assert_eq!(walk_with(0.0, disabled, &[End, Home]), Some(FileNavAction::Back));
}

#[test]
fn a_disabled_entry_is_painted_dimmed_and_does_not_light_up_under_the_pointer() {
    let s = style();
    let ctx = context();
    let colours = std::cell::RefCell::new(Vec::new());
    let run = |events| {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 700.0))), events, ..Default::default() };
        colours.borrow_mut().clear();
        let mut full = ctx.run_ui(input, |ui| {
            let record = |key: Option<&'static str>, _: &Painter, _: Rect, colour: Color32| colours.borrow_mut().push((key, colour));
            FileNav::new("Back", &[TOP, REST], Some("open"), &s, &record).foot(FOOT).disabled(&["info"]).show(ui);
        });
        full.textures_delta.clear();
        full
    };
    let first = run(vec![]);
    let colour = |key| colours.borrow().iter().find(|(k, _)| *k == Some(key)).map(|(_, c)| *c);
    assert_eq!(colour("info"), Some(s.disabled), "the icon of a disabled entry");
    assert_eq!(colour("print"), Some(s.text));
    assert_ne!(s.disabled, s.text);

    let lit = |full: &egui::FullOutput| full.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == s.hover));
    let (info, print) = (centre(&first, "Info"), centre(&first, "Print"));
    assert!(lit(&run(vec![egui::Event::PointerMoved(print)])), "an entry that answers lights up");
    assert!(!lit(&run(vec![egui::Event::PointerMoved(info)])), "a disabled one does not");
}
