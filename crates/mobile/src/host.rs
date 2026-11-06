//! A [`CraftApp`] drawn into a native view: egui and wgpu, no winit and no eframe runner.
//!
//! eframe's runner wants to own the process (`UIApplicationMain`, `NativeActivity`), which leaves
//! no room for native chrome. Here the shell owns the process and the frame clock, and the host
//! runs a frame when asked, in eframe's own order (`raw_input_hook`, then `logic` and `ui` in one
//! `run_ui` pass), so the app can't tell the difference.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{self, RenderState, RendererOptions, ScreenDescriptor, WgpuConfiguration};
use eframe::wgpu;
use egui::{Event, Pos2, RawInput, TouchDeviceId, TouchId, TouchPhase, ViewportId};

use crate::input::{Input, hover_event, key_events, scroll_events, secondary_click_events};
use crate::platform::{Pasted, Platform, Request, WriteOutcome};
use crate::pointer::TouchPointer;
use crate::{Action, CraftApp, PickedFile, Snapshot};

/// Why the host could not start.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// wgpu could not wrap the native view.
    #[error("couldn't create a surface for the view: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    /// No adapter or device.
    #[error("couldn't set up the GPU: {0}")]
    Gpu(#[from] egui_wgpu::WgpuError),
    /// The adapter has no configuration for this surface.
    #[error("the GPU can't draw to this view")]
    Unsupported,
}

/// A view's size: physical pixels and how many of them make one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewSize {
    /// Width in physical pixels.
    pub width_px: u32,
    /// Height in physical pixels.
    pub height_px: u32,
    /// Physical pixels per point.
    pub scale: f32,
}

impl ViewSize {
    /// A size the GPU can draw: at least one pixel each way and a positive, finite scale.
    #[must_use]
    pub fn new(width_px: u32, height_px: u32, scale: f32) -> Option<Self> {
        (scale.is_finite() && scale > 0.0).then_some(Self { width_px: width_px.max(1), height_px: height_px.max(1), scale })
    }

    fn points(self) -> egui::Vec2 {
        #[expect(clippy::cast_precision_loss, reason = "view sizes are a few thousand pixels, exact in f32")]
        let px = egui::vec2(self.width_px as f32, self.height_px as f32);
        px / self.scale
    }
}

/// One touch as the platform reports it, in points from the view's top-left corner.
#[derive(Clone, Copy, Debug)]
pub struct Touch {
    /// Stable for one touch from start to end.
    pub id: u64,
    /// Start, move, end or cancel.
    pub phase: TouchPhase,
    /// In points from the view's top-left corner.
    pub pos: Pos2,
    /// 0..=1 where the device reports it (Apple Pencil, most styluses), 0 otherwise.
    pub force: f32,
}

/// What the C and JNI layers call: the host without its app type.
pub(crate) trait HostOps {
    fn resize(&mut self, size: ViewSize);
    fn touch(&mut self, touch: Touch);
    fn frame(&mut self) -> bool;
    fn open_file(&mut self, file: PickedFile);
    fn action(&mut self, action: Action);
    fn answer_pick(&mut self, files: Vec<PickedFile>);
    fn take_requests(&mut self) -> Vec<Request>;
    /// Keys, text, clipboard, trackpad, stylus and life-cycle signals.
    fn input(&mut self, input: Input);
    /// Stop drawing and let go of the view (its window is going away); the app keeps running.
    #[cfg_attr(
        all(target_os = "ios", not(test)),
        expect(dead_code, reason = "only Android takes the view away; iOS keeps its layer for the app's life")
    )]
    fn detach(&mut self);
    /// Draw into a new view after [`HostOps::detach`] (Android hands over a new window whenever
    /// the app comes back on screen).
    ///
    /// # Safety
    /// As [`Host::new`]: `target` stays valid until the host is dropped or detached again.
    #[expect(unsafe_code, reason = "wgpu can only make a surface from a raw platform handle")]
    #[cfg_attr(
        all(target_os = "ios", not(test)),
        expect(dead_code, reason = "only Android takes the view away; iOS keeps its layer for the app's life")
    )]
    unsafe fn attach(&mut self, target: wgpu::SurfaceTargetUnsafe, size: ViewSize) -> Result<(), HostError>;
}

