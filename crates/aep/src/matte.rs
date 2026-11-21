//! Track mattes: the matte mode and the matte layer, two fields of the layer header `ldta`.
//!
//! Cases 402-408 (a 960x540 grey solid `M` above the red solid `L`): setting a track matte on a
//! layer changes exactly two fields of that layer's `ldta` (and the video switch of the matte
//! layer, which After Effects turns off by itself):
//!
//! * byte 107 (`u8`; bytes 104..106 are 0 in every file of the corpus): the matte mode, 1 alpha
//!   (403), 2 alpha inverted (404), 3 luma (405), 4 luma inverted (406), 0 no matte (every other
//!   case);
//! * the `u32` at byte 160: the id of the matte layer (`ldta` byte 0 of that layer), `0x1d` = `M`
//!   in 403-406 and 408, `0x1b` = `L` in 407, where the matte layer is BELOW the layer it mattes.
//!   0 in every file without a matte.
//!
//! A matte mode with a matte layer id of 0 is no matte: a project saved by After Effects 2026 has
//! such a layer (mode 1, id 0), which After Effects reads back without a track matte
//! (`hasTrackMatte` false, `trackMatteType` still Alpha) and renders unmatted. An `ldta` too short
//! to hold the id has no case: before After Effects 23.0 a track matte always used the layer
//! directly above (the documented behaviour of those versions), so such a matte is returned with
//! [`TrackMatte::layer`] `None` and the caller decides.

use crate::Layer;
use crate::bytes::be_u32;

/// The matte mode (`ldta` byte 107).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteMode {
    /// `1` (403)
    Alpha,
    /// `2` (404)
    AlphaInverted,
    /// `3` (405)
    Luma,
    /// `4` (406)
    LumaInverted,
    /// Any other non-zero value (not sampled).
    Other(u8),
}

impl MatteMode {
    /// The mode for a stored code; `None` for 0 (no matte).
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => return None,
            1 => Self::Alpha,
            2 => Self::AlphaInverted,
            3 => Self::Luma,
            4 => Self::LumaInverted,
            other => Self::Other(other),
        })
    }
}

/// The track matte of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackMatte {
    pub mode: MatteMode,
    /// The id of the matte layer (`u32` at `ldta` byte 160); `None` when the header is too short
    /// to hold it (the older "layer above" form, see the module documentation).
    pub layer: Option<u32>,
}

impl<'f, 'a> Layer<'f, 'a> {
    /// The track matte of the layer, `None` when it has none (or the header is unreadable).
    pub fn track_matte(&self) -> Option<TrackMatte> {
        let ldta = self.chunk().child(b"ldta")?.bytes()?;
        let mode = MatteMode::from_code(*ldta.get(107)?)?;
        let layer = match be_u32(ldta, 160) {
            // The header holds the id but stores 0: the mode stayed behind without a matte layer,
            // and After Effects reads the layer back without a track matte. No matte.
            Some(0) => return None,
            Some(id) => Some(id),
            // Too short to hold the id: the older "layer above" form.
            None => None,
        };
        Some(TrackMatte { mode, layer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Chunk;
    use crate::index::Node;

    /// The track matte of a layer whose `ldta` is `ldta`.
    fn matte(ldta: Vec<u8>) -> Option<TrackMatte> {
        let layr = Chunk::list(*b"Layr", vec![Chunk::data(*b"ldta", ldta)]);
        Layer { chunk: Node::from(&layr) }.track_matte()
    }

    fn ldta(len: usize, mode: u8, matte_id: u32) -> Vec<u8> {
        let mut b = vec![0u8; 164];
        b[107] = mode;
        b[160..164].copy_from_slice(&matte_id.to_be_bytes());
        b.truncate(len);
        b
    }

    #[test]
    fn a_matte_mode_with_matte_layer_id_0_is_no_matte() {
        assert_eq!(matte(ldta(164, 1, 0x1d)), Some(TrackMatte { mode: MatteMode::Alpha, layer: Some(0x1d) }));
        assert_eq!(matte(ldta(164, 1, 0)), None);
        assert_eq!(matte(ldta(164, 3, 0)), None);
        // A header too short to hold the id: the older "layer above" form, left to the caller.
        assert_eq!(matte(ldta(160, 1, 0)), Some(TrackMatte { mode: MatteMode::Alpha, layer: None }));
        assert_eq!(matte(ldta(164, 0, 0)), None);
    }
}
