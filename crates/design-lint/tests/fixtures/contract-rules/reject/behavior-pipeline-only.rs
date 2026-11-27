use craft_behavior::Gesture;
use craft_behavior::TypingRun;
use craft_behavior::press::PressMachine;
use craft_behavior::press::PassInput as Input;
use craft_behavior::press;
use crate::runtime::focus::push_scope;

fn open(ctx: &Context, id: Id, policy: FocusPolicy) {
    crate::runtime::focus::push_scope(ctx, id, policy, None);
    crate::runtime::focus::pop_scope(ctx);
}

fn run(gesture: craft_behavior::Gesture<f32>, step: PressStep<u32>) {}

fn count(previous: Option<(Pos2, f64, u32)>) -> u32 {
    count_clicks(previous, pos, time, slop)
}