pub(crate) struct Host<A> {
    ctx: egui::Context,
    app: A,
    frame: eframe::Frame,
    platform: Platform,
    render_state: RenderState,
    /// Makes surfaces for the views the host is attached to.
    #[cfg_attr(
        all(target_os = "ios", not(test)),
        expect(dead_code, reason = "only Android takes the view away; iOS keeps its layer for the app's life")
    )]
    instance: wgpu::Instance,
    /// `None` while detached: no view to draw into, so no frames.
    surface: Option<wgpu::Surface<'static>>,
    surface_config: wgpu::SurfaceConfiguration,
    size: ViewSize,
    started: Instant,
    /// Input gathered since the last frame.
    events: Vec<Event>,
    /// Native controls used since the last frame; they run inside the next one.
    actions: Vec<Action>,
    /// Which touch drives egui's pointer, a finger's press held back until it isn't a gesture.
    pointer: TouchPointer,
    /// When egui next wants a frame; worker threads move it earlier through the repaint callback.
    next_frame: Arc<Mutex<Instant>>,
    /// What the shell last heard about the panels.
    shown: Option<Snapshot>,
    /// Modifier keys held, from the last key event.
    modifiers: egui::Modifiers,
    /// Whether the shell was last told to show the on-screen keyboard.
    keyboard_shown: bool,
    /// Between the shell's suspend and resume the app is off screen: frames run only while it is
    /// busy, and never draw (iPadOS ends apps that use the GPU in the background).
    on_screen: bool,
    /// What the shell last heard of [`CraftApp::busy`].
    busy: bool,
    /// Texture changes from frames that didn't draw, uploaded with the next one that does.
    pending_textures: egui::TexturesDelta,
    /// The power state the app last heard: it hears each change once.
    power: Option<crate::Power>,
}

/// How often a busy app runs while nothing else wakes it, so it notices its work has finished
/// even if its worker doesn't ask for a frame.
const BUSY_POLL: Duration = Duration::from_millis(100);

/// A frame slower than this is logged.
const SLOW_FRAME: Duration = Duration::from_millis(250);

/// What a frame may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameKind {
    /// Run the app and draw: it has a view and is on screen.
    Draw,
    /// Run the app without drawing: off screen or without a view, but busy with work the user is
    /// waiting for, which only moves on (and hands over its result) while the app runs, or with
    /// a native control the user used just before leaving (Save, then Home at once).
    Tick,
    /// Nothing: off screen and idle.
    Skip,
}

impl FrameKind {
    fn of(attached: bool, on_screen: bool, busy_or_asked: bool) -> Self {
        if attached && on_screen {
            Self::Draw
        } else if busy_or_asked {
            Self::Tick
        } else {
            Self::Skip
        }
    }
}

impl<A: CraftApp> Host<A> {
    /// Set up wgpu on `target` and build the app.
    ///
    /// # Safety
    /// `target` must stay valid (the layer or native window stays alive) until the host is dropped.
    #[expect(unsafe_code, reason = "wgpu can only make a surface from a raw platform handle")]
    pub(crate) unsafe fn new(target: wgpu::SurfaceTargetUnsafe, size: ViewSize, config: &WgpuConfiguration) -> Result<Self, HostError> {
        let instance = pollster::block_on(config.wgpu_setup.new_instance());
        // SAFETY: the caller keeps the target alive for as long as the host, which owns the surface.
        let surface = unsafe { instance.create_surface_unsafe(target)? };
        let render_state = pollster::block_on(RenderState::create(config, &instance, Some(&surface), RendererOptions::default()))?;
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: render_state.target_format,
            present_mode: wgpu::PresentMode::AutoVsync,
            view_formats: vec![render_state.target_format],
            desired_maximum_frame_latency: 2,
            ..surface.get_default_config(&render_state.adapter, size.width_px, size.height_px).ok_or(HostError::Unsupported)?
        };
        surface.configure(&render_state.device, &surface_config);

