//! Renders a component's stories to PNG files, offscreen and in one process, for the old/new comparison pages.
//!
//! `cargo run -p story-render --features <crate> -- --only <Name> --out <dir>` writes, into `<dir>`:
//! `new-<Name>--<story>.png` for every story of the component, `new-<Name>.png` for the story its Overview card
//! shows (its first), `stories.tsv` and `components.tsv`. Each story runs in a card's story body (`StoryBody`) in a
//! fresh context, in ArtCraft Dark at 2 pixels per point (`--scale`) with the interface fonts, and its image is cropped to the
//! story's surface and the layers it opened. A story runs in an embedded shell, over which its dialogs open, and a story
//! whose dialog the card holds behind its button is shown with the dialog open.
//!
//! The bin links the framework and only the domain crates its features name (`--features color` for craft-color's
//! components; `--features apps` for every crate, as the gallery links them), so a change in one crate rebuilds that
//! crate and this bin, not the gallery.
//!
//! `--card` renders whole Overview cards instead, as the gallery builds them (`overview_card`): one card per story
//! group, selected, `--width` wide, its dialogs open, cropped to the card and the layers it opened (to the opened
//! dialogs alone when it has any). The files are `new-<Name>--<group>.png` and `new-<Name>.png` for the first group
//! (the Overview's default card); the tables list the group's name as the story and title.
//!
//! Options: `--only Name,Name` renders those components only (all registered components without it); `--story name`
//! renders the story (with `--card`, the group) of that name only; `--threads N` (default: the cores, at most 8);
//! `--width W` and `--room H` set the card's story width (with `--card`, the card's width) and the height a filling
//! story gets, in points; `--scale S` the images' pixels per point (1 matches the old gallery's shots, so a canvas
//! document shows at the same size on both sides). `stories.tsv` lists the scale of every image.

use craft_behavior::ColumnWidth;
use craft_ui::contract::{Pad, Scroll, ShowContainer as _, Surface};
use craft_ui::fonts::{I18nScripts, UiFonts};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::{RootEnv, WindowKey};
use craft_ui::shell::AppShell;
use craft_ui::testing::snapshot::{Renderer, RgbaImage, layer_rects};
use craft_ui::testing::{Frames, StoryRegistration};
use craft_ui_gallery::WINDOW;
use craft_ui_gallery::chrome::{Spec, StoryBody, overview_card};
use craft_ui_gallery::viewer::{CardState, Catalog, Cells, GallerySettings, entry_of};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The pixels per point of the images unless `--scale` gives another.
const PIXELS_PER_POINT: f32 = 2.0;
/// The width of an Overview card's story body at the gallery's default window (the room a story on another surface than
/// the card's panel gets), measured in the running gallery: the Overview's grid takes it from the dock's centre panel at
/// run time, and no function gives it for a window size. The old gallery's cards were this wide outside, so `--card`
/// uses it as the card's width, and both sides of the comparison match.
const WIDTH: f32 = 342.0;
/// The Overview's one column of cards.
const ONE: &[ColumnWidth] = &[ColumnWidth::fill(1.0)];
/// The height a filling story gets in that Overview: the visible height of its card view, measured the same way.
const ROOM: f32 = 692.0;

struct Options {
    out: PathBuf,
    only: Option<Vec<String>>,
    story: Option<String>,
    threads: usize,
    width: f32,
    room: f32,
    /// The pixels per point of the images.
    scale: f32,
    /// Whole Overview cards, one per story group, instead of story bodies.
    card: bool,
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let mut options =
        Options { out: PathBuf::new(), only: None, story: None, threads, width: WIDTH, room: ROOM, scale: PIXELS_PER_POINT, card: false };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => options.out = PathBuf::from(value()?),
            "--only" => options.only = Some(value()?.split(',').map(str::to_owned).collect()),
            "--story" => options.story = Some(value()?),
            "--threads" => options.threads = value()?.parse().map_err(|_| "--threads takes a number".to_owned())?,
            "--width" => options.width = value()?.parse().map_err(|_| "--width takes points".to_owned())?,
            "--room" => options.room = value()?.parse().map_err(|_| "--room takes points".to_owned())?,
            "--scale" => options.scale = value()?.parse().map_err(|_| "--scale takes pixels per point".to_owned())?,
            "--card" => options.card = true,
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if options.out.as_os_str().is_empty() {
        return Err("--out <dir> is required".to_owned());
    }
    Ok(options)
}

