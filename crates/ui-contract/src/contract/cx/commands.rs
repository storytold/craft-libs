//! Command access (`CommandAccess`, `Commands`) on `Cx`, `BodyCx` and `ItemCx`.

use super::body::BodyCx;
use super::context::Cx;
use super::item::ItemCx;
use crate::contract::Component;
use crate::runtime::commands::{CommandAccess, CommandChain, Commands};
use egui::{Context, Id};

impl<C: Component> CommandAccess for Cx<'_, C> {
    fn command_ctx(&self) -> &Context {
        &self.core.ctx
    }

    fn command_chain(&self) -> &CommandChain {
        self.core.env.command_chain()
    }

    fn command_owner(&self) -> Option<Id> {
        Some(self.core.id)
    }
}

impl<C: Component> Commands for Cx<'_, C> {}

impl CommandAccess for BodyCx<'_> {
    fn command_ctx(&self) -> &Context {
        self.memory.ctx()
    }

    fn command_chain(&self) -> &CommandChain {
        self.env.command_chain()
    }

    fn command_owner(&self) -> Option<Id> {
        None
    }
}

impl Commands for BodyCx<'_> {}

impl<K> CommandAccess for ItemCx<'_, '_, K> {
    fn command_ctx(&self) -> &Context {
        self.memory.ctx()
    }

    fn command_chain(&self) -> &CommandChain {
        self.env.command_chain()
    }

    fn command_owner(&self) -> Option<Id> {
        None
    }
}

impl<K> Commands for ItemCx<'_, '_, K> {}
