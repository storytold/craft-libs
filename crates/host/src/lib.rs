#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod files;
pub mod queue;
#[cfg(test)]
mod queue_tests;
mod services;
pub mod target;

use std::sync::Arc;

use craft_i18n::{I18n, Lang, Words};

pub use files::{
    FileError, FileFilter, Files, NoPickFiles, PendingPick, PickAnswer, PickId, PickKind, PickRequest, Picked, ScriptedFiles, WriteIntent,
};
#[cfg(not(target_arch = "wasm32"))]
pub use services::ThreadJobs;
pub use services::{AppInfo, Commands, InlineJobs, Jobs, ManualJobs, MemorySections, NoCommands, Section, SectionStore, Wake};
#[cfg(feature = "ui")]
pub use ui::HostScope;

/// The app's shared services, built once at startup and installed on the egui context.
/// Every library feature reads them with `Host::of(ui)`; none takes them as parameters.
#[derive(Clone)]
pub struct Host {
    app: AppInfo,
    icon_png: Option<&'static [u8]>,
    i18n: Option<Arc<I18n>>,
    lang: Lang,
    settings: Arc<dyn SectionStore>,
    commands: Arc<dyn Commands>,
    jobs: Arc<dyn Jobs>,
    files: Arc<dyn Files>,
    /// Unique per host built (`generation`).
    generation: u64,
}

/// A generation no host built before has.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Collects a [`Host`]'s services; each one defaults when not set.
pub struct HostBuilder {
    app: AppInfo,
    icon_png: Option<&'static [u8]>,
    i18n: Option<Arc<I18n>>,
    lang: Option<Lang>,
    settings: Option<Arc<dyn SectionStore>>,
    commands: Option<Arc<dyn Commands>>,
    jobs: Option<Arc<dyn Jobs>>,
    files: Option<Arc<dyn Files>>,
}

impl Host {
    /// Starts a host for `app`; every service not set on the builder gets its default.
    pub fn builder(app: AppInfo) -> HostBuilder {
        HostBuilder { app, icon_png: None, i18n: None, lang: None, settings: None, commands: None, jobs: None, files: None }
    }

    /// Which host this is: a host and its clones hold the same services and language, and one built anew
    /// ([`HostBuilder::build`], [`Host::with_lang`], [`Host::with_files`]) gets a new generation. What the services
    /// hold (settings sections, catalogs) changes inside them without changing the generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The app's name and version.
    pub fn app(&self) -> &AppInfo {
        &self.app
    }

    /// The app's icon as PNG bytes ([`HostBuilder::icon_png`]); `None` when the app gave none.
    pub fn icon_png(&self) -> Option<&'static [u8]> {
        self.icon_png
    }

    /// The app's catalogs; `None` until the app uses craft-i18n (library words are English then).
    pub fn i18n(&self) -> Option<&I18n> {
        self.i18n.as_deref()
    }

    /// [`Host::i18n`] as the shared handle, for code that keeps the catalogs (craft-ui's `Words`).
    pub fn i18n_shared(&self) -> Option<&Arc<I18n>> {
        self.i18n.as_ref()
    }

    /// The interface language.
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// The catalogs' words in the interface language, English without catalogs: the words craft-ui's env shows, for
    /// code that runs without a `Ui` (a command, a file export).
    pub fn words(&self) -> Words {
        match &self.i18n {
            Some(i18n) => Words::from_i18n(Arc::clone(i18n), self.lang),
            None => Words::english(),
        }
    }

    /// The interface language's code, lowercase BCP 47 (`"en"` without catalogs): what picks its text folding and sort
    /// order.
    pub fn language_code(&self) -> &str {
        self.i18n.as_ref().map_or("en", |i18n| i18n.code(self.lang))
    }

    /// The store of library features' settings sections.
    pub fn settings(&self) -> &dyn SectionStore {
        &*self.settings
    }

    /// Command labels and hints by command id.
    pub fn commands(&self) -> &dyn Commands {
        &*self.commands
    }

    /// The file service: pickers, reads and writes (`craft_dialogs::FileService` on native).
    pub fn files(&self) -> &dyn Files {
        &*self.files
    }

    /// The background job service.
    pub fn jobs(&self) -> &dyn Jobs {
        &*self.jobs
    }

    /// [`Host::jobs`] as the shared handle, for code that keeps it (a
    /// [`JobQueue`](queue::JobQueue) spawns its lanes on it).
    pub fn jobs_shared(&self) -> &Arc<dyn Jobs> {
        &self.jobs
    }

    /// A copy in another language; install it to switch the interface.
    pub fn with_lang(&self, lang: Lang) -> Host {
        Host { lang, generation: next_generation(), ..self.clone() }
    }

    /// A copy with another file service; install it to switch the pickers.
    pub fn with_files(&self, files: Arc<dyn Files>) -> Host {
        Host { files, generation: next_generation(), ..self.clone() }
    }

    /// The stored value of section `S`, or `S::default()` when it is missing or does not fit `S`.
    pub fn setting<S: Section>(&self) -> S {
        self.settings.read(S::KEY).and_then(|table| table.try_into().ok()).unwrap_or_default()
    }

    /// Stores `value` as section `S`.
    pub fn set_setting<S: Section>(&self, value: &S) {
        if let Ok(table) = toml::Table::try_from(value) {
            self.settings.write(S::KEY, table);
        }
    }
}

