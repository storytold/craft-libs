fn f(ctx: &Context) {
    let _ = ctx.animate_bool(id, on);
    ctx.request_repaint_after(delay);
}
