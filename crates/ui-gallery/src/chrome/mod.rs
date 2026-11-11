//! The viewer's chrome: the options bar, the dock of panels and the window that holds them.

mod card;
mod describe;
mod dock;
mod inspect;
mod options;
mod overview;
mod panels;
mod persist;
mod shell;
mod variants;

pub use card::{BodyShown, Card, CardEvents, CardStory, Look, StoryBody};
pub use inspect::{Inspect, Inspected, region_key};
pub use options::options_bar;
pub use overview::{Spec, SpecStory, overview_card};
pub use panels::Panels;
pub use persist::Persisted;
pub use shell::Gallery;
