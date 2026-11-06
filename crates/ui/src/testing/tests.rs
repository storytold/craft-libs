use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;

use super::{JAPANESE, KOREAN, Screens, compose, ime_events};
use crate::fonts::CraftFace;

const ARABIC: CraftFace = CraftFace {
    family: "Noto Sans Arabic",
    style: "Regular",
    scripts: &["Arab"],
    bytes: include_bytes!("../../tests/fonts/NotoSansArabic-screens.ttf"),
    index: 0,
};
// The subset has the test strings' kana, Han and Hangul, so it stands in for every CJK face.
const CJK: CraftFace = CraftFace {
    family: "Noto Sans CJK SC",
    style: "Regular",
    scripts: &["Jpan", "Kore", "Hans", "Hant"],
    bytes: include_bytes!("../../tests/fonts/NotoSansCJKsc-screens.otf"),
    index: 0,
};

fn screens() -> Screens {
    Screens::new("unit").size([300.0, 80.0]).fonts(|fonts| fonts.craft_faces(&[ARABIC, CJK]))
}

#[test]
fn ime_events_end_with_the_commit() {
    let events = ime_events(&KOREAN);
    assert_eq!(events.len(), 4);
    assert_eq!(events.last(), Some(&egui::Event::Ime(egui::ImeEvent::Commit("한".to_owned()))));
}

#[test]
fn compose_types_into_the_focused_text_field() {
    for (language, composition) in [("ko", KOREAN), ("ja", JAPANESE)] {
        let mut harness = screens().harness_state(
            language,
            |ui, _, text: &mut String| {
                let label = ui.label("Name");
                ui.text_edit_singleline(text).labelled_by(label.id);
            },
            String::new(),
        );
        harness.get_by_role(Role::TextInput).focus();
        harness.run();
        compose(&mut harness, &composition);
        assert_eq!(harness.state(), composition.commit, "{language}");
    }
}

#[test]
fn right_to_left_screens_start_at_the_right() {
    for (language, at_right) in [("ar", true), ("ja", false)] {
        let harness = screens().harness(language, |ui, _| _ = ui.label("ملف"));
        let label = harness.get_by_label("ملف").rect();
        assert_eq!(label.max.x > 250.0, at_right, "{language}: {label:?}");
        assert_eq!(label.min.x < 50.0, !at_right, "{language}: {label:?}");
    }
}
