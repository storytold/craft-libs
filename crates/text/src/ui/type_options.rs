//! The vocabulary of the Type Options section and the type bar: anti-aliasing methods and text orientation.

use craft_ui::runtime::env::Label;

craft_ui::words! {
    /// The words of the type options, English by default.
    #[non_exhaustive]
    pub struct TypeOptionWords {
        anti_alias: "typeOptions.antiAlias" = "Anti-aliasing method";
        orientation: "typeOptions.orientation" = "Text orientation";
        none: "typeOptions.antiAliasNone" = "None";
        sharp: "typeOptions.antiAliasSharp" = "Sharp";
        crisp: "typeOptions.antiAliasCrisp" = "Crisp";
        strong: "typeOptions.antiAliasStrong" = "Strong";
        smooth: "typeOptions.antiAliasSmooth" = "Smooth";
        windows_lcd: "typeOptions.antiAliasWindowsLcd" = "Windows LCD";
        windows: "typeOptions.antiAliasWindows" = "Windows";
        horizontal: "typeOptions.horizontal" = "Horizontal type";
        vertical: "typeOptions.vertical" = "Vertical type";
    }
}

/// How the edges of type are anti-aliased.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextAntiAlias {
    /// No anti-aliasing.
    None,
    /// Sharp.
    Sharp,
    /// Crisp.
    Crisp,
    /// Strong.
    Strong,
    /// Smooth.
    Smooth,
    /// Windows LCD.
    WindowsLcd,
    /// Windows.
    Windows,
}

impl TextAntiAlias {
    /// Every method, in menu order.
    pub const ALL: [TextAntiAlias; 7] = [
        TextAntiAlias::None,
        TextAntiAlias::Sharp,
        TextAntiAlias::Crisp,
        TextAntiAlias::Strong,
        TextAntiAlias::Smooth,
        TextAntiAlias::WindowsLcd,
        TextAntiAlias::Windows,
    ];

    /// The methods the type bar's menu offers.
    pub const BAR: [TextAntiAlias; 5] =
        [TextAntiAlias::None, TextAntiAlias::Sharp, TextAntiAlias::Crisp, TextAntiAlias::Strong, TextAntiAlias::Smooth];

    /// The method's name in `words`.
    pub fn label(self, words: &TypeOptionWords) -> &Label {
        match self {
            TextAntiAlias::None => &words.none,
            TextAntiAlias::Sharp => &words.sharp,
            TextAntiAlias::Crisp => &words.crisp,
            TextAntiAlias::Strong => &words.strong,
            TextAntiAlias::Smooth => &words.smooth,
            TextAntiAlias::WindowsLcd => &words.windows_lcd,
            TextAntiAlias::Windows => &words.windows,
        }
    }
}

/// The direction the text runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextOrientation {
    /// Horizontal lines.
    Horizontal,
    /// Vertical lines.
    Vertical,
}

impl TextOrientation {
    /// The other orientation.
    pub fn toggled(self) -> Self {
        match self {
            TextOrientation::Horizontal => TextOrientation::Vertical,
            TextOrientation::Vertical => TextOrientation::Horizontal,
        }
    }
}
