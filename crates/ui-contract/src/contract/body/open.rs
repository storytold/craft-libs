//! A container's open state: its session slot, the binding it reads and the write-back when it changed.

use super::BodyHooks;
use super::placement::opener_of;
use crate::contract::{Container, ContainerProps, Cx, Internal};
use crate::runtime::memory::{MemoryStage, SaveKey, sub};
use craft_behavior::{EditEvent, Mixed};
use egui::Id;
use serde::{Deserialize, Serialize};

/// The open state a container with no `Container::open` binding keeps: for the session, or saved under `save`.
#[derive(Clone, Default, Serialize, Deserialize)]
struct OpenState(Option<bool>);

fn open_slot<'s>(stage: &'s mut MemoryStage, id: Id, save: Option<&SaveKey>) -> &'s mut OpenState {
    match save {
        Some(key) => stage.saved::<OpenState>(key),
        None => stage.session::<OpenState>(sub(id, "open")),
    }
}

/// Whether the container is open this pass: its binding, else its opener's state, else its slot.
pub(super) fn open_now<C: Container>(comp: &mut C, stage: &mut MemoryStage, id: Id, props: &ContainerProps) -> bool {
    let initially_open = props.collapse.is_none_or(|collapse| collapse.initially_open);
    match comp.open() {
        Some(bind) => matches!(bind.get(), Some(Mixed::Same(true))),
        None => match opener_of(props) {
            Some(opener) => crate::contract::overlay::opener_open(stage, opener),
            None => *open_slot(stage, id, props.save.as_ref()).0.get_or_insert(initially_open),
        },
    }
}

impl BodyHooks<'_> {
    /// Writes the open state when it changed since it was read: to the binding, else to the session slot.
    pub(super) fn write_open<C: Container>(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        if self.open == self.open_at_read {
            return;
        }
        let id = cx.core.id;
        let opener = opener_of(comp.props(Internal::new())).cloned();
        match comp.open() {
            Some(mut bind) => {
                let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                bind.apply(&EditEvent::set(Mixed::Same(self.open_at_read), self.open, label, doc_key));
            }
            None => match &opener {
                Some(opener) => crate::contract::overlay::set_opener_open(&mut *cx.core.memory, opener, self.open),
                None => open_slot(&mut *cx.core.memory, id, self.save.as_ref()).0 = Some(self.open),
            },
        }
        self.open_at_read = self.open;
    }
}
