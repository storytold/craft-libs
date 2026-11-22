// The CPU rounds each product before adding. Canonicalizing signed zeros
// prevents contraction at ties without changing the quantized output.
fn individual_product(a: vec2f, b: vec2f) -> vec2f {
    let bits = bitcast<vec2u>(a * b);
    return bitcast<vec2f>(select(bits, vec2u(0u), bits == vec2u(0x80000000u)));
}

fn separable_blend(mode: u32, f: vec2f, b: vec2f) -> vec2f {
    switch mode {
        case 1u: { return f + b; }
        case 2u: { return b - f; }
        case 3u: { return individual_product(f, b); }
        case 4u: { return 1.0 - individual_product(1.0 - f, 1.0 - b); }
        case 5u, 13u: {
            let pivot = select(b, f, mode == 13u);
            let dark = individual_product(2.0 * f, b);
            let light = 1.0 - individual_product(2.0 * (1.0 - f), 1.0 - b);
            return select(light, dark, pivot < vec2f(0.5));
        }
        case 6u: { return max(f, b); }
        case 7u: { return min(f, b); }
        case 8u: { return abs(f - b); }
        case 9u: { return individual_product(individual_product(1.0 - 2.0 * f, b), b) + individual_product(2.0 * f, b); }
        case 10u: { return select(b / max(1.0 - f, vec2f(0.001)), vec2f(1.0), f >= vec2f(0.999)); }
        case 11u: { return select(1.0 - (1.0 - b) / max(f, vec2f(0.001)), vec2f(0.0), f <= vec2f(0.001)); }
        case 12u: { return f + b - 1.0; }
        case 14u: { return f + b - individual_product(2.0 * f, b); }
        case 15u: { return select(b / max(f, vec2f(0.001)), b, f <= vec2f(0.001)); }
        case 16u: { return b + 2.0 * f - 1.0; }
        case 20u: { return b + (f - 0.5) * 2.0; }
        case 36u: {
            let dark = b - individual_product(individual_product(1.0 - 2.0 * f, b), 1.0 - b);
            let light = b + individual_product(2.0 * f - 1.0, sqrt(max(vec2f(0.0), b)) - b);
            return select(light, dark, f <= vec2f(0.5));
        }
        default: { return f; }
    }
}

fn blend_channels(mode: u32, f: vec2f, bg: vec2f, opacity: vec2f) -> vec2f {
    switch mode {
        case 21u: { return bg + individual_product(f - bg, opacity); }
        case 32u: { return bg + individual_product(f, opacity); }
        case 33u: { return bg - individual_product(f, opacity); }
        case 34u: { return bg + individual_product(2.0 * f - 1.0, opacity); }
        case 35u: {
            let positive = f > vec2f(0.0);
            let divided = select(vec2f(1.0), bg / select(vec2f(1.0), f, positive), positive);
            return bg + individual_product(divided - bg, opacity);
        }
        default: {
            let color = clamp(separable_blend(mode, f, bg), vec2f(0.0), vec2f(1.0));
            return bg + individual_product(color - bg, opacity);
        }
    }
}
