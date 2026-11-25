//! The egui calls `craft_ui::testing` needs: frames, raw input and options that apps reach only through the runtime.

use egui::{Context, Event, Key, Modifiers, Pos2, Rect, Shape, Ui, Vec2, ViewportCommand};
use std::sync::Arc;

use super::injected;
pub(crate) use super::injected::{key_press, key_release, key_repeat, modifiers_changed, pointer_moved, text_event};
use super::memory::Service;

thread_local! {
    /// How many frames a test fixture drives are open on this thread.
    static DRIVING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether the running frame is one a test fixture drives. The records only trials read (the step log, findings, the
/// style record and the measuring-run guard) are kept in these frames only, so an app built with the testing
/// feature, such as the gallery, does not pay for them.
pub(crate) fn driving() -> bool {
    DRIVING.with(std::cell::Cell::get) > 0
}

/// Runs `f`, a frame a test fixture drives.
fn drive<R>(f: impl FnOnce() -> R) -> R {
    struct Driving;
    impl Drop for Driving {
        fn drop(&mut self) {
            DRIVING.with(|depth| depth.set(depth.get() - 1));
        }
    }
    DRIVING.with(|depth| depth.set(depth.get() + 1));
    let _driving = Driving;
    f()
}

/// What a frame sent out that trials read.
pub(crate) struct FrameResult {
    /// The frame sent a close command.
    pub closed: bool,
    /// The cursor the frame's output asked for.
    pub cursor: egui::CursorIcon,
    /// The cursor image the frame's output asked for.
    pub cursor_image: Option<egui::CustomCursorImage>,
    /// The wait egui asks for before the root viewport's next pass; `Duration::MAX` when it asks for none.
    pub repaint_delay: std::time::Duration,
    /// The AccessKit tree update of the frame.
    pub tree: Option<crate::runtime::egui::access::AccessTree>,
    /// What the frame's `PlatformOutput.ime` said.
    pub ime: Option<ImeSeen>,
    /// The texts the frame put on the clipboard.
    pub copied: Vec<String>,
    /// The URLs the frame asked the platform to open, each with whether it asked for a new tab.
    pub opened: Vec<(String, bool)>,
    /// The text runs the frame painted.
    pub texts: Vec<PaintedText>,
    /// The visual bounds of every shape the frame painted, within its clip rect. A shape with no extent (egui reports an empty galley's
    /// bounds as NaN) has none, and a shadow (a blurred rect) is no paint bound.
    pub bounds: Vec<Rect>,
    /// Half the stroke width of the shape behind `bounds[i]`: how far its edge may overshoot a rect it was drawn to.
    pub overshoot: Vec<f32>,
    /// The texture changes the frame sent, for a renderer.
    pub textures: egui::TexturesDelta,
    /// The shapes the frame painted, as a renderer takes them.
    pub shapes: Vec<egui::epaint::ClippedShape>,
    /// The frame's pixels per point.
    pub pixels_per_point: f32,
}

/// A text run a frame painted.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PaintedText {
    /// Where the run lies, in points.
    pub rect: Rect,
    /// The layout cut text.
    pub elided: bool,
    /// The run's full text.
    pub text: String,
    /// The screen y of the run's first baseline.
    pub baseline: f32,
    /// The clip rect the run was painted under.
    pub clip: Rect,
}

/// The IME output of a frame: the owner widget, whether it asks the OS to interrupt a composition, the caret rect.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImeSeen {
    /// The widget that owns the IME.
    pub owner: egui::Id,
    /// The OS composition is to be interrupted.
    pub interrupt: bool,
    /// The reset the frame asked for; a trial answers it with `Event::ImeResetCompleted`.
    pub reset: Option<egui::ImeResetId>,
    /// Where the candidate window opens.
    pub caret: Rect,

    /// The IME context the output belongs to; it changes with the owner, its epoch, its focus or its purpose.
    pub generation: u64,
    /// The bounded committed text around the caret, or `Unavailable`.
    pub surrounding: egui::output::IMESurroundingText,
    /// Whether the input method made the change to `surrounding`.
    pub cause: egui::output::ImeChangeCause,
}

/// The files the OS hovers over or drops on a frame's window, by name; a hovered file also has the media type the OS gave.
/// `read` are pathless files dropped in the frame whose bytes the page reads after it, by name with the bytes the read
/// gives. `given` are files dropped in the frame by a relative name with their bytes at hand, as a browser hands them over.
#[derive(Clone, Debug, Default)]
pub(crate) struct OsFiles {
    pub hovered: Vec<(String, String)>,
    pub dropped: Vec<String>,
    pub read: Vec<(String, Vec<u8>)>,
    pub given: Vec<(String, Vec<u8>)>,
}

