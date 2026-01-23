const PI: f32 = 3.141592653589793;

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
    time: f32,
    // padding: 4 bytes
    camera_matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> frame_uniform: FrameUniform;

@group(0)
@binding(1)
var default_sampler: sampler;

@group(0)
@binding(2)
var atlas_texture: texture_2d<f32>;

struct AtlasEntry {
    uv_offset: vec2f,
    uv_size: vec2f,
}

@group(0)
@binding(3)
var<storage, read> atlas_data: array<AtlasEntry>;




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
    // todo: figure out where the sun is (https://iurietarlev.github.io/SunpathDiagram/),
    // but move this out of shader and send light direction/color via frame uniform
    //let sun_phase = frame_uniform.time / 60.0 * 2.0 * PI;
    //let light_dir = normalize(vec3f(sin(sun_phase), cos(sun_phase), 0.5));
    let light_color = vec3f(1);
    let light_dir = normalize(vec3f(0.5, 1, 0.5));
    let normal = normalize(input.normal.xyz);
    let brightness = 0.5 * dot(normal, light_dir) + 0.5;

    let uv = atlas_map_uv(input.texture_id, input.uv);
    var color = textureSample(atlas_texture, default_sampler, uv);

    color = vec4f(color.rgb * brightness * light_color, 1);
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
