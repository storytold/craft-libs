fn style(cx: &StyleCx, key: ChipKey) -> ChipStyle {
    ChipStyle { fill: cx.tokens().accent.to_egui() }
}
