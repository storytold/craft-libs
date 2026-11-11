fn f(ctx: &Context, response: &Response) {
    ctx.accesskit_node_builder(id, |node| node.set_label("x"));
    response.widget_info(|| info);
}
