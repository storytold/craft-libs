fn alpha(state: State) -> f32 {
    match state {
        State::Rest => 0.0,
        State::Hover | State::Pressed => 0.5,
        _ => 1.0,
    }
}

fn hot(state: State) -> bool {
    matches!(state, State::Hover | State::Pressed)
}

fn same(cx: &Cx) -> bool {
    cx.state() == State::Hover
}
