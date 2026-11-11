#[non_exhaustive]
pub struct ChipStyle {
    pub fill: Stateful<StyleColor>,
    pub radius: f32,
}

fn style(cx: &StyleCx, key: ChipKey) -> ChipStyle {
    let group = cx.token_group::<ChipColors>();
    let tokens = cx.tokens();
    ChipStyle { fill: Stateful::new(cx.color(group.fill)), radius: tokens.radius.control }
}

fn plain_colour(colour: Rgba) -> Color32 {
    colour.to_egui()
}

fn kind_width(kind: ChipKind) -> f32 {
    match kind {
        ChipKind::Wide => 2.0,
        ChipKind::Narrow => 1.0,
    }
}

fn has_focus(response: &Response) -> bool {
    if response.has_focus() { true } else { response.hovered() }
}

impl Chip {
    pub fn style_name(&self) -> &str {
        "chip"
    }

    pub fn style(&self) -> &ChipStyle {
        &self.style
    }

    pub fn tone(self, tone: Tone) -> Self {
        self.with(tone)
    }
}

#[cfg(test)]
mod tests {
    fn t(ui: &mut Ui) {
        let _ = Tokens::of(ui);
    }
}

#[derive(Component)]
pub struct Button {
    #[common]
    common: Common,
}

impl Button {
    pub fn stroke_style(self, stroke: StrokeStyle) -> Self {
        self.apply(stroke)
    }

    pub fn smart_quotes(self, quotes: Option<QuoteStyle>) -> Self {
        self.apply(quotes)
    }

    pub fn swatch(self, color: DataColor) -> Self {
        self.apply(color)
    }

    pub fn preview(&self, look: &QuickMaskLook) -> Rgba {
        look.color
    }
}

pub struct Plain;

impl Plain {
    pub fn fill(self, color: Color32) -> Self {
        self.apply(color)
    }
}

fn events(response: &Response, lexer: &Lexer, theme: &Theme) {
    if response.has_focus() {
        self.width = ColumnWidth::Fill;
    }
    if response.hovered() {
        self.hot = true;
    }
    let _ = lexer.tokens();
    let _ = theme.tokens();
    let _ = matches!(kind, Kind::Hover);
    let _ = cx.state() == other;
}

/// Built with a struct expression by a higher crate of the same library: its public field is hidden, so the type
/// stays out of the public API's non-exhaustive rule.
pub struct ChipCorner {
    #[doc(hidden)]
    pub radius: f32,
}
