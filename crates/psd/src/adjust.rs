//! Adjustment and solid colour fill layers: the settings a reader needs to show them, as
//! read-only views of their blocks (`nvrt`, `brit`, `CgEd`, `hue2`, `hue `, `levl`, `expA`,
//! `SoCo`) and of the layer effects (`lfx2`).
//!
//! Only the composite (master) settings are read: the per-channel levels and the per-range
//! hue/saturation edits are not. The blocks stay raw in the model and write back unchanged.
//! Came from EffectCraft's `effectcraft-psd`.

use crate::descriptor::Descriptor;
use crate::error::Result;
use crate::io::Reader;
use crate::layer::LayerRecord;

/// The adjustment a layer applies.
#[derive(Clone, Debug, PartialEq)]
pub enum Adjustment {
    /// Invert (`nvrt`).
    Invert,
    /// Brightness/Contrast (`brit`, or `CgEd` when the layer has one).
    BrightnessContrast {
        /// Brightness.
        brightness: f64,
        /// Contrast.
        contrast: f64,
        /// The legacy algorithm: always for a `brit` block alone, `useLegacy` for `CgEd`.
        legacy: bool,
    },
    /// Hue/Saturation (`hue2`, `hue `): the colorize values when colorizing, else the master edit.
    HueSaturation {
        /// Hue.
        hue: f64,
        /// Saturation.
        saturation: f64,
        /// Lightness.
        lightness: f64,
        /// Colorize.
        colorize: bool,
    },
    /// Levels (`levl`): the composite channel's record.
    Levels {
        /// Input black, 0..255.
        in_black: f64,
        /// Input white, 0..255.
        in_white: f64,
        /// Gamma (the file stores it times 100).
        gamma: f64,
        /// Output black, 0..255.
        out_black: f64,
        /// Output white, 0..255.
        out_white: f64,
    },
    /// Exposure (`expA`). A value that is not finite reads as its default (0, 0, 1).
    Exposure {
        /// Exposure.
        exposure: f64,
        /// Offset.
        offset: f64,
        /// Gamma correction.
        gamma: f64,
    },
    /// Another adjustment, by its block key (`curv`, `blnc`, `post`, `thrs`, `selc`, `mixr`,
    /// `phfl`, `vibA`, `blwh`, `grdm`, `clrL`): its settings are in the layer's block.
    Other([u8; 4]),
}

/// Keys of the adjustment blocks that are reported as [`Adjustment::Other`].
const OTHER_KEYS: [&[u8; 4]; 11] = [b"curv", b"blnc", b"post", b"thrs", b"selc", b"mixr", b"phfl", b"vibA", b"blwh", b"grdm", b"clrL"];

impl Adjustment {
    /// Reads the data of the adjustment block `key`. `None` when `key` is not one of the typed
    /// adjustment blocks.
    pub fn from_block(key: &[u8; 4], data: &[u8]) -> Option<Result<Adjustment>> {
        let mut r = Reader::new(data);
        Some(match key {
            b"nvrt" => Ok(Adjustment::Invert),
            b"brit" => brightness(&mut r),
            b"CgEd" => content_generator(&mut r),
            b"hue2" | b"hue " => hue_saturation(&mut r),
            b"levl" => levels(&mut r),
            b"expA" => exposure(&mut r),
            _ => return None,
        })
    }
}

fn brightness(r: &mut Reader<'_>) -> Result<Adjustment> {
    Ok(Adjustment::BrightnessContrast { brightness: f64::from(r.i16()?), contrast: f64::from(r.i16()?), legacy: true })
}

fn content_generator(r: &mut Reader<'_>) -> Result<Adjustment> {
    let _version = r.u32()?;
    let d = Descriptor::read(r)?;
    Ok(Adjustment::BrightnessContrast {
        brightness: d.get_number("Brgh").unwrap_or(0.0),
        contrast: d.get_number("Cntr").unwrap_or(0.0),
        legacy: d.get_bool("useLegacy").unwrap_or(false),
    })
}

fn hue_saturation(r: &mut Reader<'_>) -> Result<Adjustment> {
    let _version = r.u16()?;
    let colorize = r.u8()? != 0;
    r.skip(1)?;
    let mut three = || -> Result<[f64; 3]> { Ok([f64::from(r.i16()?), f64::from(r.i16()?), f64::from(r.i16()?)]) };
    let colorized = three()?;
    let master = three()?;
    let [hue, saturation, lightness] = if colorize { colorized } else { master };
    Ok(Adjustment::HueSaturation { hue, saturation, lightness, colorize })
}

