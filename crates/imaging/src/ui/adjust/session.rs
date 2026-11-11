//! [`AdjustSession`]: the Properties panel's memory of the settings it found on an adjustment
//! layer, so Reset returns to them once the user edited.

#[cfg(feature = "screen-testing")]
mod stories;

/// The settings an adjustment layer had when the Properties panel started showing it, and whether
/// the panel edited them since (or since the last Reset). A plain value the app keeps per panel;
/// the app chooses the kinds that use it (PhotoCraft: Color Balance only, the one kind measured
/// in Photoshop 25.4).
///
/// - [`track`](Self::track) every pass the panel shows the layer: the first pass (or the first
///   after a pass without it, or for another target) takes the committed settings as the start
///   and clears `edited`; following passes keep them.
/// - [`edited`](Self::edited) when the panel reports an edit of the layer.
/// - [`reset`](Self::reset) on the panel's Reset: if edited, the start settings; else the kind's
///   defaults from `defaults`. Either way `edited` clears, so a second Reset gives the defaults.
///   The app applies the result as part of the layer's edit step (its `coalesce` key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdjustSession<T> {
    /// The target (layer and document, hashed by the app), the last pass it was tracked in, and
    /// the settings found.
    start: Option<(u64, u64, T)>,
    edited: bool,
}

impl<T: Clone> AdjustSession<T> {
    /// No session yet.
    pub fn new() -> Self {
        Self { start: None, edited: false }
    }

    /// The panel shows target `target` in pass `pass` (`Context::cumulative_pass_nr`) with
    /// `committed` settings. Kept while `target` is the same and `pass` is at most one past the
    /// last tracked pass; taken anew (and `edited` cleared) otherwise.
    pub fn track(&mut self, target: u64, pass: u64, committed: &T) {
        // `last + 1 >= pass` for the same document).
        match &mut self.start {
            Some((kept_target, last, _)) if *kept_target == target && last.saturating_add(1) >= pass => *last = pass,
            _ => {
                self.start = Some((target, pass, committed.clone()));
                self.edited = false;
            }
        }
    }

    /// The panel edited the settings.
    pub fn edited(&mut self) {
        self.edited = true;
    }

    /// Whether the panel edited the settings since it found them or since the last Reset.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// The settings the panel found, while edited.
    pub fn start(&self) -> Option<&T> {
        self.start.as_ref().filter(|_| self.edited).map(|(_, _, found)| found)
    }

    /// The Properties panel's Reset: the start settings if edited, else `defaults()`; clears
    /// `edited`.
    pub fn reset(&mut self, defaults: impl FnOnce() -> T) -> T {
        let settings = match (self.edited, &self.start) {
            (true, Some((_, _, found))) => found.clone(),
            _ => defaults(),
        };
        self.edited = false;
        settings
    }
}
