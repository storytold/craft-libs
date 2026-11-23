//! The toolbar's words and the annotation commands.

use crate::annotate::{AnnotationGroup, AnnotationKind, AnnotationTool};

/// The colours the toolbar offers, Acrobat's quick colours for comments: yellow, orange, red, pink, purple, blue,
/// light blue, green, grey and black. Acrobat's colour menu also has "Other Color", which the app may add.
pub const PALETTE: [[u8; 3]; 10] = [
    [255, 240, 0],
    [255, 138, 0],
    [227, 33, 33],
    [255, 94, 163],
    [138, 64, 209],
    [0, 120, 214],
    [92, 191, 250],
    [46, 158, 92],
    [128, 128, 128],
    [0, 0, 0],
];

craft_ui::words! {
    /// The toolbar's group names, the kind names the reader sees, the palette colour names, the names of the
    /// undo steps the tool's edits make and the words of the "New Annotation..." form, English by default.
    #[non_exhaustive]
    pub struct AnnotateWords {
        comment_group: "annotate.commentGroup" = "Comment";
        highlight_group: "annotate.highlightGroup" = "Highlight";
        draw_group: "annotate.drawGroup" = "Draw";
        select: "annotate.select" = "Select";
        toolbar: "annotate.toolbar" = "Annotation tools";
        color: "annotate.color" = "Color";
        palette: "annotate.palette" = "Colors";
        width: "annotate.width" = "Line width";

        /// The reader's names of the annotation kinds: card headings and sort groups.
        kind_note: "annotate.kindNote" = "Note";
        kind_text_box: "annotate.kindTextBox" = "Text box";
        kind_callout: "annotate.kindCallout" = "Callout";
        kind_attach: "annotate.kindAttach" = "Attachment";
        kind_highlight: "annotate.kindHighlight" = "Highlight";
        kind_underline: "annotate.kindUnderline" = "Underline";
        kind_strike_out: "annotate.kindStrikeOut" = "Strikethrough";
        kind_squiggly: "annotate.kindSquiggly" = "Squiggly";
        kind_caret: "annotate.kindCaret" = "Insert text";
        kind_replace_text: "annotate.kindReplaceText" = "Replace text";
        kind_ink: "annotate.kindInk" = "Drawing";
        kind_line: "annotate.kindLine" = "Line";
        kind_arrow: "annotate.kindArrow" = "Arrow";
        kind_rectangle: "annotate.kindRectangle" = "Rectangle";
        kind_oval: "annotate.kindOval" = "Oval";
        kind_poly_line: "annotate.kindPolyLine" = "Polyline";
        kind_polygon: "annotate.kindPolygon" = "Polygon";
        kind_cloud: "annotate.kindCloud" = "Cloud";

        /// The names of the palette colours, in `PALETTE` order.
        palette_yellow: "annotate.paletteYellow" = "Yellow";
        palette_orange: "annotate.paletteOrange" = "Orange";
        palette_red: "annotate.paletteRed" = "Red";
        palette_pink: "annotate.palettePink" = "Pink";
        palette_purple: "annotate.palettePurple" = "Purple";
        palette_blue: "annotate.paletteBlue" = "Blue";
        palette_light_blue: "annotate.paletteLightBlue" = "Light blue";
        palette_green: "annotate.paletteGreen" = "Green";
        palette_grey: "annotate.paletteGrey" = "Grey";
        palette_black: "annotate.paletteBlack" = "Black";

        edit_select: "annotate.editSelect" = "Select Annotations";
        edit_move: "annotate.editMove" = "Move Annotations";
        edit_resize: "annotate.editResize" = "Resize Annotation";
        edit_reshape: "annotate.editReshape" = "Reshape Annotation";
        edit_create: "annotate.editCreate" = "Add Annotation";
        edit_delete: "annotate.editDelete" = "Delete Annotations";
        edit_erase: "annotate.editErase" = "Erase Ink";
        edit_open: "annotate.editOpen" = "Open Annotation";

        form_title: "annotate.formTitle" = "New Annotation";
        form_kind: "annotate.formKind" = "Kind";
        form_page: "annotate.formPage" = "Page";
        form_x: "annotate.formX" = "X (points)";
        form_y: "annotate.formY" = "Y (points)";
        form_width: "annotate.formWidth" = "Width (points)";
        form_height: "annotate.formHeight" = "Height (points)";
    }
}

impl AnnotateWords {
    /// The reader's name of `kind` ("Note", "Text box", "Strikethrough").
    pub fn kind(&self, kind: AnnotationKind) -> &str {
        use AnnotationKind as K;
        match kind {
            K::Note => self.kind_note.as_str(),
            K::TextBox => self.kind_text_box.as_str(),
            K::Callout => self.kind_callout.as_str(),
            K::Attach => self.kind_attach.as_str(),
            K::Highlight => self.kind_highlight.as_str(),
            K::Underline => self.kind_underline.as_str(),
            K::StrikeOut => self.kind_strike_out.as_str(),
            K::Squiggly => self.kind_squiggly.as_str(),
            K::Caret => self.kind_caret.as_str(),
            K::ReplaceText => self.kind_replace_text.as_str(),
            K::Ink => self.kind_ink.as_str(),
            K::Line => self.kind_line.as_str(),
            K::Arrow => self.kind_arrow.as_str(),
            K::Rectangle => self.kind_rectangle.as_str(),
            K::Oval => self.kind_oval.as_str(),
            K::PolyLine => self.kind_poly_line.as_str(),
            K::Polygon => self.kind_polygon.as_str(),
            K::Cloud => self.kind_cloud.as_str(),
        }
    }

