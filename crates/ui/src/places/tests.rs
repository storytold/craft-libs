use super::*;

const LABELS: PlacesLabels<'static> = PlacesLabels { places: "Places", add_place: "Add a place", browse: "Browse…", loading: "Loading…" };

const PLACES: &[Place<'static>] = &[
    Place { kind: PlaceKind::Recent, label: "Recent", detail: None },
    Place { kind: PlaceKind::Account, label: "Work", detail: Some("you@company.example") },
    Place { kind: PlaceKind::Account, label: "Personal", detail: Some("you@home.example") },
    Place { kind: PlaceKind::Local, label: "This computer", detail: None },
];

const ENTRIES: &[PlaceEntry<'static>] = &[
    PlaceEntry { name: "2026", folder: true, detail: "folder", attention: false, remote: true },
    PlaceEntry { name: "Q4 proposal.docx", folder: false, detail: "yesterday", attention: false, remote: true },
    PlaceEntry { name: "Meeting notes.docx", folder: false, detail: "Anna is editing", attention: true, remote: true },
];

#[derive(Clone, Copy)]
struct Setup {
    content: PlacesContent<'static>,
    view: PlacesView,
    crumbs: &'static [&'static str],
    add_place: bool,
    browse: bool,
    size: egui::Vec2,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            content: PlacesContent::Entries(ENTRIES),
            view: PlacesView::List,
            crumbs: &[],
            add_place: false,
            browse: true,
            size: vec2(760.0, 420.0),
        }
    }
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx
}

fn input(size: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() }
}

fn frame(ctx: &egui::Context, setup: Setup, events: Vec<egui::Event>) -> (Option<PlacesAction>, egui::FullOutput) {
    let style = PlacesStyle::from_visuals(&egui::Visuals::light());
    let mut action = None;
    let mut full = ctx.run_ui(input(setup.size, events), |ui| {
        let panel = PlacesPanel::new(PLACES, Some(1), setup.content, &style, LABELS)
            .crumbs(setup.crumbs)
            .view(setup.view)
            .add_place(setup.add_place)
            .browse(setup.browse);
        action = panel.show(ui).action;
    });
    full.textures_delta.clear();
    (action, full)
}

/// The accessible node with this name. egui carries a label's text as its value.
fn node<'a>(full: &'a egui::FullOutput, label: &str) -> Option<&'a egui::accesskit::Node> {
    let update = full.platform_output.accesskit_update.as_ref().expect("accesskit is on");
    update.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(label) || n.value() == Some(label))
}

