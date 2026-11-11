//! Motion drivers (`Drives`) on every context.

use super::body::BodyCx;
use super::context::{Cx, CxCore};
use super::item::ItemCx;
use crate::contract::Component;
use crate::runtime::motion::{Driver, Drives};

impl Drives for CxCore<'_> {
    fn driver(&self) -> Driver<'_> {
        Driver { stage: self.memory, base: self.id, policy: self.env.motion() }
    }
}

impl<C: Component> Drives for Cx<'_, C> {
    fn driver(&self) -> Driver<'_> {
        self.core.driver()
    }
}

impl Drives for BodyCx<'_> {
    fn driver(&self) -> Driver<'_> {
        Driver { stage: self.memory, base: self.id, policy: self.env.motion() }
    }
}

impl<K> Drives for ItemCx<'_, '_, K> {
    fn driver(&self) -> Driver<'_> {
        Driver { stage: self.memory, base: self.id, policy: self.env.motion() }
    }
}
