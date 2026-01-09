
struct VertexInput {
    @location(0)
    position: vec4f,

    @location(1)
    normal: vec4f,

    @location(2)
    uv: vec2f
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
}

struct Camera {
    matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> camera: Camera;

@group(1)
@binding(0)
var texture_albedo: texture_2d<f32>;

@group(1)
@binding(1)
var sampler_albedo: sampler;

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let world_position = input.position;
    let normal = input.normal;

    let fragment_position = camera.matrix * world_position;

    return VertexOutput(
        fragment_position,
        world_position,
        normal,
        input.uv,
    );
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
    let normal = normalize(input.normal.xyz);
    let light_dir = vec3f(0, 0, -1);
    let attenuation = mix(0.5, 1.0, abs(dot(normal, light_dir)));

    var color = textureSample(texture_albedo, sampler_albedo, input.uv);
    return vec4f(color.rgb * attenuation, 1.0);
    //return vec4f(0.0, 1.0, 0.0, 1.0);
}
