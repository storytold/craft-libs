//! Addresses serialised as `0x…` strings, as Craft Anything reports them.

pub(crate) fn serialize<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{value:#x}"))
}
