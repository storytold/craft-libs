//! The memory budget of `DecodeOptions`: a frame whose samples (decoded tiles plus the assembled
//! frame, and the same for an auxiliary alpha frame) do not fit is refused before any allocation.

use craft_video::apv::header::{PBU_ALPHA_FRAME, PBU_PRIMARY_FRAME, write_frame_header};
use craft_video::apv::*;

const W: u32 = 64;
const H: u32 = 32;

fn encode(profile: Profile) -> Vec<u8> {
    let f = Frame::new(W, H, profile.default_chroma(), profile.default_bit_depth(), false);
    let mut cfg = EncoderConfig::new(profile, W, H);
    cfg.qp = Some(16);
    Encoder::with_config(cfg).unwrap().encode(&f).unwrap()
}

fn decode(au: &[u8], memory_budget: u64) -> Result<Frame> {
    decode_frame_with(au, &DecodeOptions { memory_budget, ..Default::default() })
}

#[test]
fn exact_budget_boundary() {
    // 4:2:2 at 64x32: 2048 luma + 2 x 1024 chroma samples, two bytes each, held twice.
    let au = encode(Profile::P422_10);
    let need = (2048 + 2 * 1024) * 2 * 2;
    assert!(decode(&au, need).is_ok());
    assert_eq!(decode(&au, need - 1).unwrap_err(), Error::MemoryBudget);
    // 4:4:4:4 holds four full planes.
    let au = encode(Profile::P4444_10);
    let need = 4 * 2048 * 2 * 2;
    assert!(decode(&au, need).is_ok());
    assert_eq!(decode(&au, need - 1).unwrap_err(), Error::MemoryBudget);
}

#[test]
fn auxiliary_alpha_frame_counts() {
    let mut au = encode(Profile::P422_10);
    let luma = encode(Profile::P400_10);
    let n = u32::from_be_bytes(luma[4..8].try_into().unwrap()) as usize;
    let pbu = &luma[8..8 + n];
    au.extend_from_slice(&(pbu.len() as u32).to_be_bytes());
    au.push(PBU_ALPHA_FRAME);
    au.extend_from_slice(&pbu[1..]);
    let need = (2048 + 2 * 1024 + 2048) * 2 * 2;
    assert!(decode(&au, need).unwrap().alpha.is_some());
    assert_eq!(decode(&au, need - 1).unwrap_err(), Error::MemoryBudget);
}

/// A 16384 x 16384 4:4:4:4 header in a few dozen bytes asks for exactly 4 GiB (the default budget
/// of a 64-bit target); the 1 GiB default of a 32-bit target refuses it.
#[test]
fn the_largest_header_needs_four_gibibytes() {
    let mut hdr = probe(&encode(Profile::P4444_10)).unwrap();
    (hdr.width, hdr.height) = (16384, 16384);
    (hdr.tile_width_in_mbs, hdr.tile_height_in_mbs) = (1024, 1024);
    let fh = write_frame_header(&hdr);
    let mut au = b"aPv1".to_vec();
    au.extend_from_slice(&(fh.len() as u32 + 4).to_be_bytes());
    au.extend_from_slice(&[PBU_PRIMARY_FRAME, 0, 1, 0]);
    au.extend_from_slice(&fh);
    assert_eq!(probe(&au).unwrap().width, 16384);
    assert_eq!(decode(&au, (4 << 30) - 1).unwrap_err(), Error::MemoryBudget);
    assert_eq!(decode(&au, 1 << 30).unwrap_err(), Error::MemoryBudget);
    // Within the budget the decoder goes on to read the tiles, which are not there.
    assert_eq!(decode(&au, 4 << 30).unwrap_err(), Error::Truncated);
}