impl HostBuilder {
    /// The app's icon as PNG bytes, its 128 px app icon: the library dialogs that name the app (the update dialogs)
    /// show it beside the name. Default: none; those dialogs then show the name's first letter in its place.
    pub fn icon_png(mut self, png: &'static [u8]) -> Self {
        self.icon_png = Some(png);
        self
    }

    /// Default: none (English).
    pub fn i18n(mut self, i18n: Arc<I18n>) -> Self {
        self.i18n = Some(i18n);
        self
    }

    /// Default: the catalogs' system language, else English.
    pub fn lang(mut self, lang: Lang) -> Self {
        self.lang = Some(lang);
        self
    }

    /// Default: [`MemorySections`].
    pub fn settings(mut self, settings: Arc<dyn SectionStore>) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Default: [`NoCommands`] (English labels).
    pub fn commands(mut self, commands: Arc<dyn Commands>) -> Self {
        self.commands = Some(commands);
        self
    }

    /// Default: `ThreadJobs`; [`InlineJobs`] on wasm32.
    pub fn jobs(mut self, jobs: Arc<dyn Jobs>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    /// Default: [`NoPickFiles`] (every picker answers Cancelled; plain file system reads and writes).
    pub fn files(mut self, files: Arc<dyn Files>) -> Self {
        self.files = Some(files);
        self
    }

    /// Builds the host, filling every unset service with its default.
    pub fn build(self) -> Host {
        let lang = self.lang.or_else(|| self.i18n.as_ref().map(|i18n| i18n.system_lang())).unwrap_or(Lang::EN);
        Host {
            app: self.app,
            icon_png: self.icon_png,
            i18n: self.i18n,
            lang,
            settings: self.settings.unwrap_or_else(|| Arc::new(MemorySections::default())),
            commands: self.commands.unwrap_or_else(|| Arc::new(NoCommands)),
            jobs: self.jobs.unwrap_or_else(default_jobs),
            files: self.files.unwrap_or_else(|| Arc::new(NoPickFiles::default())),
            generation: next_generation(),
        }
    }

    /// `build().install(ctx)`.
    #[cfg(feature = "ui")]
    pub fn install(self, ctx: &egui::Context) {
        self.build().install(ctx);
    }
}

/// The jobs a host runs when its builder sets none, and what a library feature runs its background work on when no
/// host is installed: [`ThreadJobs`] on native targets. On wasm32 it is [`InlineJobs`]; a [`queue::JobQueue`] there
/// has no lanes (`QueueOptions::default_lanes` is 0), so its owner runs one job per pass with `run_inline` and no
/// job runs inside the call that queued it.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_jobs() -> Arc<dyn Jobs> {
    Arc::new(ThreadJobs)
}

/// The jobs a host runs when its builder sets none; see the native definition.
#[cfg(target_arch = "wasm32")]
pub fn default_jobs() -> Arc<dyn Jobs> {
    Arc::new(InlineJobs)
}

#[cfg(feature = "ui")]
mod ui {
    use std::sync::Arc;

    use egui::{Context, Plugin, Ui};

    use super::Host;

    /// The installed host, kept as a plugin so that a reset of the context's memory does not drop it.
    #[derive(Default)]
    struct Installed(Option<Arc<Host>>);

