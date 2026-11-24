//! Stories of `ParamDialog`.

use super::{HostedParamDialog, ParamDialog, ParamLayout, ParamPresets, ParamTab, SharedRows};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _};
use crate::dialog::StackDialog as _;
use crate::dialog::stories::hosted_with;
use crate::patterns::form_dialogs::{DialogFields, PagerUnit, PanePager, Param, ParamKind, PreviewState};
use crate::patterns::presets::PresetRow;
use crate::primitives::{ContentPainterCx, StrokeWidth, TextureCache};
use crate::runtime::env::Label;
use crate::sections::{DialogRole, FooterButton};
use crate::style::DataColor;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Rect, Stroke, Ui, Vec2};

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom","smooth":bool}"#;

/// The picture's size in points at zoom 1.
const PICTURE: [usize; 2] = [360, 240];

/// The filtered picture, or the unfiltered one the pane shows while the Preview box is held to compare.
fn picture(ui: &Ui, unfiltered: bool) -> SizedTexture {
    let (key, color) = if unfiltered {
        ("form-dialogs-dialog-story-unfiltered", Color32::from_rgb(0x4d, 0x6f, 0x8a))
    } else {
        ("form-dialogs-dialog-story", Color32::from_rgb(0x8a, 0x6f, 0x4d))
    };
    let id = TextureCache::texture(ui.ctx(), key, || ColorImage::filled(PICTURE, color));
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

const LAYOUTS: [(&str, ParamLayout); 3] = [("Beside", ParamLayout::Beside), ("Below", ParamLayout::Below), ("Stacked", ParamLayout::Stacked)];

/// A blur dialog with the `preview` and `layout` values of the story.
fn blur_story(ui: &mut Ui, data: &mut StoryData, preview: bool, layout: ParamLayout) -> StoryOutput {
    let preview = *data.value("preview", preview);
    let layout = *data.choice("layout", layout, &LAYOUTS);
    let comparing = *data.value("comparing", false);
    let dialog = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).opened(data.bind("open", true)).history(true).layout(layout);
    let dialog = if preview { dialog.previews(true).preview(PreviewState::Ready(picture(ui, comparing))) } else { dialog };
    let shown = dialog.show(ui);
    *data.value("comparing", false) = shown.comparing;
    shown.into()
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, false, ParamLayout::Beside)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, true, ParamLayout::Beside)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_preview_below(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, true, ParamLayout::Below)
}

/// PhotoCraft's filter dialogs: stacked slider rows, the pane with the zoom row under it, the buttons across the foot.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_stacked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, true, ParamLayout::Stacked)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_close_without_reset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::new("blur", "Blur", Param::parse_list(SPEC))
        .opened(data.bind("open", true))
        .cancel_label(Label::from("Close"))
        .alt_resets(false)
        .show(ui)
        .into()
}

const COMMAND_SPEC: &str = r#"{"name":text,"size":1..100=10,"method":"fit|fill","keep":bool}"#;

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_footer_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::new("color-table", "Color Table", Param::parse_list(SPEC))
        .opened(data.bind("open", true))
        .extra_buttons(vec![
            ("load".to_owned(), FooterButton::new(Label::from("Load…"), DialogRole::Other)),
            ("save".to_owned(), FooterButton::new(Label::from("Save…"), DialogRole::Other)),
        ])
        .footer_note("About 1.2 MB")
        .show(ui)
        .into()
}

/// Why the story's host refuses every OK.
const REFUSAL: &str = "The radius is too large for this layer.";

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn hosted_param_dialog_with_a_refused_ok(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let open = move || {
        let mut dialog = HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| {
            ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(fields))
        })
        .held_ok();
        // The story opens with an OK already refused, so the notice shows under the rows.
        let _ = dialog.confirm(&ctx);
        dialog.answer_ok(Err(Label::from(REFUSAL)));
        dialog
    };
    hosted_with(ui, data, (), open, |host| {
        if let Some(dialog) = host.get_mut::<HostedParamDialog>()
            && dialog.take_ok().is_some()
        {
            dialog.answer_ok(Err(Label::from(REFUSAL)));
        }
    })
}

/// The open state of a story's dialog, which starts closed: a button opens it.
fn opener<'d>(ui: &mut Ui, data: &'d mut StoryData) -> &'d mut bool {
    let open = data.value("open", false);
    if crate::contract::clicked(&Button::new("Open").show(ui)) {
        *open = true;
    }
    open
}

/// A footer toggle: a `Bool` row drawn as a footer button that shows its on label while on.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_a_footer_toggle(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = vec![
        Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, Default::default())),
        Param::new("grid", "Show Grid", ParamKind::bool(false)).in_footer("Hide Grid"),
    ];
    ParamDialog::new("grid", "Grid", params).opened(Bind::bound(opener(ui, data))).show(ui).into()
}

/// A named presets row above the rows.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [PresetRow::item("soft".to_owned(), "Soft", 0), PresetRow::item("hard".to_owned(), "Hard", 0)];
    let values =
        [("soft".to_owned(), DialogFields::from_pairs([("radius", 2.0)])), ("hard".to_owned(), DialogFields::from_pairs([("radius", 60.0)]))];
    let built_in = ["soft".to_owned()];
    ParamDialog::new("blur", "Blur", Param::parse_list(SPEC))
        .opened(Bind::bound(opener(ui, data)))
        .presets(ParamPresets::new(&rows, &values, &built_in))
        .show(ui)
        .into()
}

