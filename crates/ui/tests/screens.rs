//! A rename dialog in the six test languages: the example of `craft_ui::testing` for app tests.
//!
//! The snapshots are in `tests/snapshots/rename-*.png`; `UPDATE_SNAPSHOTS=1` rewrites them.

use craft_ui::fonts::CraftFace;
use craft_ui::testing::{JAPANESE, KOREAN, Locale, Screens, compose};
use egui::accesskit::Role;
use egui::{FontData, FontFamily, Ui};
use egui_kittest::kittest::Queryable as _;

const ARABIC: CraftFace = CraftFace {
    family: "Noto Sans Arabic",
    style: "Regular",
    scripts: &["Arab"],
    bytes: include_bytes!("fonts/NotoSansArabic-screens.ttf"),
    index: 0,
};
// The subset has the dialog's kana, Han and Hangul, so it stands in for every CJK face. Its Latin
// glyphs, Vietnamese included, are the app's UI face: egui's default face has no Vietnamese.
const CJK: CraftFace = CraftFace {
    family: "Noto Sans CJK SC",
    style: "Regular",
    scripts: &["Jpan", "Kore", "Hans", "Hant"],
    bytes: include_bytes!("fonts/NotoSansCJKsc-screens.otf"),
    index: 0,
};
const THAI: CraftFace =
    CraftFace { family: "Noto Sans Thai", style: "Regular", scripts: &["Thai"], bytes: include_bytes!("fonts/NotoSansThai-screens.ttf"), index: 0 };

struct Strings {
    title: &'static str,
    name: &'static str,
    file: &'static str,
    keep: &'static str,
    cancel: &'static str,
    rename: &'static str,
}

fn strings(language: &str) -> Strings {
    let [title, name, file, keep, cancel, rename] = match language {
        "ar" => ["إعادة تسمية الملف", "الاسم", "تقرير ٢٠٢٦.pdf", "الإبقاء على الامتداد", "إلغاء", "إعادة التسمية"],
        "ja" => ["ファイル名を変更", "名前", "報告書 2026.pdf", "拡張子を保持", "キャンセル", "変更"],
        "zh-TW" => ["重新命名檔案", "名稱", "報告 2026.pdf", "保留副檔名", "取消", "重新命名"],
        "ko" => ["파일 이름 바꾸기", "이름", "보고서 2026.pdf", "확장자 유지", "취소", "이름 바꾸기"],
        "th" => ["เปลี่ยนชื่อไฟล์", "ชื่อ", "รายงาน 2026.pdf", "เก็บนามสกุลไฟล์", "ยกเลิก", "เปลี่ยนชื่อ"],
        "vi" => ["Đổi tên tệp", "Tên", "Báo cáo 2026.pdf", "Giữ phần mở rộng", "Hủy", "Đổi tên"],
        _ => ["Rename file", "Name", "Report 2026.pdf", "Keep extension", "Cancel", "Rename"],
    };
    Strings { title, name, file, keep, cancel, rename }
}

struct Dialog {
    file: String,
    keep: bool,
}

impl Dialog {
    fn new(locale: &Locale) -> Self {
        Self { file: strings(locale.language).file.to_owned(), keep: true }
    }

    fn ui(&mut self, ui: &mut Ui, locale: &Locale) {
        let text = strings(locale.language);
        ui.heading(text.title);
        ui.horizontal(|ui| {
            let label = ui.label(text.name);
            ui.text_edit_singleline(&mut self.file).labelled_by(label.id);
        });
        ui.checkbox(&mut self.keep, text.keep);
        ui.horizontal(|ui| {
            _ = ui.button(text.rename);
            _ = ui.button(text.cancel);
        });
    }
}

fn screens() -> Screens {
    Screens::new("rename")
        .size([320.0, 150.0])
        .fonts(|fonts| fonts.app_face("ui", FontData::from_static(CJK.bytes), &[FontFamily::Proportional]).craft_faces(&[ARABIC, CJK, THAI]))
}

#[test]
fn testing_rename_dialog_snapshots() {
    screens().snapshots(|ui, locale| Dialog::new(locale).ui(ui, locale));
}

#[test]
fn testing_rename_dialog_takes_ime_input() {
    for (language, composition) in [("ko", KOREAN), ("ja", JAPANESE)] {
        let dialog = Dialog { file: String::new(), keep: true };
        let mut harness = screens().harness_state(language, |ui, locale, dialog: &mut Dialog| dialog.ui(ui, locale), dialog);
        harness.get_by_role(Role::TextInput).focus();
        harness.run();
        compose(&mut harness, &composition);
        assert_eq!(harness.state().file, composition.commit, "{language}");
    }
}
