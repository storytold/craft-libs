//! `EditorMock`: a document editor window composed from library components.

use super::commands::AppCommand;
use super::model::{MockState, Panel, Tool, ToolKey};
use super::panels::Panels;
use super::tools::Tools;
use craft_behavior::dock::PanelState;
use craft_ui::bars::{Bar, BarKind, StatusReadout, StatusValues};
use craft_ui::canvas::{PickStep, ViewCommands};
use craft_ui::contract::{Bind, BodyCx, Props as _, Remember as _, SaveKey, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use craft_ui::display::{Divider, Text};
use craft_ui::docking::{DockConfig, DockEvent, DockPanels as _};
use craft_ui::menu::{Menu, MenuBar, MenuItem, MenuTitle};
use craft_ui::patterns::dock::DockArea;
use craft_ui::patterns::form_dialogs::{
    DialogFields, HostedImageSize, HostedParamDialog, ImageSize, ImageSizeValues, Param, ParamDialog, ParamResult,
};
use craft_ui::rails::ToolRail;
use craft_ui::runtime::commands::{CommandRef, Commands as _, EditCommands};
use craft_ui::runtime::env::{Label, WindowKey};
use craft_ui::shell::{AppShell, AppShellOutput, ShellDialogsCx as _, Title, TitleBar};
use egui::Ui;

/// The parameters of Gaussian Blur.
const BLUR: &str = r#"{"radius":0.1..1000=1}"#;

/// The command key of the Gaussian Blur dialog.
const BLUR_KEY: &str = "gaussianBlur";

/// Where Gaussian Blur remembers its values.
const BLUR_MEMORY: SaveKey = SaveKey::new("craft_ui_gallery.gaussianBlur");

/// The mock editor: the shell with its title bar and menus, options bar, tool rail and status bar around a dock whose
/// canvas group holds the document tabs, the canvas with its rulers and pasteboard, and the Navigator and layer panels.
///
/// - Tool: the rail and the tool keys pick the current tool; Hand and Zoom run in the canvas's tool host.
/// - Menus: every row is an app command with its chord; the body takes them.
/// - Dialogs: Layer, Delete, Layer and Filter, Gaussian Blur open on the window's dialog host; Image, Image Size
///   shows its dialog over the document's size.
/// - Screen modes: the shell's own commands (F) and the reveal keys act inside the window's region.
#[derive(craft_ui::contract::Pattern)]
pub struct EditorMock<'a> {
    label: Label,
    state: &'a mut MockState,
}

impl<'a> EditorMock<'a> {
    /// A mock editor named `label` over `state`.
    pub fn new(label: impl Into<Label>, state: &'a mut MockState) -> Self {
        EditorMock { label: label.into(), state }
    }

    /// Shows the window in the room offered. Each instance keeps its screen mode, Tab toggle and dialogs apart.
    pub fn show(self, ui: &mut Ui) -> AppShellOutput {
        let EditorMock { label, state } = self;
        let window = WindowKey::new(format!("mock-{:x}-{}", ui.scope_id().value(), label.as_str()));
        AppShell::embedded(window)
            .title_bar_if(true, |ui, _, state: &mut MockState| title_bar(ui, state))
            .options_bar(true, |ui, _, state: &mut MockState| options_bar(ui, state))
            .tools(true, |ui, _, state: &mut MockState| tool_rail(ui, state))
            .status(true, |ui, _, state: &mut MockState| status_bar(ui, state))
            .body_label(label)
            .body(|ui, cx, state: &mut MockState| body(ui, cx, state))
            .show(ui, state)
    }
}

