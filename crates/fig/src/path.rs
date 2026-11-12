//! Geometry blobs: the outlines a layer draws (`fillGeometry` / `strokeGeometry`, each entry's
//! `commandsBlob`). A blob is a run of commands, each one byte (`0` close, `1` move, `2` line,
//! `3` quadratic, `4` cubic) followed by its points as little-endian `f32` pairs.

/// Most commands read from one blob.
pub const MAX_COMMANDS: usize = 200_000;

/// A point: x, y.
pub type Point = [f32; 2];

/// One path command, in the order the blob holds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Close,
    Move(Point),
    Line(Point),
    /// Control point, end point.
    Quad(Point, Point),
    /// Two control points, end point.
    Cubic(Point, Point, Point),
}

/// The commands of blob `b`; `None` when it's truncated, has an unknown command, a value that
/// isn't finite, or more than [`MAX_COMMANDS`] commands.
pub fn decode(b: &[u8]) -> Option<Vec<Command>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let point = |i: &mut usize| -> Option<Point> {
        let mut p = [0f32; 2];
        for v in &mut p {
            *v = f32::from_le_bytes(b.get(*i..i.checked_add(4)?)?.try_into().ok()?);
            if !v.is_finite() {
                return None;
            }
            *i += 4;
        }
        Some(p)
    };
    while let Some(cmd) = b.get(i) {
        if out.len() >= MAX_COMMANDS {
            return None;
        }
        i += 1;
        out.push(match cmd {
            0 => Command::Close,
            1 => Command::Move(point(&mut i)?),
            2 => Command::Line(point(&mut i)?),
            3 => Command::Quad(point(&mut i)?, point(&mut i)?),
            4 => Command::Cubic(point(&mut i)?, point(&mut i)?, point(&mut i)?),
            _ => return None,
        });
    }
    Some(out)
}

/// The blob of `commands`. Callers pass finite values.
pub fn encode(commands: &[Command]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut put = |cmd: u8, points: &[Point]| {
        out.push(cmd);
        for [x, y] in points {
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
    };
    for c in commands {
        match *c {
            Command::Close => put(0, &[]),
            Command::Move(a) => put(1, &[a]),
            Command::Line(a) => put(2, &[a]),
            Command::Quad(a, b) => put(3, &[a, b]),
            Command::Cubic(a, b, c) => put(4, &[a, b, c]),
        }
    }
    out
}
