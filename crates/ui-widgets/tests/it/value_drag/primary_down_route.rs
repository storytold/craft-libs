//! Only the real capability route differs across the causal pair; all physical inputs and assertions are identical.
use craft_ui::form::SliderField;
use craft_ui::value_drag::Slider;
pub(super) fn slider(value: Slider<'_>, policy: Option<bool>, grid: Option<f64>, fine_shift: bool) -> Slider<'_> {
    let _ = (grid, fine_shift);
    match policy {
        Some(enabled) => value.propose_on_primary_down(enabled),
        None => value,
    }
}

pub(super) fn field(value: SliderField<'_>, policy: Option<bool>, grid: Option<f64>, fine_shift: bool) -> SliderField<'_> {
    let _ = (grid, fine_shift);
    match policy {
        Some(enabled) => value.propose_on_primary_down(enabled),
        None => value,
    }
}
