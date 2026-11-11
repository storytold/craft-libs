//! Collections with columns and reordering.

use super::kit::{Dismissed, ROW_H, Row};
use craft_behavior::{ItemSource, Selection, SpanLayout};
use craft_ui::contract::{Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, Role};
use craft_ui::style::StyleCx;
use craft_ui_test_kit::contract::PlainStyle;

/// A list of rows at `ROW_H` with the collection props the trial gives.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub struct Table {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
}

impl Table {
    pub fn new(items: CollectionProps) -> Self {
        Table { common: Common::new(), props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill), items }
    }
}

impl Component for Table {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: TableStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Table {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, ROW_H)
    }
}