/// The viewport facts a frame reports: the root window's inner rect when the frame is the root's (`inner`), and its
/// focus: a window-focus event also sets the root viewport's focus fact, as a host reports both, and the fact stays
/// until the next such event. Before any event the focus is unknown. The root's input
/// reports the same focus (`RawInput::focused`), a window with focus while it is unknown.
fn window_facts(
    ctx: &Context,
    events: &[Event],
    native_pixels_per_point: Option<f32>,
    inner: Option<Rect>,
) -> egui::ViewportIdMap<egui::ViewportInfo> {
    let last = events.iter().rev().find_map(|event| if let Event::WindowFocused(focused) = event { Some(*focused) } else { None });
    let focused = Service::<WindowFocus>::run(ctx, |state| {
        state.0 = last.or(state.0);
        state.0
    });
    std::iter::once((
        egui::ViewportId::ROOT,
        egui::ViewportInfo { focused, native_pixels_per_point, inner_rect: inner, ..egui::ViewportInfo::default() },
    ))
    .collect()
}

/// The focus the fixture's window last reported; unset before any focus event.
#[derive(Default)]
struct WindowFocus(Option<bool>);

/// The child windows of the root that are open: each opens with its first frame and stays listed in every frame's
/// viewports, as eframe lists every open viewport, until a test closes it ([`close_viewport`]) or the app sends it
/// `ViewportCommand::Close`.
#[derive(Default)]
struct OpenChildren(Vec<egui::ViewportId>);

/// The open child windows, with `viewport` opened first when it is a child.
fn open_children(ctx: &Context, viewport: egui::ViewportId) -> Vec<egui::ViewportId> {
    Service::<OpenChildren>::run(ctx, |open| {
        if viewport != egui::ViewportId::ROOT && !open.0.contains(&viewport) {
            open.0.push(viewport);
        }
        open.0.clone()
    })
}

/// One run of `ui` on `input`; a root pass declares the open `children` after it, as an eframe app shows each of its
/// windows every frame: egui keeps a child viewport only while its parent's passes show it. Each child's content runs
/// in its own frames, so the declaration shows nothing.
fn run_ui(ctx: &Context, input: RawInput, children: &[egui::ViewportId], mut ui: impl FnMut(&mut Ui)) -> egui::FullOutput {
    let root = input.viewport_id == egui::ViewportId::ROOT;
    ctx.run_ui(input, |top| {
        ui(top);
        if root && !children.is_empty() {
            let embedded = ctx.embed_viewports();
            ctx.set_embed_viewports(false);
            for child in children {
                ctx.show_viewport_deferred(*child, egui::ViewportBuilder::default(), |_, _| {});
            }
            ctx.set_embed_viewports(embedded);
        }
    })
}

/// Closes the child window `viewport`: later frames no longer list it.
pub(crate) fn close_viewport(ctx: &Context, viewport: egui::ViewportId) {
    Service::<OpenChildren>::run(ctx, |open| open.0.retain(|held| *held != viewport));
}

/// The fixture's reset bridge, as egui-winit keeps one per window: the reset request it holds for each viewport, and
/// the requests it dropped, which the next frame reports as `Event::ImeResetRetired`.
#[derive(Default)]
struct ImeResets {
    pending: egui::ViewportIdMap<(egui::ImeResetId, (egui::Id, u64, egui::IMEPurpose))>,
    retired: Vec<egui::ImeResetId>,
}

impl ImeResets {
    fn retire(&mut self, viewport: egui::ViewportId) {
        self.retired.extend(self.pending.remove(&viewport).map(|(id, _)| id));
    }

    /// The frame's input: a completion ends the request, a window focus loss drops it, and the requests dropped since
    /// the last frame are reported after the frame's own events.
    fn input(&mut self, viewport: egui::ViewportId, events: &mut Vec<Event>) {
        for event in events.iter() {
            match event {
                Event::ImeResetCompleted { request_id } if self.pending.get(&viewport).is_some_and(|(id, _)| id == request_id) => {
                    self.pending.remove(&viewport);
                }
                Event::WindowFocused(false) => self.retire(viewport),
                _ => {}
            }
        }
        events.extend(self.retired.drain(..).map(|request_id| Event::ImeResetRetired { request_id }));
    }