/// What a job renders.
enum Shown {
    /// One story in a card's story body.
    Story(&'static StoryRegistration),
    /// The Overview card of one story group.
    Card(Spec),
}

/// One story or card to render.
struct Job {
    /// The component's display name.
    name: String,
    level: &'static str,
    crate_name: &'static str,
    shown: Shown,
    surface: Surface,
    /// The story's name, or the card's group name.
    story: String,
    /// The story's caption, or the card's group name.
    title: String,
    /// The story or group the component's Overview card shows by default.
    first: bool,
}

/// What became of one story.
struct Done {
    status: String,
    size: Option<(u32, u32)>,
}

fn jobs(options: &Options) -> Vec<Job> {
    let catalog = Catalog::linked();
    let mut jobs = Vec::new();
    for component in &catalog.components {
        if options.only.as_ref().is_some_and(|only| !only.iter().any(|name| *name == component.name)) {
            continue;
        }
        let Some(entry) = entry_of(component.type_name) else { continue };
        if options.card {
            for (at, group) in component.groups.iter().enumerate() {
                if options.story.as_ref().is_some_and(|name| *name != group.name) {
                    continue;
                }
                let Some(mut spec) = overview_card(&catalog, component, at) else { continue };
                let Some(crate_name) = spec.stories.first().map(|story| story.story.crate_name) else { continue };
                spec.selected = true;
                jobs.push(Job {
                    name: component.name.clone(),
                    level: entry.level.title(),
                    crate_name,
                    shown: Shown::Card(spec),
                    surface: entry.surface,
                    story: group.name.clone(),
                    title: group.name.clone(),
                    first: at == 0,
                });
            }
            continue;
        }
        let stories = catalog.stories_of(component.type_name).iter().map(|&at| &catalog.entries[at]);
        for (at, story) in stories.enumerate() {
            if options.story.as_ref().is_some_and(|name| *name != story.registration.story.name) {
                continue;
            }
            jobs.push(Job {
                name: component.name.clone(),
                level: entry.level.title(),
                crate_name: story.registration.crate_name,
                shown: Shown::Story(story.registration),
                surface: entry.surface,
                story: story.registration.story.name.to_owned(),
                title: story.registration.story.caption(&component.name),
                first: at == 0,
            });
        }
    }
    jobs
}

/// The root values the stories are shown under: the gallery's defaults in dark mode, which give ArtCraft.
fn root_env() -> RootEnv {
    let mut env = RootEnv::default();
    let mut settings = GallerySettings::default();
    settings.mode = "dark".to_owned();
    settings.restore_env(&mut env);
    env
}

/// `show` at the screen's top left, `width` wide, until it settles: the frames, then a second past every fade, then the
/// frames again. It runs in a shell as large as the gallery's window, over which its dialogs open as over a gallery
/// view, so a dialog or workspace gets the room it gets in the gallery, not the card's width.
fn settle(frames: &mut Frames, width: f32, show: &mut dyn FnMut(&mut Ui)) {
    let window = Rect::from_min_size(Pos2::ZERO, Vec2::from(WINDOW));
    for at in 0..8 {
        if at == 4 {
            frames.advance(1.0);
        }
        frames.frame(Vec::new(), |ui| {
            craft_ui::testing::layout::in_region(ui, window, |ui| {
                AppShell::embedded(WindowKey::new("story-render"))
                    .body(|ui, _, _: &mut ()| {
                        let view = ui.available_rect_before_wrap();
                        let column = Rect::from_min_size(view.min, Vec2::new(width, view.height()));
                        craft_ui::testing::layout::in_region(ui, column, |ui| show(ui));
                    })
                    .show(ui, &mut ())
            });
        });
    }
}

/// The story in a card's body, with its dialog open when the card holds one behind its button: the rect it showed in.
fn story(frames: &mut Frames, story: &'static StoryRegistration, job: &Job, options: &Options) -> Option<Rect> {
    let body = StoryBody::new("sheet", story, job.surface, options.room, Pad::Regular);
    let mut state = CardState::default();
    let mut flow = None;
    settle(frames, options.width, &mut |ui| flow = body.show(ui, &mut state).flow);
    if state.dialog.hidden {
        state.show_dialog();
        settle(frames, options.width, &mut |ui| flow = body.show(ui, &mut state).flow);
    }
    flow
}

/// The Overview card of `spec`, with every dialog its stories hold behind a button open: the card's rect, and whether
/// a dialog was opened.
fn card(frames: &mut Frames, spec: &Spec, options: &Options) -> (Option<Rect>, bool) {
    let stories = spec.card_stories();
    let card = spec.card(&stories, "sheet".to_owned(), options.room);
    let mut cells = Cells::default();
    let mut rect = None;
    // In a one-column grid row in a vertical scroll, as the Overview holds its cards: a filling card is as tall as its
    // row's content.
    let show = |ui: &mut Ui, cells: &mut Cells| {
        Stack::new()
            .fill()
            .scroll(Scroll::vertical())
            .show(ui, |ui, _| Grid::new(ONE).fill_width().show(ui, |ui, _| card.show(ui, cells).rect).inner.flatten())
            .inner
            .flatten()
    };
    settle(frames, options.width, &mut |ui| rect = show(ui, &mut cells));
    let mut opened = false;
    for state in cells.values_mut().filter(|state| state.dialog.hidden) {
        state.show_dialog();
        opened = true;
    }
    if opened {
        settle(frames, options.width, &mut |ui| rect = show(ui, &mut cells));
    }
    (rect, opened)
}

/// The story or card rendered: its image cropped to what it showed and the layers it opened, in pixels.
fn render(job: &Job, renderer: &mut Renderer, env: &RootEnv, options: &Options) -> Result<RgbaImage, String> {
    let mut frames = Frames::new();
    UiFonts::new(&I18nScripts).install(&frames.ctx);
    frames.root(env.clone());
    frames.screen(Vec2::from(WINDOW));
    frames.pixels_per_point(options.scale);
    frames.record_paint();
    let (shown, dialog) = match &job.shown {
        Shown::Story(registration) => (story(&mut frames, registration, job, options), false),
        Shown::Card(spec) => card(&mut frames, spec, options),
    };
    let layers = layer_rects(&frames.ctx).into_iter().fold(Rect::NOTHING, Rect::union);
    // An opened card dialog is cropped alone, as the old gallery's dialog shots are.
    let crop = if dialog && layers.is_positive() { layers } else { shown.map_or(layers, |shown| layers.union(shown)) };
    let paint = frames.take_paint().ok_or("no paint recorded")?;
    let image = renderer.render(&frames.ctx, paint);
    renderer.finish();
    let image = image?;
    if !crop.is_positive() {
        return Err("blank: the story showed nothing".to_owned());
    }
    let (w, h) = (image.width(), image.height());
    let px = |v: f32, max: u32| ((v * options.scale).max(0.0) as u32).min(max);
    let (x0, y0) = (px(crop.min.x.floor(), w), px(crop.min.y.floor(), h));
    let (x1, y1) = (px(crop.max.x.ceil(), w), px(crop.max.y.ceil(), h));
    if x1 <= x0 || y1 <= y0 {
        return Err("blank: the story's rect is off screen".to_owned());
    }
    Ok(image::imageops::crop_imm(&image, x0, y0, x1 - x0, y1 - y0).to_image())
}

thread_local! {
    /// The message of the last panic on this thread.
    static PANIC: RefCell<String> = const { RefCell::new(String::new()) };
    /// A story is running on this thread.
    static IN_STORY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn file_name(job: &Job) -> String {
    let story: String = job.story.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
    format!("new-{}--{story}.png", job.name)
}

fn run(job: &Job, renderer: &mut Renderer, env: &RootEnv, options: &Options) -> Done {
    IN_STORY.with(|running| running.set(true));
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(job, renderer, env, options)));
    IN_STORY.with(|running| running.set(false));
    let image = match rendered {
        Ok(Ok(image)) => image,
        Ok(Err(error)) => return Done { status: error, size: None },
        Err(_) => return Done { status: format!("panic: {}", PANIC.with(|message| message.borrow().clone())), size: None },
    };
    let save = |name: &str| image.save(options.out.join(name)).map_err(|error| format!("cannot write {name}: {error}"));
    let saved = save(&file_name(job)).and_then(|()| if job.first { save(&format!("new-{}.png", job.name)) } else { Ok(()) });
    match saved {
        Ok(()) => Done { status: "ok".to_owned(), size: Some((image.width(), image.height())) },
        Err(error) => Done { status: error, size: None },
    }
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\n'], " ")
}

