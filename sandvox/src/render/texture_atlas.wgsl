
struct AtlasSlot {
    offset: vec2f,
    size: vec2f,
}

override num_source_textures: u32;

@group(0)
@binding(0)
var source_textures: binding_array<texture_2d<f32>>;

@group(0)
@binding(1)
var source_sampler: sampler;

@group(0)
@binding(2)
var<storage, read> atlas_data: array<AtlasSlot>;

struct VertexOutput {
    @builtin(position)
    position: vec4f,

    @location(0)
    uv: vec2f,

    @location(1)
    @interpolate(flat, either)
    source_index: u32,
}

@vertex
fn blit_vertex(
    @builtin(vertex_index)
    vertex_index: u32,
    @builtin(instance_index)
    instance_index: u32,
) -> VertexOutput {
    let entry = atlas_data[instance_index];

    // triangle strip vertex positions from vertex indices
    //
    // > Triangle primitives are composed from (vl.0, vl.1, vl.2), then (vl.2, vl.1, vl.3), [...]
    // https://gpuweb.github.io/gpuweb/#primitive-assembly
    //
    // 2----3
    // |\   |
    // | \  |
    // |  \ |
    // |   \|
    // 0----1
    let uv = vec2f(
        f32(vertex_index & 1),
        f32((vertex_index >> 1) & 1),
    );

    // uv in source image
    // for now the source positions are just these, but we will add padding where we sample from the source image outside of [0, 1]^2
    let source = uv;

    // uv in atlas
    let destination_uv = entry.offset + uv * entry.size;
    let destination_clip = vec4f(
        2 * destination_uv.x - 1,
        -2 * destination_uv.y + 1,
        0,
        1
    );

    return VertexOutput(
        destination_clip,
        source,
        instance_index,
    );
}

@fragment
fn blit_fragment(input: VertexOutput) -> @location(0) vec4f {
    let color = textureSample(source_textures[input.source_index], source_sampler, input.uv);
    return color;
}