    /// The frame's output: egui's dropped requests, a pass with no IME output or another IME context, and a newer
    /// request each drop the request held.
    fn output(&mut self, viewport: egui::ViewportId, output: &egui::PlatformOutput) {
        self.retired.extend(output.retired_ime_resets.iter().copied());
        let Some(ime) = &output.ime else {
            self.retire(viewport);
            return;
        };
        let context = (ime.owner, ime.context_generation, ime.purpose);
        if self.pending.get(&viewport).is_some_and(|(_, held)| *held != context) {
            self.retire(viewport);
        }
        if let Some(id) = ime.reset_request
            && self.pending.get(&viewport).is_none_or(|(held, _)| *held != id)
        {
            self.retire(viewport);
            self.pending.insert(viewport, (id, context));
        }
    }
}

/// Runs one pass of `viewport` at `time` seconds with `events` and `files`, on a window of `native_pixels_per_point`
/// (egui's default when `None`). A viewport other than the root is a child window of the root, which stays live; the
/// child stays open in every later frame, the root's included, until it is closed.
#[expect(clippy::too_many_arguments, reason = "the arguments are the facts of one frame, and every caller passes them all")]
pub(crate) fn run_frame(
    ctx: &Context,
    time: f64,
    screen: Vec2,
    native_pixels_per_point: Option<f32>,
    viewport: egui::ViewportId,
    mut events: Vec<Event>,
    files: OsFiles,
    ui: impl FnMut(&mut Ui),
) -> FrameResult {
    let screen = Rect::from_min_size(Pos2::ZERO, screen);
    let hovered_files = files.hovered.iter().map(|(name, mime)| egui::HoveredFile { path: Some(name.into()), mime: mime.clone() }).collect();
    let dropped_files = files
        .dropped
        .iter()
        .map(|name| Arc::new(super::files::DroppedPath(name.into())) as egui::DroppedFileHandle)
        .chain(
            files.given.into_iter().map(|(name, bytes)| Arc::new(super::files::DroppedBytes { name: name.into(), bytes }) as egui::DroppedFileHandle),
        )
        .collect();
    // The web drop of a pathless file: its reads open in the pass of the drop, and their bytes arrive after it.
    if !files.read.is_empty() {
        super::files::open_reads(ctx, viewport, files.read.len());
    }
    Service::<ImeResets>::run(ctx, |resets| resets.input(viewport, &mut events));
    let mut viewports = window_facts(ctx, &events, native_pixels_per_point, (viewport == egui::ViewportId::ROOT).then_some(screen));
    let focused = viewport != egui::ViewportId::ROOT || viewports.get(&egui::ViewportId::ROOT).and_then(|info| info.focused).unwrap_or(true);
    let children = open_children(ctx, viewport);
    for child in &children {
        viewports
            .insert(*child, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), native_pixels_per_point, ..egui::ViewportInfo::default() });
    }
    let input = RawInput {
        screen_rect: Some(screen),
        time: Some(time),
        viewport_id: viewport,
        focused,
        events,
        hovered_files,
        dropped_files,
        viewports,
        ..RawInput::default()
    };
    let mut output = drive(|| run_ui(ctx, input, &children, ui));
    Service::<ImeResets>::run(ctx, |resets| resets.output(viewport, &output.platform_output));
    for (name, bytes) in files.read {
        let file = crate::runtime::drag::OsFile { name, path: None, mime: String::new(), bytes: Some(bytes.into()) };
        super::files::post(ctx, viewport, Some(file));
    }
    let mut flat = Vec::new();
    for clipped in &output.shapes {
        let mut shapes = Vec::new();
        flatten_ref(&clipped.shape, &mut shapes);
        flat.extend(shapes.into_iter().map(|shape| (shape, clipped.clip_rect)));
    }
    // A shape shows only inside its clip rect.
    let (bounds, overshoot): (Vec<Rect>, Vec<f32>) = flat
        .iter()
        .filter(|(shape, _)| !is_shadow(shape) && shape.visual_bounding_rect().is_finite())
        .map(|(shape, clip)| (shape.visual_bounding_rect().intersect(*clip), stroke_width(shape) / 2.0))
        .filter(|(rect, _)| rect.is_finite() && rect.min.x <= rect.max.x && rect.min.y <= rect.max.y)
        .unzip();
    let mut painted_before: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let texts = flat
        .iter()
        .filter_map(|(shape, clip)| match shape {
            Shape::Text(text) => {
                let nth = painted_before.entry(Arc::as_ptr(&text.galley) as usize).or_insert(0);
                let full = super::painted::full_text_of_painted(ctx, &text.galley, *nth);
                *nth += 1;
                let baseline = text.galley.rows.first().map_or(0.0, |placed| match placed.row.glyphs.first() {
                    Some(glyph) => placed.pos.y + glyph.pos.y,
                    None => placed.pos.y + placed.row.size.y,
                });
                Some(PaintedText {
                    rect: text.galley.rect.translate(text.pos.to_vec2()),
                    baseline: text.pos.y + baseline,
                    elided: full.is_some(),
                    text: full.unwrap_or_else(|| text.galley.job.text.clone()),
                    clip: *clip,
                })
            }
            _ => None,
        })
        .collect();
    let closed = output.viewport_output.values().any(|viewport| viewport.commands.iter().any(|command| matches!(command, ViewportCommand::Close)));
    let closing: Vec<egui::ViewportId> = output
        .viewport_output
        .iter()
        .filter(|(id, out)| **id != egui::ViewportId::ROOT && out.commands.iter().any(|command| matches!(command, ViewportCommand::Close)))
        .map(|(id, _)| *id)
        .collect();
    for child in closing {
        close_viewport(ctx, child);
    }
    let repaint_delay = output.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
    let copied = output
        .platform_output
        .commands
        .iter()
        .filter_map(|command| if let egui::OutputCommand::CopyText(text) = command { Some(text.clone()) } else { None })
        .collect();
    let opened = output
        .platform_output
        .commands
        .iter()
        .filter_map(|command| if let egui::OutputCommand::OpenUrl(open) = command { Some((open.url.clone(), open.new_tab)) } else { None })
        .collect();
    FrameResult {
        closed,
        copied,
        opened,
        texts,
        bounds,
        overshoot,
        cursor: output.platform_output.cursor_icon,
        cursor_image: output.platform_output.cursor_image.clone(),
        repaint_delay,
        tree: crate::runtime::egui::access::tree_of(&mut output),
        ime: output.platform_output.ime.as_ref().map(|ime| ImeSeen {
            owner: ime.owner,
            interrupt: ime.should_interrupt_composition,
            reset: ime.reset_request,
            caret: ime.cursor_rect,
            generation: ime.context_generation,
            surrounding: ime.surrounding_text.clone(),
            cause: ime.change_cause,
        }),
        textures: std::mem::take(&mut output.textures_delta),
        shapes: output.shapes,
        pixels_per_point: output.pixels_per_point,
    }
}

