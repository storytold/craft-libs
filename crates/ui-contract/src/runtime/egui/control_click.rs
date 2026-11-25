//! macOS Control-click: a primary press with Control held is a secondary press.

/// The state that keeps a Control-click's events secondary until its release.
#[derive(Default)]
pub(crate) struct ControlClick {
    secondary_down: bool,
}

impl ControlClick {
    /// On macOS a primary press with Control and neither Option nor Command becomes a secondary press, and its events
    /// stay secondary until the release; Control-Option and Control-Command presses stay primary.
    pub(crate) fn normalize(&mut self, events: &mut [egui::Event], macos: bool) {
        if !macos {
            return;
        }
        for event in events {
            match event {
                egui::Event::PointerButton { button, pressed, modifiers, .. } if *button == egui::PointerButton::Primary => {
                    if *pressed {
                        // Control-Option is the existing brush resize gesture.
                        self.secondary_down = modifiers.ctrl && !modifiers.alt && !modifiers.command;
                    }
                    if self.secondary_down {
                        *button = egui::PointerButton::Secondary;
                    }
                    if !*pressed {
                        self.secondary_down = false;
                    }
                }
                // A drag keeps the mapping when the pointer leaves the window or focus is lost: the
                // release still arrives and must match the secondary press egui already saw. The
                // next press recomputes it, so a missed release can't remap a plain click.
                _ => {}
            }
        }
    }
}
