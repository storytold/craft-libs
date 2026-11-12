//! Walking the paragraphs of a block, through tables of any depth.

use std::sync::Arc;

use crate::{Block, Paragraph};

/// Call `f` on every paragraph of a block, in document order, however deep its tables nest. The
/// walk keeps its own stack instead of recursing, so it is complete: a text box it doesn't reach
/// is shown nowhere, which pruning relies on. `_depth` is unused (callers keep passing it).
pub fn each_para<'a>(b: &'a Block, _depth: usize, f: &mut dyn FnMut(&'a Paragraph)) {
    // Most blocks are paragraphs, which need no stack.
    if let Block::Para(p) = b {
        return f(p);
    }
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                // Last first, so they come off the stack in document order.
                let cells = t.rows.iter().rev().flat_map(|r| r.cells.iter().rev());
                todo.extend(cells.flat_map(|c| c.blocks.iter().rev()).map(|cb| &**cb));
            }
        }
    }
}

/// [`each_para`], changing the paragraphs.
pub fn each_para_mut(b: &mut Block, _depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
    if let Block::Para(p) = b {
        return f(p);
    }
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                let cells = t.rows.iter_mut().rev().flat_map(|r| r.cells.iter_mut().rev());
                todo.extend(cells.flat_map(|c| c.blocks.iter_mut().rev()).map(Arc::make_mut));
            }
        }
    }
}
