
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

const MAX_PLANETS: u32 = 2;

struct SkyboxData {
    model_matrix: mat4x4f,
    planets: array<PlanetData, MAX_PLANETS>,
}

struct PlanetData {
    model_matrix: mat4x4f,
    texture_id: u32,
    size: f32,
    // padding 8 bytes
}

@group(1)
@binding(0)
var<uniform> skybox_data: SkyboxData;

@group(1)
@binding(1)
var skybox_texture: texture_cube<f32>;



@vertex
fn skybox_vertex(@builtin(vertex_index) vertex_index: u32) -> SkyboxOutput {
    // screen filling triangle
    let position = vec4f(
        f32((vertex_index & 1) << 2) - 1,
        f32((vertex_index & 2) << 1) - 1,
        0.99999,
        1,
    );

    var unprojected = main_pass_uniform.camera.projection_inverse * position;
    var view_vector = main_pass_uniform.camera.view_inverse * vec4f(unprojected.xyz, 0);
    var texture_position = skybox_data.model_matrix * view_vector;

    return SkyboxOutput(
        position,
        texture_position.xyz,
    );
}

struct SkyboxOutput {
    @builtin(position)
    position: vec4f,

    @location(2)
    texture_position: vec3f,
}

@fragment
fn skybox_fragment(input: SkyboxOutput) -> @location(0) vec4f {
    let color = textureSample(skybox_texture, default_sampler, input.texture_position);
    return color;
}


const QUAD_VERTICES = array(
    vec2f(0, 0), vec2f(0, 1), vec2f(1, 0),
    vec2f(1, 1), vec2f(1, 0), vec2f(0, 1),
);

@vertex
fn planet_vertex(@builtin(vertex_index) vertex_index: u32) -> PlanetOutput {
    let planet = skybox_data.planets[vertex_index / 6];

    let uv = QUAD_VERTICES[vertex_index % 6];
    let vertex_offset = planet.size * (vec2f(1, -1) * uv + vec2f(-1, 1));

    //let far = (main_pass_uniform.camera.projection_inverse * vec4f(0, 0, 1, 1));

    var position = vec4f(0, 0, 1.0, 0);

    position = planet.model_matrix * position;

    // transform planet position to camera coordinate frame, but without translations
    position.w = 0;
    position = main_pass_uniform.camera.view * position;

    // add vertex offset so we're actually drawing the relevant vertex
    position += vec4f(vertex_offset, 0, 0);

    // apply camera projection for correct aspect ratio
    position = main_pass_uniform.camera.projection * position;

    // we only care about the screen position.
    // depth is used for depth testing
    // no perspective distortion
    position.x /= position.w;
    position.y /= position.w;
    position.z = 0.99999 * sign(position.w);
    position.w = 1;

    return PlanetOutput(position, uv, planet.texture_id);
}

struct PlanetOutput {
    @builtin(position)
    position: vec4f,

    @location(0)
    uv: vec2f,

    @location(1)
    @interpolate(flat, either)
    texture_id: u32,
}

@fragment
fn planet_fragment(input: PlanetOutput) -> @location(0) vec4f {
    let uv = atlas_map_uv(input.texture_id, input.uv);
    return textureSample(atlas_texture, default_sampler, uv);
}


fn atlas_map_uv(texture_id: u32, uv: vec2f) -> vec2f {
    let entry = atlas_data[texture_id];
    return entry.uv_offset + (uv % vec2f(1)) * entry.uv_size;
}
