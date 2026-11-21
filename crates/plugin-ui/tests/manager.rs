//! The plug-in manager window, the permission prompt window and panel windows. egui_kittest drives a
//! headless context with scripted clicks. The tests assert the `Act`s the frame returns and the
//! words on screen, because the app carries the acts out.

use craft_plugin_abi::Permission;
use craft_plugin_host::contrib::hook::HookRegistration;
use craft_plugin_host::contrib::surface::SurfaceRegistration;
use craft_plugin_host::lifecycle::{GrantRequest, PluginInfo, PluginState, UpdateRisk};
use craft_plugin_ui::manager::{Act, BrokenRow, ManagerView, PluginRow, UiState, draw, state_text};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

struct App {
    ui: UiState,
    view: ManagerView,
    acts: Vec<Act>,
    /// How many times `draw` asked for the rows.
    asked: usize,
}

fn info(id: &str, enabled: bool, state: PluginState) -> PluginInfo {
    PluginInfo {
        id: id.into(),
        version: "1.2.0".into(),
        state,
        enabled,
        unsupported_kinds: Vec::new(),
        commands: vec![format!("plugin.{id}.go")],
        hooks: HookRegistration::default(),
        surfaces: SurfaceRegistration::default(),
        legacy: None,
        update_pending: false,
        grants: vec!["clock".into(), "doc.write".into()],
    }
}

fn row(info: PluginInfo) -> PluginRow {
    PluginRow { info, progress: None, panels: Vec::new() }
}

fn view(rows: Vec<PluginRow>) -> ManagerView {
    ManagerView { app: "TestCraft".into(), rows, broken: Vec::new() }
}

fn harness(ui: UiState, view: ManagerView) -> Harness<'static, App> {
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut App| {
            let ctx = ui.ctx().clone();
            let rows = app.view.clone();
            let mut asked = 0;
            let acts = draw(&ctx, &mut app.ui, || {
                asked += 1;
                rows
            });
            app.asked += asked;
            app.acts.extend(acts);
        },
        App { ui, view, acts: Vec::new(), asked: 0 },
    );
    h.run();
    h
}

fn open() -> UiState {
    let mut ui = UiState::default();
    ui.manager_open = true;
    ui
}

fn take(h: &mut Harness<'static, App>) -> Vec<Act> {
    std::mem::take(&mut h.state_mut().acts)
}

#[test]
fn a_closed_manager_draws_no_window_and_asks_nothing_of_the_view() {
    let h = harness(UiState::default(), view(Vec::new()));
    assert_eq!(h.state().asked, 0, "the app builds the rows only when the window is open");
    assert!(h.query_by_label("No plug-ins are installed.").is_none());
    let open = harness(open(), view(Vec::new()));
    assert!(open.state().asked > 0);
}

#[test]
fn an_empty_manager_says_so() {
    let h = harness(open(), view(Vec::new()));
    assert!(h.query_by_label("No plug-ins are installed.").is_some());
}

#[test]
fn a_row_shows_the_state_in_words_and_disable_returns_an_act() {
    let mut h = harness(open(), view(vec![row(info("org.x", true, PluginState::Active { generation: 3 }))]));
    assert!(h.query_by_label("State: active (generation 3)").is_some());
    h.get_by_label("Disable").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Disable("org.x".into())]);
}

#[test]
fn a_disabled_row_offers_enable() {
    let mut h = harness(open(), view(vec![row(info("org.x", false, PluginState::Disabled))]));
    h.get_by_label("Enable").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Enable("org.x".into())]);
}

#[test]
fn remove_asks_first_and_only_the_confirmation_returns_an_act() {
    let mut h = harness(open(), view(vec![row(info("org.x", true, PluginState::Active { generation: 1 }))]));
    h.get_by_label("Remove").click();
    h.run();
    assert_eq!(take(&mut h), vec![], "the first click only asks");
    assert!(h.query_by_label("Remove it and its saved data?").is_some());
    h.get_by_label("Keep").click();
    h.run();
    assert!(h.query_by_label("Remove it and its saved data?").is_none(), "Keep closes the question");
    h.get_by_label("Remove").click();
    h.run();
    h.get_by_label("Yes, remove").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Remove("org.x".into())]);
}

