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
    scale_y: f32,
    use_original_color: f32,
    hue_shift: f32,
    saturation_scale: f32,
    brightness_scale: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var shapes: texture_2d_array<f32>;
@group(0) @binding(1) var shape_sampler: sampler;
@group(0) @binding(2) var<uniform> candidate: CandidateParams;

// Convert RGB (0..1) to HSV (h,s,v in 0..1). Branchless (IQ-style).
fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.bg, K.wz), vec4<f32>(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

// Convert HSV (0..1) back to RGB (0..1).
fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(vec3<f32>(c.x) + K.xyz) * 6.0 - vec3<f32>(K.w));
    return c.z * mix(vec3<f32>(K.x), clamp(p - vec3<f32>(K.x), vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

// Apply hue rotation + saturation scaling to an original shape color. Skips the
// HSV round-trip entirely when both are neutral (the common case), so unevolved
// shapes pay no extra cost.
fn evolve_original_color(rgb: vec3<f32>, hue_shift: f32, saturation_scale: f32, brightness_scale: f32) -> vec3<f32> {
    if (hue_shift == 0.0 && saturation_scale == 1.0 && brightness_scale == 1.0) {
        return rgb;
    }
    var hsv = rgb2hsv(rgb);
    hsv.x = fract(hsv.x + hue_shift);
    hsv.y = clamp(hsv.y * saturation_scale, 0.0, 1.0);
    hsv.z = clamp(hsv.z * brightness_scale, 0.0, 1.0);
    return hsv2rgb(hsv);
}

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

    // Shape size in pixels (scale * shape_resolution) per axis. scale_y lets the
    // shape stretch/squash independently along its local Y axis.
    let shape_res = f32(textureDimensions(shapes).x);
    let shape_size_x = candidate.scale * shape_res;
    let shape_size_y = candidate.scale_y * shape_res;

    // Vector from candidate center to this pixel
    let delta = pixel_pos - center;

    // Apply inverse rotation to get shape-local coordinates
    let cos_r = cos(-candidate.rotation);
    let sin_r = sin(-candidate.rotation);
    let local_x = delta.x * cos_r - delta.y * sin_r;
    let local_y = delta.x * sin_r + delta.y * cos_r;

    // Convert to UV in shape texture space [0, 1] using per-axis sizes
    let shape_uv = vec2<f32>(
        (local_x / shape_size_x) + 0.5,
        (local_y / shape_size_y) + 0.5,
    );

    // If outside the shape bounds, output fully transparent (no contribution)
    if (shape_uv.x < 0.0 || shape_uv.x > 1.0 || shape_uv.y < 0.0 || shape_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Sample from the shape texture array at the candidate's shape_index
    let shape_color = textureSample(shapes, shape_sampler, shape_uv, candidate.shape_index);

    // Final alpha = shape's alpha × candidate's opacity.
    let final_alpha = shape_color.a * candidate.alpha;

    // Color: either keep the shape's ORIGINAL texture color (use_original_color)
    // or tint the candidate's (r, g, b) by the grayscale luminance (classic mode).
    var out_color: vec3<f32>;
    if (candidate.use_original_color > 0.5) {
        out_color = evolve_original_color(shape_color.rgb, candidate.hue_shift, candidate.saturation_scale, candidate.brightness_scale);
    } else {
        let luminance = shape_color.r; // grayscale, so r=g=b
        out_color = vec3<f32>(candidate.r, candidate.g, candidate.b) * luminance;
    }

    return vec4<f32>(out_color, final_alpha);
}