/// A dialog whose values the host refuses with a reason until it withdraws it: OK is off and the reason shows under
/// the rows.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_refused_values(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let open = move || {
        let mut dialog = HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| {
            ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(fields))
        })
        .held_ok();
        dialog.refuse(Some(Label::from(REFUSAL)));
        dialog
    };
    hosted_with(ui, data, (), open, |_| {})
}

/// The note the story's host gives while its check runs.
const CHECKING: &str = "Checking pages…";

/// Why the story's host holds OK back while its check runs.
const CHECK_RUNS: &str = "The pages are still being checked.";

/// The host's job notes under the rows and OK held back with its reason, as a host sets them between passes while a
/// check over the document runs.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn hosted_param_dialog_with_host_notes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let open = move || {
        let mut dialog = HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| {
            ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(fields))
        });
        dialog.set_notes(vec![Label::from(CHECKING)]);
        dialog.set_ready(Err(Label::from(CHECK_RUNS)));
        dialog
    };
    hosted_with(ui, data, (), open, |_| {})
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn command_form(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::command("fit-image", "Fit Image", Param::parse_list(COMMAND_SPEC)).opened(data.bind("open", true)).show(ui).into()
}

/// The rows of the tabbed stories' first tab.
const GENERAL_SPEC: &str = r#"{"language":"english|german|french","deskew":bool,"resolution":72..600=300}"#;
/// The rows of the tabbed stories' second tab.
const OUTPUT_SPEC: &str = r#"{"layer":"text|image","keepOriginal":bool}"#;

/// The editable tabs of the tabbed stories.
fn tabs() -> Vec<ParamTab> {
    vec![ParamTab::new("General", Param::parse_list(GENERAL_SPEC)), ParamTab::new("Output", Param::parse_list(OUTPUT_SPEC))]
}

/// A tabbed dialog over `tabs`, with `shared` as its own rows, open while the story's `open` value is on.
fn tabbed(data: &mut StoryData, tabs: Vec<ParamTab>, shared: Vec<Param>) -> ParamDialog<'_> {
    ParamDialog::new("ocr", "Recognize Text", shared).tabs(tabs).opened(data.bind("open", true))
}

/// Editable tabs: the strip shows the titles and the shown tab's rows; the values of both tabs answer at OK.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    tabbed(data, tabs(), Vec::new()).show(ui).into()
}

/// A read-only tab, shown first: its values without controls under an informational note.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_tabs_read_only_note(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = vec![
        Param::read_only_text("producer", "Producer", "PdfCraft"),
        Param::read_only_text("version", "PDF Version", "1.7"),
        Param::new("tagged", "Tagged", ParamKind::bool(true)),
    ];
    let note = Label::from("These values come from the file and cannot be changed here.");
    let mut all = vec![ParamTab::new("Document", document).read_only(Some(note))];
    all.extend(tabs());
    tabbed(data, all, Vec::new()).show(ui).into()
}

/// Shared rows under the shown tab's rows.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_shared_rows_below(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shared = Param::parse_list(r#"{"allPages":bool}"#);
    tabbed(data, tabs(), shared).shared_rows(SharedRows::Below).show(ui).into()
}

/// The preview pager of the pending and tinted stories: page 2 of 5.
fn pager() -> PanePager {
    PanePager { shown: 2, count: 5, shown_label: None, back: true, next: true, unit: PagerUnit::Page }
}

/// The host's drawing over the preview picture: the picture's edge in the surface's ink.
fn outline(cx: &ContentPainterCx, rect: Rect) {
    let stroke = Stroke::new(cx.stroke_width(StrokeWidth::Thin), cx.ink());
    let [top_left, top_right, bottom_right, bottom_left] = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for edge in [[top_left, top_right], [top_right, bottom_right], [bottom_right, bottom_left], [bottom_left, top_left]] {
        cx.painter.line_segment(edge, stroke);
    }
}

/// A preview the host is still rendering: the well's progress line, with the pager under the pane and the host's
/// overlay given for when the picture comes.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_preview_pending(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::new("ocr", "Recognize Text", Param::parse_list(GENERAL_SPEC))
        .opened(data.bind("open", true))
        .previews(true)
        .preview(PreviewState::Pending(Some(0.4)))
        .preview_pager(pager())
        .preview_overlay(|cx, _, rect| outline(cx, rect))
        .show(ui)
        .into()
}

/// A tinted preview: the picture's colours multiplied by the tint, with the pager and the overlay.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_preview_tinted(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let picture = picture(ui, false);
    ParamDialog::new("ocr", "Recognize Text", Param::parse_list(GENERAL_SPEC))
        .opened(data.bind("open", true))
        .previews(true)
        .preview(PreviewState::Ready(picture))
        .preview_pager(pager())
        .preview_overlay(|cx, _, rect| outline(cx, rect))
        .preview_tint(DataColor::new(Color32::from_rgb(0x9c, 0xc2, 0xe6)))
        .show(ui)
        .into()
}

/// A muted note after the title.
#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_title_note(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).opened(data.bind("open", true)).title_note("Layer 1").show(ui).into()
}
