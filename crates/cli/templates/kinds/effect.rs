
    /// Runs on one band of pixels. The host tiles the image, so this sees a strip, not the whole frame.
    fn invert(band: &mut Band) -> Result<(), String> {
        use craft_plugin_sdk::abi::descriptor::Sample;

        if band.desc.sample != Sample::U8 {
            return Err(format!("this effect handles 8 bit samples, got {:?}", band.desc.sample));
        }
        let amount = band.param_f64("amount", 1.0).clamp(0.0, 1.0);
        let channels = band.channels().max(1);
        // Alpha, when there is a fourth channel, passes through unchanged.
        let color_channels = if channels == 4 { 3 } else { channels };
        for (src_row, dst_row) in band.rows() {
            for (src, dst) in src_row.chunks(channels).zip(dst_row.chunks_mut(channels)) {
                for (c, (s, d)) in src.iter().zip(dst.iter_mut()).enumerate() {
                    *d = if c < color_channels {
                        let from = f64::from(*s);
                        let to = f64::from(255 - *s);
                        (from + (to - from) * amount).round() as u8
                    } else {
                        *s
                    };
                }
            }
        }
        Ok(())
    }
