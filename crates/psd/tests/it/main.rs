//! The crate's integration tests, linked into one test binary.

mod animation;
mod atn_contract;
mod atn_v12;
mod builder;
mod corpus;
mod corpus_atn;
mod layer_block_order;
mod malformed;
mod merged_recovery;
mod proptests;
mod roundtrip;
mod swatch_malformed;
mod tree;

/// A directory of the real-file corpus (feature `corpus`): under `CRAFT_PSD_CORPUS` when set (a
/// PhotoCraft checkout's `corpus` directory), else `corpus/` at the workspace root.
#[cfg(feature = "corpus")]
fn corpus_dir(sub: &str) -> std::path::PathBuf {
    let root = std::env::var_os("CRAFT_PSD_CORPUS")
        .map_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), std::path::PathBuf::from);
    root.join(sub)
}

/// The self dev-dependency builds these tests with `test-support`, the feature that gates
/// `craft_psd::fixtures` and `craft_psd::testgen`.
#[test]
fn tests_build_with_the_test_support_feature() {
    const { assert!(cfg!(feature = "test-support")) };
    let file = craft_psd::testgen::small(craft_psd::Version::Psd, craft_psd::Compression::Raw);
    assert_eq!(file.header.version, craft_psd::Version::Psd);
    let _sample: fn(u32, u32, u32, bool) -> u8 = craft_psd::fixtures::sample;
}

mod atn_sidecar;
mod csh;
mod metadata_resources;
mod resource_limits;
mod tbr;
