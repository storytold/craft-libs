use craft_behavior::Sense;

pub struct Env;

pub fn sense() -> Sense {
    Sense
}

#[expect(clippy::disallowed_methods, reason = "the adapter calls egui")]
pub fn adapter() {}
