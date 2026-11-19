//! Exact Olympus / OM System sensor-pixel regressions on CC0 raw.pixls.us files.
//! The reference is LibRaw 0.22.2's unprocessed_raw executable, run as a black box;
//! SHA256 covers the entire row-major u16 mosaic in little-endian order, including
//! borders. Input files stay outside this repository; see docs/orf.md for acquisition
//! and the distinction between reference agreement and historical provenance.

#![cfg(feature = "orf")]

use craft_raw::{RawData, RawFormat, decode, probe_info};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// (filename, input SHA256, sensor dimensions, reference sensor SHA256)
const CASES: &[(&str, &str, (usize, usize), &str)] = &[
    (
        "orf-olympus-epl7.orf",
        "e0e138e9d33b0d2a16c0a9378582ddaac71d0b2cf07bc9ed459df275aa025c60",
        (4640, 3472),
        "ecab62e4a1e15bc08e739bbf187dbbc6190bbee44120231d8abbcad8fa6431b6",
    ),
    (
        "orf-e-3-1803.orf",
        "ba4605bbd9fe9b77971da444bcdb1698904c0ad4ebdf9dd42dd45ab255f3f6d4",
        (3720, 2800),
        "557f4496f7b31596e05ccf6ebaa656dec6ab223dd8ed6ef55442cd1b18c3a9a7",
    ),
    (
        "orf-e-410-1173.orf",
        "b545f3d831275d7febb1d02bfa22b03078eeff27fcb4c5be6806e89857bb1ed7",
        (3720, 2800),
        "6fd3d64305d4be04759d9ba0599a7ae1d8b07a287171c4f58df837580243f3fd",
    ),
    (
        "orf-e-620-1805.orf",
        "b3aff4240fc5b3f772c58f2b651c9d47904a575d5a22fcac49ed43374a5d779a",
        (4100, 3084),
        "97230d68c1b73b05f2b69f88c56c193e88392e03950218b8d35f1e26ffe44568",
    ),
    (
        "orf-e-m10-mark-iv-4126.orf",
        "ce03e936cb2e8131343e7045a3249f78477bd2ae944e121ace72058b89fef509",
        (5200, 3904),
        "59771f57c96f242bf1c80e3dc261ec8bea45fece333372b502261211ff54a90a",
    ),
    (
        "orf-e-m1markii-1993.orf",
        "0b433019e5fa61548d7eb0eb7bb90894251542c056da72355202382696d0ceb8",
        (5240, 3912),
        "09630ade7c0b48dccb11bfb4c092580eac4d9423c380b975129ef700dcadf419",
    ),
    (
        "orf-e-m1markiii-3800.orf",
        "1e7a256eff14df75055dd7fe7a68aabefb4d8ed475317e80ebf7ff77d5bcfd3f",
        (5240, 3912),
        "ddb9d5d34e8db9393d5d455a6242b10d81db097a05be6b397b6601b4bb59d19f",
    ),
    (
        "orf-e-m5-mark-ii-2857.orf",
        "f2c7a1dfbfcc6093b28373a1616569b8bd9c2d91cb708480b8312d510c57fbd6",
        (4640, 3472),
        "036fcd4de8f14b1b084919f528d1203f71c87569b24810751b45da5a0dc99967",
    ),
    (
        "orf-e-m5-mark-iii-3573.orf",
        "391f2f6ba712a517212cae53bfc9a313fc4d106d0b04d3b9af68a4ed2d5d944e",
        (10400, 7792),
        "5a19cf3a01e13ae5fd7ef238d735cc89f8907fd620f106c1c3a82140a806def9",
    ),
    (
        "orf-e-p1-5099.orf",
        "862b9d409279af0528810aa02eb5b4140d7854f9d637d4188b82ebaf4bf0d4cf",
        (4100, 3084),
        "975d4a8a9a991e7d2250e455cc3c0f99ccb65df3b46d6a90df2436ea59ea8ecb",
    ),
    (
        "orf-e-p3-241.orf",
        "540edabdd9d4e6ef8602f56443730cc3133d714dc4d3d2c607e2f0aa6be5cbcd",
        (4080, 3040),
        "911db5b569c66dd21e196badb5da8b60d2b4d51418ba4decfa1b016b3a48297a",
    ),
    (
        "orf-e-p7-5980.orf",
        "391b7a28d088f093a93d05c43b82bc8e9403579dea8cdc066d28af53a773d9c5",
        (5200, 3904),
        "b6dd128dd6b6b17b2ff25bd323d0ef8e09cbf26117e2e6a97fa7714281cfe688",
    ),
    (
        "orf-olympus-em1.orf",
        "19ba17e778ae802d4039f0e88bb1cd68825780788b8058c479067e32b9a63eda",
        (4640, 3472),
        "f66855309d3a6b19598952649c382dfbaaf92eea1581142d8a693f1988b17433",
    ),
    (
        "orf-olympus-em10iii.orf",
        "11d90cbb564ad2c660c7a8d5d81d99c2c018aa27fdec45ed6c461fa5d24f942b",
        (4640, 3472),
        "431e3081ea9bc354f93e809c413b6a0232ae8ffd13073fbb3235a787680f150a",
    ),
    (
        "orf-om-1-5283.orf",
        "bf2eef6becc0851e48231617c3e2eac27a2cdb44c75b93035c561d837952c18b",
        (5220, 3912),
        "e76d36fd88a7559d30f4e93147ed9f09b3a4573578e311b53f767bdaf4857c31",
    ),
    (
        "orf-om-1-mark-ii-7262.orf",
        "d0fe58daf2f0bd749ecb18311e55408f0e7804de8e5e6580e815eae50d834cbd",
        (5220, 3912),
        "c833242a92839771353e70a0f11620ce14f9b02f7c373f1217c2aa9c0845280c",
    ),
    (
        "orf-om-3-7796.orf",
        "1cee7deb661e45101ab3ca6c4240d24ed949b167ed423703fca15c09dd8ec176",
        (5220, 3912),
        "2cc34fdbc6d8ea292ba1bedbac534628edf43ed94d559c42ca4630ab56e3b9da",
    ),
    (
        "orf-om-5-6343.orf",
        "ee0213ee4899d677d9bd538d8cfa4d0f69bdbacd462c1cf3ca3901036cf2fce7",
        (5240, 3912),
        "c1b90d64d0cf5ea8c2fb0b50e09bef375bfffcb99117533030d5e4fecbfe7947",
    ),
    (
        "orf-om-5-mark-ii-8549.orf",
        "c6fe861982ddc18464cf6858d2700f6813cc3a17956bcb12d1179212c999c76a",
        (5240, 3912),
        "ff3224ce2d75a8cabd07dfcc8522e95462fd4f22b0ce2452a5e469a8e43fcc8f",
    ),
    (
        "orf-pen-f-2977.orf",
        "f87a5f3295aeb4d6ab27bb5b41c5e0f3a211ec9b4e3b1d9f105a650bee1a2f13",
        (10400, 7796),
        "32c3bc92092bc11e7d854d08dc9c151bd5525262072a3dc32f88b4b941ad80c6",
    ),
    (
        "orf-sh-2-792.orf",
        "6e6b9b7dd9ded7c4b364b83116c817ca2d6daac15f2e9ef7796159b08c3288a1",
        (4624, 3470),
        "21fe65f6975d951b83aba92818339d915c9b76d927810d35b7f7f05fc5b89752",
    ),
    (
        "orf-tg-6-3210.orf",
        "edd6f928bc790c79852a17fce56718d26563c7a593894d22970ef0fdb86ba45f",
        (4040, 3016),
        "8cd1c2249e6910e8680c8028401cf32efda47c11e489bcb7c2797a2fc342fb03",
    ),
    (
        "orf-tg-7-6946.orf",
        "4f7344c67bfcfcfec45f754195ba612fc2ad6312f5fab87812bf04b9e9e7d46a",
        (4040, 3016),
        "28a13068be6c25fb2116b52662f4d20b526d7ce6c53a2150dc280fbfe5a909a2",
    ),
];

