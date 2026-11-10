//! Smoke crate for stock crates.io egui 0.37 (see README.md and check.sh).
//!
//! With default features this crate compiles and tests against registry egui only.
//! `cargo tree -i egui` (run by check.sh) proves that adding craft-ui by path without
//! any patch keeps exactly one egui source, the registry one, and that the fork-only
//! `Style::set_right_to_left` call in `craft_ui::direction::apply` resolves to the
//! no-op trait shim here.

#[cfg(test)]
mod tests {
    use craft_ui::direction::{self, Direction, HasDirection, Setting};

    #[test]
    fn stock_egui_runs_a_frame() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("stock egui 0.37");
        });
        assert!(!out.shapes.is_empty());
        out.textures_delta.clear();
    }

    #[test]
    fn direction_resolves_from_the_language_script() {
        assert_eq!(direction::resolve("ar", Setting::Language), Direction::Rtl);
        assert_eq!(direction::resolve("he", Setting::Language), Direction::Rtl);
        assert_eq!(direction::resolve("en", Setting::Language), Direction::Ltr);
        assert_eq!(direction::resolve("ar", Setting::Ltr), Direction::Ltr);
    }

    #[test]
    fn apply_stores_the_direction_on_a_stock_egui_context() {
        let ctx = egui::Context::default();
        assert_eq!(ctx.direction(), Direction::Ltr);
        direction::apply(&ctx, Direction::Rtl);
        assert_eq!(ctx.direction(), Direction::Rtl);
    }
}