        let ctx = egui::Context::default();
        let next_frame = Arc::new(Mutex::new(Instant::now()));
        let wake = next_frame.clone();
        ctx.set_request_repaint_callback(move |info| {
            let at = Instant::now() + info.delay;
            let mut next = wake.lock().unwrap_or_else(PoisonError::into_inner);
            *next = (*next).min(at);
        });
        let platform = Platform::default();
        let app = A::create(&ctx, Some(render_state.clone()), platform.clone());
        Ok(Self {
            ctx,
            app,
            frame: eframe::Frame::_new_kittest(),
            platform,
            render_state,
            instance,
            surface: Some(surface),
            surface_config,
            size,
            started: Instant::now(),
            events: Vec::new(),
            actions: Vec::new(),
            pointer: TouchPointer::default(),
            next_frame,
            shown: None,
            modifiers: egui::Modifiers::NONE,
            keyboard_shown: false,
            on_screen: true,
            busy: false,
            pending_textures: egui::TexturesDelta::default(),
            power: None,
        })
    }

    fn raw_input(&mut self) -> RawInput {
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size.points())),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: true,
            ..RawInput::default()
        };
        if let Some(viewport) = raw.viewports.get_mut(&ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(self.size.scale);
            viewport.inner_rect = raw.screen_rect;
            viewport.focused = Some(true);
        }
        raw
    }

    fn paint(&self, surface: &wgpu::Surface<'static>, primitives: &[egui::ClippedPrimitive], mut textures: egui::TexturesDelta, ppp: f32) {
        let rs = &self.render_state;
        let screen = ScreenDescriptor { size_in_pixels: [self.surface_config.width, self.surface_config.height], pixels_per_point: ppp };
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craft-mobile") });
        let user_buffers = {
            let mut renderer = rs.renderer.write();
            for (id, deltas) in textures.set.drain() {
                for delta in &deltas {
                    renderer.update_texture(&rs.device, &rs.queue, id, delta);
                }
            }
            renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, primitives, &screen)
        };
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                log::warn!("craft-mobile: no surface texture ({other:?}); reconfiguring");
                surface.configure(&rs.device, &self.surface_config);
                rs.queue.submit(user_buffers);
                self.wake();
                return;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let renderer = rs.renderer.read();
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, primitives, &screen);
        }
        rs.queue.submit(user_buffers.into_iter().chain(std::iter::once(encoder.finish())));
        rs.queue.present(texture);
        let mut renderer = rs.renderer.write();
        for id in &textures.free {
            renderer.free_texture(id);
        }
    }

    /// Tell the shell when the panels' state changed.
    fn publish(&mut self) {
        let snapshot = self.app.snapshot();
        if self.shown.as_ref() == Some(&snapshot) {
            return;
        }
        match serde_json::to_string(&snapshot) {
            Ok(json) => self.platform.state(json),
            Err(e) => log::error!("craft-mobile: panel state: {e}"),
        }
        self.shown = Some(snapshot);
    }

    fn wake(&self) {
        *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

impl<A: CraftApp> HostOps for Host<A> {
    fn resize(&mut self, size: ViewSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.surface_config.width = size.width_px;
        self.surface_config.height = size.height_px;
        if let Some(surface) = &self.surface {
            surface.configure(&self.render_state.device, &self.surface_config);
        }
        self.wake();
    }

    /// Touches become egui touch events (pinch and two-finger pans read these), and the first
    /// finger down also drives the pointer, so taps and drags work on every widget.
    /// Touches become egui touch events (pinch and two-finger pans read these), and the first
    /// finger down also drives the pointer (see [`TouchPointer`]), so taps and drags work on every widget.
    fn touch(&mut self, t: Touch) {
        self.touch_as(t, false);
    }

    /// Run one frame if one is due: drawn when the host has a view on screen, else only while the
    /// app is busy or a native control waits, without drawing. Otherwise input waits for the next view.
    fn frame(&mut self) -> bool {
        match FrameKind::of(self.surface.is_some(), self.on_screen, self.busy || !self.actions.is_empty()) {
            FrameKind::Draw => {
                let surface = self.surface.take();
                let drew = self.frame_into(surface.as_ref());
                self.surface = surface;
                drew
            }
            FrameKind::Tick => {
                self.frame_into(None);
                false
            }
            FrameKind::Skip => false,
        }
    }

    fn detach(&mut self) {
        self.surface = None;
    }

    #[expect(unsafe_code, reason = "wgpu can only make a surface from a raw platform handle")]
    unsafe fn attach(&mut self, target: wgpu::SurfaceTargetUnsafe, size: ViewSize) -> Result<(), HostError> {
        // The old surface goes before the new one is made: both may wrap the same window.
        self.surface = None;
        // SAFETY: the caller keeps the target alive until the host is dropped or detached, and
        // the surface goes with it.
        let surface = unsafe { self.instance.create_surface_unsafe(target)? };
        let caps = surface.get_capabilities(&self.render_state.adapter);
        // The renderer's pipelines are built for this format; a view that can't take it can't be drawn.
        if !caps.formats.contains(&self.surface_config.format) {
            return Err(HostError::Unsupported);
        }
        self.size = size;
        self.surface_config.width = size.width_px;
        self.surface_config.height = size.height_px;
        surface.configure(&self.render_state.device, &self.surface_config);
        self.surface = Some(surface);
        self.wake();
        Ok(())
    }

    fn open_file(&mut self, file: PickedFile) {
        self.platform.remember(&file);
        self.app.open_file(&self.ctx, file);
        self.wake();
    }

    fn action(&mut self, action: Action) {
        self.actions.push(action);
        self.wake();
    }

    fn answer_pick(&mut self, files: Vec<PickedFile>) {
        self.platform.answer_pick(files);
        self.wake();
    }

    fn take_requests(&mut self) -> Vec<Request> {
        self.platform.take_requests()
    }

    fn input(&mut self, input: Input) {
        match input {
            Input::Key { key, pressed, modifiers } => {
                self.modifiers = modifiers;
                self.events.extend(key_events(&key, pressed, modifiers));
            }
            Input::Text(text) => self.events.push(Event::Text(text)),
            Input::PasteText(text) => {
                self.events.push(Event::Paste(text.clone()));
                self.platform.set_pasted(Some(Pasted::Text(text)));
            }
            Input::PasteImage { width, height, rgba } => match color_image(width, height, &rgba) {
                Some(image) => {
                    self.events.push(Event::PasteImage(Arc::new(image)));
                    self.platform.set_pasted(Some(Pasted::Image { width, height, rgba }));
                }
                None => log::warn!("craft-mobile: a pasted image's pixels don't match its {width} × {height} size"),
            },
            Input::PasteOwnImage => {
                // The app pastes its own copy (`Platform::pasted`); egui hears that an image came.
                self.events.push(Event::PasteImage(Arc::new(egui::ColorImage::default())));
                self.platform.set_pasted(Some(Pasted::OwnImage));
            }
            Input::Hover(pos) => self.events.extend(hover_event(self.pointer.touching(), Some(pos))),
            Input::HoverGone => self.events.extend(hover_event(self.pointer.touching(), None)),
            Input::Scroll { pos, delta } => self.events.extend(scroll_events(pos, delta, self.modifiers)),
            Input::SecondaryClick(pos) => self.events.extend(secondary_click_events(pos, self.modifiers)),
            Input::Stylus(sample) => {
                self.app.stylus(&sample);
                // Then the same contact as a touch, so widgets and gestures see it too.
                self.touch_as(Touch { id: sample.id, phase: sample.phase, pos: sample.pos, force: sample.pressure }, true);
            }
            Input::Suspend => {
                self.on_screen = false;
                let started = Instant::now();
                self.app.suspend(&self.ctx);
                // The shell waits for this on the thread the OS watches: work belongs on a worker,
                // with the app busy until it's done.
                log::info!("craft-mobile: the app's suspend took {} ms", started.elapsed().as_millis());
                // Busy with what it started (recovery writes): the shell holds background time
                // from now, before its own for the suspend ends.
                self.publish_busy();
            }
            Input::Resume => self.on_screen = true,
            Input::MemoryWarning => self.app.trim_memory(&self.ctx),
            Input::Power(power) => {
                if self.power.replace(power) != Some(power) {
                    self.app.power(&self.ctx, power);
                }
            }
            Input::WriteDone { location, result } => match self.platform.finish_write(&location) {
                Some(path) => self.app.written(&self.ctx, &WriteOutcome { path, result }),
                None => log::warn!("craft-mobile: an answer for a write to {location} nobody asked for"),
            },
        }
        self.wake();
    }
}

/// An RGBA8 buffer as an egui image, when its length matches its size.
fn color_image(width: u32, height: u32, rgba: &[u8]) -> Option<egui::ColorImage> {
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    (w.checked_mul(h)?.checked_mul(4)? == rgba.len()).then(|| egui::ColorImage::from_rgba_unmultiplied([w, h], rgba))
}

impl<A: CraftApp> Host<A> {
    /// Run one frame if one is due (input or an action arrived, or egui asked), drawing into
    /// `surface` when there is one. Returns whether it ran.
    fn frame_into(&mut self, surface: Option<&wgpu::Surface<'static>>) -> bool {
        let now = Instant::now();
        // A finger whose hold ran out without a second finger presses now.
        let held = self.pointer.tick(now);
        self.events.extend(held);
        let idle = self.events.is_empty() && self.actions.is_empty();
        if idle && now < *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) {
            return false;
        }
        // Until egui says otherwise (or a held finger's press is due), no further frame is needed.
        *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) = self.pointer.due().unwrap_or(now + Duration::from_secs(3600));

        let mut raw = self.raw_input();
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let ran = Instant::now();
        let actions = std::mem::take(&mut self.actions);
        let (app, frame) = (&mut self.app, &mut self.frame);
        let output = self.ctx.run_ui(raw, |ui| {
            for action in &actions {
                if let Err(e) = app.action(ui.ctx(), action) {
                    log::warn!("craft-mobile: {action:?}: {e}");
                }
            }
            app.logic(ui.ctx(), frame);
            app.ui(ui, frame);
        });
        // A frame this long holds up the thread the OS watches (iPadOS's main thread): work this
        // big belongs on a worker.
        let ran = ran.elapsed();
        if ran > SLOW_FRAME {
            log::warn!("craft-mobile: the app took {} ms for one frame", ran.as_millis());
        }
        // The paste this frame's input carried has been handled.
        self.platform.set_pasted(None);
        self.platform_output(&output.platform_output);
        self.viewport_commands(output.viewport_output.values().flat_map(|v| &v.commands));
        let mut textures = std::mem::take(&mut self.pending_textures);
        textures.append(output.textures_delta);
        match surface {
            Some(surface) => {
                let ppp = output.pixels_per_point;
                let primitives = self.ctx.tessellate(output.shapes, ppp);
                self.paint(surface, &primitives, textures, ppp);
            }
            None => self.pending_textures = textures,
        }
        self.publish();
        self.publish_busy();
        true
    }

    /// Tell the shell when the app starts or stops being busy (its own work, or a write in place
    /// the shell hasn't finished), and keep a busy app running.
    fn publish_busy(&mut self) {
        let busy = self.app.busy() || self.platform.writing();
        if busy {
            let mut next = self.next_frame.lock().unwrap_or_else(PoisonError::into_inner);
            *next = (*next).min(Instant::now() + BUSY_POLL);
        }
        if busy != self.busy {
            self.busy = busy;
            self.platform.push(Request::Busy(busy));
        }
    }
}

