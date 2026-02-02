const PI: f32 = 3.141592653589793;

struct MainPassUniform {
    camera: Camera,
    time: f32,
    // padding: 12 bytes
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
var<uniform> main_pass_uniform: MainPassUniform;

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



struct Vertex {
    position: vec4f,
    normal: vec4f,
    uv: vec2f,
    texture_id: u32,
    // padding: 4 bytes
}

struct Instance {
    model_matrix: mat4x4f,
}

@group(1)
@binding(0)
var<storage, read> instance_buffer: array<Instance>;

@group(2)
@binding(0)
var<storage, read> vertex_buffer: array<Vertex>;

@group(2)
@binding(1)
var<storage, read> index_buffer: array<u32>;


@vertex
fn mesh_shaded_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> ShadedOutput {
    let resolved_vertex_index = index_buffer[vertex_index];
    let vertex = vertex_buffer[resolved_vertex_index];
    let instance = instance_buffer[instance_index];

    let world_position = instance.model_matrix * vertex.position;
    let normal = instance.model_matrix * vertex.normal;

    let position = main_pass_uniform.camera.projection * main_pass_uniform.camera.view * world_position;

    return ShadedOutput(
        position,
        world_position,
        normal,
        vertex.uv,
        vertex.texture_id,
    );
}

struct ShadedOutput {
    @builtin(position)
    @invariant
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


@fragment
fn mesh_shaded_fragment(input: ShadedOutput) -> @location(0) vec4f {
    var color: vec4f;

    // todo: figure out where the sun is (https://iurietarlev.github.io/SunpathDiagram/),
    // but move this out of shader and send light direction/color via frame uniform
    //let sun_phase = main_pass_uniform.time / 60.0 * 2.0 * PI;
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


struct WireframeOutput {
    @builtin(position)
    @invariant
    position: vec4f,
}

@vertex
fn mesh_wireframe_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> WireframeOutput {
    /*
        0----2
        |   /
        | /
        1

        shader will be called with vertex_index = [0, 1, 2, 3, 4, 5] (2 * number of vertices)

        vertex_index | draw vertex
                   0 | 0
                   1 | 1
                   2 | 1
                   3 | 2
                   4 | 2
                   5 | 0

        `(i + 1) % 6 / 2` gives the vertex indices for lines of a single triangle.
        `(i / 6) * 3` gives the vertex index for the first index of a triangle.
    */

    var line_vertex_index = ((vertex_index + 1) % 6) / 2 + (vertex_index / 6) * 3;
    let resolved_vertex_index = index_buffer[line_vertex_index];
    let vertex = vertex_buffer[resolved_vertex_index];
    let instance = instance_buffer[instance_index];

    let world_position = instance.model_matrix * vertex.position;
    let position = main_pass_uniform.camera.projection * main_pass_uniform.camera.view * world_position;

    return WireframeOutput(
        position,
    );
}

@fragment
fn mesh_wireframe_fragment(input: WireframeOutput) -> @location(0) vec4f {
    const plum: vec4f = vec4f(0.86, 0.62, 0.86, 1);
    return plum;
}




@vertex
fn mesh_depth_prepass_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> DepthPrepassOutput {
    let resolved_vertex_index = index_buffer[vertex_index];
    let vertex = vertex_buffer[resolved_vertex_index];
    let instance = instance_buffer[instance_index];

    let world_position = instance.model_matrix * vertex.position;
    let position = main_pass_uniform.camera.projection * main_pass_uniform.camera.view * world_position;

    return DepthPrepassOutput(
        position,
    );
}

struct DepthPrepassOutput {
    @builtin(position)
    @invariant
    position: vec4f,
}





fn atlas_map_uv(texture_id: u32, uv: vec2f) -> vec2f {
    let entry = atlas_data[texture_id];
    return entry.uv_offset + (uv % vec2f(1)) * entry.uv_size;
}
