@group(0) @binding(0) var<storage, read> parameters: array<vec4f>;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let dimensions = vec4u(parameters[0]);
    let stride = dimensions.x / 2u;
    if id.x >= stride || id.y >= dimensions.w {
        return;
    }
    let pair = id.y * stride + id.x;
    let pixels = compiled_16bit(vec2f(input[pair * 2u], input[pair * 2u + 1u]));
    output[pair + dimensions.z * stride] = pixels.x | (pixels.y << 16u);
}