#[test]
fn a_revoke_click_returns_the_abi_permission() {
    let mut h = harness(open(), view(vec![row(info("org.x", true, PluginState::Active { generation: 1 }))]));
    h.get_by_label("Permissions (2)").click();
    h.run();
    // `doc.write` is high level and sorts first.
    h.get_all_by_label("Revoke").next().expect("a revoke button").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Revoke("org.x".into(), "doc.write".parse::<Permission>().expect("parses"))]);
}

#[test]
fn the_unsupported_line_names_the_app() {
    let mut partly = info("org.x", true, PluginState::Active { generation: 1 });
    partly.unsupported_kinds = vec!["audio-effect".into()];
    let h = harness(open(), view(vec![row(partly)]));
    assert!(h.query_by_label("Partly supported. This version of TestCraft cannot run: audio-effect.").is_some());
}

#[test]
fn a_plugin_that_failed_to_restore_can_be_removed() {
    let mut v = view(Vec::new());
    v.broken = vec![BrokenRow { id: "org.broken".into(), state: "faulted: bad wasm".into() }];
    let mut h = harness(open(), v);
    assert!(h.query_by_label("State: faulted: bad wasm").is_some());
    h.get_by_label("Remove").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Remove("org.broken".into())]);
}

#[test]
fn install_takes_the_typed_path_and_browse_fills_it_in() {
    let mut ui = open();
    ui.install_path = "/tmp/a.wasm".into();
    let mut h = harness(ui, view(Vec::new()));
    assert!(h.query_by_label("Browse...").is_none(), "no picker, no browse button");
    h.get_by_label("Install").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Install("/tmp/a.wasm".into())]);

    let mut ui = open();
    ui.set_picker(Box::new(|| Some(std::path::PathBuf::from("/tmp/picked.wasm"))));
    let mut h = harness(ui, view(Vec::new()));
    h.get_by_label("Browse...").click();
    h.run();
    assert_eq!(h.state().ui.install_path, "/tmp/picked.wasm");
    h.get_by_label("Install").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Install("/tmp/picked.wasm".into())]);
}

#[test]
fn the_message_line_shows_the_last_thing_that_happened() {
    let mut ui = open();
    ui.message = Some("installed: Loaded".into());
    let h = harness(ui, view(Vec::new()));
    assert!(h.query_by_label("installed: Loaded").is_some());
}

fn request(id: &str) -> GrantRequest {
    let p = |name: &str| name.parse::<Permission>().expect("parses");
    GrantRequest {
        plugin_id: id.into(),
        version: "2.0.0".into(),
        publisher: Some("Acme".into()),
        required: vec![p("doc.write")],
        new_required: vec![p("doc.write")],
        optional: vec![p("clock")],
        risk: UpdateRisk::None,
    }
}

#[test]
fn a_prompt_says_nothing_has_run_and_cancel_answers_no() {
    let mut ui = UiState::default();
    ui.queue_prompt(&request("org.x"));
    assert_eq!(ui.waiting(), vec!["org.x".to_string()]);
    assert_eq!(ui.dialog_label().as_deref(), Some("Plug-in permissions"));
    let mut h = harness(ui, view(Vec::new()));
    assert!(h.query_by_label("org.x 2.0.0").is_some());
    assert!(h.query_by_label("Publisher: Acme (not verified)").is_some());
    assert!(h.query_by_label("Nothing of this plug-in has run. It runs after you answer.").is_some());
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(take(&mut h), vec![Act::Answer { id: "org.x".into(), accept: false, optional: Vec::new() }]);
}

#[test]
fn a_risky_update_prompt_warns_that_the_old_grants_are_gone() {
    let mut req = request("org.x");
    req.risk = UpdateRisk::Publisher { installed: Some("Alice".into()) };
    let mut ui = UiState::default();
    ui.queue_prompt(&req);
    let h = harness(ui, view(Vec::new()));
    let warning = "WARNING: this replaces the installed plug-in, and its old permissions are dropped. Reason: different publisher (installed code is from \"Alice\").";
    assert!(h.query_by_label(warning).is_some(), "the warning line is missing");
    // A clean prompt has no such line.
    let mut ui = UiState::default();
    ui.queue_prompt(&request("org.x"));
    let h = harness(ui, view(Vec::new()));
    assert!(h.query_by_label_contains("WARNING").is_none());
}

