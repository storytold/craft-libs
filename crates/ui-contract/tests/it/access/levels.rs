//! The levels and link addresses that sub-nodes state reach the tree, where the kit's readers find them.

use craft_behavior::PartKey;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, Name, PaintUi, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response, Vec2};

const HEADING: PartKey = PartKey::new("heading");
const ITEM: PartKey = PartKey::new("item");
const LINK: PartKey = PartKey::new("link");
const URL: &str = "https://example.org/harbour";

/// A content painter's page: a heading at level 2, a list item at level 3 and a link, each a sub-node with bounds.
#[derive(craft_ui::contract::Component)]
struct Page {
    #[common]
    common: Common,
}

impl Component for Page {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PageStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(300.0, 90.0)
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, _: &Cx<'_, Self>) {}

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let line = |at: f32| Rect::from_min_size(Pos2::new(cx.rect.min.x, cx.rect.min.y + 30.0 * at), Vec2::new(cx.rect.width(), 30.0));
        let heading = access.node(HEADING, Role::HEADING).level(2).bounds(line(0.0));
        heading.name = Name::Text("The harbour".to_owned());
        let item = access.node(ITEM, Role::LIST_ITEM).level(3).bounds(line(1.0));
        item.name = Name::Text("Boats".to_owned());
        let link = access.node(LINK, Role::LINK).url(URL).bounds(line(2.0));
        link.name = Name::Text("Charts".to_owned());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A heading's and a list item's level and a link's address, as a sub-node states them, read back by key. A reader
/// that looked at another property, or an emitter that dropped a sub-node's address, would read none.
#[trial]
fn a_sub_nodes_level_and_link_address_read_back_from_the_tree() {
    let mut fixture = Fixture::new(|ui| {
        let _ = Page { common: Common::new() }.label("Page").show(ui);
    });
    fixture.settle();
    let tree = fixture.tree();
    let one = |role: Role| *tree.with_role(role).first().unwrap_or_else(|| panic!("a {role:?} node"));
    let (heading, item, link) = (one(Role::HEADING), one(Role::LIST_ITEM), one(Role::LINK));
    assert_eq!(tree.level_of(heading), Some(2), "the heading's level");
    assert_eq!(tree.level_of(item), Some(3), "the list item's level");
    assert_eq!(tree.level_of(link), None, "a link states no level");
    assert_eq!(tree.url_of(link), Some(URL), "the link's address");
    assert_eq!(tree.url_of(heading), None, "a heading has no address");
}
