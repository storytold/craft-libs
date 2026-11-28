//! [`Remembered`]: last-used options of a dialog, panel or tool, kept in the app's settings
//! through the `Host` (craft-host).

use std::marker::PhantomData;

use craft_host::{Host, Section};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The settings section every remembered value lives in: one TOML table per scope key.
///
/// ```toml
/// [remembered."filter.gaussianBlur"]
/// radius = 2.5
/// [remembered."prompt.layer.delete"]
/// skip = true
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct RememberedSection {
    /// Each scope key with its stored table.
    #[serde(flatten)]
    pub scopes: toml::Table,
}

impl Section for RememberedSection {
    const KEY: &'static str = "remembered";
}

/// A value remembered under a scope key: a dialog's last-used fields (`"filter.gaussianBlur"`),
/// a panel's view options (`"panel.layers"`), a tool's settings (`"tool.crop"`), a prompt's
/// "Don't show again" (`"prompt.layer.delete"`). The value is any serde type; a dialog's is its
/// `DialogFields`.
///
/// ```ignore
/// let last = Remembered::<DialogFields>::new("filter.gaussianBlur");
/// let fields = last.recall(&host).unwrap_or_default();
/// // … the dialog runs; on OK:
/// last.store(&host, dialog.fields());
/// ```
///
/// - Stored on the dialog's OK only; Cancel keeps the previous value (Photoshop, PhotoCraft).
/// - A value that no longer deserializes (the type changed) reads as `None`.
/// - Preferences' "Reset All Warning Dialogs" calls [`forget_prefix`](Remembered::forget_prefix)
///   with `"prompt."`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remembered<T> {
    scope: String,
    _value: PhantomData<fn() -> T>,
}

impl<T: Serialize + DeserializeOwned> Remembered<T> {
    /// A handle for the value stored under `scope`.
    pub fn new(scope: impl Into<String>) -> Self {
        Self { scope: scope.into(), _value: PhantomData }
    }

    /// The scope key.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The stored value, or `None` when there is none or it does not fit `T`.
    // pc: crates/ui-egui/src/dialogs/filter_dialog.rs:345 `restore_remembered`
    // (`prefs.dialogs[command]`); here through `Host::setting::<RememberedSection>`.
    pub fn recall(&self, host: &Host) -> Option<T> {
        host.setting::<RememberedSection>().scopes.get(&self.scope)?.clone().try_into().ok()
    }

    /// Stores `value` under the scope, replacing the previous one.
    // pc: crates/ui-egui/src/dialogs/filter_dialog.rs:363 `remember` (`prefs.edit(|p| p.dialogs.insert(..))`).
    pub fn store(&self, host: &Host, value: &T) {
        let Ok(value) = toml::Value::try_from(value) else { return };
        let mut section = host.setting::<RememberedSection>();
        section.scopes.insert(self.scope.clone(), value);
        host.set_setting(&section);
    }

    /// Removes the stored value.
    pub fn forget(&self, host: &Host) {
        let mut section = host.setting::<RememberedSection>();
        if section.scopes.remove(&self.scope).is_some() {
            host.set_setting(&section);
        }
    }

    /// Removes every scope that starts with `prefix` (`"prompt."` resets all warning dialogs).
    // new: Photoshop's Preferences › General › Reset All Warning Dialogs.
    pub fn forget_prefix(host: &Host, prefix: &str) {
        let mut section = host.setting::<RememberedSection>();
        let before = section.scopes.len();
        section.scopes.retain(|scope, _| !scope.starts_with(prefix));
        if section.scopes.len() != before {
            host.set_setting(&section);
        }
    }
}
