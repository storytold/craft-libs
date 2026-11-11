//! The detail pane: the selected command's name and place, its current shortcuts with Remove, the New Shortcut
//! field (recording, or typed entry), Context, the holders of the new chord, and Assign, Use Default and Clear.

use craft_keymap::command::CommandRef;
use craft_keymap::keymap::Binding;
use craft_keymap::scope::Scope;
use craft_ui::buttons::{Button, IconButton, Link};
use craft_ui::contract::{Bind, BodyCx, Entry, Props as _, Scroll, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::style::TypeRole;
use craft_ui::text::TextField;
use egui::Ui;

use crate::shortcuts::recorder::ChordRecorder;
use crate::shortcuts::state::{DialogState, Env, Target};
use crate::shortcuts::view::recording_text;
use crate::shortcuts::words::around_chord;

const PATH_SEPARATOR: &str = " \u{203a} ";

/// The detail pane for one pass.
pub(super) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
        let selected = state.selected.clone().filter(|command| env.keymap.bindable(command).is_some());
        match selected {
            Some(command) => of_command(ui, now, env, state, &command),
            None => {
                Text::new(env.text.words().select_a_command_to_see_and.clone()).shade(Shade::Muted).show(ui);
            }
        }
    });
}

fn scope_name(env: &Env<'_>, scope: &Scope) -> String {
    if scope.is_app() { env.text.words().application.as_str().to_owned() } else { env.text.segment(scope.as_str()).into_owned() }
}

/// A binding as text, with its scope when that is not the command's home scope.
fn binding_text(env: &Env<'_>, binding: &Binding, home: &Scope) -> String {
    let chord = env.text.trigger(&binding.trigger, env.keymap.platform());
    match &binding.scope {
        Some(scope) if scope != home => env.text.words().chord_scope.fill(&[("chord", &chord), ("scope", &scope_name(env, scope))]),
        _ => chord,
    }
}

fn label_of(env: &Env<'_>, command: &CommandRef) -> String {
    env.keymap.bindable(command).map_or_else(|| command.id.clone(), |b| env.text.command(b))
}

fn of_command(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState, command: &CommandRef) {
    let Some(b) = env.keymap.bindable(command).cloned() else { return };
    let words = env.text.words();
    let editable = env.options.editable;
    let label = env.text.command(&b);
    let home = b.scope.clone();

    Text::new(label.clone()).role(TypeRole::BodyStrong).show(ui);
    let path: Vec<String> = b.paths.first().map(|p| p.iter().filter(|s| !s.is_empty()).cloned().collect()).unwrap_or_default();
    let place = if path.is_empty() {
        b.command.id.clone()
    } else {
        let names: Vec<String> = path.iter().map(|s| env.text.segment(s).into_owned()).collect();
        format!("{} \u{b7} {}", names.join(PATH_SEPARATOR), b.command.id)
    };
    Text::new(place).shade(Shade::Muted).show(ui);
    if let Some(hint) = env.text.hint(&b) {
        Text::new(hint).shade(Shade::Muted).show(ui);
    }

    Text::new(words.current_shortcuts.clone()).show(ui);
    let bindings = env.keymap.bindings(command);
    let defaults = env.keymap.default_bindings(command);
    if bindings.is_empty() {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
    }
    let mut remove = None;
    for (n, binding) in bindings.iter().enumerate() {
        let text = binding_text(env, binding, &home);
        let shown = if defaults.contains(binding) { words.chord_default.fill(&[("chord", &text)]) } else { text.clone() };
        let chord = env.text.trigger(&binding.trigger, env.keymap.platform());
        Row::new().show(ui, |ui, _| {
            Text::new(around_chord(env.text.rtl(), &shown, &chord)).show(ui);
            if editable {
                let name = words.remove_chord_from_label.fill(&[("chord", &text), ("label", &label)]);
                let button = IconButton::new(IconSpec::svg(craft_behavior::IconId::new("close")))
                    .id_salt(("remove", n))
                    .label(name.clone())
                    .tooltip(name)
                    .show(ui);
                if clicked(&button) {
                    remove = Some(binding.clone());
                }
            }
        });
    }
    if let Some(binding) = remove {
        state.remove(env, command, &binding);
    }
    if !editable {
        defaults_line(ui, env, &defaults, &home);
        return;
    }
    new_shortcut(ui, now, env, state, command, &home);
    context(ui, env, state, command);
    holders(ui, env, state);
    Row::new().show(ui, |ui, _| {
        if clicked(&Button::new(words.assign.clone()).id_salt("assign").enabled(state.field.is_some()).show(ui)) {
            state.assign_field(env);
        }
        if clicked(&Button::new(words.use_default.clone()).id_salt("useDefault").enabled(DialogState::can_use_default(env, command)).show(ui)) {
            state.use_default(env, command);
        }
        if clicked(&Button::new(words.clear.clone()).id_salt("clear").enabled(!bindings.is_empty()).show(ui)) {
            state.clear(env, command);
        }
    });
    defaults_line(ui, env, &defaults, &home);
}

