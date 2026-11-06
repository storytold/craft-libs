//! Tests of the codecs ported from FilmCraft, built as one test binary: each file is a module, as
//! in their original crates.
// Index loops follow the specifications, as in FilmCraft (which allows this lint workspace-wide).
#![allow(clippy::needless_range_loop)]

use craft_media_testkit as common;
#[macro_use]
mod testkit;

mod aac;
mod concealment;
mod routing_oracle;
mod track_decoder;