/// Whether a press of `key` is in this pass's input, that is, no component took it.
pub(crate) fn key_reached(ctx: &Context, key: Key) -> bool {
    ctx.input(|input| input.key_pressed(key))
}

/// Gives egui back the option the runtime turns off at install: `Cmd` with `=` or `-` zooms the interface.
pub(crate) fn allow_zoom_keys(ctx: &Context) {
    ctx.options_mut(|options| options.zoom_with_keyboard = true);
}

/// Sets egui's click options, as an app does for its own widgets: the longest movement and the longest press that
/// still count as a click.
#[cfg(feature = "suite")]
pub(crate) fn set_click_options(ctx: &Context, distance: f32, seconds: f64) {
    ctx.options_mut(|options| {
        options.input_options.max_click_dist = distance;
        options.input_options.max_click_duration = seconds;
    });
}

/// Sets egui's surrender option, the app's rule for its own focused widgets: with `never`, a click elsewhere leaves
/// their focus; otherwise egui's default, which surrenders it on a click elsewhere.
#[cfg(feature = "suite")]
pub(crate) fn set_app_surrender(ctx: &Context, never: bool) {
    let rule = if never { egui::SurrenderFocusOn::Never } else { egui::SurrenderFocusOn::Clicks };
    ctx.options_mut(|options| options.input_options.surrender_focus_on = rule);
}

/// Whether egui's surrender option is egui's default, a click elsewhere.
#[cfg(feature = "suite")]
pub(crate) fn app_surrenders_on_click(ctx: &Context) -> bool {
    ctx.options(|options| options.input_options.surrender_focus_on == egui::SurrenderFocusOn::Clicks)
}

/// egui's click options: the longest movement and the longest press that still count as a click.
#[cfg(feature = "suite")]
pub(crate) fn click_options(ctx: &Context) -> (f32, f64) {
    ctx.options(|options| (options.input_options.max_click_dist, options.input_options.max_click_duration))
}

/// A fresh context with no plugin and AccessKit on.
pub(crate) fn bare_context() -> Context {
    let ctx = Context::default();
    super::fonts::set_definitions(&ctx, test_fonts());
    ctx.enable_accesskit();
    ctx
}

