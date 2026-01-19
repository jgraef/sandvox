struct VertexInput {
    @location(0)
    position: vec4f,

    @location(1)
    color: vec4f,
}

struct VertexOutput {
    @builtin(position)
    fragment_position: vec4f,

    @location(1)
    color: vec4f,
}

struct FrameUniform {
    viewport_size: vec2u,
    // padding: 8 bytes
    matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> frame_uniform: FrameUniform;


@vertex
fn debug_vertex(input: VertexInput) -> VertexOutput {
    let clip_position = vec4f(vec2f(2, -2) * input.position.xy / vec2f(frame_uniform.viewport_size) + vec2f(-1, 1), 0.0, 1.0);

    return VertexOutput(
        clip_position,
        input.color,
    );
}

@fragment
fn debug_fragment(input: VertexOutput) -> @location(0) vec4f {
    return input.color;
}
