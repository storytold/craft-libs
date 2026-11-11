//! The fixture, the suite runner and the story helpers.
//!
//! `Fixture` shows a closure or a story over [`Frames`]; `suite` runs the shared cases over every registered type.
//!
//! The registration half of the kit: `#[trial]` functions list themselves in [`TRIALS`], and [`run`],
//! every test executable's runner, collects them. The fixture half drives frames ([`Frames`]) and reaches the runtime's
//! crate-private services the trials check.

pub mod access;
pub mod bypass;
pub mod contract;
pub mod dump;
pub mod fixture;
pub mod focus;
pub mod frames;
pub mod index;
pub mod input;
mod key_trials;
pub mod layout;
pub mod memory;
pub mod pictures;
pub mod pointer;
pub mod relations;
pub mod sensing;
#[cfg(all(feature = "snapshot", not(target_arch = "wasm32")))]
pub mod snapshot;
mod stage;
mod stories;
pub mod story;
pub mod style;
pub mod suite;
pub mod surface;
pub mod text;
pub mod wheel;
pub mod windows;

pub use crate::contract::{Exemption, PatternKind, PatternProbe, Probe, Registration};
#[cfg(feature = "testing")]
pub use crate::runtime::layout_memory::forget_kept as forget_measurements;
pub use crate::runtime::measure::{
    active as is_measuring, body_runs as measuring_runs, last_pass_body_runs as last_pass_measuring_runs, run as measuring_run, take_effects,
    widest_runs,
};
pub use craft_ui_macros::{story, trial};
pub use fixture::{Fixture, Impure, Room, Target};
pub use frames::Frames;
pub use stage::{stage, stage_fitting};
#[doc(hidden)]
pub use story::{
    CollectionKind, Drive, EditKind, Grip, Required, Story, StoryData, StoryOutput, StoryRegistration, StorySnapshot, show_as_only_item, stories,
};
pub use windows::install_spec;

/// One registered trial.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Trial {
    /// The module path of the function, crate name first.
    pub module: &'static str,
    /// The function's name.
    pub name: &'static str,
    /// The trial; it fails by panicking.
    pub run: fn(),
    /// Whether it runs only when asked for (`--ignored`): a fixture that another trial starts as a subprocess.
    pub ignored: bool,
}

impl Trial {
    /// A registration, as `#[trial]` writes it.
    pub const fn new(module: &'static str, name: &'static str, run: fn()) -> Trial {
        Trial { module, name, run, ignored: false }
    }

    /// The registration of an `#[ignore]`d trial, as `#[trial]` writes it.
    pub const fn ignored(module: &'static str, name: &'static str, run: fn()) -> Trial {
        Trial { module, name, run, ignored: true }
    }

    /// The name filters match: the module path below the crate, then the function.
    pub fn full_name(&self) -> String {
        match self.module.split_once("::") {
            Some((_, below)) => format!("{below}::{}", self.name),
            None => self.name.to_owned(),
        }
    }
}

inventory::collect!(Trial);

/// Every `#[trial]` linked into the running binary, by full name.
pub fn trials() -> Vec<&'static Trial> {
    let mut all: Vec<_> = inventory::iter::<Trial>().collect();
    all.sort_by_key(|trial| trial.full_name());
    all
}

/// Runs a test executable and exits: the `#[trial]`s declared in the crate whose root module is `crate_root` or in
/// `package`'s library crate, and the suite's trials over `registered`. Every test executable's `main` calls it, with
/// its own `module_path!()`.
pub fn run(crate_root: &str, package: &'static str, registered: &[&'static Registration]) -> ! {
    let library = package.replace('-', "_");
    let own = |module: &str| {
        [crate_root, library.as_str()].into_iter().any(|root| module.strip_prefix(root).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")))
    };
    let mut tests: Vec<libtest_mimic::Trial> = trials()
        .into_iter()
        .filter(|trial| own(trial.module))
        .map(|trial| {
            let run = trial.run;
            libtest_mimic::Trial::test(trial.full_name(), move || {
                run();
                Ok(())
            })
            .with_ignored_flag(trial.ignored)
        })
        .collect();
    tests.extend(suite::trials(package, registered));
    // A trial that re-runs its executable as a stand-in for another program passes that program's arguments after
    // `--`; they are the stand-in's, not the runner's.
    let args = libtest_mimic::Arguments::from_iter(std::env::args().take_while(|arg| arg != "--"));
    libtest_mimic::run(&args, tests).exit()
}

/// Every registration declared in the library crate of the package `crate_name` and linked into the running
/// binary, by module and type name. Types declared in the package's integration binaries are fixtures, not the package's.
pub fn registered_in(crate_name: &str) -> Vec<&'static Registration> {
    let library = crate_name.replace('-', "_");
    crate::contract::registrations()
        .into_iter()
        .filter(|registration| registration.crate_name() == crate_name && registration.crate_target() == library)
        .collect()
}

/// Writes `pub fn registered()` for the invoking package: its library crate's registrations in the running binary.
/// A crate invokes it once, in its `testing` module, and its integration binary calls the function by name. The
/// call references the library, so the linker includes it; the registrations are constructors in the library's
/// object files, which run as long as those files are linked.
#[macro_export]
macro_rules! testing_registered {
    () => {
        /// Every registration of this package linked into the running binary.
        pub fn registered() -> ::std::vec::Vec<&'static $crate::testing::Registration> {
            $crate::testing::registered_in(env!("CARGO_PKG_NAME"))
        }
    };
}

testing_registered!();

#[doc(hidden)]
pub mod __private {
    pub use inventory;
}