impl<A: CraftApp> Host<A> {
    /// A touch, as egui touch events and, for the touch driving it, pointer events. `immediate` for a
    /// stylus, whose press is never held back as the start of a gesture.
    fn touch_as(&mut self, t: Touch, immediate: bool) {
        self.events.push(Event::Touch { device_id: TouchDeviceId(0), id: TouchId(t.id), phase: t.phase, pos: t.pos, force: Some(t.force) });
        let pointer = self.pointer.touch(t, Instant::now(), immediate);
        self.events.extend(pointer);
        self.wake();
    }

    /// What egui asks of the platform: the clipboard, links, the on-screen keyboard.
    fn platform_output(&mut self, output: &egui::PlatformOutput) {
        // `if let`, not `match`: egui forks differ in their other commands (selection, ...),
        // and none of those is the platform's business here.
        for command in &output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.platform.push(Request::CopyText(text.clone()));
            } else if let egui::OutputCommand::CopyImage(image) = command {
                let rgba = image.pixels.iter().flat_map(egui::Color32::to_srgba_unmultiplied).collect();
                let [w, h] = image.size;
                if let (Ok(width), Ok(height)) = (u32::try_from(w), u32::try_from(h)) {
                    self.platform.push(Request::CopyImage { width, height, rgba });
                }
            } else if let egui::OutputCommand::OpenUrl(open) = command {
                self.platform.push(Request::OpenUrl(open.url.clone()));
            }
        }
        // egui asks for IME (and so a keyboard) while a text field has focus.
        let wants_keyboard = output.ime.is_some();
        if wants_keyboard != self.keyboard_shown {
            self.keyboard_shown = wants_keyboard;
            self.platform.push(Request::Keyboard(wants_keyboard));
        }
    }
}

