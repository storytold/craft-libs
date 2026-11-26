use super::palettes::{Brightness, Scheme, apply_corners, definition};
use super::*;

/// The palette/corner assertions from VectorCraft's Preferences regression now exercise the
/// shared resolver directly, at both densities. Preferences selection stays an application test.
#[test]
fn quiet_resolves_the_moved_palette_and_corner_scale_at_every_brightness() {
    for (brightness, panel, field) in [
        (Brightness::Dark, 0x151618, 0x0f1012),
        (Brightness::MediumDark, 0x1d1e21, 0x16171a),
        (Brightness::MediumLight, 0xe5e7ea, 0xdadde1),
        (Brightness::Light, 0xf3f4f6, 0xe9ebee),
    ] {
        for size in [Size::Compact, Size::Comfortable] {
            let mode = if matches!(brightness, Brightness::Dark | Brightness::MediumDark) { Mode::Dark } else { Mode::Light };
            let appearance = Appearance { mode, contrast: Contrast::Standard, size, text_size: TextSize::default(), reduced_motion: false };
            let mut sizes = AppSizes::default();
            apply_corners(Scheme::Quiet, &mut sizes);
            let actual = resolve_sized(&definition(Scheme::Quiet, brightness), appearance, None, &sizes).tokens;
            let rgb = |v: u32| Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            assert_eq!(actual.color.surface.panel, rgb(panel), "{brightness:?} {size:?}");
            assert_eq!(actual.color.surface.chrome, rgb(panel), "{brightness:?} {size:?}: one chrome surface");
            assert_eq!(actual.color.surface.sunken, rgb(field), "{brightness:?} {size:?}: inset fields");
            let r = actual.shape.radius;
            assert_eq!((r.sm, r.control, r.surface, r.window), (4.0, 8.0, 14.0, 14.0), "{brightness:?} {size:?}");
        }
    }
}

/// Applying only the corner scale must not replace a host's production sizes, spacing or type.
#[test]
fn palette_corners_preserve_the_hosts_other_size_overrides() {
    let mut original = AppSizes::default();
    original.all.insert("size.control.md".into(), SizeValue::Number(37.0));
    original.compact.insert("size.button".into(), SizeValue::Number(41.0));
    original.comfortable.insert("size.button".into(), SizeValue::Number(55.0));
    for scheme in [Scheme::Flat, Scheme::Quiet] {
        let theme = definition(scheme, Brightness::MediumDark);
        let mut sizes = original.clone();
        apply_corners(scheme, &mut sizes);
        for size in [Size::Compact, Size::Comfortable] {
            let appearance =
                Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size, text_size: TextSize::default(), reduced_motion: false };
            let before = resolve_sized(&theme, appearance, None, &original).tokens;
            let after = resolve_sized(&theme, appearance, None, &sizes).tokens;
            assert_eq!(after.size, before.size, "{scheme:?} {size:?}");
            assert_eq!(after.space, before.space, "{scheme:?} {size:?}");
            assert_eq!(after.typography, before.typography, "{scheme:?} {size:?}");
            assert_eq!(after.stroke, before.stroke, "{scheme:?} {size:?}");
            assert_eq!(after.shape.radius.panel, 0.0, "{scheme:?} {size:?}: docked panels stay square");
            assert!(after.shape.radius.control > 0.0);
            assert!(after.shape.radius.surface > after.shape.radius.control);
            assert_eq!(after.shape.radius.surface, after.shape.radius.window);
            assert_eq!(after.shape.radius.control, after.shape.radius.button);
        }
    }
}
