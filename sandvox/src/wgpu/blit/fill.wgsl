
struct VertexOutput {
    @builtin(position)
    position: vec4f,

    @location(0)
    color: vec4f,
}

struct FillData {
    color: vec4f,
    target_offset: vec2f,
    target_size: vec2f,
}

@group(0)
@binding(0)
var<storage, read> fill_data: array<FillData>;

const quad: array<vec2f, 4> = array(
    vec2f(0.0, 0.0),
    vec2f(0.0, 1.0),
    vec2f(1.0, 0.0),
    vec2f(1.0, 1.0),
);

@vertex
fn fill_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let uv = quad[vertex_index];
    let fill_data = fill_data[instance_index];

    let target_position = fill_data.target_offset + uv * fill_data.target_size;
    let clip_position = vec4f(target_position * vec2f(2, -2) + vec2f(-1, 1), 0, 1);

    return VertexOutput(
        clip_position,
        fill_data.color,
    );
}

@fragment
fn fill_fragment(input: VertexOutput) -> @location(0) vec4f {
    return input.color;
}