impl<A: CraftApp> Host<A> {
    /// What the app asks of its window: only a paste (the clipboard is read by the shell, on
    /// the user's request); the shell owns the window otherwise.
    fn viewport_commands<'a>(&mut self, mut commands: impl Iterator<Item = &'a egui::ViewportCommand>) {
        if commands.any(|c| matches!(c, egui::ViewportCommand::RequestPaste)) {
            self.platform.request_paste();
        }
    }
}

/// A touch as the shells pass it: `phase` 0 began, 1 moved, 2 ended, 3 cancelled; `force` 0..=1.
/// `None` for an unknown phase or a position that isn't a number.
pub(crate) fn decode_touch(id: u64, phase: u32, x: f32, y: f32, force: f32) -> Option<Touch> {
    let phase = match phase {
        0 => TouchPhase::Start,
        1 => TouchPhase::Move,
        2 => TouchPhase::End,
        3 => TouchPhase::Cancel,
        _ => return None,
    };
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let force = if force.is_finite() { force.clamp(0.0, 1.0) } else { 0.0 };
    Some(Touch { id, phase, pos: Pos2::new(x, y), force })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Off screen nothing draws (no GPU work in the background); a busy app still runs.
    #[test]
    fn only_an_app_on_screen_draws_and_only_a_busy_one_runs_off_screen() {
        assert_eq!(FrameKind::of(true, true, false), FrameKind::Draw);
        assert_eq!(FrameKind::of(true, true, true), FrameKind::Draw);
        assert_eq!(FrameKind::of(true, false, true), FrameKind::Tick, "suspended, the view still there (iPadOS): busy, or Save was just tapped");
        assert_eq!(FrameKind::of(false, true, true), FrameKind::Tick, "no view (Android, after surfaceDestroyed)");
        assert_eq!(FrameKind::of(true, false, false), FrameKind::Skip);
        assert_eq!(FrameKind::of(false, true, false), FrameKind::Skip);
    }

    #[test]
    fn a_view_size_is_measured_in_points_and_never_empty() {
        let size = ViewSize::new(2064, 2752, 2.0);
        assert_eq!(size.map(ViewSize::points), Some(egui::vec2(1032.0, 1376.0)));
        assert_eq!(ViewSize::new(0, 0, 2.0).map(|s| (s.width_px, s.height_px)), Some((1, 1)));
        assert_eq!(ViewSize::new(10, 10, f32::NAN), None);
        assert_eq!(ViewSize::new(10, 10, 0.0), None);
    }
}
