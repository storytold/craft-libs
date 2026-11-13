#![cfg(feature = "snapshot")]
//! Renders the update dialogs to PNG, light and dark at 1.5x:
//! `CRAFT_UPDATE_SNAPSHOT_DIR=~/.cache/cl-update-shots cargo test -p craft-update --features snapshot --test dialog_screenshots`
//! Without the directory variable the test does nothing; these are not golden comparisons.

use std::path::PathBuf;

use craft_update::dialog::{CheckResultDialog, StartupPrompt};
use craft_update::release::Release;
use craft_update::suite::PHOTOCRAFT;
use craft_update::updater::Status;
use egui::Theme;
use egui_kittest::Harness;

fn fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    for (name, bytes, family) in [
        ("Hack", epaint_default_fonts::HACK_REGULAR, egui::FontFamily::Monospace),
        ("Ubuntu", epaint_default_fonts::UBUNTU_LIGHT, egui::FontFamily::Proportional),
    ] {
        defs.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
        defs.families.entry(family).or_default().push(name.into());
    }
    // Both faces serve both families: the card uses monospace labels and proportional text.
    for family in defs.families.values_mut() {
        for name in ["Ubuntu", "Hack"] {
            if !family.iter().any(|f| f == name) {
                family.push(name.into());
            }
        }
    }
    ctx.set_fonts(defs);
}

static APP: craft_update::App = PHOTOCRAFT;

#[test]
fn dialog_screenshots() {
    let Some(dir) = std::env::var_os("CRAFT_UPDATE_SNAPSHOT_DIR").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let long_error = "couldn't reach https://api.github.com/repos/storytold/photocraft/releases/latest: \
        connection timed out after 30 seconds (os error 110); check the network or proxy settings";
    let states: [(&str, Option<Status>); 4] = [
        ("checking", Some(Status::Checking)),
        ("up-to-date", Some(Status::UpToDate)),
        ("failed", Some(Status::Failed(long_error.into()))),
        ("startup-prompt", None),
    ];
    for (name, status) in states {
        for (theme_name, theme) in [("light", Theme::Light), ("dark", Theme::Dark)] {
            let dialog = CheckResultDialog::new(&APP, "0.1.0");
            let mut prompt = StartupPrompt::new(
                &APP,
                Release { version: Some(semver::Version::new(0, 2, 0)), tag: "v0.2.0".into(), ..Release::default() },
                "0.1.0",
            );
            prompt.is_self_updating = false;
            let mut h = Harness::builder()
                .with_size(egui::vec2(560.0, 360.0))
                .with_pixels_per_point(1.5)
                .with_theme(theme)
                .with_context_setup(fonts)
                .wgpu()
                .build_ui(|ui| match &status {
                    Some(status) => {
                        dialog.show(ui.ctx(), status);
                    }
                    None => {
                        prompt.show(ui.ctx());
                    }
                });
            h.run_steps(4);
            let image = h.render().unwrap_or_else(|e| panic!("render {name}: {e}"));
            let path = dir.join(format!("update-{name}-{theme_name}.png"));
            image.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}
