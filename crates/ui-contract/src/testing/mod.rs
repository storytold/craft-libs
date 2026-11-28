//! The fixture, the suite runner and the story helpers.
//!
//! `Fixture` shows a closure or a story over [`Frames`]; `suite` runs the shared cases over every registered type.
//!
//! The registration half of the kit: `#[trial]` functions list themselves in `TRIALS`, and [`run`],
//! every test executable's runner, collects them. The fixture half drives frames ([`Frames`]) and reaches the runtime's
//! crate-private services the trials check.

pub mod access;
pub mod bypass;
pub mod contract;
pub mod dump;
pub mod face_runs;
pub mod fixture;
pub mod focus;
pub mod frames;
#[cfg(feature = "suite")]
pub(crate) mod host_key_trials;
#[cfg(feature = "suite")]
pub(crate) mod icon_trials;
#[cfg(feature = "suite")]
pub mod index;
pub mod input;
#[cfg(feature = "suite")]
pub(crate) mod key_trials;
pub mod layout;
pub mod memory;
pub mod pictures;
pub mod pointer;
#[cfg(feature = "suite")]
mod registry;
pub mod relations;
#[cfg(feature = "suite")]
pub(crate) mod replay_trials;
pub mod sensing;
#[cfg(all(feature = "snapshot", not(target_arch = "wasm32")))]
pub mod snapshot;
mod stage;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub mod story;
pub mod style;
#[cfg(feature = "suite")]
pub mod suite;
pub mod surface;
pub mod text;
pub mod wheel;
pub mod windows;

pub use crate::contract::{Exemption, PatternKind, PatternProbe, Probe, Registration};
#[cfg(feature = "testing")]
pub use crate::fonts::document::set_budget as document_face_budget;
pub use crate::runtime::audit::{in_run as in_audit_run, set as audit_replays};
#[cfg(feature = "testing")]
pub use crate::runtime::env::env_walks;
#[cfg(feature = "testing")]
pub use crate::runtime::layout_memory::forget_kept as forget_measurements;
#[cfg(feature = "testing")]
pub use crate::runtime::layout_memory::kept_leaves as kept_measurement_leaves;
pub use crate::runtime::measure::{
    MemoRun, active as is_measuring, body_runs as measuring_runs, contributions as measured_contributions, last_frame_derived_builds,
    last_frame_memo_runs, last_pass_access_builds, last_pass_body_runs as last_pass_measuring_runs, last_pass_derived_builds, last_pass_measure_asks,
    last_pass_memo_key_bytes, last_pass_memo_runs, last_pass_runs, last_pass_text_layouts, last_pass_tip_texts, least_runs as shrink_least_runs,
    row_builds, row_measures, run as measuring_run, take_effects, widest_runs,
};
#[cfg(feature = "testing")]
pub use crate::runtime::pass_state::unbound_reads as unbound_pass_state_reads;
pub use craft_ui_macros::{story, trial};
pub use fixture::{Fixture, Impure, Room, Target};
pub use frames::Frames;
#[cfg(feature = "suite")]
pub use registry::{Registered, Registry};
pub use stage::{least_stage_width, room_height, stage, stage_fitting, stage_width};
#[doc(hidden)]
pub use story::{
    CollectionKind, Drive, EditKind, Grip, Required, Story, StoryData, StoryOutput, StoryRegistration, StorySnapshot, ValueAt, show_as_only_item,
    show_as_only_item_across,
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

/// Runs a test executable and exits: the `#[trial]`s declared in the crate whose root module is `crate_root` or in
/// `package`'s library crate, and the suite's trials over `registered`. Every test executable's `main` calls it, with
/// its own `module_path!()`.
#[cfg(feature = "suite")]
pub fn run(crate_root: &str, package: &'static str, registry: &Registry) -> ! {
    if !registry.install_story_sets() {
        eprintln!("the selected story commands are invalid");
        std::process::exit(1);
    }
    let library = package.replace('-', "_");
    let own = |module: &str| {
        [crate_root, library.as_str()].into_iter().any(|root| module.strip_prefix(root).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")))
    };
    let mut tests: Vec<libtest_mimic::Trial> = registry
        .trials()
        .iter()
        .filter(|trial| own(trial.module))
        .map(|trial| {
            let run = trial.run;
            let story_sets = registry.story_selection();
            libtest_mimic::Trial::test(trial.full_name(), move || {
                crate::runtime::commands::with_story_sets(&story_sets, || {
                    run();
                    Ok(())
                })
            })
            .with_ignored_flag(trial.ignored)
        })
        .collect();
    tests.extend(suite::trials(package, registry));
    // A trial that re-runs its executable as a stand-in for another program passes that program's arguments after
    // `--`; they are the stand-in's, not the runner's.
    let args = libtest_mimic::Arguments::from_iter(std::env::args().take_while(|arg| arg != "--"));
    libtest_mimic::run(&args, tests).exit()
}

#[cfg(feature = "suite")]
craft_ui_macros::registry!();