/// The font definitions of every test context: egui's default faces from `fonts::deflated_default_definitions`, the
/// set the font stack starts from. `egui::FontDefinitions::default()` would not do: it holds egui's faces only when
/// some crate in the build turns on egui's `default_fonts` feature, so the fixture would measure differently from
/// one build to the next.
fn test_fonts() -> egui::FontDefinitions {
    static FONTS: std::sync::OnceLock<egui::FontDefinitions> = std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut definitions = crate::fonts::deflated_default_definitions();
            // A fixture reads text through the AccessKit tree and the painted bounds, never the atlas pixels, and
            // hinting changes only how outlines are rasterized (advances and layout do not read it). Without it no
            // face builds a hinting instance and no glyph runs the hinter. An app's own stack (`UiFonts::install`)
            // keeps hinting, which the review renders use.
            for data in definitions.font_data.values_mut() {
                let mut face = (**data).clone();
                face.tweak.hinting = Some(false);
                *data = Arc::new(face);
            }
            definitions
        })
        .clone()
}

/// A paragraph of `text` wrapped to the available width.
pub(crate) fn wrapped_text(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(text).wrap());
}

/// The test font definitions with the proportional family's first font moved to the end, a change that
/// needs a new layout of every text.
pub(crate) fn reordered_fonts() -> egui::FontDefinitions {
    let mut definitions = test_fonts();
    if let Some(proportional) = definitions.families.get_mut(&egui::FontFamily::Proportional)
        && !proportional.is_empty()
    {
        proportional.rotate_left(1);
    }
    definitions
}

/// Runs one frame like `run_frame` and returns the shapes it painted, nested shapes flattened.
pub(crate) fn run_frame_shapes(ctx: &Context, time: f64, screen: Vec2, ui: impl FnMut(&mut Ui)) -> Vec<Shape> {
    let screen = Rect::from_min_size(Pos2::ZERO, screen);
    let children = open_children(ctx, egui::ViewportId::ROOT);
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo::default());
    for child in &children {
        viewports.insert(*child, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..egui::ViewportInfo::default() });
    }
    let input = RawInput { screen_rect: Some(screen), time: Some(time), viewports, ..RawInput::default() };
    let mut output = drive(|| run_ui(ctx, input, &children, ui));
    output.textures_delta.clear();
    let mut flat = Vec::new();
    for clipped in output.shapes {
        flatten(clipped.shape, &mut flat);
    }
    flat
}

/// A blurred rect: the shadow of a surface.
fn is_shadow(shape: &Shape) -> bool {
    matches!(shape, Shape::Rect(rect) if rect.blur_width > 0.0)
}

fn stroke_width(shape: &Shape) -> f32 {
    match shape {
        Shape::Circle(circle) => circle.stroke.width,
        Shape::Ellipse(ellipse) => ellipse.stroke.width,
        Shape::LineSegment { stroke, .. } => stroke.width,
        Shape::Path(path) => path.stroke.width,
        Shape::Rect(rect) => rect.stroke.width,
        _ => 0.0,
    }
}

fn flatten_ref<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| flatten_ref(shape, out)),
        other => out.push(other),
    }
}

pub(crate) fn flatten(shape: Shape, out: &mut Vec<Shape>) {
    match shape {
        Shape::Vec(shapes) => shapes.into_iter().for_each(|shape| flatten(shape, out)),
        other => out.push(other),
    }
}

/// Shows a focusable egui widget of `id` at the `Ui` cursor, as the sensing core of a component root does.
pub(crate) fn focusable_widget(ui: &mut Ui, id: egui::Id) -> Rect {
    let rect = Rect::from_min_size(ui.cursor().min, ui.spacing().interact_size);
    ui.advance_cursor_after_rect(rect);
    ui.interact(rect, id, egui::Sense::click());
    rect
}

/// A host's own single-line egui text field of `id` over `text`, as an app that keeps its own widgets beside the
/// framework's draws it; returns its rect.
#[cfg(feature = "suite")]
pub(crate) fn host_field(ui: &mut Ui, id: egui::Id, text: &mut String) -> Rect {
    ui.add(egui::TextEdit::singleline(text).id(id)).rect
}

/// A host's own multi-line egui text field of `id` over `text` that locks focus, as a code editor of the app's own:
/// its egui key filter keeps Tab, which it types.
#[cfg(feature = "suite")]
pub(crate) fn host_code_field(ui: &mut Ui, id: egui::Id, text: &mut String) -> Rect {
    ui.add(egui::TextEdit::multiline(text).id(id).lock_focus(true)).rect
}