fn defaults_line(ui: &mut Ui, env: &Env<'_>, defaults: &[Binding], home: &Scope) {
    let words = env.text.words();
    let chords = if defaults.is_empty() {
        words.none.as_str().to_owned()
    } else {
        defaults.iter().map(|d| binding_text(env, d, home)).collect::<Vec<_>>().join(", ")
    };
    let text = around_chord(env.text.rtl(), &words.default_chord.fill(&[("chord", &chords)]), &chords);
    Text::new(text).shade(Shade::Muted).show(ui);
}

/// New Shortcut: a recorder that takes the chord pressed, or typed entry.
fn new_shortcut(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState, command: &CommandRef, home: &Scope) {
    let words = env.text.words();
    let title = words.new_shortcut.clone();
    if let Some(mut typed) = state.typing.clone() {
        let take_focus = state.typing_focus;
        let out = Field::new(title.clone()).id_salt("shortcuts.type").show(ui, |ui, _| {
            let field = TextField::new(title.clone(), Bind::bound(&mut typed)).placeholder("Cmd+Shift+K").id_salt("type").show(ui);
            if take_focus {
                focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
            }
            field
        });
        state.typing = Some(typed.clone());
        if let Some(edit) = out.inner {
            if edit.response.has_focus() {
                state.typing_focus = false;
            }
            if edit.escaped {
                state.typing = None;
                state.type_error = None;
                return;
            }
            if edit.submitted {
                match state.type_binding(env, command, &typed) {
                    Ok(()) => {
                        state.typing = None;
                        state.type_error = None;
                    }
                    Err(e) => state.type_error = Some(e),
                }
            }
        }
        if let Some(error) = state.type_error.clone() {
            Text::new(error).tone(craft_theme::variant::Tone::Warn).show(ui);
        }
        return;
    }
    let recording = matches!(state.recording.as_ref().map(|r| &r.target), Some(Target::Field { command: c }) if c == command);
    let shown = if recording {
        recording_text(env, state)
    } else {
        state.field.as_ref().filter(|f| f.command == *command).map(|f| binding_text(env, &f.binding, home)).unwrap_or_default()
    };
    let value = if shown.is_empty() { words.none.as_str().to_owned() } else { shown.clone() };
    let name = words.new_shortcut_chord.fill(&[("chord", &value)]);
    let shown_row = Row::new()
        .show(ui, |ui, _| {
            let out = ChordRecorder::new(name.clone(), recording, shown.clone()).id_salt("new").show(ui);
            let type_more = clicked(&Link::new(words.type_more.clone()).id_salt("typeMore").show(ui));
            (out, type_more)
        })
        .inner;
    let Some((out, type_more)) = shown_row else { return };
    if out.started && !recording {
        state.record(env, Target::Field { command: command.clone() });
    }
    if recording {
        if let Some(recording) = state.recording.as_mut() {
            recording.started = true;
        }
        if !out.chords.is_empty() {
            state.feed(env, &out.chords, now);
        } else if out.response.lost_focus() {
            state.stop_recording();
        }
    }
    if type_more {
        state.stop_recording();
        state.typing = Some(String::new());
        state.typing_focus = true;
        state.type_error = None;
    }
}

fn context(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState, command: &CommandRef) {
    let words = env.text.words();
    let contexts = DialogState::contexts(env, command);
    let name = |scope: &Option<Scope>| match scope {
        None => words.default.as_str().to_owned(),
        Some(scope) => scope_name(env, scope),
    };
    let entries: Vec<Entry<Option<Scope>>> = contexts.iter().map(|scope| Entry::new(scope.clone(), name(scope))).collect();
    let mut context = state.context.clone();
    Field::new(words.context.clone()).id_salt("shortcuts.context").show(ui, |ui, _| {
        Select::new(words.context.clone(), Bind::bound(&mut context), &entries).id_salt("context").show(ui);
    });
    if context != state.context {
        state.set_context(env, context);
    }
}

/// "Currently assigned to:" for the field's chord, with links to the holders.
fn holders(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    let Some(field) = state.field.clone() else { return };
    if field.holders.is_empty() && field.shadows.is_empty() {
        Text::new(words.not_assigned.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    Text::new(words.currently_assigned_to.clone()).show(ui);
    let mut go = None;
    for (n, holder) in field.holders.iter().enumerate() {
        let label = label_of(env, &holder.command);
        let home = env.keymap.bindable(&holder.command).map(|b| b.scope.clone());
        let text = match &holder.binding.scope {
            Some(scope) if Some(scope) != home.as_ref() => words.label_scope.fill(&[("label", &label), ("scope", &scope_name(env, scope))]),
            _ => label,
        };
        if super::warn_link(ui, text, "holder", n) {
            go = Some(holder.command.clone());
        }
    }
    for (n, holder) in field.shadows.iter().enumerate() {
        let label = label_of(env, &holder.command);
        let home = env.keymap.bindable(&holder.command).map(|b| b.scope.clone()).unwrap_or_else(Scope::app);
        let scope = scope_name(env, holder.binding.scope_or(&home));
        let text = words.label_scope_both_work.fill(&[("label", &label), ("scope", &scope)]);
        if clicked(&Link::new(text).id_salt(("shadow", n)).show(ui)) {
            go = Some(holder.command.clone());
        }
    }
    if let Some(command) = go {
        state.go_to(&command);
    }
}