    impl Plugin for Installed {
        fn debug_name(&self) -> &'static str {
            "craft_host::Host"
        }
    }

    /// Puts the host that was installed before back when it drops; see [`Host::install_scoped`].
    #[must_use = "the host stays installed only while the scope is held"]
    pub struct HostScope {
        ctx: Context,
        before: Option<Arc<Host>>,
    }

    impl Drop for HostScope {
        fn drop(&mut self) {
            self.ctx.plugin_or_default::<Installed>().lock().0 = self.before.take();
        }
    }

    impl Host {
        /// Stores the host in a context plugin, so a memory reset leaves it installed.
        pub fn install(self, ctx: &Context) {
            ctx.plugin_or_default::<Installed>().lock().0 = Some(Arc::new(self));
        }

        /// Installs the host until the returned scope drops, then puts back the host that was
        /// installed before (or none). A story swaps a service this way for the time it draws, so
        /// the next story sees the app's own host again.
        pub fn install_scoped(self, ctx: &Context) -> HostScope {
            let before = ctx.plugin_or_default::<Installed>().lock().0.replace(Arc::new(self));
            HostScope { ctx: ctx.clone(), before }
        }

        /// The installed host, or `None`. Library words and numbers use this and fall back to
        /// English, so tests, stories and apps without a Host keep working.
        pub fn try_of(ui: &Ui) -> Option<Arc<Host>> {
            Host::try_of_ctx(ui.ctx())
        }

        /// [`Host::try_of`] from an egui context.
        pub fn try_of_ctx(ctx: &Context) -> Option<Arc<Host>> {
            ctx.plugin_opt::<Installed>().and_then(|installed| installed.lock().0.clone())
        }

        /// The installed host. Panics with "Host::install was not called"; only for features that
        /// cannot work without a service.
        #[allow(clippy::panic, reason = "a missing Host is a programming error of the app")]
        pub fn of(ui: &Ui) -> Arc<Host> {
            Host::try_of(ui).expect("Host::install was not called")
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    const APP: AppInfo = AppInfo { id: "test", name: "Test", version: "1.0" };

    /// A write to a path set with `failing_writes` returns the refused-write error and keeps what the path held; every
    /// other path writes: a refusal that wrote anyway, or that refused every path, would test a failed save on the
    /// wrong path.
    #[test]
    fn failing_writes_refuse_only_their_path() {
        let files = ScriptedFiles::new().with_file("/docs/a.pdf", b"old".to_vec()).failing_writes("/docs/a.pdf");
        let refused = files.write(std::path::Path::new("/docs/a.pdf"), b"new", WriteIntent::Save);
        assert_eq!(refused, Err(FileError::PermissionDenied("/docs/a.pdf".into())));
        assert_eq!(files.file(std::path::Path::new("/docs/a.pdf")).as_deref(), Some(&b"old"[..]), "the refused write kept the old bytes");
        assert_eq!(files.write(std::path::Path::new("/docs/b.pdf"), b"new", WriteIntent::Save), Ok(false));
        assert_eq!(files.file(std::path::Path::new("/docs/b.pdf")).as_deref(), Some(&b"new"[..]));
    }

    /// The written paths list each write that took, in order, and nothing else: a list that took in a refused write or a
    /// file the test put there would let "nothing was written outside the folder" fail, or pass, on the wrong paths.
    #[test]
    fn written_lists_the_writes_that_took_in_order() {
        let files = ScriptedFiles::new().with_file("/docs/kept.pdf", b"old".to_vec()).failing_writes("/docs/refused.pdf");
        for path in ["/out/b.pdf", "/docs/refused.pdf", "/out/a.pdf", "/out/b.pdf"] {
            let _ = files.write(std::path::Path::new(path), b"new", WriteIntent::Save);
        }
        let written: Vec<std::path::PathBuf> = ["/out/b.pdf", "/out/a.pdf", "/out/b.pdf"].map(Into::into).into();
        assert_eq!(files.written(), written);
    }

    /// `Files::exists` reports files already present or written through the service and ignores refused writes:
    /// batch writers rely on it to pick a fresh name instead of replacing an existing file.
    #[test]
    fn exists_reports_present_and_written_files() {
        let existing = std::path::Path::new("/batch/notes.pdf");
        let missing = std::path::Path::new("/batch/new.pdf");
        let refused = std::path::Path::new("/batch/refused.pdf");
        let files = ScriptedFiles::new().with_file(existing, b"keep".to_vec()).failing_writes(refused);
        assert!(files.exists(existing), "a pre-existing scripted file exists");
        assert!(!files.exists(missing), "a path not in the service does not exist");
        assert!(files.write(refused, b"no", WriteIntent::SaveAs).is_err());
        assert!(!files.exists(refused), "a refused write does not create an entry");
        assert_eq!(files.write(missing, b"ok", WriteIntent::SaveAs), Ok(false));
        assert!(files.exists(missing), "an accepted write exists afterwards");

        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = std::env::temp_dir().join(format!("craft-host-exists-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let on_disk = dir.join("present.pdf");
            let absent = dir.join("absent.pdf");
            let _ = std::fs::remove_file(&on_disk);
            let _ = std::fs::remove_file(&absent);
            let disk = NoPickFiles::default();
            assert!(!disk.exists(&on_disk));
            assert!(!disk.exists(&absent));
            assert_eq!(disk.write(&on_disk, b"data", WriteIntent::SaveAs), Ok(false));
            assert!(disk.exists(&on_disk), "NoPickFiles reports a written file on disk");
            assert!(!disk.exists(&absent), "NoPickFiles reports a missing path as absent");
            let _ = std::fs::remove_file(&on_disk);
            let _ = std::fs::remove_dir(&dir);
        }
    }

    #[cfg(feature = "ui")]
    #[test]
    fn installed_host_survives_a_memory_reset() {
        let ctx = egui::Context::default();
        assert!(Host::try_of_ctx(&ctx).is_none());
        Host::builder(APP).lang(Lang::PSEUDO).install(&ctx);
        ctx.memory_mut(|m| *m = Default::default());
        assert_eq!(Host::try_of_ctx(&ctx).map(|h| h.lang()), Some(Lang::PSEUDO));
    }
}