/// A host's own text editor of `id` at `rect`, as a canvas text tool draws it: it reads the IME events from egui's
/// input into `text` (a commit appends, a deletion before the caret removes characters from its end) and writes the
/// pass's IME output, context generation 1.
#[cfg(feature = "suite")]
pub(crate) fn host_ime_editor(ui: &Ui, id: egui::Id, rect: Rect, text: &mut String) {
    let _ = ui.interact(rect, id, egui::Sense::click());
    for event in ui.input(|input| input.events.clone()) {
        match event {
            Event::Ime(egui::ImeEvent::Commit(committed)) => text.push_str(&committed),
            Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, .. }) => {
                for _ in 0..before_chars {
                    text.pop();
                }
            }
            _ => {}
        }
    }
    ui.ctx().output_mut(|output| output.ime = Some(egui::output::IMEOutput::new(id, 1, egui::IMEPurpose::Normal, rect, rect)));
}

/// What [`host_resetting_ime_editor`] holds between passes.
#[cfg(feature = "suite")]
#[derive(Default)]
pub(crate) struct ResettingEditor {
    pub(crate) text: String,
    pub(crate) composing: bool,
    /// The reset the editor asked for last, until its completion arrives; the editor takes no IME event meanwhile.
    pub(crate) reset: Option<egui::ImeResetId>,
    /// The editor also writes its request into its IME output's `reset_request`. Otherwise it leaves that field to egui,
    /// which publishes the request asked through `Context::request_ime_reset`, as EffectCraft's editor does.
    pub(crate) in_output: bool,
    /// Ask for a reset at the next pass, as EffectCraft's editor does when its caret moves after an outside press.
    pub(crate) ask: bool,
}

/// A host's own text editor of `id` at `rect` that fences its own interrupted composition, as EffectCraft's viewer text
/// editor does: a primary press on it while it composes ends the composition and asks the platform for a reset, and the
/// editor ignores IME events until that reset's completion. A press elsewhere asks for nothing. Otherwise as
/// [`host_ime_editor`].
#[cfg(feature = "suite")]
pub(crate) fn host_resetting_ime_editor(ui: &Ui, id: egui::Id, rect: Rect, editor: &mut ResettingEditor) {
    let _ = ui.interact(rect, id, egui::Sense::click());
    let pressed = ui.input(|input| input.pointer.primary_pressed() && input.pointer.press_origin().is_some_and(|at| rect.contains(at)));
    for event in ui.input(|input| input.events.clone()) {
        match event {
            Event::ImeResetCompleted { request_id } if editor.reset == Some(request_id) => editor.reset = None,
            _ if editor.reset.is_some() => {}
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => editor.composing = !text.is_empty(),
            Event::Ime(egui::ImeEvent::Commit(committed)) => {
                editor.text.push_str(&committed);
                editor.composing = false;
            }
            Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, .. }) => {
                for _ in 0..before_chars {
                    editor.text.pop();
                }
            }
            _ => {}
        }
    }
    let mut ime = egui::output::IMEOutput::new(id, 1, egui::IMEPurpose::Normal, rect, rect);
    let interrupted = editor.composing && pressed;
    if interrupted || std::mem::take(&mut editor.ask) {
        editor.composing = false;
        editor.reset = ui.ctx().request_ime_reset(ui.ctx().viewport_id());
        ime.should_interrupt_composition = interrupted;
        if editor.in_output {
            ime.reset_request = editor.reset;
        }
    }
    ui.ctx().output_mut(|output| output.ime = Some(ime));
}

/// Gives the host's own egui widget `id` key focus, as the host's code does.
#[cfg(feature = "suite")]
pub(crate) fn host_focus(ctx: &Context, id: egui::Id) {
    ctx.memory_mut(|memory| memory.request_focus(id));
}

/// A host's own clickable egui widget `id` at `rect`, a focus stop for egui's own focus moves; returns whether egui
/// reports a click on it, by egui's click options.
#[cfg(feature = "suite")]
pub(crate) fn host_button(ui: &Ui, id: egui::Id, rect: Rect) -> bool {
    ui.interact(rect, id, egui::Sense::click()).clicked()
}

/// A host's own egui drag area `id` at `rect`; returns whether egui reports it dragged and whether its drag stopped
/// this pass.
#[cfg(feature = "suite")]
pub(crate) fn host_drag_area(ui: &Ui, id: egui::Id, rect: Rect) -> (bool, bool) {
    let response = ui.interact(rect, id, egui::Sense::drag());
    (response.dragged(), response.drag_stopped())
}

/// A host's own clickable egui widget `id` at `rect` in a disabled `Ui`.
#[cfg(feature = "suite")]
pub(crate) fn host_disabled_button(ui: &mut Ui, id: egui::Id, rect: Rect) {
    ui.scope_builder(egui::UiBuilder::new().disabled(), |ui| host_button(ui, id, rect));
}

