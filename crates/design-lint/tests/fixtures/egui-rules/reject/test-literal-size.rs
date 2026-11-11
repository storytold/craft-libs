#[test]
fn button_is_28_wide() {
    let fixture = Fixture::new(show_button);
    assert_eq!(fixture.size().x, 28.0);
}
