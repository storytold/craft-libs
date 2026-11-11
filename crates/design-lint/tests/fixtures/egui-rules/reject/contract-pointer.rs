fn f(r: &Response) {
    if r.drag_started() || r.clicked() {}
    let _ = r.drag_delta();
}
