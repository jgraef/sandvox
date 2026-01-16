
struct VertexInput {
    @location(0)
    position: vec4f,

    @location(1)
    color: vec4f,

    @location(2)
    uv: vec2f,

    @location(3)
    glyph_id: u32,
}

struct VertexOutput {
    @builtin(position)
    fragment_position: vec4f,

    @location(1)
    color: vec4f,

    @location(2)
    @interpolate(linear, center)
    uv: vec2f,

    @location(3)
    @interpolate(flat, either)
    glyph_id: u32,
}

struct FontData {
    num_glyphs: u32,
    // padding: 4 bytes
    atlas_size: vec2u,
    glyphs: array<Glyph>
}

struct Glyph {
    atlas_offset: vec2u,
    size: vec2u,
    offset: vec2u,
}

struct FrameUniform {
    viewport_size: vec2u,
    // padding: 8 bytes
    matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> frame_uniform: FrameUniform;

@group(1)
@binding(0)
var<storage, read> font_data: FontData;

@group(1)
@binding(1)
var font_texture: texture_2d<f32>;

@group(1)
@binding(2)
var font_sampler: sampler;


@vertex
fn text_vertex(input: VertexInput) -> VertexOutput {
    let fragment_position = vec4f(vec2f(2, -2) * input.position.xy / vec2f(frame_uniform.viewport_size) + vec2f(-1, 1), 0.0, 1.0);

    return VertexOutput(
        fragment_position,
        input.color,
        input.uv,
        input.glyph_id,
    );
}

@fragment
fn text_fragment(input: VertexOutput) -> @location(0) vec4f {
    let uv = font_map_uv(input.glyph_id, input.uv);

    let pixel = textureSample(font_texture, font_sampler, uv).r;

    var color: vec4f;
    if pixel > 0.5 {
        color = input.color;
    }
    else {
        discard;
    }

    return color;

    //return input.color;
}

fn font_map_uv(glyph_id: u32, uv: vec2f) -> vec2f {
    let glyph = font_data.glyphs[glyph_id];

    return (vec2f(glyph.size) * clamp(uv, vec2f(0), vec2f(1)) + vec2f(glyph.atlas_offset)) / vec2f(font_data.atlas_size);
}