#[test]
fn a_prompt_confirms_with_the_optional_permissions_the_user_allowed() {
    let mut ui = UiState::default();
    ui.queue_prompt(&request("org.x"));
    let mut h = harness(ui, view(Vec::new()));
    for allow in h.get_all_by_label("Allow").collect::<Vec<_>>() {
        allow.click();
    }
    h.run();
    h.get_by_label("Install").click();
    h.run();
    let acts = take(&mut h);
    let optional = vec!["clock".parse::<Permission>().expect("parses")];
    assert_eq!(acts, vec![Act::Answer { id: "org.x".into(), accept: true, optional }]);
}

#[test]
fn a_newer_prompt_replaces_the_older_one_of_the_same_plugin_and_drop_removes_it() {
    let mut ui = UiState::default();
    ui.queue_prompt(&request("org.x"));
    ui.queue_prompt(&request("org.y"));
    ui.queue_prompt(&request("org.x"));
    assert_eq!(ui.waiting(), vec!["org.y".to_string(), "org.x".to_string()]);
    ui.drop_prompt("org.y");
    assert_eq!(ui.waiting(), vec!["org.x".to_string()]);
    ui.drop_prompt("org.x");
    assert_eq!(ui.dialog_label(), None);
}

#[test]
fn the_dialog_label_names_the_manager_when_no_prompt_waits() {
    assert_eq!(open().dialog_label().as_deref(), Some("Plug-ins"));
    assert_eq!(UiState::default().dialog_label(), None);
}

#[test]
fn state_words_are_stable() {
    assert_eq!(state_text(&PluginState::AwaitingGrant), "awaiting permission");
    assert_eq!(state_text(&PluginState::Disabled), "disabled");
    assert_eq!(state_text(&PluginState::Faulted { reason: "x".into() }), "faulted: x");
}

fn panel_answer() -> serde_json::Value {
    json!({"tree": {"type": "column", "id": "root", "children": [{"type": "button", "id": "go", "label": "Go"}]}})
}

#[test]
fn a_panel_window_shows_the_plugins_tree_and_a_click_returns_an_event_act() {
    let mut ui = UiState::default();
    ui.open_panel_window("org.x", "panel", "Tools", Ok(panel_answer()));
    assert!(ui.panel_open("org.x", "panel"));
    let mut h = harness(ui, view(Vec::new()));
    h.get_by_label("Go").click();
    h.run();
    assert_eq!(
        take(&mut h),
        vec![Act::Panel { plugin: "org.x".into(), contribution: "panel".into(), event: json!({"event": "action", "widget": "go", "value": null}) }]
    );
}

#[test]
fn a_bad_panel_answer_is_shown_and_a_good_one_clears_it() {
    let mut ui = UiState::default();
    ui.open_panel_window("org.x", "panel", "Tools", Err("the plug-in timed out".into()));
    let mut h = harness(ui, view(Vec::new()));
    assert!(h.query_by_label("Panel error: the plug-in timed out").is_some());
    h.state_mut().ui.apply_panel_answer("org.x", "panel", Ok(json!({"tree": {"type": "nope"}})));
    h.run();
    assert!(h.query_by_label_contains("the plug-in sent a bad panel").is_some());
    h.state_mut().ui.apply_panel_answer("org.x", "panel", Ok(panel_answer()));
    h.run();
    assert!(h.query_by_label_contains("Panel error").is_none());
    assert!(h.query_by_label("Go").is_some());
}

#[test]
fn closing_a_panel_window_returns_a_close_act_and_close_removes_it() {
    let mut ui = UiState::default();
    ui.open_panel_window("org.x", "panel", "Tools", Ok(panel_answer()));
    assert!(ui.panel_open("org.x", "panel"));
    ui.close_panel_window("org.x", "panel");
    assert!(!ui.panel_open("org.x", "panel"));
}
