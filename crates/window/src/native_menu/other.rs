//! Platforms without an application menu bar: every call does nothing.

use std::convert::Infallible;

use super::{Bar, Handler, Options};

/// Never built: [`Installed::install`] returns `None`.
pub(super) struct Installed(Infallible);

impl Installed {
    pub(super) fn install(_: &Bar, _: Options) -> Option<Installed> {
        None
    }

    pub(super) fn set_handler(&self, _: Handler) {
        match self.0 {}
    }

    pub(super) fn apply(&mut self, _: &Bar) -> bool {
        match self.0 {}
    }

    pub(super) fn tracking(&self) -> bool {
        match self.0 {}
    }

    pub(super) fn cancel_tracking(&self) {
        match self.0 {}
    }
}

/// A minimal application menu whose Quit asks the app first. Does nothing here.
pub fn install_app_menu(_app_name: &str, _quit_id: &str, _handler: Handler) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
}

/// The platform's own title for the English menu title `english`; `english` here.
pub fn system_title(english: &str) -> String {
    english.to_owned()
}

/// Hide the application. Does nothing here.
pub fn hide_app() {}

/// Hide the other applications. Does nothing here.
pub fn hide_other_apps() {}
