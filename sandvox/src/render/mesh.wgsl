
struct VertexInput {
    // from vertex buffer
    @location(0)
    position: vec4f,

    @location(1)
    normal: vec4f,

    @location(2)
    uv: vec2f,

    @location(3)
    texture_id: u32,

    // from instance buffer
    @location(4) model0: vec4f,
    @location(5) model1: vec4f,
    @location(6) model2: vec4f,
    @location(7) model3: vec4f,
}

struct VertexOutput {
    @builtin(position)
    fragment_position: vec4f,

    @location(0)
    world_position: vec4f,

    @location(1)
    normal: vec4f,

    @location(2)
    uv: vec2f,

    @location(3)
    @interpolate(flat, either)
    texture_id: u32,
}

struct VertexOutputWireframe {
    @builtin(position)
    fragment_position: vec4f,
}

struct FrameUniform {
    viewport_size: vec2u,
    // padding: 8 bytes
    camera_matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> frame_uniform: FrameUniform;

@group(0)
@binding(1)
var atlas_texture: texture_2d<f32>;

struct AtlasEntry {
    uv_offset: vec2f,
    uv_size: vec2f,
}

@group(0)
@binding(2)
var<storage, read> atlas_data: array<AtlasEntry>;


@group(0)
@binding(3)
var default_sampler: sampler;



@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let model_matrix = mat4x4f(input.model0, input.model1, input.model2, input.model3);

    let world_position = model_matrix * input.position;
    let normal = model_matrix * input.normal;

    let fragment_position = frame_uniform.camera_matrix * world_position;

    return VertexOutput(
        fragment_position,
        world_position,
        normal,
        input.uv,
        input.texture_id,
    );
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
    let normal = normalize(input.normal.xyz);
    let light_dir = -normalize(vec3f(0.5, 1, 0.5));
    let attenuation = mix(0.6, 1.0, dot(normal, light_dir));

    let uv = atlas_map_uv(input.texture_id, input.uv);
    var color = textureSample(atlas_texture, default_sampler, uv);

    color = vec4f(color.rgb * attenuation, 1);
    return color;
}


@vertex
fn vertex_main_wireframe(input: VertexInput) -> VertexOutputWireframe {
    let model_matrix = mat4x4f(input.model0, input.model1, input.model2, input.model3);
    let world_position = model_matrix * input.position;
    let fragment_position = frame_uniform.camera_matrix * world_position;

    return VertexOutputWireframe(
        fragment_position,
    );
}

@fragment
fn fragment_main_wireframe(input: VertexOutputWireframe) -> @location(0) vec4f {
    return vec4f(0, 0, 0, 1);
}


fn atlas_map_uv(texture_id: u32, uv: vec2f) -> vec2f {
    let entry = atlas_data[texture_id];
    return entry.uv_offset + (uv % vec2f(1)) * entry.uv_size;
}
