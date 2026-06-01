// Composite shader: renders a single candidate shape onto the canvas.
// Uses a fullscreen quad (6 vertices from vertex_index) with UV coordinates
// transformed by the candidate's position, rotation, and scale.

struct CandidateParams {
    shape_index: u32,
    x: f32,
    y: f32,
    rotation: f32,
    scale: f32,
    r: f32,
    g: f32,
    b: f32,
    alpha: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var shapes: texture_2d_array<f32>;
@group(0) @binding(1) var shape_sampler: sampler;
@group(0) @binding(2) var<uniform> candidate: CandidateParams;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Generate a fullscreen quad from 6 vertices (2 triangles)
    // Positions in clip space: covers [-1, 1] range
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );

    // UV coordinates: [0, 1] range
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Transform UV from canvas space back to shape space.
    // The fragment covers the entire canvas; we need to determine if this pixel
    // falls within the candidate's transformed shape region.

    // Get canvas dimensions from texture
    let canvas_size = vec2<f32>(textureDimensions(shapes).xy);

    // Convert fragment position to canvas pixel coordinates
    let pixel_pos = input.position.xy;

    // Candidate center in pixel coordinates
    let center = vec2<f32>(candidate.x, candidate.y);

    // Shape size in pixels (scale * shape_resolution)
    let shape_res = f32(textureDimensions(shapes).x);
    let shape_size = candidate.scale * shape_res;
    let half_size = shape_size * 0.5;

    // Vector from candidate center to this pixel
    let delta = pixel_pos - center;

    // Apply inverse rotation to get shape-local coordinates
    let cos_r = cos(-candidate.rotation);
    let sin_r = sin(-candidate.rotation);
    let local_x = delta.x * cos_r - delta.y * sin_r;
    let local_y = delta.x * sin_r + delta.y * cos_r;

    // Convert to UV in shape texture space [0, 1]
    let shape_uv = vec2<f32>(
        (local_x / shape_size) + 0.5,
        (local_y / shape_size) + 0.5,
    );

    // If outside the shape bounds, output fully transparent (no contribution)
    if (shape_uv.x < 0.0 || shape_uv.x > 1.0 || shape_uv.y < 0.0 || shape_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Sample from the shape texture array at the candidate's shape_index
    let shape_color = textureSample(shapes, shape_sampler, shape_uv, candidate.shape_index);

    // The shape texture is grayscale+alpha: use the alpha from the shape texture
    // multiplied by the candidate's alpha as the final alpha.
    // Tint with the candidate's color (r, g, b).
    let final_alpha = shape_color.a * candidate.alpha;

    // Output: candidate color tinted by shape luminance, with combined alpha
    let luminance = shape_color.r; // grayscale, so r=g=b
    let out_color = vec3<f32>(candidate.r, candidate.g, candidate.b) * luminance;

    return vec4<f32>(out_color, final_alpha);
}
