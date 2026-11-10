//! [`RawEntry`]: an object the model does not understand, kept by class name and uid so nothing
//! silently disappears; [`super::ModelReport`] counts them.

use super::raw::RawObject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub class: String,
    pub uid: Option<u64>,
}

impl RawEntry {
    pub fn of(object: &RawObject) -> RawEntry {
        RawEntry { class: object.class.clone(), uid: object.uid() }
    }
}
