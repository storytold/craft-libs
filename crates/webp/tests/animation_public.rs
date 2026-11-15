//! Guard the public module wiring: the implementation must be usable by consumers.
#[test]
fn public_animation_api_refuses_empty_movies() {
    let result = craft_webp::anim::animation(1, 1, &[], &craft_webp::anim::AnimOptions::default());
    assert!(result.is_err());
}