/// A host's own egui widget `id` at `rect` that senses drags and takes no focus.
#[cfg(feature = "suite")]
pub(crate) fn host_unfocusable_area(ui: &Ui, id: egui::Id, rect: Rect) {
    ui.interact(rect, id, egui::Sense::DRAG);
}

/// A host's own selectable egui label of `text`; returns its rect.
#[cfg(feature = "suite")]
pub(crate) fn host_label(ui: &mut Ui, text: &str) -> Rect {
    ui.add(egui::Label::new(text).selectable(true)).rect
}

/// The egui layer `ui` paints on.
pub(crate) fn layer_of(ui: &Ui) -> egui::LayerId {
    ui.layer_id()
}

/// The primary button going down or up at `pos`.
pub(crate) fn primary_button(pos: Pos2, pressed: bool) -> Event {
    injected::button(pos, egui::PointerButton::Primary, pressed, Modifiers::NONE)
}

/// The secondary button going down or up at `pos`.
pub(crate) fn secondary_button(pos: Pos2, pressed: bool) -> Event {
    injected::button(pos, egui::PointerButton::Secondary, pressed, Modifiers::NONE)
}

/// The middle button going down or up at `pos`.
pub(crate) fn middle_button(pos: Pos2, pressed: bool) -> Event {
    injected::button(pos, egui::PointerButton::Middle, pressed, Modifiers::NONE)
}

/// A wheel event of `dy` points with the zoom modifier held.
pub(crate) fn zoom_wheel_event(dy: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::COMMAND,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// The actions the focus trials request.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AccessActionKind {
    /// AccessKit `Focus`.
    Focus,
    /// AccessKit `Click`.
    Click,
    /// AccessKit `Increment`.
    Increment,
    /// AccessKit `Decrement`.
    Decrement,
    /// AccessKit `SetValue` with a number.
    SetValue(f64),
    /// AccessKit `Expand`, an action with no intent of its own.
    Expand,
    /// AccessKit `Collapse`.
    Collapse,
    /// AccessKit `ScrollUp`.
    ScrollUp,
    /// AccessKit `ScrollDown`.
    ScrollDown,
    /// AccessKit `ScrollLeft`.
    ScrollLeft,
    /// AccessKit `ScrollRight`.
    ScrollRight,
    /// AccessKit `ScrollIntoView`.
    ScrollIntoView,
}

impl AccessActionKind {
    /// Whether `action` is this one.
    pub fn is(self, action: &craft_behavior::AccessRequest) -> bool {
        action.action == self.action()
    }

    fn action(self) -> accesskit::Action {
        match self {
            AccessActionKind::Focus => accesskit::Action::Focus,
            AccessActionKind::Click => accesskit::Action::Click,
            AccessActionKind::Increment => accesskit::Action::Increment,
            AccessActionKind::Decrement => accesskit::Action::Decrement,
            AccessActionKind::SetValue(_) => accesskit::Action::SetValue,
            AccessActionKind::Expand => accesskit::Action::Expand,
            AccessActionKind::Collapse => accesskit::Action::Collapse,
            AccessActionKind::ScrollUp => accesskit::Action::ScrollUp,
            AccessActionKind::ScrollDown => accesskit::Action::ScrollDown,
            AccessActionKind::ScrollLeft => accesskit::Action::ScrollLeft,
            AccessActionKind::ScrollRight => accesskit::Action::ScrollRight,
            AccessActionKind::ScrollIntoView => accesskit::Action::ScrollIntoView,
        }
    }

    fn data(self) -> Option<accesskit::ActionData> {
        match self {
            AccessActionKind::SetValue(value) => Some(accesskit::ActionData::NumericValue(value)),
            _ => None,
        }
    }
}

/// An AccessKit action request for the node of `target`.
pub(crate) fn access_request(kind: AccessActionKind, target: egui::Id) -> Event {
    let action = kind.action();
    let target_node = accesskit::NodeId(crate::runtime::egui::access::node_key(target));
    Event::AccessKitActionRequest(accesskit::ActionRequest { action, target_tree: accesskit::TreeId::ROOT, target_node, data: kind.data() })
}

/// An AccessKit action request for the node `target`.
pub(crate) fn access_request_node(kind: AccessActionKind, target: crate::runtime::egui::access::NodeKey) -> Event {
    let target_node = accesskit::NodeId(target.raw());
    Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: kind.action(),
        target_tree: accesskit::TreeId::ROOT,
        target_node,
        data: kind.data(),
    })
}

/// An AccessKit `SetValue` request carrying `value` for the node of `target`.
pub(crate) fn access_set_value(target: egui::Id, value: f64) -> Event {
    let target_node = accesskit::NodeId(crate::runtime::egui::access::node_key(target));
    Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: accesskit::Action::SetValue,
        target_tree: accesskit::TreeId::ROOT,
        target_node,
        data: Some(accesskit::ActionData::NumericValue(value)),
    })
}

