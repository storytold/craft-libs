//! Dialog droppers (F04.02): a dialog's pipette that samples the canvas. The dialog's `DropperButton` asks the host to
//! arm its canvas ([`EyedropperRequest`]); the host runs a [`DropperTool`] on the canvas, which samples under the
//! dialog's modal layer (the dialog registers the canvas with the sampler) and answers the request with the sample.
//!
//! ```ignore
//! // In a Levels dialog: the black-point dropper.
//! let button = DropperButton::new("Sample in image to set black point", Bind::bound(&mut armed)).kind(DropperKind::Black).show(ui);
//! if let Some(request) = button.request {
//!     host.arm(request);
//! }
//! // In the app's canvas pass, while a dropper is armed:
//! let mut tool = DropperTool::new(|doc| composite_color(doc));
//! tool_host.run(ui, &canvas, &mut tool);
//! if let Some(color) = tool.take() {
//!     requests.answer_eyedropper(key, color);
//! }
//! ```

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, IconId, PaintedCursor};
use craft_theme::Srgba;
use craft_ui::buttons::ToggleButton;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, EditResponse, Outcome, Pattern, Props as _, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::requests::EyedropperRequest;
use egui::{Pos2, Response, Ui, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

/// The pipette icon's tip, in the icon's own 24-unit box.
const HOTSPOT: [f32; 2] = [2.0, 22.0];

/// What a dropper samples into: the colour itself, or one of a levels dialog's three points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DropperKind {
    /// The sampled colour is the colour.
    #[default]
    Sample,
    /// The black point.
    Black,
    /// The gray point.
    Gray,
    /// The white point.
    White,
}

impl DropperKind {
    /// The key a dropper of this kind files its request under, so one dialog can hold several droppers.
    fn key(self) -> &'static str {
        match self {
            DropperKind::Sample => "dropper",
            DropperKind::Black => "dropper.black",
            DropperKind::Gray => "dropper.gray",
            DropperKind::White => "dropper.white",
        }
    }
}

/// The pipette button of a dialog: a pressed toggle while its dropper is armed.
///
/// The button reports the toggle as an [`EyedropperRequest`] the dialog files with its `HostRequests`; the host owns
/// the armed state and the sampling.
#[must_use = "show the button with `show`"]
#[derive(Pattern)]
pub struct DropperButton<'a> {
    label: Label,
    armed: Bind<'a, bool>,
    kind: DropperKind,
    key: Option<String>,
}

/// What [`DropperButton::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the dialog files the request"]
pub struct DropperButtonOutput {
    /// The toggle's edit: the new armed state when the user flipped it.
    pub edit: EditResponse<bool>,
    /// The request to arm the host's canvas, in the pass the user armed the dropper.
    pub request: Option<EyedropperRequest>,
}

impl Outcome for DropperButtonOutput {
    type Value = bool;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl<'a> DropperButton<'a> {
    /// A pipette button named `label` ("Sample in image to set black point"), pressed while `armed`.
    pub fn new(label: impl Into<Label>, armed: Bind<'a, bool>) -> Self {
        DropperButton { label: label.into(), armed, kind: DropperKind::Sample, key: None }
    }

    /// What the dropper samples into; the kind is the request's key unless [`key`](Self::key) says otherwise.
    pub fn kind(mut self, kind: DropperKind) -> Self {
        self.kind = kind;
        self
    }

    /// The key of the request the button files.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Shows the button.
    pub fn show(self, ui: &mut Ui) -> DropperButtonOutput {
        let DropperButton { label, armed, kind, key } = self;
        let edit = ToggleButton::new(label, armed).icon(IconId::new("eyedropper")).icon_only().id_salt(kind.key()).show(ui);
        let request = edit.value().copied().filter(|on| *on).map(|_| EyedropperRequest::new(key.unwrap_or_else(|| kind.key().to_owned())));
        DropperButtonOutput { edit, request }
    }
}

/// The canvas tool of an armed dropper: a press, drag and release sample through the app's closure (Photoshop updates
/// the dialog while the button is held), and Escape on the canvas cancels. The host disarms on
/// [`cancelled`](DropperTool::cancelled).
pub struct DropperTool<'a> {
    sample: Box<dyn FnMut([f64; 2]) -> Option<Srgba> + 'a>,
    sampled: Option<Srgba>,
    cancelled: bool,
}

impl<'a> DropperTool<'a> {
    /// A dropper sampling the document point it gets through `sample` (`None` outside the document).
    pub fn new(sample: impl FnMut([f64; 2]) -> Option<Srgba> + 'a) -> Self {
        DropperTool { sample: Box::new(sample), sampled: None, cancelled: false }
    }

    /// The newest sample, once.
    pub fn take(&mut self) -> Option<Srgba> {
        self.sampled.take()
    }

    /// Whether the user cancelled the dropper (Escape on the canvas) since the last call.
    pub fn cancelled(&mut self) -> bool {
        std::mem::take(&mut self.cancelled)
    }

    fn sample_at(&mut self, at: Pos2, cx: &ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        let Point { x, y } = space.view.to_doc(at);
        if let Some(color) = (self.sample)([x, y]) {
            self.sampled = Some(color);
        }
    }
}

impl CanvasTool for DropperTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.sample_at(pointer.pos, cx);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.sample_at(pointer.pos, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.sample_at(pointer.pos, cx);
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.cancelled = true;
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        if matches!(key, ToolKey::Cancel) {
            self.cancelled = true;
            return true;
        }
        false
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    /// The pipette with its tip on the sampled pixel (the icon's lower left, in its 24-unit box).
    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        PaintedCursor::tool(IconId::new("eyedropper"), Vec2::from(HOTSPOT), None).into()
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Color Picker command")
    }
}
