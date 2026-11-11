//! `NavigatorOptionsDialog`: Panel Options of the Navigator panel.

use super::color_choice::{ColorChoice, ColorRowTexts, apply_color_answers, color_row};
use crate::runtime::requests::HostAnswer;

use super::{FormDialog, HostRequests};
use crate::canvas_data::{NavigatorOptions, ProxyColor};

use crate::contract::{Bind, NoValue, Outcome, ShowContainer as _};

use crate::form::FieldGroup;
use crate::runtime::env::{Env, Label};

use crate::runtime::overlay::CloseReason;
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// The [`ColorRequest::key`](crate::runtime::requests::ColorRequest::key) of Panel Options' Custom… colour.
pub const NAVIGATOR_PROXY_COLOR_KEY: &str = "navigatorOptionsDialog.proxyColor";

craft_ui::words! {
    /// The words of Panel Options (`navigatorOptionsDialog`), English by default (Photoshop's). OK, Cancel and Reset are the common words.
    pub struct NavigatorOptionsWords {
        title: "navigatorOptionsDialog.title" = "Navigator Panel Options";
        proxy_color: "navigatorOptionsDialog.proxyColor" = "Color";
        picker_title: "navigatorOptionsDialog.pickerTitle" = "Color Picker (Proxy Color)";
        default_color: "navigatorOptionsDialog.default" = "Default";
        red: "navigatorOptionsDialog.red" = "Red";
        orange: "navigatorOptionsDialog.orange" = "Orange";
        yellow: "navigatorOptionsDialog.yellow" = "Yellow";
        green: "navigatorOptionsDialog.green" = "Green";
        blue: "navigatorOptionsDialog.blue" = "Blue";
        violet: "navigatorOptionsDialog.violet" = "Violet";
        gray: "navigatorOptionsDialog.gray" = "Gray";
        custom_color: "navigatorOptionsDialog.custom" = "Custom…";
    }
}

impl ColorChoice for ProxyColor {
    const NAMED: &'static [Self] = &ProxyColor::ALL;
    const CUSTOM_FIRST: bool = false;

    fn from_picked(color: craft_theme::Srgba) -> Self {
        ProxyColor::custom(color)
    }

    fn is_custom(self) -> bool {
        matches!(self, ProxyColor::Custom(_))
    }

    fn srgba(self) -> craft_theme::Srgba {
        self.color()
    }
}

/// The name of `color` in the list.
fn color_label(color: ProxyColor, words: &NavigatorOptionsWords) -> Label {
    match color {
        ProxyColor::Default => words.default_color.clone(),
        ProxyColor::Red => words.red.clone(),
        ProxyColor::Orange => words.orange.clone(),
        ProxyColor::Yellow => words.yellow.clone(),
        ProxyColor::Green => words.green.clone(),
        ProxyColor::Blue => words.blue.clone(),
        ProxyColor::Violet => words.violet.clone(),
        ProxyColor::Gray => words.gray.clone(),
        ProxyColor::Custom(_) => words.custom_color.clone(),
    }
}

/// What Panel Options reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigatorOptionsDialogOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The options, the pass the dialog closed as OK.
    pub accepted: Option<NavigatorOptions>,
    /// The options after the pass when they differ from the bind's at the start of the pass: a bind that does not
    /// write through applies them.
    pub value: Option<NavigatorOptions>,
    /// The colour request of Custom…: the app opens its colour picker and answers with
    /// [`HostRequests::answer_color`], then hands the answers back with [`NavigatorOptionsDialog::answers`].
    pub requests: HostRequests,
}

impl Outcome for NavigatorOptionsDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Panel Options of the Navigator: the colour of the proxy rectangle.
///
/// - The Color list shows the named colours, each with its swatch, then Custom…. One Tab stop; the arrows pick.
/// - Custom… asks the app for a colour with a request keyed [`NAVIGATOR_PROXY_COLOR_KEY`], every time it is picked, also while a custom
///   colour is selected; the colour becomes Custom when the app answers, and a cancelled picker leaves it as it was.
/// - Enter is OK and Escape cancels. Holding Alt turns Cancel into Reset, which restores the options from when the
///   dialog opened.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct NavigatorOptionsDialog<'a> {
    options: Bind<'a, NavigatorOptions>,
    /// The app's answers to earlier colour requests.
    #[prop]
    answers: Vec<HostAnswer>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> NavigatorOptionsDialog<'a> {
    /// Panel Options over the options in `options`.
    pub fn new(options: Bind<'a, NavigatorOptions>) -> Self {
        NavigatorOptionsDialog { options, answers: Vec::new(), open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NavigatorOptionsDialogOutput {
        let NavigatorOptionsDialog { options, answers, open } = self;
        let words = Env::of(ui).words::<NavigatorOptionsWords>();
        let title = words.title.clone();
        let texts = ColorRowTexts { label: words.proxy_color.clone(), picker_title: words.picker_title.clone() };
        let mut form = FormDialog::new(title.clone(), options).answers(answers);
        if let Some(open) = open {
            form = form.opened(open);
        }
        let shown = form.show(ui, |ui, cx, options| {
            apply_color_answers(cx.take_answers(), NAVIGATOR_PROXY_COLOR_KEY, &mut options.proxy_color);
            FieldGroup::new(title.clone()).show(ui, |ui, _| {
                color_row(ui, &texts, &mut options.proxy_color, NAVIGATOR_PROXY_COLOR_KEY, cx.requests(), |color| color_label(color, &words));
            });
            true
        });
        NavigatorOptionsDialogOutput {
            response: shown.response,
            closed: shown.closed,
            accepted: shown.accepted,
            value: shown.value,
            requests: shown.requests,
        }
    }
}