/// An IME preedit event of `text` with no cursor range.
pub fn ime_preedit(text: &str) -> Event {
    Event::Ime(egui::ImeEvent::Preedit { text: text.into(), active_range_chars: None })
}

pub use egui::{FullOutput, RawInput};

/// The id of the widget egui is dragging.
pub fn dragged_id(ctx: &Context) -> Option<egui::Id> {
    ctx.dragged_id()
}

/// Adds a plugin to `ctx` that records, for each input hook call, the viewport of the raw input and the
/// viewport that is current on the context.
pub fn record_input_hooks(ctx: &Context, log: std::sync::Arc<std::sync::Mutex<Vec<(egui::ViewportId, egui::ViewportId)>>>) {
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(egui::ViewportId, egui::ViewportId)>>>);
    impl egui::Plugin for Recorder {
        fn debug_name(&self) -> &'static str {
            "craft_ui.fixture.input_hooks"
        }

        fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
            if let Ok(mut log) = self.0.lock() {
                log.push((input.viewport_id, ctx.viewport_id()));
            }
        }
    }
    ctx.add_plugin(Recorder(log));
}

/// The area `ui` has allocated so far.
pub(crate) fn used_rect(ui: &Ui) -> Rect {
    ui.min_rect()
}

/// An AccessKit `SetValue` request carrying `text` for the node of `target`.
pub fn set_value_request(target: egui::Id, text: &str) -> Event {
    let target_node = accesskit::NodeId(crate::runtime::egui::access::node_key(target));
    Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: accesskit::Action::SetValue,
        target_tree: accesskit::TreeId::ROOT,
        target_node,
        data: Some(accesskit::ActionData::Value(text.into())),
    })
}

/// An AccessKit `SetTextSelection` request for the node of `target`, from character `anchor.1` of the run node
/// `anchor.0` to character `focus.1` of `focus.0`.
pub fn text_selection_request(target: egui::Id, anchor: (egui::Id, usize), focus: (egui::Id, usize)) -> Event {
    let target_node = accesskit::NodeId(crate::runtime::egui::access::node_key(target));
    let position = |(node, character_index): (egui::Id, usize)| accesskit::TextPosition { node: node.accesskit_id(), character_index };
    Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: accesskit::Action::SetTextSelection,
        target_tree: accesskit::TreeId::ROOT,
        target_node,
        data: Some(accesskit::ActionData::SetTextSelection(accesskit::TextSelection { anchor: position(anchor), focus: position(focus) })),
    })
}

/// A globally unique id named by `name`.
pub(crate) fn named_id(name: &str) -> egui::Id {
    crate::runtime::egui::memory::unique(("focus-trial", name))
}

/// An allocating widget over `size` on `ui`: what app code may add over a canvas body.
pub(crate) fn allocate_blocker(ui: &mut Ui, size: egui::Vec2) {
    ui.allocate_response(size, egui::Sense::hover());
}

/// A clickable egui widget placed by hand in `rect`: what a component's own widgets never are.
pub(crate) fn raw_widget(ui: &mut Ui, rect: egui::Rect) {
    ui.interact(rect, crate::runtime::egui::memory::unique("raw_widget"), egui::Sense::click());
}

/// Whether a component's `Response` reports a click, by pointer or by keyboard activation.
pub(crate) fn clicked(response: &egui::Response) -> bool {
    super::response::clicked(response)
}

/// The widget id egui's hit test chose as the click target at the latest pointer position.
pub(crate) fn click_target(ctx: &Context) -> Option<egui::Id> {
    crate::runtime::egui::input::hit_targets(ctx).0
}

/// A wheel event at the pointer, `dy` lines of vertical scroll.
pub(crate) fn wheel_lines_event(dy: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// A wheel event at the pointer, `dy` points of vertical scroll.
pub(crate) fn wheel_event(dy: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// The pointer leaving the window.
pub(crate) fn pointer_gone() -> Event {
    Event::PointerGone
}

/// The window losing focus.
pub(crate) fn window_blurred() -> Event {
    Event::WindowFocused(false)
}

/// Gives key focus to the widget `id`.
pub(crate) fn focus(ctx: &Context, id: egui::Id) {
    ctx.memory_mut(|memory| memory.request_focus(id));
}

/// Whether a repaint was requested since the last pass.
pub(crate) fn repaint_requested(ctx: &Context) -> bool {
    ctx.has_requested_repaint()
}