#[test]
fn olympus_and_om_sensor_pixels_match_independent_reference() {
    let Some(root) = std::env::var_os("CRAFT_RAW_ORF_CORPUS").map(PathBuf::from) else {
        eprintln!("skipping external ORF corpus: set CRAFT_RAW_ORF_CORPUS to the LightCraft corpus directory");
        return;
    };
    let mut checked = 0;
    for &(name, input_hash, sensor, pixel_hash) in CASES {
        let path = root.join("raw").join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), input_hash, "{name}: fixture identity");
        let raw = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(raw.format, RawFormat::Orf);
        raw.validate().unwrap();
        assert_eq!((raw.width, raw.height), sensor, "{name}: sensor dimensions");
        assert_eq!(raw.bits, 12, "{name}: sample depth");
        assert_eq!(raw.info(), probe_info(&bytes).unwrap(), "{name}: fast probe differs");
        let RawData::U16(samples) = &raw.data else { panic!("{name}: integer mosaic required") };
        let mut hash = Sha256::new();
        let mut block = [0u8; 8192];
        for chunk in samples.chunks(4096) {
            for (&sample, out) in chunk.iter().zip(block.as_chunks_mut::<2>().0) {
                out.copy_from_slice(&sample.to_le_bytes());
            }
            hash.update(&block[..chunk.len() * 2]);
        }
        assert_eq!(format!("{:x}", hash.finalize()), pixel_hash, "{name}: sensor pixels differ");
        assert!(raw.wb_multipliers.is_some(), "{name}: missing as-shot WB");
        assert!(raw.white_at(0) > raw.black.mean(), "{name}: invalid sensor levels");
        eprintln!(
            "{name}: full sensor exact, crop {:?}, CFA {:?}, WB {:?}, black {}",
            raw.active_area,
            raw.cfa.as_ref().map(|c| c.name()),
            raw.wb_multipliers,
            raw.black.mean()
        );
        checked += 1;
    }
    assert_eq!(checked, CASES.len(), "all pinned ORF references must run");
    eprintln!("{checked} ORF files checked against independent sensor pixels");
}