/// The centre of the accessible node with this label.
fn centre(full: &egui::FullOutput, label: &str) -> Pos2 {
    let b = node(full, label).expect("node with the label").bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

fn click(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// Clicks the node with this label and returns what the panel reported.
fn press(setup: Setup, label: &str) -> Option<PlacesAction> {
    let ctx = context();
    let (_, full) = frame(&ctx, setup, vec![]);
    let p = centre(&full, label);
    frame(&ctx, setup, vec![egui::Event::PointerMoved(p), click(p, true)]);
    frame(&ctx, setup, vec![click(p, false)]).0
}

#[test]
fn choosing_a_place_reports_its_index() {
    assert_eq!(press(Setup::default(), "Recent"), Some(PlacesAction::Select(0)));
    assert_eq!(press(Setup::default(), "Personal, you@home.example"), Some(PlacesAction::Select(2)), "an account is named with its address");
    assert_eq!(press(Setup::default(), "This computer"), Some(PlacesAction::Select(3)));
}

#[test]
fn the_chosen_place_is_the_only_one_announced_as_selected() {
    let (_, full) = frame(&context(), Setup::default(), vec![]);
    let toggled = |label: &str| node(&full, label).expect("place").toggled();
    assert_eq!(toggled("Work, you@company.example"), Some(egui::accesskit::Toggled::True));
    assert_eq!(toggled("Recent"), Some(egui::accesskit::Toggled::False));
}

#[test]
fn choosing_an_entry_reports_its_index_in_both_views() {
    for view in [PlacesView::List, PlacesView::Thumbnails] {
        let setup = Setup { view, ..Setup::default() };
        assert_eq!(press(setup, "2026, folder"), Some(PlacesAction::Activate(0)), "{view:?}: a folder");
        assert_eq!(press(setup, "Meeting notes.docx, Anna is editing"), Some(PlacesAction::Activate(2)), "{view:?}: a file");
    }
}

#[test]
fn thumbnails_wrap_to_the_width_and_keep_their_indices() {
    // Room for one tile beside the places, so the third entry sits on the third row.
    let setup = Setup { view: PlacesView::Thumbnails, size: vec2(400.0, 600.0), ..Setup::default() };
    let (_, full) = frame(&context(), setup, vec![]);
    assert!(centre(&full, "Meeting notes.docx, Anna is editing").y > centre(&full, "Q4 proposal.docx, yesterday").y + THUMBNAIL_HEIGHT);
    assert_eq!(press(setup, "Meeting notes.docx, Anna is editing"), Some(PlacesAction::Activate(2)));
}

#[test]
fn add_a_place_is_absent_unless_the_application_offers_remote_storage() {
    let (_, full) = frame(&context(), Setup::default(), vec![]);
    assert!(node(&full, "Add a place").is_none(), "off by default");
    assert_eq!(press(Setup { add_place: true, ..Setup::default() }, "Add a place"), Some(PlacesAction::AddPlace));
}

#[test]
fn browse_leads_to_the_system_dialog_and_can_be_left_to_the_host() {
    assert_eq!(press(Setup::default(), "Browse…"), Some(PlacesAction::Browse));
    let (_, full) = frame(&context(), Setup { browse: false, ..Setup::default() }, vec![]);
    assert!(node(&full, "Browse…").is_none());
}

#[test]
fn every_crumb_but_the_last_can_be_chosen() {
    let setup = Setup { crumbs: &["Work", "Documents", "Proposals"], ..Setup::default() };
    assert_eq!(press(setup, "Work"), Some(PlacesAction::Crumb(0)));
    assert_eq!(press(setup, "Documents"), Some(PlacesAction::Crumb(1)));
    assert_eq!(press(setup, "Proposals"), None, "the last step is where the user already is");
}

#[test]
fn an_entry_is_announced_with_its_detail() {
    let (_, full) = frame(&context(), Setup::default(), vec![]);
    assert!(node(&full, "Meeting notes.docx, Anna is editing").is_some(), "the status is not only for people who see the row");
    assert_eq!(spoken(&PlaceEntry { name: "Notes.docx", folder: false, detail: "", attention: false, remote: false }), "Notes.docx");
}

#[test]
fn a_path_too_long_for_the_width_keeps_its_first_and_last_steps() {
    let all = |n: usize| (0..n).map(Some).collect::<Vec<_>>();
    // Six steps of 50 with 10 between: 350 wide in all.
    let widths = [50.0; 6];
    assert_eq!(fit_crumbs(&widths, 10.0, 10.0, 350.0), all(6), "it fits");
    assert_eq!(fit_crumbs(&widths, 10.0, 10.0, 349.0), [Some(0), None, Some(2), Some(3), Some(4), Some(5)], "one step makes way for the dots");
    assert_eq!(fit_crumbs(&widths, 10.0, 10.0, 200.0), [Some(0), None, Some(4), Some(5)]);
    assert_eq!(fit_crumbs(&widths, 10.0, 10.0, 0.0), [Some(0), None, Some(5)], "the last step is never dropped");
    assert_eq!(fit_crumbs(&[50.0, 50.0], 10.0, 10.0, 0.0), all(2), "two steps have nothing between them to leave out");
    assert_eq!(fit_crumbs(&[], 10.0, 10.0, 0.0), all(0));

    const DEEP: &[&str] = &["This computer", "Users", "ada", "Documents", "Clients", "Northwind", "Proposals", "2026", "Drafts"];
    let setup = Setup { crumbs: DEEP, size: vec2(620.0, 400.0), ..Setup::default() };
    let (_, full) = frame(&context(), setup, vec![]);
    assert!(node(&full, "This computer").is_some() && node(&full, "Drafts").is_some(), "both ends of the path stay");
    assert!(node(&full, "Clients").is_none(), "the middle makes way");
    let right = node(&full, "Drafts").and_then(|n| n.bounds()).map(|b| b.x1).unwrap_or(f64::MAX);
    assert!(right <= 620.0, "the path stays inside the panel, at {right}");
    assert_eq!(press(setup, "2026"), Some(PlacesAction::Crumb(7)), "a step keeps its own index when others are left out");
}

#[test]
fn loading_and_a_message_stand_in_for_the_entries() {
    for (content, shown) in [(PlacesContent::Loading, "Loading…"), (PlacesContent::Message("This folder is empty."), "This folder is empty.")] {
        let (action, full) = frame(&context(), Setup { content, ..Setup::default() }, vec![]);
        assert!(node(&full, shown).is_some(), "{shown} is shown");
        assert!(node(&full, "Q4 proposal.docx, yesterday").is_none(), "no entries beside {shown}");
        assert_eq!(action, None);
    }
}

const NO_ACCESS: PlacesNotice<'static> = PlacesNotice {
    icon: PlacesIcon::Locked,
    title: "No access to this folder",
    body: "The system has not allowed the app to read it. Browse opens a document from it.",
    action: Some("Open another way"),
};

#[test]
fn a_notice_says_why_and_offers_its_button() {
    let setup = Setup { content: PlacesContent::Notice(NO_ACCESS), ..Setup::default() };
    let (action, full) = frame(&context(), setup, vec![]);
    assert_eq!(action, None);
    assert!(node(&full, NO_ACCESS.title).is_some() && node(&full, NO_ACCESS.body).is_some());
    assert!(node(&full, "Q4 proposal.docx, yesterday").is_none(), "a notice stands in for the entries");
    assert_eq!(press(setup, "Open another way"), Some(PlacesAction::Notice));

    let quiet = PlacesNotice { body: "", action: None, ..NO_ACCESS };
    let (_, full) = frame(&context(), Setup { content: PlacesContent::Notice(quiet), ..Setup::default() }, vec![]);
    assert!(node(&full, NO_ACCESS.title).is_some() && node(&full, "Open another way").is_none(), "no button without something to do");
}

#[test]
fn running_text_keeps_to_a_readable_width() {
    const LONG: &str = "A sentence long enough to run right across a very wide window if nothing held it back, which makes it hard to find the start of the next line.";
    for content in [PlacesContent::Message(LONG), PlacesContent::Notice(PlacesNotice { body: LONG, ..NO_ACCESS })] {
        let (_, full) = frame(&context(), Setup { content, size: vec2(2400.0, 600.0), ..Setup::default() }, vec![]);
        let b = node(&full, LONG).expect("the text").bounds().expect("bounds");
        assert!(b.x1 - b.x0 <= f64::from(READING_WIDTH) + 1.0, "{} wide", b.x1 - b.x0);
    }
}

#[test]
fn nothing_is_reported_without_input() {
    let ctx = context();
    for _ in 0..3 {
        assert_eq!(frame(&ctx, Setup::default(), vec![]).0, None);
    }
}

#[test]
fn the_keyboard_reaches_every_row_and_enter_chooses_it() {
    let ctx = context();
    let setup = Setup { add_place: true, crumbs: &["Work", "Documents"], ..Setup::default() };
    let key = |key: egui::Key, pressed: bool| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&ctx, setup, vec![]);
    // Tab order follows the layout: four places, Add a place, Browse, the first crumb, the entries.
    let mut reported = Vec::new();
    for _ in 0..9 {
        frame(&ctx, setup, vec![key(egui::Key::Tab, true), key(egui::Key::Tab, false)]);
        reported.extend(frame(&ctx, setup, vec![key(egui::Key::Enter, true)]).0);
        frame(&ctx, setup, vec![key(egui::Key::Enter, false)]);
    }
    assert!(reported.contains(&PlacesAction::Select(0)) && reported.contains(&PlacesAction::Select(3)), "{reported:?}");
    assert!(reported.contains(&PlacesAction::AddPlace) && reported.contains(&PlacesAction::Browse), "{reported:?}");
    assert!(reported.contains(&PlacesAction::Crumb(0)) && reported.contains(&PlacesAction::Activate(1)), "{reported:?}");
}