fn write_tables(out: &Path, jobs: &[Job], done: &[Option<Done>], scale: f32) -> std::io::Result<()> {
    let mut stories = String::from("name\tlevel\tcrate\tstory\ttitle\tfile\tstatus\twidth\theight\tscale\n");
    let mut components = String::from("name\tlevel\tcrate\tstory used\tstatus\n");
    for (job, done) in jobs.iter().zip(done) {
        let (status, size) = done.as_ref().map_or(("not run".to_owned(), None), |done| (clean(&done.status), done.size));
        let (w, h) = size.map_or((String::new(), String::new()), |(w, h)| (w.to_string(), h.to_string()));
        let story = &job.story;
        let title = clean(&job.title);
        let _ =
            writeln!(stories, "{}\t{}\t{}\t{story}\t{title}\t{}\t{status}\t{w}\t{h}\t{scale}", job.name, job.level, job.crate_name, file_name(job));
        if job.first {
            let _ = writeln!(components, "{}\t{}\t{}\t{story}\t{status}", job.name, job.level, job.crate_name);
        }
    }
    std::fs::write(out.join("stories.tsv"), stories)?;
    std::fs::write(out.join("components.tsv"), components)
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "story-render: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&options.out) {
        let _ = writeln!(std::io::stderr(), "story-render: cannot create {}: {error}", options.out.display());
        return ExitCode::FAILURE;
    }
    // A story's panic is its status; any other panic goes to the default hook.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_STORY.with(std::cell::Cell::get) {
            default(info);
            return;
        }
        let message = info.payload().downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| info.payload().downcast_ref::<String>().cloned());
        let at = info.location().map(|at| format!(" at {}:{}", at.file(), at.line())).unwrap_or_default();
        PANIC.with(|last| *last.borrow_mut() = clean(&format!("{}{at}", message.unwrap_or_default())));
    }));
    let jobs = jobs(&options);
    if jobs.is_empty() {
        let _ = writeln!(std::io::stderr(), "story-render: no story matches; is the component's crate among this bin's features?");
        return ExitCode::FAILURE;
    }
    let done: Vec<Mutex<Option<Done>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..options.threads.max(1).min(jobs.len()) {
            scope.spawn(|| {
                let env = root_env();
                let mut renderer = Renderer::new();
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(at) else { break };
                    let result = run(job, &mut renderer, &env, &options);
                    if let Ok(mut slot) = done[at].lock() {
                        *slot = Some(result);
                    }
                }
            });
        }
    });
    let done: Vec<Option<Done>> = done.into_iter().map(|slot| slot.into_inner().ok().flatten()).collect();
    if let Err(error) = write_tables(&options.out, &jobs, &done, options.scale) {
        let _ = writeln!(std::io::stderr(), "story-render: cannot write the tables: {error}");
        return ExitCode::FAILURE;
    }
    let ok = done.iter().flatten().filter(|done| done.status == "ok").count();
    let components = jobs.iter().filter(|job| job.first).count();
    let what = if options.card { "cards" } else { "stories" };
    let _ = writeln!(std::io::stdout(), "story-render: {ok} of {} {what} of {components} components rendered", jobs.len());
    ExitCode::SUCCESS
}
