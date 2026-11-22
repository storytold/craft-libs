//! Frame and unit facts for consumers that apply their own fallback scale.
//! The WMF map-mode scan is adapted from VectorCraft’s metafile importer.

use crate::{Kind, MAX_STACK, bytes::Bytes, sniff, wmf::PLACEABLE_KEY};

/// File-declared frame and source-unit information; does not assign a default physical scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitFacts {
    /// A placeable WMF with positive frame extents.
    WmfPlaceableFrame,
    /// Without a placeable frame, a WMF whose drawing records predominantly use legacy map
    /// modes 2–6 (LOMETRIC, HIMETRIC, LOENGLISH, HIENGLISH, TWIPS). This compatibility group
    /// does not imply a particular physical unit. Ties use the last map mode seen.
    WmfMetricMap,
    /// Without a placeable frame, a WMF whose drawing records use logical units.
    WmfLogical {
        /// Positive units per inch from a placeable header, when one is present.
        units_per_inch: Option<u16>,
    },
    Emf {
        /// The frame coordinates have positive extents, independently of device calibration.
        has_frame: bool,
        /// The device calibration, when both axes are positive.
        device: Option<EmfDevice>,
    },
}

/// The two axes of device calibration in an EMF header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmfDevice {
    /// Width and height, in pixels.
    pub pixels: (u32, u32),
    /// Device width and height, in the units supplied by the header.
    pub physical_size: EmfPhysicalSize,
}

/// The precision of the physical device size supplied by an EMF header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmfPhysicalSize {
    /// The header's extended micrometre fields were both positive.
    Micrometres(u32, u32),
    /// The extension was missing or incomplete; the millimetre fields were both positive.
    Millimetres(u32, u32),
}

/// Inspects a recognized WMF or EMF without decoding images, allocating geometry, or assigning
/// a physical size to a file that supplies none. A matching header is not proof that its records
/// parse successfully: use [`crate::parse`] for the actual picture.
pub fn unit_facts(bytes: &[u8]) -> Option<UnitFacts> {
    let b = Bytes(bytes);
    Some(match sniff(bytes)? {
        Kind::Emf => UnitFacts::Emf {
            has_frame: matches!((b.i32(24), b.i32(28), b.i32(32), b.i32(36)), (Some(l), Some(t), Some(r), Some(bt)) if r > l && bt > t),
            device: emf_device(b),
        },
        Kind::Wmf => {
            let placeable = b.u32(0) == Some(PLACEABLE_KEY);
            if placeable && matches!((b.i16(6), b.i16(8), b.i16(10), b.i16(12)), (Some(l), Some(t), Some(r), Some(bt)) if r > l && bt > t) {
                UnitFacts::WmfPlaceableFrame
            } else if wmf_metric(b, if placeable { 22 } else { 0 }) {
                UnitFacts::WmfMetricMap
            } else {
                let units_per_inch = if placeable { b.u16(14).filter(|&inch| inch > 0) } else { None };
                UnitFacts::WmfLogical { units_per_inch }
            }
        }
    })
}

// MS-EMF §2.3.4.2: optional description and pixel-format data follow
// the fixed fields. The total record size by itself cannot identify an extension.
fn emf_fixed_header_size(b: Bytes<'_>) -> Option<usize> {
    let mut end = usize::try_from(b.u32(4)?).ok()?.min(b.0.len());
    if b.u32(60)? != 0 {
        end = end.min(usize::try_from(b.u32(64)?).ok()?);
    }
    // Only inspect the extension-1 descriptor if it fits before any description.
    if end >= 100 && b.u32(88)? != 0 {
        end = end.min(usize::try_from(b.u32(92)?).ok()?);
    }
    Some(end)
}

pub(crate) fn emf_device(b: Bytes<'_>) -> Option<EmfDevice> {
    let positive = |v: i32| u32::try_from(v).ok().filter(|&v| v > 0);
    let pixels = (positive(b.i32(72)?)?, positive(b.i32(76)?)?);
    let physical_size = if emf_fixed_header_size(b)? >= 108
        && let (Some(w), Some(h)) = (positive(b.i32(100)?), positive(b.i32(104)?))
    {
        EmfPhysicalSize::Micrometres(w, h)
    } else {
        EmfPhysicalSize::Millimetres(positive(b.i32(80)?)?, positive(b.i32(84)?)?)
    };
    Some(EmfDevice { pixels, physical_size })
}

