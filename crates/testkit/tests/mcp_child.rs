//! `McpChild` against a real child process. The child is this test program, started again with
//! `CRAFT_TESTKIT_MCP_CHILD` naming the server it acts as. The target has `harness = false`
//! because the standard test harness writes its own lines on stdout, which an MCP child must not
//! do. `main` runs each test whose name contains every argument that does not start with `-`, and
//! `--list` lists them. On `wasm32`, where the `mcp` module is not built, `main` does nothing.

#[cfg(not(target_arch = "wasm32"))]
#[path = "mcp_child/child.rs"]
mod child;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    child::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