/// The dock, then the commands and dialog answers of this pass.
fn body(ui: &mut Ui, cx: &mut BodyCx<'_>, state: &mut MockState) {
    let mut tools = cx.keep::<Tools>().clone();
    let events = {
        let mut panels = Panels { document: &mut state.document, tool: state.tool, tools: &mut tools };
        let mut area = DockArea::new("Dock", Bind::bound(&mut state.workspace), &mut panels, DockConfig::default());
        if let Some(panel) = state.maximized {
            area = area.maximized(panel);
        }
        area.show(ui).events
    };
    for event in events {
        if let DockEvent::ToggleMaximize { panel, .. } = event {
            state.maximized = if state.maximized == Some(panel) { None } else { Some(panel) };
        }
    }

    while let Some((command, _)) = cx.take_command::<Tool>() {
        state.tool = command;
    }
    while let Some((key, _)) = cx.take_command::<ToolKey>() {
        match key {
            ToolKey::PickBrush => pick(state, Tool::Brush, PickStep::Last),
            ToolKey::CycleBrush => pick(state, Tool::Brush, PickStep::Next),
            ToolKey::SwapColors => state.colors.swap(),
            ToolKey::ResetColors => state.colors.reset(),
        }
    }
    let mut asked = Vec::new();
    while let Some((command, _)) = cx.take_command::<AppCommand>() {
        asked.push(command);
    }
    let mut dialogs = Vec::new();
    for command in asked {
        if let Some(panel) = command.panel() {
            let shown = state.workspace.layout.panel_state(&panel) != PanelState::Hidden;
            let panels = Panels { document: &mut state.document, tool: state.tool, tools: &mut tools };
            let _ = state.workspace.set_visible(panel, !shown, None, false, |panel| panels.permissions(panel));
            continue;
        }
        match command {
            AppCommand::NewLayer => state.document.add_layer(),
            AppCommand::Rulers => state.document.rulers = !state.document.rulers,
            AppCommand::ImageSize | AppCommand::DeleteLayer | AppCommand::GaussianBlur => dialogs.push(command),
            _ => {}
        }
    }
    *cx.keep::<Tools>() = tools;

    if let Some(host) = cx.dialogs() {
        host.with(|host| {
            for command in dialogs {
                match command {
                    AppCommand::GaussianBlur => {
                        host.open(HostedParamDialog::new(BLUR_KEY, DialogFields::new(), |fields| {
                            ParamDialog::new(BLUR_KEY, "Gaussian Blur", Param::parse_list(BLUR)).values(Bind::bound(fields)).remember(BLUR_MEMORY)
                        }));
                    }
                    AppCommand::DeleteLayer => {
                        let name = state.document.current_layer().map(|layer| layer.name.clone()).unwrap_or_default();
                        host.open(StandardPrompt::confirm_delete([name], 1));
                    }
                    AppCommand::ImageSize => {
                        let values = ImageSizeValues::new(state.document.pixel_size());
                        host.open(HostedImageSize::new(values, |values: Bind<'_, ImageSizeValues>| ImageSize::new(values)));
                    }
                    _ => {}
                }
            }
            if let Some(result) = host.take::<HostedParamDialog>().and_then(DialogResult::<ParamResult>::ok) {
                result.remember(ui);
                state.document.dirty = true;
            }
            if let Some(result) = host.take::<HostedImageSize>().and_then(DialogResult::ok) {
                state.document.resize(result.size);
            }
            let confirmed = host.take::<StandardPrompt>().and_then(DialogResult::ok).is_some_and(|answer| answer.choice == PromptChoice::Proceed);
            if confirmed {
                state.document.delete_current();
            }
        });
    }
}

/// Makes the tool a key picks in `tool`'s slot the current tool.
fn pick(state: &mut MockState, tool: Tool, step: PickStep) {
    if let Some(picked) = Tool::pick(tool.slot(), state.tool, &state.last_used, step) {
        state.tool = picked;
    }
}

fn title_bar(ui: &mut Ui, state: &MockState) {
    let document = &state.document;
    let title = if document.dirty { Title::unsaved(document.title.clone()) } else { Title::new(document.title.clone()) };
    TitleBar::new().show_titled(
        ui,
        title,
        |ui, _| {
            let titles = vec![
                MenuTitle::new("&File", |ui| {
                    for command in [AppCommand::New, AppCommand::Open, AppCommand::Save] {
                        MenuItem::command(command).enabled(false).show(ui);
                    }
                }),
                MenuTitle::new("&Edit", |ui| {
                    for command in [EditCommands::Undo, EditCommands::Redo] {
                        MenuItem::command(command).enabled(false).show(ui);
                    }
                    Divider::horizontal().show(ui);
                    for command in [EditCommands::Cut, EditCommands::Copy, EditCommands::Paste] {
                        MenuItem::command(command).enabled(false).show(ui);
                    }
                }),
                MenuTitle::new("&Image", |ui| {
                    MenuItem::command(AppCommand::ImageSize).show(ui);
                }),
                MenuTitle::new("&Layer", |ui| {
                    MenuItem::command(AppCommand::NewLayer).show(ui);
                    Menu::submenu_row(ui, "Delete", |ui, _| {
                        MenuItem::command(AppCommand::DeleteLayer).enabled(document.layers.len() > 1).show(ui);
                    });
                }),
                MenuTitle::new("Fil&ter", |ui| {
                    MenuItem::command(AppCommand::GaussianBlur).show(ui);
                }),
                MenuTitle::new("&View", |ui| {
                    for command in
                        [ViewCommands::FitOnScreen, ViewCommands::FillScreen, ViewCommands::ActualPixels, ViewCommands::ZoomIn, ViewCommands::ZoomOut]
                    {
                        MenuItem::command(command).show(ui);
                    }
                    Divider::horizontal().show(ui);
                    MenuItem::command(AppCommand::Rulers).check(document.rulers).show(ui);
                }),
                MenuTitle::new("&Window", |ui| {
                    for panel in Panel::ALL {
                        let shown = state.workspace.layout.panel_state(&panel) != PanelState::Hidden;
                        MenuItem::command(AppCommand::showing(panel)).check(shown).enabled(!panel.protected()).show(ui);
                    }
                }),
            ];
            MenuBar::new("Main menu", titles).show(ui);
        },
        |_, _| {},
    );
}

fn options_bar(ui: &mut Ui, state: &MockState) {
    let colors = state.colors;
    let _ = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        Text::new(format!("Foreground #{:06X}   Background #{:06X}", colors.foreground, colors.background)).show(ui);
    });
}

fn tool_rail(ui: &mut Ui, state: &mut MockState) {
    let slots = Tool::slots();
    let current = state.tool;
    let last_used = state.last_used.clone();
    let output = ToolRail::new("Tools", &slots, &current, &last_used).show_rail(ui, |_, _| {});
    if let Some(tool) = output.select {
        state.tool = tool;
    }
    if let Some(last_used) = output.last_used {
        state.last_used = last_used;
    }
}

fn status_bar(ui: &mut Ui, state: &mut MockState) {
    let MockState { document, tool, status_item, .. } = state;
    let mut values = StatusValues::default();
    values.document_dimensions = Some(document.pixel_size());
    values.current_tool = Some(CommandRef::from(*tool));
    values.layer_count = Some(document.layers.len());
    let mut chosen = None;
    let _ = Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
        chosen = StatusReadout::new(Bind::bound(&mut document.view), document.size, *status_item, &values).show(ui).item;
    });
    if let Some(item) = chosen {
        *status_item = item;
    }
}
