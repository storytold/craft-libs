
#[cfg(test)]
mod effect_tests {
    use craft_plugin_sdk::abi::ProcessDescriptor;
    use craft_plugin_sdk::abi::descriptor::{Alpha, Kind, Sample};
    use craft_plugin_sdk::json;

    use super::plugin::craft_testing;

    fn two_pixel_rgba() -> ProcessDescriptor {
        ProcessDescriptor {
            kind: Kind::Image,
            entry: 0,
            w: 2,
            h: 1,
            stride_bytes: 8,
            channels: 4,
            sample: Sample::U8,
            alpha: Alpha::Straight,
            color_mode: 0,
            depth: 8,
            origin_x: 0,
            origin_y: 0,
            overlap: 0,
            scale: 1.0,
            time: 0.0,
            sample_rate: 0,
            frames: 0,
            in_ptr: 0,
            out_ptr: 0,
            params_ptr: 0,
            params_len: 0,
        }
    }

    #[test]
    fn full_amount_inverts_color_and_keeps_alpha() {
        let input = [10, 20, 30, 255, 0, 128, 255, 7];
        let out = craft_testing::process("invert", &two_pixel_rgba(), &input, json!({ "amount": 1.0 })).expect("process");
        assert_eq!(out, vec![245, 235, 225, 255, 255, 127, 0, 7]);
    }

    #[test]
    fn zero_amount_copies_the_input() {
        let input = [10, 20, 30, 255, 0, 128, 255, 7];
        let out = craft_testing::process("invert", &two_pixel_rgba(), &input, json!({ "amount": 0.0 })).expect("process");
        assert_eq!(out, input.to_vec());
    }
}
