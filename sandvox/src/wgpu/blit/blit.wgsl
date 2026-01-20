
struct VertexOutput {
    @builtin(position)
    position: vec4f,

    @location(0)
    uv: vec2f,
}

struct BlitData {
    source_offset: vec2f,
    source_size: vec2f,
    target_offset: vec2f,
    target_size: vec2f,
}

@group(0)
@binding(0)
var source_texture: texture_2d<f32>;

@group(0)
@binding(1)
var source_sampler: sampler;

@group(0)
@binding(2)
var<storage, read> blit_data: array<BlitData>;

const quad: array<vec2f, 4> = array(
    vec2f(0.0, 0.0),
    vec2f(0.0, 1.0),
    vec2f(1.0, 0.0),
    vec2f(1.0, 1.0),
);

@vertex
fn blit_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let uv = quad[vertex_index];
    let blit_data = blit_data[instance_index];

    let source_position = blit_data.source_offset + uv * blit_data.source_size;
    let target_position = blit_data.target_offset + uv * blit_data.target_size;

    let clip_position = vec4f(target_position * vec2f(2, -2) + vec2f(-1, 1), 0, 1);

    return VertexOutput(
        clip_position,
        source_position,
    );
}

@fragment
fn blit_fragment(input: VertexOutput) -> @location(0) vec4f {
    let color = textureSample(source_texture, source_sampler, input.uv);
    return color;
}
