//! Independent known-answer test vectors (raw bitstreams and hand-verified spec outputs)
//! for BC6H (unsigned/signed), BC7, ETC1, and EAC R/RG (unsigned/signed).

use craft_dds::DdsError;
use craft_dds::pixels::{self, PixelLayout};

#[test]
fn bc7_mode6_known_bitstream() {
    // Raw 128-bit BC7 Mode 6 block (7-bit RGBA endpoints + per-endpoint P-bits, 4-bit indices):
    // e0 = (241, 121, 21, 255), e1 = (40, 200, 254, 128); texels 0..3 use indices 0, 15, 8, 4.
    let block: [u8; 16] = [0x40, 0x3c, 0x85, 0x47, 0x56, 0xfc, 0xff, 0xc0, 0xf0, 0x48, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44];
    let texels = pixels::decode_bc7_block(&block);
    assert_eq!(texels[0], [241, 121, 21, 255]);
    assert_eq!(texels[1], [40, 200, 254, 128]);
    assert_eq!(texels[2], [134, 163, 145, 188]);
    assert_eq!(texels[3], [188, 142, 83, 221]);

    // Reserved mode (low byte 0x00) decodes to all zeroes.
    assert_eq!(pixels::decode_bc7_block(&[0; 16]), [[0, 0, 0, 0]; 16]);

    // Cropped 3x2 image keeps the top-left 3x2 texels in row-major order.
    let rgba = pixels::decode(PixelLayout::Bc7, 3, 2, &block).unwrap();
    assert_eq!(rgba.len(), 3 * 2 * 4);
    assert_eq!(&rgba[..4], &[241, 121, 21, 255]);
    assert_eq!(&rgba[4..8], &[40, 200, 254, 128]);
    assert_eq!(&rgba[8..12], &[134, 163, 145, 188]);
}

#[test]
fn bc6h_unsigned_and_signed_known_bitstreams() {
    // Mode 11 (untransformed 10-bit endpoints, 1 subset, 4-bit indices):
    // e0 = (480, 400, 200), e1 = (200, 480, 400); texels 0..2 use indices 0, 15, 8.
    let block: [u8; 16] = [0x03, 0x3c, 0xc8, 0x90, 0x41, 0x06, 0x3c, 0xc8, 0xf0, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    let unsigned = pixels::decode_bc6h_block(&block, false);
    assert_eq!(unsigned[0], [228, 105, 7, 255]);
    assert_eq!(unsigned[1], [7, 228, 105, 255]);
    assert_eq!(unsigned[2], [51, 160, 39, 255]);

    let signed_pos = pixels::decode_bc6h_block(&block, true);
    assert_eq!(signed_pos[0], [255, 255, 105, 255]);
    assert_eq!(signed_pos[1], [105, 255, 255, 255]);
    assert_eq!(signed_pos[2], [255, 255, 255, 255]);

    // Signed Mode 11 with negative 10-bit endpoints: R0 = -200 (0x338), G1 = -200 (0x338).
    // Negative linear values clip to 0 in sRGB8 output.
    let neg_block: [u8; 16] = [0x03, 0x67, 0x64, 0x20, 0x43, 0x06, 0x67, 0xc8, 0xf0, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let signed_neg = pixels::decode_bc6h_block(&neg_block, true);
    assert_eq!(signed_neg[0], [0, 105, 255, 255]);
    assert_eq!(signed_neg[1], [105, 0, 255, 255]);
    assert_eq!(signed_neg[2], [0, 0, 255, 255]);

    let rgba = pixels::decode(PixelLayout::Bc6h, 2, 1, &block).unwrap();
    assert_eq!(rgba, [228, 105, 7, 255, 7, 228, 105, 255]);
    let rgba_signed = pixels::decode(PixelLayout::Bc6hSigned, 2, 1, &neg_block).unwrap();
    assert_eq!(rgba_signed, [0, 105, 255, 255, 105, 0, 255, 255]);
}

#[test]
fn etc1_known_bitstreams_and_invalid_differential() {
    // Individual mode (diff=0, flip=0): left half (x<2) RGB4=(15,0,8) + 2, right half (x>=2) RGB4=(0,15,4) + 5.
    let individual: [u8; 8] = [0xf0, 0x0f, 0x84, 0x04, 0x00, 0x00, 0x00, 0x00];
    let texels = pixels::decode_etc1_block(&individual).unwrap();
    for y in 0..4 {
        for x in 0..4 {
            let expected = if x < 2 { [255, 2, 138, 255] } else { [5, 255, 73, 255] };
            assert_eq!(texels[y * 4 + x], expected);
        }
    }

    // Differential mode (diff=1, flip=1 horizontal): base0=(20,10,4), delta=(+2,-2,+1), tables=(2,3).
    let differential: [u8; 8] = [0xa2, 0x56, 0x21, 0x4f, 0x00, 0x00, 0x00, 0x00];
    let texels = pixels::decode_etc1_block(&differential).unwrap();
    for y in 0..4 {
        for x in 0..4 {
            let expected = if y < 2 { [174, 91, 42, 255] } else { [194, 79, 54, 255] };
            assert_eq!(texels[y * 4 + x], expected);
        }
    }

    // Differential base + delta outside 0..=31 must be rejected.
    let invalid: [u8; 8] = [0x04, 0x40, 0x40, 0x02, 0x00, 0x00, 0x00, 0x00];
    assert!(pixels::decode_etc1_block(&invalid).is_none());
    assert!(matches!(pixels::decode(PixelLayout::Etc1, 4, 4, &invalid), Err(DdsError::Malformed(_))));
}

#[test]
fn eac_r_and_rg_unsigned_and_signed_known_bitstreams() {
    // Unsigned EAC_R: base=100, mult=2, table=0, all selectors=7 (+14) -> 1028/2047 -> 128.
    let eac_r_u: [u8; 8] = [0x64, 0x20, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(pixels::decode_eac_r_block(&eac_r_u, false), [[128, 128, 128, 255]; 16]);

    // Signed EAC_R: base=-64 (0xc0), mult=2, table=0, all selectors=0 (-3) -> (-560+1023)/2046 -> 58.
    let eac_r_s: [u8; 8] = [0xc0, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(pixels::decode_eac_r_block(&eac_r_s, true), [[58, 58, 58, 255]; 16]);

    // Column-major selector order: texel (x=1, y=0) has selector 7 (+14 -> 142), others selector 0 (-3 -> 125).
    let eac_col: [u8; 8] = [0x80, 0x10, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x00];
    let col_texels = pixels::decode_eac_r_block(&eac_col, false);
    assert_eq!(col_texels[1], [142, 142, 142, 255]);
    assert_eq!(col_texels[0], [125, 125, 125, 255]);
    assert_eq!(col_texels[4], [125, 125, 125, 255]);

    // Dual-channel EAC_RG (unsigned and signed): R then G.
    let mut eac_rg_u = [0u8; 16];
    eac_rg_u[..8].copy_from_slice(&eac_r_u);
    eac_rg_u[8..].copy_from_slice(&[0xff, 0xf0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(pixels::decode(PixelLayout::EacRg, 2, 2, &eac_rg_u).unwrap(), [128, 255, 0, 255].repeat(4));

    let mut eac_rg_s = [0u8; 16];
    eac_rg_s[..8].copy_from_slice(&eac_r_s);
    eac_rg_s[8..].copy_from_slice(&[0x7f, 0xf0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(pixels::decode(PixelLayout::EacRgSigned, 2, 2, &eac_rg_s).unwrap(), [58, 255, 0, 255].repeat(4));
}
