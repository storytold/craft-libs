fn f(r: &Response, ctx: &Context) {
    if r.drag_started() || r.clicked() {}
    let _ = r.drag_delta();
    let _ = r.hover_pos();
    let _ = ctx.pointer_latest_pos();
}
