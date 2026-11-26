//! The `Role` constants: one per AccessKit role the library uses.

use crate::runtime::access::Role;

impl Role {
    /// A colour well.
    pub const COLOR_WELL: Role = Role::Node(accesskit::Role::ColorWell);
    /// A button.
    pub const BUTTON: Role = Role::Node(accesskit::Role::Button);
    /// A check box.
    pub const CHECK_BOX: Role = Role::Node(accesskit::Role::CheckBox);
    /// A radio button.
    pub const RADIO_BUTTON: Role = Role::Node(accesskit::Role::RadioButton);
    /// A switch.
    pub const SWITCH: Role = Role::Node(accesskit::Role::Switch);
    /// A link.
    pub const LINK: Role = Role::Node(accesskit::Role::Link);
    /// A grouping of controls.
    pub const GROUP: Role = Role::Node(accesskit::Role::Group);
    /// A radio group.
    pub const RADIO_GROUP: Role = Role::Node(accesskit::Role::RadioGroup);
    /// A list.
    pub const LIST: Role = Role::Node(accesskit::Role::List);
    /// A list item.
    pub const LIST_ITEM: Role = Role::Node(accesskit::Role::ListItem);
    /// A list box.
    pub const LIST_BOX: Role = Role::Node(accesskit::Role::ListBox);
    /// An option of a list box.
    pub const LIST_BOX_OPTION: Role = Role::Node(accesskit::Role::ListBoxOption);
    /// A slider.
    pub const SLIDER: Role = Role::Node(accesskit::Role::Slider);
    /// A spin button.
    pub const SPIN_BUTTON: Role = Role::Node(accesskit::Role::SpinButton);
    /// A progress bar.
    pub const PROGRESS_INDICATOR: Role = Role::Node(accesskit::Role::ProgressIndicator);
    /// A scroll bar.
    pub const SCROLL_BAR: Role = Role::Node(accesskit::Role::ScrollBar);
    /// A splitter between panes.
    pub const SPLITTER: Role = Role::Node(accesskit::Role::Splitter);
    /// A tab.
    pub const TAB: Role = Role::Node(accesskit::Role::Tab);
    /// A row of tabs.
    pub const TAB_LIST: Role = Role::Node(accesskit::Role::TabList);
    /// The panel a tab shows.
    pub const TAB_PANEL: Role = Role::Node(accesskit::Role::TabPanel);
    /// A menu.
    pub const MENU: Role = Role::Node(accesskit::Role::Menu);
    /// A region of navigation links.
    pub const NAVIGATION: Role = Role::Node(accesskit::Role::Navigation);
    /// A menu bar.
    pub const MENU_BAR: Role = Role::Node(accesskit::Role::MenuBar);
    /// A menu item.
    pub const MENU_ITEM: Role = Role::Node(accesskit::Role::MenuItem);
    /// A menu item with a check mark.
    pub const MENU_ITEM_CHECK_BOX: Role = Role::Node(accesskit::Role::MenuItemCheckBox);
    /// A menu item that is one of a group.
    pub const MENU_ITEM_RADIO: Role = Role::Node(accesskit::Role::MenuItemRadio);
    /// A single-line text input.
    pub const TEXT_INPUT: Role = Role::Node(accesskit::Role::TextInput);
    /// A multi-line text input.
    pub const MULTILINE_TEXT_INPUT: Role = Role::Node(accesskit::Role::MultilineTextInput);
    /// Static text.
    pub const LABEL: Role = Role::Node(accesskit::Role::Label);
    /// A heading.
    pub const HEADING: Role = Role::Node(accesskit::Role::Heading);
    /// A dialog.
    pub const DIALOG: Role = Role::Node(accesskit::Role::Dialog);
    /// A combo box.
    pub const COMBO_BOX: Role = Role::Node(accesskit::Role::ComboBox);
    /// A tree.
    pub const TREE: Role = Role::Node(accesskit::Role::Tree);
    /// A tree item.
    pub const TREE_ITEM: Role = Role::Node(accesskit::Role::TreeItem);
    /// A table.
    pub const TABLE: Role = Role::Node(accesskit::Role::Table);
    /// A table row.
    pub const ROW: Role = Role::Node(accesskit::Role::Row);
    /// A table cell.
    pub const CELL: Role = Role::Node(accesskit::Role::Cell);
    /// A grid: a table whose cells are navigated by arrow keys, like a calendar.
    pub const GRID: Role = Role::Node(accesskit::Role::Grid);
    /// A cell of a grid.
    pub const GRID_CELL: Role = Role::Node(accesskit::Role::GridCell);
    /// A column header.
    pub const COLUMN_HEADER: Role = Role::Node(accesskit::Role::ColumnHeader);
    /// A row header.
    pub const ROW_HEADER: Role = Role::Node(accesskit::Role::RowHeader);
    /// An image.
    pub const IMAGE: Role = Role::Node(accesskit::Role::Image);
    /// A tooltip.
    pub const TOOLTIP: Role = Role::Node(accesskit::Role::Tooltip);
    /// A status message.
    pub const STATUS: Role = Role::Node(accesskit::Role::Status);
    /// An alert.
    pub const ALERT: Role = Role::Node(accesskit::Role::Alert);
    /// An alert dialog.
    pub const ALERT_DIALOG: Role = Role::Node(accesskit::Role::AlertDialog);
    /// A toolbar.
    pub const TOOLBAR: Role = Role::Node(accesskit::Role::Toolbar);
    /// A canvas.
    pub const CANVAS: Role = Role::Node(accesskit::Role::Canvas);
    /// An object drawn on a canvas: a node of a node graph.
    pub const GRAPHICS_OBJECT: Role = Role::Node(accesskit::Role::GraphicsObject);
    /// A document: the readable page or text body of a reading view.
    pub const DOCUMENT: Role = Role::Node(accesskit::Role::Document);
    /// A paragraph of text.
    pub const PARAGRAPH: Role = Role::Node(accesskit::Role::Paragraph);
    /// One line of text, with its characters' places: only a text-run sub-node (`SubNode::text_run`) has it.
    pub const TEXT_RUN: Role = Role::Node(accesskit::Role::TextRun);
    /// The marker of a list item: its bullet or number.
    pub const LIST_MARKER: Role = Role::Node(accesskit::Role::ListMarker);
}