fn levels(r: &mut Reader<'_>) -> Result<Adjustment> {
    let _version = r.u16()?;
    let (in_black, in_white, out_black, out_white, gamma) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?, r.i16()?);
    Ok(Adjustment::Levels {
        in_black: f64::from(in_black),
        in_white: f64::from(in_white),
        gamma: f64::from(gamma) / 100.0,
        out_black: f64::from(out_black),
        out_white: f64::from(out_white),
    })
}

fn exposure(r: &mut Reader<'_>) -> Result<Adjustment> {
    let _version = r.u16()?;
    let mut finite = |default: f64| -> Result<f64> {
        let v = f64::from(f32::from_be_bytes(r.array()?));
        Ok(if v.is_finite() { v } else { default })
    };
    Ok(Adjustment::Exposure { exposure: finite(0.0)?, offset: finite(0.0)?, gamma: finite(1.0)? })
}

/// Reads the data of a `SoCo` block: the fill colour as RGB in 0..1 (see
/// [`Descriptor::get_color`]), `None` for a colour model that is not RGB or gray.
pub fn solid_color(data: &[u8]) -> Result<Option<[f64; 3]>> {
    let mut r = Reader::new(data);
    let _version = r.u32()?;
    Ok(Descriptor::read(&mut r)?.get_color("Clr "))
}

/// Reads the data of an `lfx2` block: the layer effects descriptor.
pub fn effects(data: &[u8]) -> Result<Descriptor> {
    let mut r = Reader::new(data);
    let _effects_version = r.u32()?;
    let _descriptor_version = r.u32()?;
    Descriptor::read(&mut r)
}

impl LayerRecord {
    /// The adjustment the layer applies, when it is an adjustment layer. Blocks are taken in
    /// order: a typed block replaces what came before, except that a `brit` block or one of
    /// the untyped adjustments ([`Adjustment::Other`]) only counts when nothing was found yet
    /// (Photoshop writes `brit` beside `CgEd`). A malformed block is skipped.
    pub fn adjustment(&self) -> Option<Adjustment> {
        let mut found = None;
        for b in &self.blocks {
            if OTHER_KEYS.contains(&&b.key) {
                found = found.or(Some(Adjustment::Other(b.key)));
            } else if let Some(Ok(a)) = Adjustment::from_block(&b.key, &b.data)
                && (&b.key != b"brit" || found.is_none())
            {
                found = Some(a);
            }
        }
        found
    }

    /// The colour of a solid colour fill layer (`SoCo`), RGB in 0..1.
    pub fn solid_color(&self) -> Option<[f64; 3]> {
        solid_color(&self.block(b"SoCo")?.data).ok()?
    }