// Preserve the VectorCraft compatibility scan: count recognized drawing records under the map
// mode in which they appear, skipping suppressed shape and PATBLT classes. This is not replay.
fn wmf_metric(b: Bytes<'_>, start: usize) -> bool {
    const SHAPES: [u16; 10] = [0x0213, 0x0324, 0x0325, 0x0418, 0x041B, 0x0538, 0x061C, 0x0817, 0x081A, 0x0830];
    const DRAW: [u16; 5] = [0x0521, 0x0940, 0x0A32, 0x0B41, 0x0F43];
    const MIX_SKIPS: [u16; 2] = [7, 11]; // R2_XORPEN and R2_NOP
    const PATCOPY: u32 = 0x00F0_0021;
    const BLACKNESS: u32 = 0x0000_0042;
    const WHITENESS: u32 = 0x00FF_0062;
    let Some(words) = start.checked_add(2).and_then(|at| b.u16(at)) else { return false };
    let mut at = start.saturating_add(usize::from(words) * 2);
    let (mut mode, mut mix) = (1, 13); // MM_TEXT, R2_COPYPEN
    let metric = |mode: u16| (2..=6).contains(&mode);
    let (mut in_metric, mut in_other) = (0usize, 0usize);
    let mut saved = Vec::new();
    while let (Some(words), Some(function)) = (b.u32(at), b.u16(at.saturating_add(4))) {
        let size = (words as usize).saturating_mul(2);
        let Some(next) = at.checked_add(size) else { break };
        if function == 0 || size < 6 || next > b.0.len() {
            break;
        }
        match function {
            0x0103 if size >= 8 => mode = b.u16(at + 6).unwrap_or(mode), // SETMAPMODE
            0x0104 if size >= 8 => mix = b.u16(at + 6).unwrap_or(mix),   // SETROP2
            0x001E => {
                // SAVEDC
                if saved.len() >= MAX_STACK {
                    break;
                }
                saved.push((mode, mix));
            }
            0x0127 if size >= 8 => {
                // RESTOREDC
                let n = b.i16(at + 6).map_or(0, i32::from);
                let level = if n < 0 { saved.len().checked_sub(n.unsigned_abs() as usize) } else { (n as usize).checked_sub(1) };
                if let Some(level) = level
                    && let Some((m, x)) = saved.get(level).copied()
                {
                    mode = m;
                    mix = x;
                    saved.truncate(level);
                }
            }
            f => {
                let draws = match f {
                    _ if DRAW.contains(&f) => true,
                    _ if SHAPES.contains(&f) => !MIX_SKIPS.contains(&mix),
                    0x061D => match b.u32(at + 6).filter(|_| size >= 10) {
                        // PATBLT
                        Some(PATCOPY) => !MIX_SKIPS.contains(&mix),
                        Some(rop) => rop == BLACKNESS || rop == WHITENESS,
                        None => false,
                    },
                    _ => false,
                };
                if draws && metric(mode) {
                    in_metric += 1;
                } else if draws {
                    in_other += 1;
                }
            }
        }
        at = next;
    }
    if in_metric == in_other { metric(mode) } else { in_metric > in_other }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Spec-generated headers and records; no external fixture bytes.
    fn plain_wmf(records: &[(u16, &[u8])]) -> Vec<u8> {
        let mut bytes = vec![0; 18];
        bytes[..6].copy_from_slice(&[1, 0, 9, 0, 0, 3]);
        for (kind, parameters) in records {
            bytes.extend_from_slice(&((parameters.len() / 2 + 3) as u32).to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(parameters);
        }
        bytes
    }

    #[test]
    fn placeable_bounds_win_or_leave_the_declared_scale_for_logical_units() {
        let mut bytes = vec![0; 22];
        bytes[..4].copy_from_slice(&PLACEABLE_KEY.to_le_bytes());
        bytes[10..12].copy_from_slice(&100i16.to_le_bytes());
        bytes[12..14].copy_from_slice(&80i16.to_le_bytes());
        bytes[14..16].copy_from_slice(&720u16.to_le_bytes());
        bytes.extend_from_slice(&plain_wmf(&[(0x0103, &[3, 0])]));
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfPlaceableFrame));
        bytes[12..14].fill(0);
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfMetricMap));
        bytes[46..48].copy_from_slice(&[1, 0]);
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfLogical { units_per_inch: Some(720) }));
        bytes[14..16].fill(0);
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfLogical { units_per_inch: None }));
        assert_eq!(unit_facts(&plain_wmf(&[])), Some(UnitFacts::WmfLogical { units_per_inch: None }));
    }

    #[test]
    fn wmf_dominance_counts_drawing_modes_and_respects_save_restore() {
        let metric = [3, 0];
        let logical = [1, 0];
        let nop = [11, 0];
        let restore = (-1i16).to_le_bytes();
        let line = [20, 0, 10, 0];
        let unsupported_rop = 0x0055_0009u32.to_le_bytes();
        let bytes = plain_wmf(&[
            (0x0103, &metric),
            (0x0213, &line),
            (0x001E, &[]),
            (0x0103, &logical),
            (0x0104, &nop),
            (0x0213, &line),
            (0x0213, &line),
            (0x061D, &unsupported_rop),
            (0x0127, &restore),
            (0x0213, &line),
            (0x0103, &logical),
        ]);
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfMetricMap));
        let mut bytes = plain_wmf(&[(0x0103, &metric)]);
        bytes.extend_from_slice(&[4, 0, 0, 0, 3, 1]); // Declares a mode change with a missing parameter.
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::WmfMetricMap));
    }

    #[test]
    fn emf_frame_and_device_calibration_are_independent_facts() {
        let mut bytes = vec![0; 108];
        let set = |buf: &mut [u8], i: usize, value: i32| buf[i..i + 4].copy_from_slice(&value.to_le_bytes());
        for (i, v) in
            [(0, 1), (4, 108), (32, 100), (36, 50), (40, 0x464d_4520), (72, 800), (76, 600), (80, 200), (84, 150), (100, 200_150), (104, 149_900)]
        {
            set(&mut bytes, i, v);
        }
        let facts = |has_frame, physical_size| Some(UnitFacts::Emf { has_frame, device: Some(EmfDevice { pixels: (800, 600), physical_size }) });
        assert_eq!(unit_facts(&bytes), facts(true, EmfPhysicalSize::Micrometres(200_150, 149_900)));
        assert_eq!(unit_facts(&bytes[..104]), facts(true, EmfPhysicalSize::Millimetres(200, 150)));
        set(&mut bytes, 100, -1);
        assert_eq!(unit_facts(&bytes), facts(true, EmfPhysicalSize::Millimetres(200, 150)));
        set(&mut bytes, 100, 200_150);
        set(&mut bytes, 104, -1);
        assert_eq!(unit_facts(&bytes), facts(true, EmfPhysicalSize::Millimetres(200, 150)));
        set(&mut bytes, 104, 149_900);
        set(&mut bytes, 4, 88);
        set(&mut bytes, 36, 0);
        assert_eq!(unit_facts(&bytes), facts(false, EmfPhysicalSize::Millimetres(200, 150)));
        set(&mut bytes, 76, -1);
        assert_eq!(unit_facts(&bytes), Some(UnitFacts::Emf { has_frame: false, device: None }));
        assert_eq!(unit_facts(b"not a metafile"), None);
    }
    #[test]
    fn emf_variable_payloads_do_not_supply_micrometres() {
        let mut bytes = vec![0; 148];
        let set = |buf: &mut [u8], i: usize, value: u32| buf[i..i + 4].copy_from_slice(&value.to_le_bytes());
        for (at, value) in [(0, 1), (4, 112), (40, 0x464d_4520), (60, 11), (64, 88), (72, 800), (76, 600), (80, 200), (84, 150)] {
            set(&mut bytes, at, value);
        }
        for (dst, src) in bytes[88..110].as_chunks_mut::<2>().0.iter_mut().zip("app\0title\0\0".encode_utf16()) {
            dst.copy_from_slice(&src.to_le_bytes());
        }
        let mm = Some(EmfDevice { pixels: (800, 600), physical_size: EmfPhysicalSize::Millimetres(200, 150) });
        assert_eq!(emf_device(Bytes(&bytes)), mm);

        // The same numeric bytes are part of a pixel descriptor in extension 1,
        // but are fixed calibration fields when extension 2 precedes that descriptor.
        for (at, value) in [(4, 148), (60, 0), (64, 0), (88, 40), (92, 100), (100, 201_000), (104, 151_000)] {
            set(&mut bytes, at, value);
        }
        assert_eq!(emf_device(Bytes(&bytes)), mm);
        set(&mut bytes, 92, 108);
        assert_eq!(emf_device(Bytes(&bytes)), Some(EmfDevice { pixels: (800, 600), physical_size: EmfPhysicalSize::Micrometres(201_000, 151_000) }));
    }
}
