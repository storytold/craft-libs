//! What an object header describes: a group or a dataset.

use super::dataset::Dataset;
use super::error::Hdf5Error;
use super::group::Group;
use super::message::Message;
use super::object_header;
use super::sizes::Address;
use super::storage::Storage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Object {
    Group(Group),
    Dataset(Dataset),
}

impl Object {
    /// Reads and classifies the object whose header is at `addr`.
    pub(crate) fn load(storage: &Storage, addr: Address) -> Result<Self, Hdf5Error> {
        let messages =
            object_header::read_messages(storage, addr)?.iter().map(|raw| Message::decode(raw, storage.sizes())).collect::<Result<Vec<_>, _>>()?;
        Self::classify(messages)
    }

    fn classify(messages: Vec<Message>) -> Result<Self, Hdf5Error> {
        let has = |f: fn(&Message) -> bool| messages.iter().any(f);
        if let Some(Message::SymbolTable { btree, heap }) = messages.iter().find(|m| matches!(m, Message::SymbolTable { .. })) {
            return Ok(Object::Group(Group { btree: *btree, heap: *heap }));
        }
        if has(|m| matches!(m, Message::NewStyleGroup)) {
            return Err(Hdf5Error::Unsupported("new-style group (link messages)".into()));
        }
        if has(|m| matches!(m, Message::Layout(_))) {
            return Ok(Object::Dataset(Dataset::from_messages(messages)?));
        }
        Err(Hdf5Error::Unsupported("object that is neither a group nor a dataset".into()))
    }
}