#[test]
fn the_header_sits_above_the_places() {
    let style = PlacesStyle::from_visuals(&egui::Visuals::dark());
    let mut full = context().run_ui(input(vec2(760.0, 420.0), vec![]), |ui| {
        PlacesPanel::new(PLACES, None, PlacesContent::Entries(ENTRIES), &style, LABELS)
            .header(|ui| {
                let _ = ui.button("New file");
            })
            .show(ui);
    });
    full.textures_delta.clear();
    assert!(centre(&full, "New file").y < centre(&full, "Recent").y);
}

#[test]
fn cramped_empty_and_unbounded_spaces_are_survived() {
    for size in [vec2(1.0, 1.0), vec2(40.0, 30.0), vec2(200.0, 2000.0), vec2(3000.0, 60.0)] {
        for view in [PlacesView::List, PlacesView::Thumbnails] {
            for content in [PlacesContent::Entries(ENTRIES), PlacesContent::Entries(&[]), PlacesContent::Loading] {
                let setup = Setup { content, view, size, add_place: true, crumbs: &["Work", "Documents"], ..Setup::default() };
                assert_eq!(frame(&context(), setup, vec![]).0, None);
            }
        }
    }
    // A scroll area offers its viewport, and the panel fills it.
    let style = PlacesStyle::from_visuals(&egui::Visuals::light());
    let mut size = egui::Vec2::ZERO;
    let mut full = context().run_ui(input(vec2(500.0, 400.0), vec![]), |ui| {
        ScrollArea::both().show(ui, |ui| {
            size = PlacesPanel::new(&[], None, PlacesContent::Entries(&[]), &style, LABELS).show(ui).response.rect.size();
        });
    });
    full.textures_delta.clear();
    assert_eq!(size, vec2(500.0, 400.0));
    assert_eq!((finite(f32::INFINITY, 640.0), finite(f32::NAN, 360.0), finite(-4.0, 360.0), finite(12.0, 360.0)), (640.0, 360.0, 0.0, 12.0));
}

