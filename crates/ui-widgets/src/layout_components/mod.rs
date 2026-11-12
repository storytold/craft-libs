//! The layout components `Stack`, `Row` and `Grid`: containers with no surface, header or footer whose body
//! lays its children out (6.3).

use crate::contract::{
    Access, BodyCx, Common, Component, Container, ContainerProps, Cx, EndRegion, FooterPlace, FooterRegion, Gap, HeaderRegion, LabelScheme, Pad,
    Role, Scroll, Sizing, ThemeScope,
};
use crate::layout::{BodyLayout, RowAlign};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{ColumnWidth, Insets, Metrics, Sense};
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// The style of the layout components: no minimum size, and padding only at the start edge, one
/// `space.pad.regular` per level of indent.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub struct LayoutStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl LayoutStyle {
    fn new(indent: f32) -> Self {
        let mut metrics = Metrics::new();
        metrics.padding = Insets::new(indent, 0.0, 0.0, 0.0);
        LayoutStyle { metrics }
    }
}

/// The closure that fills a header or footer region.
type RegionFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// The layout components. `[..]` names the `ContainerProps` builders the component forwards. A component that carries closures names its lifetime and its extra fields after the key.
/// The `Container` impl: the regions only a component with closures fills.
macro_rules! impl_container {
    ($name:ident) => {
        impl Container for $name {}
    };
    ($name:ident <$life:lifetime>) => {
        impl<$life> Container for $name<$life> {
            fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
                if let Some(content) = self.header.as_mut() {
                    content(ui, cx);
                }
            }

            fn footer(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
                if let Some(content) = self.footer.as_mut() {
                    content(ui, cx);
                }
            }
        }
    };
}

