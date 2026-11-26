fn height(size: ControlSize) -> f32 {
    match size {
        ControlSize::Sm => 20.0,
        ControlSize::Md => 24.0,
        _ => 28.0,
    }
}
