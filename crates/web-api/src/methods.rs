//! The method table: which control method backs each `window.<app>` function. Target-independent,
//! so it is natively tested and usable from any build.

/// The method table one app hands the crate: the control method behind each `window.<app>`
/// function. `None` means that function is not installed; `request` is always installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Methods {
    /// Behind `commands()`.
    pub commands: Option<&'static str>,
    /// Behind `execute(command, params)`, sent as `{command, params}`.
    pub execute: Option<&'static str>,
    /// Behind `docInspect(params)`.
    pub doc_inspect: Option<&'static str>,
    /// Behind `uiInspect(params)`.
    pub ui_inspect: Option<&'static str>,
    /// Behind `screenshot(params)`, sent with `data: true` unless the caller set `data`.
    pub screenshot: Option<&'static str>,
}

impl Methods {
    /// A table with no methods: only `request` is installed.
    pub const NONE: Self = Self { commands: None, execute: None, doc_inspect: None, ui_inspect: None, screenshot: None };
}

impl Default for Methods {
    /// The desktop control-server method names every Craft web build shares.
    fn default() -> Self {
        Self {
            commands: Some("engine.commands"),
            execute: Some("engine.execute"),
            doc_inspect: Some("document.inspect"),
            ui_inspect: Some("ui.inspect"),
            screenshot: Some("ui.screenshot"),
        }
    }
}