macro_rules! layout_component {
    ($(#[$doc:meta])* $name:ident, $key:ident, [$($fwd:ident),*] $(, <$life:lifetime> { $($field:ident: $ty:ty),* })?) => {
        $(#[$doc])*
        #[derive(crate::contract::Component, crate::contract::Props)]
        #[component(container)]
        pub struct $name $(<$life>)? {
            #[common]
            common: Common,
            #[container($($fwd),*)]
            props: ContainerProps,
            /// Levels of indent at the start edge.
            #[style_key]
            indent: u8,
            /// Whether the root senses the pointer over it.
            hover: bool,
            $($($field: $ty,)*)?
        }

        impl $(<$life>)? $name $(<$life>)? {
            /// The spacing between the children, and between a `Stack`'s header or footer and its body (unless
            /// `Stack::footer_gap` set the footer's).
            pub fn gap(mut self, gap: Gap) -> Self {
                let follows = Some(self.props.body_gap);
                if let Some(header) = self.props.header.filter(|header| header.gap == follows) {
                    self.props = self.props.header(header.gap(gap));
                }
                if let Some(footer) = self.props.footer.filter(|footer| footer.gap == follows) {
                    self.props = self.props.footer(footer.gap(gap));
                }
                self.props = self.props.body_gap(gap);
                self
            }

            /// The padding tier around the children.
            pub fn pad(mut self, pad: Pad) -> Self {
                self.props = self.props.pad(pad);
                self
            }

            /// Scrolls the children.
            pub fn scroll(mut self, scroll: Scroll) -> Self {
                self.props = self.props.scroll(scroll);
                self
            }

            /// Takes the available rect instead of the size of the children. Along the main axis of a `Stack` (y) or a
            /// `Row` (x) that is given the axis, each child that fills takes an equal share of the room less the
            /// extents of the children that do not fill and the gaps, whether they come before or after it.
            pub fn fill(mut self) -> Self {
                self.props = self.props.sizing(Sizing::Fill);
                self
            }

            /// Offers the children `height` of room, also inside a scrolling body: every child takes its content
            /// height, a child that fills or scrolls at most `height`, and the container takes what they took. For a
            /// host that shows content of either kind at a definite room, such as a story preview.
            pub fn room_height(mut self, height: f32) -> Self {
                self.props = self.props.room_height(height);
                self
            }

            // bregion: a row across a region, as high as its children.
            /// Takes the available width and the height of the children.
            pub fn fill_width(mut self) -> Self {
                self.props = self.props.sizing(Sizing::FillWidth);
                self
            }

            /// The labelled rows of the body and everything under it show their labels in `scheme`.
            pub fn labels(mut self, scheme: LabelScheme) -> Self {
                self.props = self.props.labels(scheme);
                self
            }

            /// The body and everything under it resolve their tokens through `theme`.
            pub fn theme(mut self, theme: ThemeScope) -> Self {
                self.props = self.props.theme(theme);
                self
            }

            /// Indents the children `levels` steps of `space.pad.regular` from the start edge: a row of a tree drawn
            /// as plain rows.
            pub fn indent(mut self, levels: u8) -> Self {
                self.indent = levels;
                self
            }

            /// Senses the pointer over the container, so its response reports hover: a row that marks what it
            /// stands for while the pointer is over it.
            pub fn hover(mut self) -> Self {
                self.hover = true;
                self
            }
        }

        impl $(<$life>)? Component for $name $(<$life>)? {
            const ROLE: Role = Role::GROUP;
            const ALT_ROLES: &'static [Role] = &[Role::None];
            type Style = LayoutStyle;
            type Output = Response;

            fn style(cx: &StyleCx<'_>, key: $key) -> LayoutStyle {
                LayoutStyle::new(f32::from(key.indent) * cx.tokens.space.pad.regular)
            }

            fn sense(&self) -> Sense {
                if self.hover { Sense::HOVER } else { Sense::empty() }
            }

            /// A labelled body is a group named by its label, so a named region of a panel reaches assistive tech. A
            /// scrolled body is a group too: its scroll region is a Tab stop and a target of Page keys. An unlabelled,
            /// unscrolled one lays out its children and is no node of its own.
            fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
                if self.props.scroll.is_none() && cx.resolved.label.is_empty() {
                    access.role(Role::None);
                }
            }

            fn finish(self, cx: Cx<'_, Self>) -> Response {
                cx.core.response
            }
        }

        impl_container!($name $(<$life>)?);
    };
}

layout_component!(
    /// Children one under another. A header and a footer sit outside the body: the footer stays in the room under
    /// a collection that fills it.
    Stack,
    StackStyleKey,
    [command_scope, surface],
    <'a> { header: Option<RegionFn<'a>>, footer: Option<RegionFn<'a>> }
);
layout_component!(
    /// Children side by side.
    Row,
    RowStyleKey,
    []
);
layout_component!(
    /// Children row-major over a column list: a child in flow takes the next cell.
    Grid,
    GridStyleKey,
    []
);

impl<'a> Stack<'a> {
    /// A stack that takes the size of its children.
    pub fn new() -> Self {
        Stack {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Content).layout(BodyLayout::stack()),
            indent: 0,
            hover: false,
            header: None,
            footer: None,
        }
    }

    /// The header's content: the closure fills a region above the body, outside its scroll. The header is the first
    /// of the stack's rows: the stack's gap separates it from the body.
    #[must_use]
    pub fn header(mut self, content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        let gap = self.props.body_gap;
        self.props = self.props.header(HeaderRegion::new().gap(gap));
        self.header = Some(Box::new(content));
        self
    }

    /// The footer's content: the closure fills a region under the body, outside its scroll. The body takes the room
    /// the footer leaves. The footer is the last of the stack's rows: the stack's gap separates it from the body.
    #[must_use]
    pub fn footer(mut self, content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        let gap = self.props.body_gap;
        self.props = self.props.footer(FooterRegion::new().gap(gap));
        self.footer = Some(Box::new(content));
        self
    }

    /// The footer stands in a column at the end edge, beside the body and at its top, as wide as its widest child;
    /// a child that fills the width (a button with `Sizing::FillWidth`) takes that width. Set it after [`footer`](Stack::footer).
    #[must_use]
    pub fn footer_beside(mut self) -> Self {
        self.props.end = Some(EndRegion::new());
        self.props = self.props.footer(FooterRegion::new().place(FooterPlace::EndTop));
        self
    }

    /// The space `gap` between the body and the footer's content. Set it after [`footer`](Stack::footer).
    #[must_use]
    pub fn footer_gap(mut self, gap: Gap) -> Self {
        if let Some(footer) = self.props.footer {
            self.props = self.props.footer(footer.gap(gap));
        }
        self
    }

    /// The padding `pad` on the header's two sides, none above or below it: a field over a table starts where the
    /// table's text does. Set it after [`header`](Stack::header).
    #[must_use]
    pub fn header_inset(mut self, pad: Pad) -> Self {
        if let Some(header) = self.props.header {
            self.props = self.props.header(header.inset(pad));
        }
        self
    }
}

impl Default for Stack<'_> {
    fn default() -> Self {
        Stack::new()
    }
}

impl Row {
    /// A row that takes the size of its children.
    pub fn new() -> Self {
        Row { common: Common::new(), props: ContainerProps::new().sizing(Sizing::Content).layout(BodyLayout::row()), indent: 0, hover: false }
    }

    /// Whether children continue on a new line.
    pub fn wrap(mut self, on: bool) -> Self {
        self.props.layout = std::mem::replace(&mut self.props.layout, BodyLayout::Stack).wrap(on);
        self
    }

    /// Where the block of children sits along the row.
    pub fn align(mut self, align: RowAlign) -> Self {
        self.props.layout = std::mem::replace(&mut self.props.layout, BodyLayout::Stack).align(align);
        self
    }
}

impl Default for Row {
    fn default() -> Self {
        Row::new()
    }
}

impl Grid {
    /// A grid over `columns` that takes the size of its cells.
    pub fn new(columns: &'static [ColumnWidth]) -> Self {
        Grid {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Content).layout(BodyLayout::grid(columns)),
            indent: 0,
            hover: false,
        }
    }
}