    /// The layer effects descriptor (`lfx2`). A malformed block counts as absent.
    pub fn effects(&self) -> Option<Descriptor> {
        effects(&self.block(b"lfx2")?.data).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Value;
    use crate::io::WriteExt;
    use crate::tagged::TaggedBlock;

    fn layer(blocks: Vec<([u8; 4], Vec<u8>)>) -> LayerRecord {
        LayerRecord { blocks: blocks.into_iter().map(|(k, d)| TaggedBlock::new(k, d)).collect(), ..Default::default() }
    }

    fn i16s(prefix: &[u8], v: &[i16]) -> Vec<u8> {
        let mut out = prefix.to_vec();
        for x in v {
            out.put_i16(*x);
        }
        out
    }

    fn versioned(d: Descriptor) -> Vec<u8> {
        let mut out = Vec::new();
        out.put_u32(16);
        d.write(&mut out);
        out
    }

    #[test]
    fn each_typed_adjustment_reads_its_settings() {
        assert_eq!(layer(vec![(*b"nvrt", vec![])]).adjustment(), Some(Adjustment::Invert));
        let brit = i16s(&[], &[40, -15, 127, 0]);
        assert_eq!(
            layer(vec![(*b"brit", brit.clone())]).adjustment(),
            Some(Adjustment::BrightnessContrast { brightness: 40.0, contrast: -15.0, legacy: true })
        );
        // Version 2, colorize off: the master edit is the second triple.
        let hue = i16s(&[0, 2, 0, 0], &[10, 20, 30, -40, 50, -60]);
        assert_eq!(
            layer(vec![(*b"hue2", hue.clone())]).adjustment(),
            Some(Adjustment::HueSaturation { hue: -40.0, saturation: 50.0, lightness: -60.0, colorize: false })
        );
        let mut colorized = hue;
        colorized[2] = 1;
        assert_eq!(
            layer(vec![(*b"hue ", colorized)]).adjustment(),
            Some(Adjustment::HueSaturation { hue: 10.0, saturation: 20.0, lightness: 30.0, colorize: true })
        );
        let levl = i16s(&[0, 2], &[12, 240, 5, 250, 150]);
        assert_eq!(
            layer(vec![(*b"levl", levl)]).adjustment(),
            Some(Adjustment::Levels { in_black: 12.0, in_white: 240.0, gamma: 1.5, out_black: 5.0, out_white: 250.0 })
        );
        let mut exp = vec![0, 1];
        for v in [1.5f32, -0.25, 0.5] {
            exp.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(layer(vec![(*b"expA", exp)]).adjustment(), Some(Adjustment::Exposure { exposure: 1.5, offset: -0.25, gamma: 0.5 }));
        assert_eq!(layer(vec![(*b"curv", vec![1, 2, 3])]).adjustment(), Some(Adjustment::Other(*b"curv")));
        assert_eq!(layer(vec![(*b"luni", vec![0; 4])]).adjustment(), None);
        assert!(Adjustment::from_block(b"luni", &[]).is_none());
    }

    #[test]
    fn the_descriptor_form_of_brightness_wins_over_the_legacy_block_in_either_order() {
        let cged = versioned(
            Descriptor::new("null").with("Brgh", Value::Integer(25)).with("Cntr", Value::Integer(-5)).with("useLegacy", Value::Boolean(false)),
        );
        let brit = i16s(&[], &[1, 2, 127, 0]);
        let want = Some(Adjustment::BrightnessContrast { brightness: 25.0, contrast: -5.0, legacy: false });
        assert_eq!(layer(vec![(*b"brit", brit.clone()), (*b"CgEd", cged.clone())]).adjustment(), want);
        assert_eq!(layer(vec![(*b"CgEd", cged), (*b"brit", brit)]).adjustment(), want);
    }

    #[test]
    fn exposure_values_that_are_not_finite_read_as_their_defaults() {
        let mut exp = vec![0, 1];
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            exp.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(layer(vec![(*b"expA", exp)]).adjustment(), Some(Adjustment::Exposure { exposure: 0.0, offset: 0.0, gamma: 1.0 }));
    }

    #[test]
    fn a_truncated_block_is_an_error_and_the_layer_skips_it() {
        for (key, len) in [(b"brit", 4usize), (b"hue2", 16), (b"levl", 12), (b"expA", 14)] {
            for cut in 0..len {
                assert!(matches!(Adjustment::from_block(key, &vec![0; cut]), Some(Err(_))), "{} cut {cut}", String::from_utf8_lossy(key));
            }
            assert!(matches!(Adjustment::from_block(key, &vec![0; len]), Some(Ok(_))));
        }
        assert_eq!(layer(vec![(*b"levl", vec![0; 3])]).adjustment(), None);
        // The damaged block does not hide a good one.
        assert_eq!(layer(vec![(*b"nvrt", vec![]), (*b"levl", vec![0; 3])]).adjustment(), Some(Adjustment::Invert));
    }

    #[test]
    fn solid_colour_and_effects_read_their_descriptors() {
        let rgb = Descriptor::new("RGBC").with("Rd  ", Value::Double(255.0)).with("Grn ", Value::Double(127.5)).with("Bl  ", Value::Double(0.0));
        let soco = versioned(Descriptor::new("null").with("Clr ", Value::Descriptor(rgb)));
        let mut lfx = vec![0, 0, 0, 0];
        lfx.extend(versioned(Descriptor::new("null").with("Scl ", Value::UnitFloat { unit: *b"#Prc", value: 100.0 })));
        let l = layer(vec![(*b"SoCo", soco.clone()), (*b"lfx2", lfx)]);
        assert_eq!(l.solid_color(), Some([1.0, 0.5, 0.0]));
        assert_eq!(l.effects().unwrap().get_number("Scl "), Some(100.0));
        let float = Descriptor::new("RGBC")
            .with("redFloat", Value::Double(0.25))
            .with("greenFloat", Value::Double(0.5))
            .with("blueFloat", Value::Double(1.0));
        assert_eq!(solid_color(&versioned(Descriptor::new("null").with("Clr ", Value::Descriptor(float)))).unwrap(), Some([0.25, 0.5, 1.0]));
        let gray = Descriptor::new("Grsc").with("Gry ", Value::Double(25.0));
        assert_eq!(solid_color(&versioned(Descriptor::new("null").with("Clr ", Value::Descriptor(gray)))).unwrap(), Some([0.75; 3]));
        // A colour model that is not RGB or gray is no colour; a cut block is an error.
        let lab = Descriptor::new("LbCl").with("Lmnc", Value::Double(50.0));
        assert_eq!(solid_color(&versioned(Descriptor::new("null").with("Clr ", Value::Descriptor(lab)))).unwrap(), None);
        assert!(solid_color(&soco[..soco.len() - 1]).is_err());
        assert_eq!(layer(vec![(*b"SoCo", vec![0; 5]), (*b"lfx2", vec![0; 5])]).solid_color(), None);
        assert_eq!(layer(vec![(*b"lfx2", vec![0; 5])]).effects(), None);
    }
}