    /// The name of palette colour `at` (an index into [`PALETTE`]); empty past the last.
    pub fn palette_color(&self, at: usize) -> &str {
        [
            &self.palette_yellow,
            &self.palette_orange,
            &self.palette_red,
            &self.palette_pink,
            &self.palette_purple,
            &self.palette_blue,
            &self.palette_light_blue,
            &self.palette_green,
            &self.palette_grey,
            &self.palette_black,
        ]
        .get(at)
        .map_or("", |name| name.as_str())
    }

    pub(super) fn group(&self, group: AnnotationGroup) -> &str {
        match group {
            AnnotationGroup::Comment => self.comment_group.as_str(),
            AnnotationGroup::Highlight => self.highlight_group.as_str(),
            AnnotationGroup::Draw => self.draw_group.as_str(),
        }
    }
}

/// The annotation tools as commands: the toolbar's buttons and flyout rows show their labels and icons, and an app
/// binds chords to them. `New` opens the "New Annotation..." form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "annotate")]
#[non_exhaustive]
pub enum AnnotateCommand {
    /// Select and edit annotations.
    #[command(label = "Select", icon = "pointer")]
    Select,
    /// Place a sticky note.
    #[command(label = "Add a sticky note", icon = "note")]
    Note,
    /// Place a text box.
    #[command(label = "Add a text box", icon = "text")]
    TextBox,
    /// Place a callout.
    #[command(label = "Add a callout", icon = "text")]
    Callout,
    /// Attach a file.
    #[command(label = "Attach file", icon = "link")]
    Attach,
    /// Highlight page text.
    #[command(label = "Highlight", icon = "brush")]
    Highlight,
    /// Underline page text.
    #[command(label = "Underline", icon = "text_underline")]
    Underline,
    /// Strike out page text.
    #[command(label = "Strikethrough", icon = "text_strikethrough")]
    StrikeOut,
    /// Underline page text with a wave.
    #[command(label = "Squiggly underline", icon = "spline")]
    Squiggly,
    /// Mark an insertion point.
    #[command(label = "Insert text", icon = "text_cursor")]
    Caret,
    /// Replace page text.
    #[command(label = "Replace text", icon = "brush_replace")]
    ReplaceText,
    /// Draw freehand.
    #[command(label = "Draw", icon = "pencil")]
    Ink,
    /// Draw a line.
    #[command(label = "Line", icon = "slash")]
    Line,
    /// Draw an arrow.
    #[command(label = "Arrow", icon = "triangle_right")]
    Arrow,
    /// Draw a rectangle.
    #[command(label = "Rectangle", icon = "square")]
    Rectangle,
    /// Draw an oval.
    #[command(label = "Oval", icon = "circle")]
    Oval,
    /// Draw connected lines.
    #[command(label = "Connected lines", icon = "pen_tool")]
    PolyLine,
    /// Draw a polygon.
    #[command(label = "Polygon", icon = "pentagon")]
    Polygon,
    /// Draw a cloud.
    #[command(label = "Cloud", icon = "cloud")]
    Cloud,
    /// Erase ink strokes.
    #[command(label = "Eraser", icon = "eraser")]
    Eraser,
    /// Place a shape or note by numbers.
    #[command(label = "New Annotation...")]
    New,
}

impl AnnotateCommand {
    /// The command that chooses `tool`.
    pub fn of(tool: AnnotationTool) -> Self {
        use AnnotationKind as K;
        match tool {
            AnnotationTool::Select => AnnotateCommand::Select,
            AnnotationTool::Eraser => AnnotateCommand::Eraser,
            AnnotationTool::Draw(kind) => match kind {
                K::Note => AnnotateCommand::Note,
                K::TextBox => AnnotateCommand::TextBox,
                K::Callout => AnnotateCommand::Callout,
                K::Attach => AnnotateCommand::Attach,
                K::Highlight => AnnotateCommand::Highlight,
                K::Underline => AnnotateCommand::Underline,
                K::StrikeOut => AnnotateCommand::StrikeOut,
                K::Squiggly => AnnotateCommand::Squiggly,
                K::Caret => AnnotateCommand::Caret,
                K::ReplaceText => AnnotateCommand::ReplaceText,
                K::Ink => AnnotateCommand::Ink,
                K::Line => AnnotateCommand::Line,
                K::Arrow => AnnotateCommand::Arrow,
                K::Rectangle => AnnotateCommand::Rectangle,
                K::Oval => AnnotateCommand::Oval,
                K::PolyLine => AnnotateCommand::PolyLine,
                K::Polygon => AnnotateCommand::Polygon,
                K::Cloud => AnnotateCommand::Cloud,
            },
        }
    }
}
