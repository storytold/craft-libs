//! Tests of the codecs ported from FilmCraft and EffectCraft, built as one test binary: each file
//! is a module, as in their original crates.
// Index loops follow the specifications, as in FilmCraft (which allows this lint workspace-wide).
#![allow(clippy::needless_range_loop)]

use craft_media_testkit as common;
#[macro_use]
mod testkit;

mod aac;
mod ac3_encoder_oracle;
mod ac3_oracle;
mod alac;
mod concealment;
mod mp3_decoder_oracle;
mod mp3_encoder_oracle;
mod mpa_oracle;
mod opus;
mod opusenc;
mod routing_oracle;
mod track_decoder;
