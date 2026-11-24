// Addressing and 1/256-step bilinear taps shared by compiled grayscale and color filters.
fn limit(p: i32, n: i32, wrap: bool) -> u32 {
    if wrap {
        // The packed input axes are powers of two
        return u32(p) & u32(n - 1);
    }
    return u32(clamp(p, 0, n - 1));
}

fn address(p: vec2<i32>, size: vec2<u32>, wraps: vec2<bool>) -> u32 {
    let x = limit(p.x, i32(size.x), wraps.x);
    let y = limit(p.y, i32(size.y), wraps.y);
    return y * size.x + x;
}

struct BilinearTaps {
    base: vec2i,
    weights: vec2f,
}

fn stepped_taps(uv: vec2f, extent: vec2u, wraps: vec2<bool>) -> BilinearTaps {
    let size = vec2f(extent);
    let half_texel = 0.5 / size;
    let coords = select(clamp(uv, half_texel, 1.0 - half_texel), uv, wraps);
    let scaled = coords * size;
    // Split at truncation: that remainder is exact, while scaled - floor(scaled) can round a negative coordinate onto a tie
    let toward_zero = trunc(scaled);
    let remainder = scaled - toward_zero;
    let negative = remainder < vec2<f32>(0.0);
    let whole = vec2<i32>(toward_zero) - select(vec2<i32>(0), vec2<i32>(1), negative);
    // Below zero the fraction is remainder + 1: compare and round it in that form
    let previous = select(remainder < vec2<f32>(0.5), remainder < vec2<f32>(-0.5), negative);
    // Subtract the half-texel in separate integer units so a large coordinate cannot move a rounding tie
    let base = whole - select(vec2<i32>(0), vec2<i32>(1), previous);
    let steps = remainder * 256.0;
    let half_up = select(vec2<f32>(0.0), vec2<f32>(1.0), steps - floor(steps) >= vec2<f32>(0.5));
    let rounded = floor(steps) + half_up + select(vec2<f32>(0.0), vec2<f32>(256.0), negative);
    let weights = (rounded + select(vec2<f32>(-128.0), vec2<f32>(128.0), previous)) / 256.0;
    return BilinearTaps(base, weights);
}
