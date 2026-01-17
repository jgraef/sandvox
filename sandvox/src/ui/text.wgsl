
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
}

struct FrameUniform {
    viewport_size: vec2u,
    // padding: 8 bytes
    matrix: mat4x4f,
}

@group(0)
@binding(0)
var<uniform> frame_uniform: FrameUniform;



struct FontData {
    num_glyphs: u32,
    // padding: 4 bytes
    atlas_size: vec2u,
    glyphs: array<FontGlyph>
}

struct FontGlyph {
    atlas_offset: vec2u,
    size: vec2u,
    offset: vec2u,
}

@group(1)
@binding(0)
var<storage, read> font_data: FontData;

@group(1)
@binding(1)
var font_texture: texture_2d<f32>;

@group(1)
@binding(2)
var font_sampler: sampler;


struct TextGlyph {
    offset: vec2f,
    glyph_id: u32,
    scaling: f32,
}

@group(2)
@binding(0)
var<storage, read> text_glyphs: array<TextGlyph>;


const quad_vertices = array(
    vec2f(0, 0),
    vec2f(0, 1),
    vec2f(1, 1),

    vec2f(0, 0),
    vec2f(1, 1),
    vec2f(1, 0),
);

@vertex
fn text_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let vertex = quad_vertices[vertex_index % 6];
    let text_glyph = text_glyphs[vertex_index / 6];
    let font_glyph = font_data.glyphs[text_glyph.glyph_id];

    // todo: add offset from ui layout
    let glyph_position = text_glyph.offset + text_glyph.scaling * (vec2f(font_glyph.offset) + vertex * vec2f(font_glyph.size));
    let clip_position = vec4f(vec2f(2, -2) * glyph_position / vec2f(frame_uniform.viewport_size) + vec2f(-1, 1), 0.0, 1.0);
    let glyph_uv = (vec2f(font_glyph.atlas_offset) + vertex * vec2f(font_glyph.size)) / vec2f(font_data.atlas_size);

    // todo
    let text_color = vec4f(0.0, 0.0, 0.0, 1.0);

    return VertexOutput(
        clip_position,
        text_color,
        glyph_uv,
    );
}

@fragment
fn text_fragment(input: VertexOutput) -> @location(0) vec4f {
    let pixel = textureSample(font_texture, font_sampler, input.uv).r;

    if pixel > 0.5 {
        return input.color;
    }
    else {
        discard;
    }
}
