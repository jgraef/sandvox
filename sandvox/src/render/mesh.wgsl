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
    position: vec4f,

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
    position: vec4f,
}

struct FrameUniform {
    viewport_size: vec2u,
    time: f32,
    // padding: 4 bytes
    camera: Camera,
}

struct Camera {
    projection: mat4x4f,
    projection_inverse: mat4x4f,
    view: mat4x4f,
    view_inverse: mat4x4f,
    position: vec4f,
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
fn mesh_shaded_vertex(input: VertexInput) -> VertexOutput {
    let model_matrix = mat4x4f(input.model0, input.model1, input.model2, input.model3);

    let world_position = model_matrix * input.position;
    let normal = model_matrix * input.normal;

    let position = frame_uniform.camera.projection * frame_uniform.camera.view * world_position;

    return VertexOutput(
        position,
        world_position,
        normal,
        input.uv,
        input.texture_id,
    );
}

@fragment
fn mesh_shaded_fragment(input: VertexOutput) -> @location(0) vec4f {
    var color: vec4f;

    // todo: figure out where the sun is (https://iurietarlev.github.io/SunpathDiagram/),
    // but move this out of shader and send light direction/color via frame uniform
    //let sun_phase = frame_uniform.time / 60.0 * 2.0 * PI;
    //let light_dir = normalize(vec3f(sin(sun_phase), cos(sun_phase), 0.5));

    let light_color = vec3f(1);
    let light_dir = normalize(vec3f(0.5, 1, 0.5));
    let normal = normalize(input.normal.xyz);
    let brightness = 0.5 * dot(normal, light_dir) + 0.5;

    // color sampled from texture
    let uv = atlas_map_uv(input.texture_id, input.uv);
    color = textureSample(atlas_texture, default_sampler, uv);
    color = vec4f(color.rgb * brightness * light_color, 1);

    return color;
}

@vertex
fn mesh_wireframe_vertex(input: VertexInput) -> VertexOutputWireframe {
    let model_matrix = mat4x4f(input.model0, input.model1, input.model2, input.model3);
    let world_position = model_matrix * input.position;
    let position =  frame_uniform.camera.projection * frame_uniform.camera.view * world_position;

    return VertexOutputWireframe(
        position,
    );
}

@fragment
fn mesh_wireframe_fragment(input: VertexOutputWireframe) -> @location(0) vec4f {
    return vec4f(0, 0, 0, 1);
}


fn atlas_map_uv(texture_id: u32, uv: vec2f) -> vec2f {
    let entry = atlas_data[texture_id];
    return entry.uv_offset + (uv % vec2f(1)) * entry.uv_size;
}
