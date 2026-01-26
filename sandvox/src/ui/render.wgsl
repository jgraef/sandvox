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

@group(0)
@binding(4)
var font_texture: texture_2d<f32>;

@group(0)
@binding(5)
var<storage, read> font_data: FontData;


struct Quad {
    position: vec2f,
    size: vec2f,
    texture_id: u32,
    depth: f32,
    // padding 8 bytes
    tint: vec4f,
}

@group(1)
@binding(0)
var<storage, read> quad_buffer: array<Quad>;



const DEBUG_VERTICES = array(
    vec2f(0, 0), vec2f(0, 1),
    vec2f(0, 1), vec2f(1, 1),
    vec2f(1, 1), vec2f(1, 0),
    vec2f(1, 0), vec2f(0, 0),
);
const DEBUG_COLOR: vec4f = vec4f(1, 0, 0, 1);

@vertex
fn debug_vertex(@builtin(vertex_index) vertex_index: u32) -> DebugVertexOutput {
    let quad = quad_buffer[vertex_index / 8];
    let vertex = DEBUG_VERTICES[vertex_index % 8];

    let position = vertex * quad.size + quad.position;
    let clip_position = vec4f(vec2f(2, -2) * position / vec2f(frame_uniform.viewport_size) + vec2f(-1, 1), quad.depth, 1.0);

    return DebugVertexOutput(
        clip_position,
    );
}

struct DebugVertexOutput {
    @builtin(position)
    position: vec4f,
}

@fragment
fn debug_fragment(input: DebugVertexOutput) -> @location(0) vec4f {
    return DEBUG_COLOR;
}



const QUAD_VERTICES = array(
    vec2f(0, 0), vec2f(1, 0), vec2f(0, 1),
    vec2f(1, 1), vec2f(1, 0), vec2f(0, 1),
);

@vertex
fn quad_vertex(@builtin(vertex_index) vertex_index: u32) -> QuadVertexOutput {
    var output: QuadVertexOutput;

    let quad = quad_buffer[vertex_index / 6];

    if quad.texture_id == 0xffffffff {
        // anything with u32::MAX as texture ID will not be displayed
        // note: we could of course just not put these into the buffer, but we might want to draw them in a different way (e.g. debug outlines)

        output.position = vec4f(1, 1, 1, 0); // will be clipped
    }
    else {
        let vertex = QUAD_VERTICES[vertex_index % 6];
        let position = vertex * quad.size + quad.position;
        output.position = vec4f(vec2f(2, -2) * position / vec2f(frame_uniform.viewport_size) + vec2f(-1, 1), quad.depth, 1.0);
        output.uv = vertex;
        output.texture_id = quad.texture_id;
        output.tint = quad.tint;
    }

    return output;
}

struct QuadVertexOutput {
    @builtin(position)
    position: vec4f,

    @location(0)
    uv: vec2f,

    @location(1)
    @interpolate(flat, either)
    texture_id: u32,

    @location(2)
    tint: vec4f,
}

@fragment
fn quad_fragment(input: QuadVertexOutput) -> @location(0) vec4f {
    const GLYPH_BIT: u32 = 0x80000000;

    if (input.texture_id & GLYPH_BIT) == 0 {
        // atlas texture

        let atlas_id = input.texture_id;
        let uv = atlas_map_uv(atlas_id, input.uv);

        let color = textureSample(atlas_texture, default_sampler, uv);

        if color.a < 0.1 {
            discard;
        }

        return color;
    }
    else {
        // font glyph

        let glyph_id = input.texture_id & (~GLYPH_BIT);
        let uv = glyph_map_uv(glyph_id, input.uv);

        let luma = textureSample(font_texture, default_sampler, uv).r;

        if luma < 0.5 {
            discard;
        }

        return input.tint;
    }
}

fn atlas_map_uv(atlas_id: u32, uv: vec2f) -> vec2f {
    let entry = atlas_data[atlas_id];
    return entry.uv_offset + (uv % vec2f(1)) * entry.uv_size;
}

fn glyph_map_uv(glyph_id: u32, uv: vec2f) -> vec2f {
    let glyph = font_data.glyphs[glyph_id];
    return (vec2f(glyph.atlas_offset) + uv * vec2f(glyph.size)) / vec2f(font_data.atlas_size);
}



@vertex
fn clear_depth_vertex(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4f {
    return vec4f(
        f32((vertex_index & 1) << 2) - 1,
        f32((vertex_index & 2) << 1) - 1,
        1,
        1,
    );
}

@fragment
fn clear_depth_fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return vec4f(0, 0, 0, 0);
}