#[test]
fn the_menu_lists_the_same_places() {
    let run = |ctx: &egui::Context, add_place: bool, events: Vec<egui::Event>| {
        let mut action = None;
        let mut full = ctx.run_ui(input(vec2(400.0, 300.0), events), |ui| action = menu(ui, PLACES, LABELS, add_place));
        full.textures_delta.clear();
        (action, full)
    };
    let press = |add_place: bool, label: &str| {
        let ctx = context();
        let p = centre(&run(&ctx, add_place, vec![]).1, label);
        run(&ctx, add_place, vec![egui::Event::PointerMoved(p), click(p, true)]);
        run(&ctx, add_place, vec![click(p, false)]).0
    };
    assert_eq!(press(false, "This computer"), Some(PlacesAction::Select(3)));
    assert_eq!(press(true, "Add a place"), Some(PlacesAction::AddPlace));
    assert!(node(&run(&context(), false, vec![]).1, "Add a place").is_none());
}

#[test]
fn colours_come_from_the_visuals_and_differ_between_themes() {
    let (light, dark) = (PlacesStyle::from_visuals(&egui::Visuals::light()), PlacesStyle::from_visuals(&egui::Visuals::dark()));
    assert_ne!(light.fill, dark.fill);
    for style in [light, dark] {
        assert_ne!(style.selected, style.fill, "the chosen place stands out");
        assert_ne!(style.thumbnail, style.fill);
    }
    assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
    assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
}
