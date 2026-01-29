
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

@group(1)
@binding(0)
var skybox_texture: texture_cube<f32>;

struct SkyboxData {
    transform: mat4x4f,
}

@group(1)
@binding(1)
var<uniform> skybox_data: SkyboxData;


@vertex
fn skybox_vertex(@builtin(vertex_index) vertex_index: u32) -> SkyboxOutput {
    // screen filling triangle
    let position = vec4f(
        f32((vertex_index & 1) << 2) - 1,
        f32((vertex_index & 2) << 1) - 1,
        0.99999,
        1,
    );

    var unprojected = frame_uniform.camera.projection_inverse * position;
    var view_vector = frame_uniform.camera.view_inverse * vec4f(unprojected.xyz, 0);
    var texture_position = skybox_data.transform * view_vector;

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
