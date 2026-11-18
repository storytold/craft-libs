//! `PreferencesPage`: the preferences body without a window, applying each value at once.

use super::model::PreferencesModel;
use super::window::PreferencesState;
use crate::contract::{Bind, Pattern, Props};
use crate::runtime::requests::ColorRequest;
use egui::{Response, Ui};

/// A preferences page that applies each value at once, with search and Reset Section. [`PreferencesWindow`] shows its
/// body through this page, so one page path remains.
///
/// [`PreferencesWindow`]: super::PreferencesWindow
#[derive(Pattern, Props)]
pub struct PreferencesPage<'a> {
    #[allow(dead_code)] // P3(chrome-w4): ch.C10
    state: Bind<'a, PreferencesState>,
}

/// What a [`PreferencesPage`] reports: its response, the colour the host should pick and the page shown.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreferencesPageOutput {
    /// The page's response.
    pub response: Response,
    /// A colour row asked for the picker; the host answers through [`PreferencesModel::set_color`].
    pub pick_color: Option<ColorRequest>,
    /// The page the user switched to in this pass.
    pub page: Option<String>,
}

impl<'a> PreferencesPage<'a> {
    /// A page over the host's `state`.
    pub fn new(state: Bind<'a, PreferencesState>) -> Self {
        PreferencesPage { state }
    }

    /// Shows the page for this pass over `host`.
    pub fn show<H: PreferencesModel + ?Sized>(self, ui: &mut Ui, host: &mut H) -> PreferencesPageOutput {
        let _ = (ui, host);
        todo!("ch.C10")
    }
}
