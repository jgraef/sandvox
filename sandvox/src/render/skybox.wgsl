struct VertexOutput {
    @builtin(position)
    position: vec4f,

    @location(2)
    uv: vec3f,
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

@group(1)
@binding(0)
var skybox_texture: texture_cube<f32>;


@vertex
fn skybox_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // screen filling triangle
    let position = vec4f(
        f32((vertex_index & 1) << 2) - 1,
        f32((vertex_index & 2) << 1) - 1,
        1,
        1,
    );

    var unprojected = frame_uniform.camera.projection_inverse * position;
    //unprojected.z *= -1.0;

    // what!? why does it look right with the view matrix instead of its inverse???
    // well after looking at it for a bit both are a bit wonky in different ways.

    var uv = frame_uniform.camera.view_inverse * vec4f(unprojected.xyz, 0);

    return VertexOutput(
        position,
        uv.xyz,
    );
}

@fragment
fn skybox_fragment(input: VertexOutput) -> @location(0) vec4f {
    /*let v = normalize(input.uv);
    return vec4f(
        (0.5 * v.x + 0.5),
        (0.5 * v.y + 0.5),
        (0.5 * v.z + 0.5),
        1
    );*/

    let color = textureSample(skybox_texture, default_sampler, input.uv);
    return color;
}
