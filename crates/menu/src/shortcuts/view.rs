//! The dialog body: the app's tabs, the set row, the header, the table and the detail pane side by side, the
//! keyboard pane, the import review in place of them, and the bars under them.
//!
//! Focus order is the order the parts are shown: set row, Show, Filter, Search, the keyboard toggle, the table,
//! the detail pane, the conflict bar, the footer.

mod bars;
mod detail;
mod header;
#[cfg(feature = "screen-testing")]
mod stories;
mod table;

use std::borrow::Cow;

use craft_behavior::IconId;
use craft_behavior::{ColumnWidth, DisplayOrder, ItemSource, SelectionChange};
use craft_keymap::chord::Platform;
use craft_theme::variant::Tone;
use craft_ui::buttons::Link;
use craft_ui::contract::{BodyCx, CollectionEvent, Props as _, Scroll, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft_ui::display::Icon;
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::motion::Animate as _;
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use egui::Ui;

use crate::shortcuts::rows::AreaId;
use crate::shortcuts::state::{DialogState, Env};
use crate::shortcuts::words::around_chord;
use crate::shortcuts::{keyboard, review, sets};

/// The table's share of the width beside the detail pane.
const TABLE_SHARE: f32 = 0.62;
/// The detail pane's share of the width beside the table.
const DETAIL_SHARE: f32 = 0.38;
static PANES: [ColumnWidth; 2] = [ColumnWidth::fill(TABLE_SHARE), ColumnWidth::fill(DETAIL_SHARE)];

/// One frame of the dialog body.
pub(crate) fn body(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let now = cx.now();
    state.tick(env, now);
    if let Some(at) = state.wake() {
        cx.wake_at(at);
    }
    if let Some(index) = app_tabs(ui, env, state) {
        let changed = Stack::new()
            .fill()
            .scroll(Scroll::vertical())
            .show(ui, |ui, _| env.tabs.as_deref_mut().is_some_and(|tabs| tabs.draw(index, ui)))
            .inner
            .unwrap_or(false);
        if changed {
            state.app_tab_changed();
        }
        return;
    }
    sets::show(ui, env, state);
    let browsing = state.review.is_none();
    Stack::new().enabled(browsing).show(ui, |ui, cx| header::show(ui, cx, now, env, state));
    if !browsing {
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, cx| review::show(ui, cx, env, state));
    } else {
        let idle = state.prompt.is_none();
        Stack::new().fill().enabled(idle).show(ui, |ui, cx| middle(ui, cx, now, env, state));
    }
    bars::show(ui, cx, env, state);
    // The request outlives every run of the table's body in this frame; the frame's last line spends it.
    state.reveal = false;
}

/// The table beside the detail pane, with the keyboard pane under both while it is on.
fn middle(ui: &mut Ui, cx: &mut BodyCx<'_>, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    Grid::new(&PANES).fill().show(ui, |ui, _| {
        Stack::new().fill().show(ui, |ui, _| table::show(ui, now, env, state));
        Stack::new().fill().show(ui, |ui, cx| detail::show(ui, cx, now, env, state));
    });
    if env.options.keyboard && !state.keyboard.hidden {
        keyboard::show(ui, cx, env, state);
    }
}

/// The titles of a strip of tabs, keyed by position.
pub(super) struct Titles<'a>(pub &'a [String]);

impl DisplayOrder<usize> for Titles<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|title| Cow::Borrowed(title.as_str()))
    }
}

impl ItemSource<usize> for Titles<'_> {}

/// A strip of text tabs over `titles` with `shown` selected; the tab picked this pass.
pub(super) fn tab_strip(ui: &mut Ui, name: &'static str, titles: &[String], shown: usize) -> Option<usize> {
    let strip = TabStrip::new(name)
        .id_salt(name)
        .kind(TabKind::Text)
        .shown(shown)
        .show_items(ui, &Titles(titles), &mut |key, _| Tab::new(titles[*key].clone()).id_salt(("tab", *key)));
    strip.response.events.into_iter().find_map(|event| match event {
        CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last().copied(),
        _ => None,
    })
}

/// The tab strip over the dialog when the app has tabs of its own, and the app tab shown (an index into them);
/// `None` shows Keyboard Shortcuts.
fn app_tabs(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) -> Option<usize> {
    let tabs = env.tabs.as_deref().filter(|tabs| tabs.len() > 0)?;
    let mut titles = vec![env.text.words().keyboard_shortcuts.as_str().to_owned()];
    let described: Vec<(&'static str, &str)> = (0..tabs.len()).filter_map(|i| tabs.describe(i)).collect();
    titles.extend(described.iter().map(|(_, title)| (*title).to_owned()));
    let ids: Vec<&'static str> = described.iter().map(|(id, _)| *id).collect();
    let current = state.tab.and_then(|id| ids.iter().position(|tab| *tab == id));
    if state.tab.is_some() && current.is_none() {
        state.set_tab(None);
    }
    let shown = current.map_or(0, |i| i + 1);
    let picked = Stack::new().enabled(!state.modal()).show(ui, |ui, _| tab_strip(ui, "shortcuts.tab", &titles, shown)).inner.flatten();
    let shown = match picked {
        Some(i) => {
            state.set_tab(i.checked_sub(1).and_then(|k| ids.get(k).copied()));
            i
        }
        None => shown,
    };
    shown.checked_sub(1)
}

/// The key of an area for automation ids and the app's own area ids.
pub(crate) fn area_key(id: &AreaId) -> String {
    match id {
        AreaId::All => "all".into(),
        AreaId::Menu(m) => format!("menu.{m}"),
        AreaId::Tools => "tools".into(),
        AreaId::Panel(p) => format!("panel.{p}"),
        AreaId::Other => "other".into(),
        AreaId::App(a) => (*a).to_string(),
        AreaId::Fixed => "fixed".into(),
    }
}

/// "press keys", or the chord waiting for a second press.
pub(super) fn recording_text(env: &Env<'_>, state: &DialogState) -> String {
    let words = env.text.words();
    match state.recording.as_ref().and_then(|recording| recording.waiting.as_ref()) {
        Some((trigger, _)) => {
            let platform: Platform = env.keymap.platform();
            let chord = env.text.trigger(trigger, platform);
            around_chord(env.text.rtl(), &words.chord_press_again_for_a_double.fill(&[("chord", &chord)]), &chord)
        }
        None => words.press_keys.as_str().to_owned(),
    }
}

/// A link led by the warning icon: a problem the user can follow to its cause. The words carry no glyph.
pub(super) fn warn_link(ui: &mut Ui, text: String, salt: &'static str, n: usize) -> bool {
    let mut followed = false;
    Row::new().show(ui, |ui, _| {
        Icon::new(IconSpec::svg(IconId::new("warning"))).tone(Tone::Warn).show(ui);
        followed = clicked(&Link::new(text.clone()).tone(Tone::Warn).id_salt((salt, n)).show(ui));
    });
    followed
}
